use super::archive::{self, CurseFile, IndexFile, Pack, add_size, invalid};
use hydrogen_plugin_services::{
    contracts::{ApiError, Result},
    curseforge::{self, CurseFile as CurseMetadata},
    jobs::Job,
    net::{self, Client},
};
use reqwest::Url;
use serde::Deserialize;
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use sha2::Sha512;
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
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

struct Download {
    path: String,
    urls: Vec<String>,
    sha1: Option<String>,
    sha512: Option<String>,
    fingerprint: Option<u32>,
    size: u64,
}
impl Download {
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
            let result = match result.await {
                Ok(path) => path,
                Err(error) => {
                    last_error = error;
                    continue;
                }
            };
            let valid = if let Some(expected) = &self.sha512 {
                checksums(&result, false, job)
                    .await?
                    .sha512
                    .eq_ignore_ascii_case(expected)
            } else if let Some(expected) = self.fingerprint {
                curseforge::fingerprint(&result, job).await? == expected
            } else {
                true
            };
            if valid {
                return Ok(());
            }
            std::fs::remove_file(result)?;
            last_error = ApiError::new(
                "HASH_MISMATCH",
                format!("整合包文件校验失败（{}）。", self.path),
            );
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
    let api_key = api_key.ok_or_else(curseforge::missing_key)?;
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
            let sha1 = file.sha1().map(str::to_owned);
            result.push(Download {
                path: format!("{directory}/{}", file.file_name),
                urls: vec![file.url()?.to_owned()],
                fingerprint: sha1.is_none().then_some(file.file_fingerprint),
                sha1,
                sha512: None,
                size: file.file_length,
            });
        }
    }
    Ok(result)
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
            sha1: file.hashes.get("sha1").cloned(),
            sha512: file.hashes.get("sha512").cloned(),
            fingerprint: None,
            size: file.file_size,
        })
        .collect();
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
    job.phase("downloading", "正在下载并校验整合包文件…");
    job.begin_download(files.iter().map(|f| (f.path.clone(), Some(f.size))), true);
    for file in &files {
        if overrides.contains_key(&file.path.to_lowercase()) {
            job.file_progress(&file.path, file.size, true);
        } else {
            file.install(client, &stage, downloads_enabled, job).await?;
        }
    }
    job.phase("extracting", "正在解包整合包配置…");
    for file in overrides.values() {
        file.extract(&mut zip, &stage, job).await?;
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
        let hashes: Vec<_> = batch.iter().map(|(_, sum)| &sum.sha512).collect();
        let response = client
            .post(format!("{MODRINTH}/version_files"))
            .json(&json!({"algorithm":"sha512","hashes":hashes}))
            .send()
            .await?;
        let response: Value =
            serde_json::from_slice(&net::response_bytes(response, 32 * 1024 * 1024).await?)?;
        for (path, sum) in batch {
            if let Some(url) = response[&sum.sha512]["files"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|file| file["hashes"]["sha512"] == sum.sha512 && file["size"] == sum.size)
                .and_then(|file| file["url"].as_str())
                .filter(|url| modrinth_url(url))
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
    use super::import_url;

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
}
