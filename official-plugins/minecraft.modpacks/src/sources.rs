use super::archive::{self, CurseFile, IndexFile, Pack, add_size, invalid};
use hydrogen_plugin_services::{
    contracts::{ApiError, Result},
    curseforge::{self, CurseFile as CurseMetadata},
    jobs::{Job, service_task},
    net::{self, Client},
    storage,
};
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use sha2::Sha512;
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};

const MODRINTH: &str = "https://api.modrinth.com/v2";

fn url_allowed(value: &str, hosts: &[&str]) -> bool {
    Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.port().is_none_or(|p| p == 443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.host_str().is_some_and(|host| hosts.contains(&host))
    })
}
pub fn modrinth_url(value: &str) -> bool {
    url_allowed(
        value,
        &[
            "cdn.modrinth.com",
            "github.com",
            "raw.githubusercontent.com",
            "gitlab.com",
        ],
    )
}
fn import_url(value: &str) -> bool {
    modrinth_url(value) || curseforge::download_url(value)
}

fn curseforge_cdn_urls(id: u32, name: &str) -> [String; 2] {
    let mut url = Url::parse(&format!(
        "https://mediafilez.forgecdn.net/files/{}/{}",
        id / 1000,
        id % 1000
    ))
    .expect("valid CurseForge CDN URL");
    url.path_segments_mut()
        .expect("hierarchical CurseForge CDN URL")
        .push(name);
    let path = url.path().replace('+', "%2B").replace('\'', "%27");
    ["mediafilez.forgecdn.net", "edge.forgecdn.net"].map(|host| format!("https://{host}{path}"))
}

struct Download {
    path: String,
    urls: Vec<String>,
    curse_file_id: Option<u32>,
    sha1: Option<String>,
    sha512: Option<String>,
    fingerprint: Option<u32>,
    size: u64,
}
impl Download {
    fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    async fn stage_local(&self, directory: &Path, stage: &Path, job: &Job) -> Result<bool> {
        let path = storage::within(directory, self.name())?;
        if !path.is_file() {
            return Ok(false);
        }
        let mut input = File::open(path)?;
        if input.metadata()?.len() != self.size {
            return Ok(false);
        }
        let target = storage::within(stage, &self.path)?;
        fs::create_dir_all(target.parent().unwrap())?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        let size = archive::copy(&mut input, &mut output, self.size, job).await?;
        drop(output);
        let valid = size == self.size
            && if let Some(sha1) = &self.sha1 {
                storage::hash(&target).await?.eq_ignore_ascii_case(sha1)
            } else if let Some(fingerprint) = self.fingerprint {
                curseforge::fingerprint(&target, job).await? == fingerprint
            } else {
                false
            };
        if !valid {
            fs::remove_file(target)?;
        }
        Ok(valid)
    }

    async fn verified(&self, path: &Path, job: &Job) -> Result<bool> {
        if let Some(expected) = &self.sha512 {
            Ok(checksums(path, false, job)
                .await?
                .sha512
                .eq_ignore_ascii_case(expected))
        } else if let Some(expected) = self.fingerprint {
            Ok(curseforge::fingerprint(path, job).await? == expected)
        } else {
            Ok(true)
        }
    }

    async fn install(&self, client: &Client, stage: &Path, enabled: bool, job: &Job) -> Result<()> {
        let mut last_error = invalid(format!("整合包文件没有可用的下载地址（{}）。", self.path));
        for url in &self.urls {
            job.check()?;
            let result = net::ensure_game_file(
                client,
                stage,
                &self.path,
                url,
                self.sha1.as_deref(),
                Some(self.size),
                false,
                enabled,
                job,
            );
            // Keep each verified download in the temporary instance directory.
            let mut result = match result.await {
                Ok(path) => path,
                Err(error) => {
                    last_error = error;
                    continue;
                }
            };
            if !self.verified(&result, job).await? {
                fs::remove_file(&result)?;
                // SHA-512 and CurseForge fingerprints are verified here, after transfer.
                let origin = service_task(
                    "minecraft.download", "once",
                    json!({"root":stage,"relative":self.path,"url":url,"hash":self.sha1,"size":self.size}),
                    job,
                ).await;
                result = match origin {
                    Ok(path) => serde_json::from_value(path)?,
                    Err(error) => {
                        last_error = error;
                        continue;
                    }
                };
            }
            if self.verified(&result, job).await? {
                return Ok(());
            }
            std::fs::remove_file(result)?;
            last_error = ApiError::new(
                "HASH_MISMATCH",
                format!("整合包文件校验失败（{}）。", self.path),
            );
        }
        if self.curse_file_id.is_some() && last_error.code != "CANCELLED" {
            last_error.message.push_str(&format!(
                " 可将从 CurseForge 官网下载的 {} 放到整合包 ZIP 同一目录后重试安装。",
                self.name()
            ));
        }
        Err(last_error)
    }
}

async fn curse_downloads(
    client: &Client,
    files: &[CurseFile],
    optional: bool,
    api_key: Option<&str>,
) -> Result<Vec<Download>> {
    let wanted: Vec<_> = files.iter().filter(|f| f.required || optional).collect();
    if wanted.is_empty() {
        return Ok(vec![]);
    }
    let api_key = api_key.unwrap_or_default();
    let mut projects: Vec<_> = wanted.iter().map(|f| f.project).collect();
    projects.sort_unstable();
    projects.dedup();
    let mut classes = HashMap::new();
    for batch in projects.chunks(100) {
        let response = curseforge::fetch(
            client
                .post(curseforge::endpoint("mods"))
                .json(&json!({"modIds":batch})),
            api_key,
        )
        .await?;
        for project in response["data"]
            .as_array()
            .ok_or_else(|| invalid("CurseForge 项目信息无效。"))?
        {
            if project["gameId"] == 432
                && let (Some(id), Some(class)) =
                    (project["id"].as_u64(), project["classId"].as_u64())
            {
                classes.insert(id, class);
            }
        }
    }
    let mut result = Vec::with_capacity(wanted.len());
    for batch in wanted.chunks(100) {
        let ids: Vec<_> = batch.iter().map(|f| f.file).collect();
        let response = curseforge::fetch(
            client
                .post(curseforge::endpoint("mods/files"))
                .json(&json!({"fileIds":ids})),
            api_key,
        )
        .await?;
        let files: Vec<CurseMetadata> = serde_json::from_value(response["data"].clone())?;
        let mut files: HashMap<_, _> = files.into_iter().map(|f| (f.id, f)).collect();
        for spec in batch {
            let file = files
                .remove(&spec.file)
                .filter(|f| f.mod_id == spec.project && f.game_id == 432)
                .ok_or_else(|| {
                    invalid(format!(
                        "找不到 CurseForge 文件（{} / {}）。",
                        spec.project, spec.file
                    ))
                })?;
            let directory = match classes.get(&u64::from(spec.project)) {
                Some(6) => "mods",
                Some(12) => "resourcepacks",
                Some(6552) => "shaderpacks",
                Some(6945) => "datapacks",
                _ => {
                    return Err(invalid(format!(
                        "暂不支持此 CurseForge 项目类型（{}）。",
                        spec.project
                    )));
                }
            };
            if file.file_name.contains('/') || !archive::portable_path(&file.file_name) {
                return Err(invalid("CurseForge 文件名无效。"));
            }
            let sha1 = file.sha1().map(str::to_ascii_lowercase);
            let url = file.url().ok();
            result.push(Download {
                path: format!("{directory}/{}", file.file_name),
                urls: url.map(str::to_owned).into_iter().collect(),
                curse_file_id: url.is_none().then_some(file.id),
                fingerprint: sha1.is_none().then_some(file.file_fingerprint),
                sha1,
                sha512: None,
                size: file.file_length,
            });
        }
    }
    Ok(result)
}

async fn modrinth_versions(client: &Client, algorithm: &str, hashes: Vec<&str>) -> Result<Value> {
    let response = client
        .post(format!("{MODRINTH}/version_files"))
        .json(&json!({"algorithm":algorithm,"hashes":hashes}))
        .send()
        .await?;
    Ok(serde_json::from_slice(
        &net::response_bytes(response, 32 * 1024 * 1024).await?,
    )?)
}

fn modrinth_file_url<'a>(
    version: &'a Value,
    algorithm: &str,
    hash: &str,
    size: u64,
) -> Option<&'a str> {
    version["files"].as_array()?.iter().find_map(|file| {
        (file["hashes"][algorithm] == hash && file["size"] == size)
            .then(|| file["url"].as_str().filter(|url| modrinth_url(url)))
            .flatten()
    })
}

async fn resolve_curse_downloads(
    client: &Client,
    files: &mut [Download],
    overrides: &BTreeMap<String, archive::Override>,
    directory: &Path,
    stage: &Path,
    job: &Job,
) -> Result<()> {
    let mut missing = Vec::new();
    for file in files {
        job.check()?;
        if file.urls.is_empty()
            && !overrides.contains_key(&file.path.to_lowercase())
            && !file.stage_local(directory, stage, job).await?
        {
            missing.push(file);
        }
    }
    for batch in missing.chunks_mut(100) {
        job.check()?;
        let hashes = batch
            .iter()
            .filter_map(|file| file.sha1.as_deref())
            .collect::<Vec<_>>();
        if hashes.is_empty() {
            continue;
        }
        let Ok(versions) = modrinth_versions(client, "sha1", hashes).await else {
            break;
        };
        for file in batch {
            if let Some(hash) = file.sha1.as_deref()
                && let Some(url) = modrinth_file_url(&versions[hash], "sha1", hash, file.size)
            {
                file.urls.push(url.to_owned());
            }
        }
    }
    job.check()?;
    for file in &mut missing {
        if let Some(id) = file.curse_file_id
            && file.sha1.is_some()
        {
            let urls = curseforge_cdn_urls(id, file.name());
            file.urls.extend(urls);
        }
    }
    let missing: Vec<_> = missing.iter().filter(|file| file.urls.is_empty()).collect();
    if !missing.is_empty() {
        let names = missing
            .iter()
            .take(10)
            .map(|file| file.name())
            .collect::<Vec<_>>()
            .join("、");
        return Err(ApiError::new(
            "CURSEFORGE_DOWNLOAD",
            format!(
                "有 {} 个 CurseForge 文件无法自动下载。请从 CurseForge 官网下载所列文件，保留原文件名，放到整合包 ZIP 同一目录后重试安装。缺少或校验未通过的文件包括 {names}。",
                missing.len()
            ),
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prepare {
    source: PathBuf,
    stage: PathBuf,
    directory: String,
    optional: bool,
    downloads_enabled: bool,
    preview: Value,
}

pub async fn prepare(client: &Client, options: Prepare, job: &Job) -> Result<Value> {
    let Prepare {
        source,
        stage,
        directory,
        optional,
        downloads_enabled,
        preview,
    } = options;
    if !archive::portable_path(&directory) || directory.contains('/') {
        return Err(invalid("实例名称包含无效路径字符。"));
    }
    let (mut zip, pack) = Pack::open(&source)?;
    if serde_json::to_value(pack.preview()?)? != preview {
        return Err(invalid("整合包清单已更改，请重新选择文件。"));
    }
    let overrides = pack.overrides(&mut zip, &directory)?;
    let mut files: Vec<_> = pack
        .downloads(optional)
        .map(|file| Download {
            path: file.path.clone(),
            urls: file
                .downloads
                .iter()
                .filter(|url| import_url(url))
                .cloned()
                .collect(),
            curse_file_id: None,
            sha1: file.hashes.get("sha1").cloned(),
            sha512: file.hashes.get("sha512").cloned(),
            fingerprint: None,
            size: file.file_size,
        })
        .collect();
    let curseforge_pack = matches!(&pack, Pack::Curseforge(_));
    if let Pack::Curseforge(manifest) = &pack {
        files = curse_downloads(
            client,
            &manifest.files,
            optional,
            curseforge::api_key().as_deref(),
        )
        .await?;
    }
    let mut total = 0;
    let mut seen = BTreeMap::new();
    for file in &files {
        if !archive::content_path(&file.path, &directory)
            || seen
                .insert(file.path.to_lowercase(), file.path.as_str())
                .is_some()
        {
            return Err(invalid(format!(
                "整合包下载路径无效或冲突（{}）。",
                file.path
            )));
        }
        add_size(&mut total, file.size)?;
    }
    for (key, file) in &overrides {
        if seen.get(key).is_some_and(|path| *path != file.path) {
            return Err(invalid(format!("整合包文件路径冲突（{}）。", file.path)));
        }
        add_size(&mut total, file.size)?;
    }
    if curseforge_pack {
        let directory = source
            .parent()
            .ok_or_else(|| invalid("无法确定整合包所在目录。"))?;
        resolve_curse_downloads(client, &mut files, &overrides, directory, &stage, job).await?;
    }
    job.phase("downloading", "正在下载并校验整合包文件…");
    job.begin_download(files.iter().map(|f| (f.path.clone(), Some(f.size))), true);
    for file in &files {
        // In a CurseForge pack, unresolved URLs have already been supplied locally or by overrides.
        if overrides.contains_key(&file.path.to_lowercase())
            || curseforge_pack && file.urls.is_empty()
        {
            job.file_progress(&file.path, file.size, true);
        } else {
            file.install(client, &stage, downloads_enabled, job).await?;
        }
    }
    job.phase("extracting", "正在解包整合包配置…");
    for file in overrides.values() {
        file.extract(&mut zip, &stage, job).await?;
    }
    // Root artwork is outside the override layers and cannot overwrite pack content.
    for icon in pack.icons(&mut zip)? {
        if icon.extract(&mut zip, &stage, job).await.is_err() {
            job.check()?;
            // Optional artwork must not prevent an otherwise valid pack from installing.
        }
    }
    Ok(Value::Null)
}

pub struct Checksums {
    pub sha1: String,
    pub sha512: String,
    pub fingerprint: Option<u32>,
    pub size: u64,
}

pub async fn checksums(path: &Path, fingerprint: bool, job: &Job) -> Result<Checksums> {
    let mut input = File::open(path)?;
    let mut buffer = vec![0; 128 * 1024];
    let (mut sha1, mut sha512) = (Sha1::new(), Sha512::new());
    let mut size = 0;
    loop {
        job.check()?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        add_size(&mut size, count as u64)?;
        if size > archive::MAX_FILE {
            return Err(invalid("整合包单个文件超过大小限制。"));
        }
        let bytes = &buffer[..count];
        sha1.update(bytes);
        sha512.update(bytes);
        tokio::task::yield_now().await;
    }
    let fingerprint = if fingerprint {
        Some(curseforge::fingerprint(path, job).await?)
    } else {
        None
    };
    Ok(Checksums {
        sha1: format!("{:x}", sha1.finalize()),
        sha512: format!("{:x}", sha512.finalize()),
        fingerprint,
        size,
    })
}

pub async fn modrinth_matches(
    client: &Client,
    candidates: &BTreeMap<String, Checksums>,
) -> Result<Vec<IndexFile>> {
    let items: Vec<_> = candidates.iter().collect();
    let mut result = Vec::new();
    for batch in items.chunks(100) {
        let hashes = batch.iter().map(|(_, sum)| sum.sha512.as_str()).collect();
        let response = modrinth_versions(client, "sha512", hashes).await?;
        for (path, sum) in batch {
            if let Some(url) =
                modrinth_file_url(&response[&sum.sha512], "sha512", &sum.sha512, sum.size)
            {
                result.push(IndexFile {
                    path: (*path).clone(),
                    hashes: BTreeMap::from([
                        ("sha1".into(), sum.sha1.clone()),
                        ("sha512".into(), sum.sha512.clone()),
                    ]),
                    downloads: vec![url.into()],
                    file_size: sum.size,
                    env: None,
                });
            }
        }
    }
    Ok(result)
}

pub async fn curse_matches(
    client: &Client,
    candidates: &BTreeMap<String, Checksums>,
    api_key: &str,
) -> Result<BTreeMap<String, CurseFile>> {
    let items: Vec<_> = candidates.iter().collect();
    let mut result = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for batch in items.chunks(100) {
        let fingerprints: Vec<_> = batch
            .iter()
            .filter_map(|(_, sum)| sum.fingerprint)
            .collect();
        let mut matches = curseforge::exact_matches(client, api_key, &fingerprints).await?;
        matches.sort_by_key(|file| file.id);
        for (path, sum) in batch {
            if let Some(file) = matches.iter().find(|file| {
                file.game_id == 432
                    && Some(file.file_fingerprint) == sum.fingerprint
                    && file.file_length == sum.size
                    && file
                        .sha1()
                        .is_some_and(|sha1| sha1.eq_ignore_ascii_case(&sum.sha1))
                    && path.rsplit('/').next() == Some(&file.file_name)
                    && !seen.contains(&file.id)
            }) {
                seen.insert(file.id);
                result.insert(
                    (*path).clone(),
                    CurseFile {
                        project: file.mod_id,
                        file: file.id,
                        required: true,
                    },
                );
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{Download, Job, curseforge_cdn_urls, fs, import_url, modrinth_file_url};
    use serde_json::json;

    #[test]
    fn modrinth_import_accepts_curseforge_cdn_downloads() {
        for host in ["mediafilez.forgecdn.net", "edge.forgecdn.net"] {
            assert!(import_url(&format!(
                "https://{host}/files/6665/986/mod.jar"
            )));
        }
        assert!(import_url("https://cdn.modrinth.com/data/mod.jar"));
        assert!(!import_url("https://edge.forgecdn.net.example.com/mod.jar"));
        assert!(!import_url("http://edge.forgecdn.net/mod.jar"));
        assert!(!import_url("https://edge.forgecdn.net:8443/mod.jar"));
    }

    #[test]
    fn restricted_curseforge_file_requires_an_exact_copy() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let stage = root.path().join("stage");
        let file = Download {
            path: "mods/example.jar".into(),
            urls: vec![],
            curse_file_id: Some(1234008),
            sha1: Some("a9993e364706816aba3e25717850c26c9cd0d89d".into()),
            sha512: None,
            fingerprint: None,
            size: 3,
        };
        let job = Job::default();
        let source = root.path().join("example.jar");
        let target = stage.join(&file.path);
        runtime.block_on(async {
            assert!(!file.stage_local(root.path(), &stage, &job).await.unwrap());
            fs::create_dir(&stage).unwrap();
            fs::write(&source, b"bad").unwrap();
            assert!(!file.stage_local(root.path(), &stage, &job).await.unwrap());
            assert!(!target.exists());
            fs::write(&source, b"abc").unwrap();
            assert!(file.stage_local(root.path(), &stage, &job).await.unwrap());
            assert_eq!(fs::read(target).unwrap(), b"abc");
        });
    }

    #[test]
    fn curseforge_cdn_urls_use_the_exact_file_id_and_encoded_name() {
        assert_eq!(
            curseforge_cdn_urls(123008, "Author's Example + 模组.jar"),
            [
                "https://mediafilez.forgecdn.net/files/123/8/Author%27s%20Example%20%2B%20%E6%A8%A1%E7%BB%84.jar",
                "https://edge.forgecdn.net/files/123/8/Author%27s%20Example%20%2B%20%E6%A8%A1%E7%BB%84.jar"
            ]
        );
    }

    #[test]
    fn modrinth_fallback_requires_a_matching_hash_size_and_host() {
        let cdn = "https://cdn.modrinth.com/data/example.jar";
        let version = json!({"files":[
            {"hashes":{"sha1":"other"},"size":3,"url":cdn},
            {"hashes":{"sha1":"wanted"},"size":2,"url":cdn},
            {"hashes":{"sha1":"wanted"},"size":3,"url":"https://example.com/example.jar"},
            {"hashes":{"sha1":"wanted"},"size":4,"url":cdn}
        ]});
        assert_eq!(modrinth_file_url(&version, "sha1", "wanted", 3), None);
        assert_eq!(modrinth_file_url(&version, "sha1", "wanted", 4), Some(cdn));
    }
}
