use super::{PAGE_SIZE, Project, Target, invalid};
use hydrogen_plugin_services::{
    contracts::Result,
    curseforge::{self as api, CurseFile},
    http::Client,
    jobs::{Job, service_task},
    net, storage, text,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

const GAME_ID: u32 = 432;
const MOD_CLASS: u32 = 6;

fn loader_type(loader: &str) -> Option<&'static str> {
    match loader {
        "forge" => Some("1"),
        "fabric" => Some("4"),
        "quilt" => Some("5"),
        "neoforge" => Some("6"),
        _ => None,
    }
}

fn id(value: &str) -> Result<u32> {
    value
        .parse::<u32>()
        .ok()
        .filter(|&id| id > 0)
        .ok_or_else(|| invalid("CurseForge 标识无效。"))
}

async fn get(client: &Client, key: &str, path: &str) -> Result<Value> {
    api::fetch(client.get(api::endpoint(path)), key).await
}

fn project(value: &Value) -> Option<Project> {
    if value["gameId"] != GAME_ID || value["classId"] != MOD_CLASS {
        return None;
    }
    let categories: Vec<_> = value["categories"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|category| category["id"].as_u64().map(|id| id.to_string()))
        .collect();
    let indexes = value["latestFilesIndexes"].as_array().into_iter().flatten();
    let mut versions = BTreeSet::new();
    let mut tags: BTreeSet<_> = categories.iter().cloned().collect();
    for index in indexes {
        if let Some(version) = index["gameVersion"].as_str() {
            versions.insert(version.to_owned());
        }
        let loader = match index["modLoader"].as_u64() {
            Some(1) => "forge",
            Some(4) => "fabric",
            Some(5) => "quilt",
            Some(6) => "neoforge",
            _ => continue,
        };
        tags.insert(loader.into());
    }
    let icon_url = value["logo"]["thumbnailUrl"]
        .as_str()
        .filter(|url| api::download_url(url))
        .map(str::to_owned);
    Some(Project {
        project_id: value["id"].as_u64()?.to_string(),
        title: value["name"].as_str()?.into(),
        slug: value["slug"].as_str().unwrap_or_default().into(),
        description: value["summary"].as_str().unwrap_or_default().into(),
        categories: tags.into_iter().collect(),
        display_categories: categories,
        versions: versions.into_iter().collect(),
        downloads: value["downloadCount"].as_u64().unwrap_or_default(),
        date_modified: value["dateModified"].as_str().unwrap_or_default().into(),
        icon_url,
        latest_version: value["mainFileId"]
            .as_u64()
            .filter(|&id| id > 0)
            .map(|id| id.to_string()),
    })
}

async fn filters(client: &Client, key: &str) -> Result<Value> {
    let categories = get(client, key, "categories?gameId=432&classId=6").await?;
    let mut category_names = BTreeMap::new();
    for category in categories["data"].as_array().into_iter().flatten() {
        if let (Some(id), Some(name)) = (category["id"].as_u64(), category["name"].as_str()) {
            category_names.insert(id.to_string(), name);
        }
    }
    let versions = get(client, key, "games/432/versions").await?;
    let mut versions: Vec<_> = versions["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value["versions"].as_array())
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|version| {
            let number: Vec<u32> = version
                .split('.')
                .map(str::parse)
                .collect::<std::result::Result<_, _>>()
                .ok()?;
            (number.len() >= 2).then_some((number, version))
        })
        .collect();
    versions.sort_unstable();
    versions.dedup();
    let mut categories: Vec<_> = category_names.keys().collect();
    categories.sort_by_key(|id| category_names[*id]);
    Ok(
        json!({"categories":categories,"categoryNames":category_names,
        "gameVersions":versions.into_iter().rev().map(|(_, version)| version).collect::<Vec<_>>()}),
    )
}

async fn search(client: &Client, key: &str, params: &Value) -> Result<Value> {
    let query = text(params, "query")?.trim();
    let version = text(params, "gameVersion")?.trim();
    let category = text(params, "category")?;
    let loader = params["loader"].as_str().unwrap_or_default();
    let offset = params["offset"]
        .as_u64()
        .filter(|&n| n < 10_000)
        .ok_or_else(|| invalid("页码无效。"))?;
    if query.len() > 512 || version.len() > 128 {
        return Err(invalid("搜索条件过长。"));
    }
    let mut url = api::endpoint("mods/search");
    {
        let mut query_pairs = url.query_pairs_mut();
        query_pairs
            .append_pair("gameId", "432")
            .append_pair("classId", "6")
            .append_pair("index", &offset.to_string())
            .append_pair("pageSize", &PAGE_SIZE.to_string())
            .append_pair("sortField", if query.is_empty() { "6" } else { "1" })
            .append_pair("sortOrder", "desc");
        if !query.is_empty() {
            query_pairs.append_pair("searchFilter", query);
        }
        if !version.is_empty() {
            query_pairs.append_pair("gameVersion", version);
        }
        if !category.is_empty() {
            query_pairs.append_pair("categoryId", &id(category)?.to_string());
        }
        if !loader.is_empty() {
            if version.is_empty() {
                return Err(invalid("请选择加载器对应的游戏版本。"));
            }
            query_pairs.append_pair(
                "modLoaderType",
                loader_type(loader).ok_or_else(|| invalid("加载器筛选无效。"))?,
            );
        }
    }
    let response = api::fetch(client.get(url), key).await?;
    let hits: Vec<_> = response["data"]
        .as_array()
        .ok_or_else(|| invalid("CurseForge 搜索结果无效。"))?
        .iter()
        .filter_map(project)
        .collect();
    Ok(
        json!({"hits":hits,"total_hits":response["pagination"]["totalCount"].as_u64().unwrap_or_default().min(10_000)}),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Version {
    #[serde(flatten)]
    file: CurseFile,
    display_name: String,
    release_type: u32,
    file_date: String,
    game_versions: Vec<String>,
    #[serde(default)]
    dependencies: Vec<Dependency>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Dependency {
    mod_id: u32,
    relation_type: u32,
}

impl Version {
    fn required(&self) -> impl Iterator<Item = u32> + '_ {
        self.dependencies
            .iter()
            .filter(|d| d.relation_type == 3)
            .map(|d| d.mod_id)
    }
    fn compatible(&self, target: &Target) -> bool {
        self.game_versions.contains(&target.game_version)
            && self
                .game_versions
                .iter()
                .any(|tag| tag.eq_ignore_ascii_case(&target.loader))
    }
    fn valid_file(&self) -> bool {
        let name = &self.file.file_name;
        storage::safe_relative(name)
            && !name.contains('/')
            && name.to_ascii_lowercase().ends_with(".jar")
            && (1..=2 * 1024 * 1024 * 1024).contains(&self.file.file_length)
    }
}

async fn versions(
    client: &Client,
    key: &str,
    project: u32,
    target: &Target,
) -> Result<Vec<Version>> {
    let mut url = api::endpoint(&format!("mods/{project}/files"));
    url.query_pairs_mut()
        .append_pair("gameVersion", &target.game_version)
        .append_pair(
            "modLoaderType",
            loader_type(&target.loader).ok_or_else(|| invalid("不支持此加载器。"))?,
        )
        .append_pair("pageSize", "50");
    let response = api::fetch(client.get(url), key).await?;
    let mut versions: Vec<Version> = serde_json::from_value(response["data"].clone())?;
    versions.retain(|version| {
        version.file.mod_id == project
            && version.file.game_id == GAME_ID
            && version.compatible(target)
            && version.valid_file()
    });
    versions.sort_by(|a, b| b.file_date.cmp(&a.file_date));
    Ok(versions)
}

async fn version(client: &Client, key: &str, project: u32, file: u32) -> Result<Version> {
    let response = get(client, key, &format!("mods/{project}/files/{file}")).await?;
    let version: Version = serde_json::from_value(response["data"].clone())?;
    if version.file.id != file || version.file.mod_id != project || version.file.game_id != GAME_ID
    {
        return Err(invalid("CurseForge 返回了错误的文件标识。"));
    }
    Ok(version)
}

async fn project_metadata(
    client: &Client,
    key: &str,
    ids: &[u32],
) -> Result<BTreeMap<u32, Project>> {
    let mut result = BTreeMap::new();
    for batch in ids.chunks(100) {
        let response = api::fetch(
            client
                .post(api::endpoint("mods"))
                .json(&json!({"modIds":batch})),
            key,
        )
        .await?;
        for value in response["data"]
            .as_array()
            .ok_or_else(|| invalid("CurseForge 项目信息无效。"))?
        {
            if let Some(project) = project(value) {
                result.insert(id(&project.project_id)?, project);
            }
        }
    }
    Ok(result)
}

async fn describe(client: &Client, key: &str, project: u32, file: Option<u32>) -> Result<Value> {
    let file = match file {
        Some(file) => file,
        None => id(project_metadata(client, key, &[project])
            .await?
            .get(&project)
            .and_then(|metadata| metadata.latest_version.as_deref())
            .ok_or_else(|| invalid("此模组暂无可查看的版本。"))?)?,
    };
    let version = version(client, key, project, file).await?;
    let dependencies: Vec<_> = version
        .required()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let metadata = project_metadata(client, key, &dependencies).await?;
    let items: Vec<_> = dependencies.into_iter().map(|id| {
        let details = metadata.get(&id);
        json!({"name":details.map_or_else(|| id.to_string(), |item| item.title.clone()),
            "version":null,"iconUrl":details.and_then(|item| item.icon_url.as_deref()),"project":details})
    }).collect();
    Ok(json!({"version":version.display_name,"items":items}))
}

async fn installed(
    client: &Client,
    key: &str,
    game: &Path,
    job: &Job,
) -> Result<BTreeMap<u32, Vec<(CurseFile, bool)>>> {
    let entries = match fs::read_dir(storage::within(game, "mods")?) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error.into()),
    };
    let mut local = BTreeMap::<u32, Vec<(PathBuf, bool)>>::new();
    for (index, entry) in entries.enumerate() {
        if index >= 10_000 {
            return Err(invalid("实例超过 10000 个模组文件，请先整理目录。"));
        }
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let lower = name.to_ascii_lowercase();
        let enabled = lower.ends_with(".jar");
        if (enabled || lower.ends_with(".jar.disabled"))
            && entry.file_type()?.is_file()
            && storage::safe_relative(&name)
        {
            let path = storage::within(game, &format!("mods/{name}"))?;
            let fingerprint = api::fingerprint(&path, job).await?;
            local.entry(fingerprint).or_default().push((path, enabled));
        }
    }
    let fingerprints: Vec<_> = local.keys().copied().collect();
    let mut result: BTreeMap<u32, Vec<_>> = BTreeMap::new();
    for batch in fingerprints.chunks(100) {
        for matched in api::exact_matches(client, key, batch).await? {
            if let (Some(sha1), Some(files)) =
                (matched.sha1(), local.get(&matched.file_fingerprint))
            {
                let mut found = None;
                for (path, enabled) in files {
                    if path.metadata()?.len() == matched.file_length
                        && storage::hash(path).await?.eq_ignore_ascii_case(sha1)
                    {
                        found = Some(found.unwrap_or(false) || *enabled);
                    }
                }
                if let Some(enabled) = found {
                    result
                        .entry(matched.mod_id)
                        .or_default()
                        .push((matched, enabled));
                }
            }
        }
    }
    Ok(result)
}

async fn download(client: &Client, key: &str, params: &Value, job: &Job) -> Result<Value> {
    let project = id(text(params, "project")?)?;
    let requested = id(text(params, "versionId")?)?;
    let target = Target::from_params(params)?;
    let game = Path::new(text(params, "game")?);
    let root = Path::new(text(params, "root")?);
    job.phase("resolving", "正在获取模组版本信息…");
    let requested = version(client, key, project, requested).await?;
    if !requested.compatible(&target) || !requested.valid_file() {
        return Err(invalid("此模组版本与目标实例不兼容。"));
    }
    let existing = installed(client, key, game, job).await?;
    let mut pending = VecDeque::from([(project, Some(requested))]);
    let mut visited = BTreeSet::new();
    let mut files = Vec::new();
    let mut names = BTreeSet::new();
    while let Some((project, requested)) = pending.pop_front() {
        job.check()?;
        if !visited.insert(project) {
            continue;
        }
        if visited.len() > 1000 {
            return Err(invalid("CurseForge 必需依赖数量超出限制。"));
        }
        let (selected, local) = if let Some(existing) = existing.get(&project) {
            let installed = existing
                .iter()
                .find(|(file, enabled)| {
                    *enabled
                        && requested
                            .as_ref()
                            .is_none_or(|requested| requested.file.id == file.id)
                })
                .ok_or_else(|| {
                    invalid(
                        "已安装此 CurseForge 项目的其他版本或该模组已停用，请在模组管理中处理。",
                    )
                })?;
            let installed = if let Some(requested) = requested {
                requested
            } else {
                version(client, key, project, installed.0.id).await?
            };
            if !installed.compatible(&target) {
                return Err(invalid("已安装的 CurseForge 依赖与目标实例不兼容。"));
            }
            (installed, true)
        } else if let Some(requested) = requested {
            (requested, false)
        } else {
            let compatible = versions(client, key, project, &target).await?;
            let selected = compatible
                .into_iter()
                .filter(|version| version.file.url().is_ok())
                .min_by_key(|version| version.release_type)
                .ok_or_else(|| {
                    invalid(&format!(
                        "找不到可下载的 CurseForge 必需依赖（{project}）。"
                    ))
                })?;
            (selected, false)
        };
        pending.extend(
            selected
                .required()
                .filter(|id| !visited.contains(id))
                .map(|id| (id, None)),
        );
        if local {
            continue;
        }
        let file = selected.file;
        if !names.insert(file.file_name.to_lowercase()) {
            return Err(invalid("下载计划包含同名的模组文件。"));
        }
        if storage::within(game, &format!("mods/{}.disabled", file.file_name))?.exists() {
            return Err(invalid("同名模组已停用，请在模组管理中处理。"));
        }
        file.url()?;
        files.push(file);
    }
    job.phase("downloading", "正在下载模组及必需依赖…");
    job.begin_download(
        files
            .iter()
            .map(|file| (file.file_name.clone(), Some(file.file_length))),
        true,
    );
    let mut result = Vec::new();
    for file in files {
        let cache = storage::within(root, &format!("cache/curseforge-mod-downloads/{}", file.id))?;
        let mut path = net::ensure_game_file(
            client,
            &cache,
            &file.file_name,
            file.url()?,
            file.sha1(),
            Some(file.file_length),
            false,
            true,
            job,
        )
        .await?;
        if file.sha1().is_none() && api::fingerprint(&path, job).await? != file.file_fingerprint {
            fs::remove_file(&path)?;
            path = serde_json::from_value(service_task(
                "minecraft.download", "once",
                json!({"root":cache,"relative":file.file_name,"url":file.url()?,"size":file.file_length}),
                job,
            ).await?)?;
            if api::fingerprint(&path, job).await? != file.file_fingerprint {
                fs::remove_file(path)?;
                return Err(invalid("CurseForge 模组校验失败。"));
            }
        }
        result.push(path);
    }
    Ok(json!(result))
}

pub async fn call(client: &Client, method: &str, params: &Value, job: &Job) -> Result<Value> {
    let key = api::api_key().unwrap_or_default();
    match method {
        "filters" => filters(client, &key).await,
        "search" => search(client, &key, params).await,
        "versions" => {
            let versions = versions(
                client,
                &key,
                id(text(params, "project")?)?,
                &Target::from_params(params)?,
            )
            .await?;
            Ok(json!(versions.iter().map(|v| json!({
                "id":v.file.id.to_string(), "number":v.display_name,
                "channel":match v.release_type { 1 => "release", 2 => "beta", _ => "alpha" },
                "published":v.file_date, "filename":v.file.file_name, "size":v.file.file_length,
                "requiredDependencies":v.required().count(), "blocked":v.file.url().is_err(),
            })).collect::<Vec<_>>()))
        }
        "dependencies" => {
            describe(
                client,
                &key,
                id(text(params, "project")?)?,
                params["versionId"].as_str().map(id).transpose()?,
            )
            .await
        }
        "download" => download(client, &key, params, job).await,
        _ => Err(invalid("未知 CurseForge 资源操作。")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curseforge_file_metadata_preserves_version_compatibility_and_required_dependencies() {
        let mut data = json!({
            "id":9,"modId":8,"gameId":432,"fileName":"example.jar","fileLength":128,
            "fileFingerprint":123,"downloadUrl":"https://edge.forgecdn.net/files/1/9/example.jar",
            "hashes":[{"algo":1,"value":"0123456789abcdef0123456789abcdef01234567"}],
            "displayName":"Example","releaseType":1,"fileDate":"2026-01-01T00:00:00Z",
            "gameVersions":["1.21.1","NeoForge"],"dependencies":[
                {"modId":10,"relationType":3},{"modId":11,"relationType":2}
            ]
        });
        let file: Version = serde_json::from_value(data.clone()).unwrap();
        let target = Target {
            game_version: "1.21.1".into(),
            loader: "neoforge".into(),
        };
        assert!(file.compatible(&target) && file.valid_file());
        assert_eq!(file.required().collect::<Vec<_>>(), vec![10]);
        assert!(file.file.sha1().is_some() && file.file.url().is_ok());
        data["downloadUrl"] = Value::Null;
        let blocked: Version = serde_json::from_value(data).unwrap();
        assert_eq!(blocked.file.url().unwrap_err().code, "CURSEFORGE_DOWNLOAD");
    }
}
