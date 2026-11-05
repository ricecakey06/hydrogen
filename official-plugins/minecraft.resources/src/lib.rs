mod curse;
mod dependencies;

use hydrogen_plugin_services::{
    PluginHost, Settings,
    contracts::{ApiError, Result},
    http::{Client, RequestBuilder},
    jobs::{Job, Jobs},
    net, storage, text,
};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

const MODRINTH_API: &str = "https://api.modrinth.com/v2/";
const PAGE_SIZE: u64 = 20;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateSource {
    file_name: String,
    sha1: String,
}

struct State {
    jobs: Jobs,
}
impl Default for State {
    fn default() -> Self {
        Self {
            jobs: Jobs::with_limit(16),
        }
    }
}

fn invalid(message: &str) -> ApiError {
    ApiError::new("RESOURCE_DOWNLOAD", message)
}

fn project_id(value: &Value, key: &str) -> Result<String> {
    Ok(valid_id(text(value, key)?)?.into())
}

fn valid_id(id: &str) -> Result<&str> {
    if id.len() != 8 || !id.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return Err(invalid("Modrinth 标识无效。"));
    }
    Ok(id)
}

fn supported_loader(loader: &str) -> bool {
    matches!(loader, "fabric" | "forge" | "neoforge" | "quilt")
}

fn api_url(path: &str) -> Url {
    Url::parse(&format!("{MODRINTH_API}{path}")).expect("valid Modrinth API URL")
}

async fn fetch<T: serde::de::DeserializeOwned>(request: RequestBuilder) -> Result<T> {
    let response = request
        .header(
            reqwest::header::USER_AGENT,
            "ricecakey06/hydrogen (https://github.com/ricecakey06/hydrogen)",
        )
        .send()
        .await?;
    Ok(serde_json::from_slice(
        &net::response_bytes(response, 16 * 1024 * 1024).await?,
    )?)
}

#[derive(Deserialize, Serialize)]
struct SearchPage {
    hits: Vec<Project>,
    total_hits: u64,
}

#[derive(Deserialize, Serialize)]
struct Project {
    project_id: String,
    title: String,
    slug: String,
    description: String,
    categories: Vec<String>,
    display_categories: Vec<String>,
    versions: Vec<String>,
    downloads: u64,
    date_modified: String,
    icon_url: Option<String>,
    latest_version: Option<String>,
}

fn search_url(params: &Value) -> Result<Url> {
    let query = text(params, "query")?.trim();
    let version = text(params, "gameVersion")?.trim();
    let category = text(params, "category")?;
    let loader = params["loader"].as_str().unwrap_or_default();
    let offset = params["offset"]
        .as_u64()
        .filter(|&n| n <= 10_000)
        .ok_or_else(|| invalid("页码无效。"))?;
    if query.len() > 512 || version.len() > 128 || category.len() > 64 {
        return Err(invalid("搜索条件过长。"));
    }
    let mut facets = vec![vec!["project_type:mod".to_owned()]];
    if !version.is_empty() {
        facets.push(vec![format!("versions:{version}")]);
    }
    if !category.is_empty() {
        facets.push(vec![format!("categories:{category}")]);
    }
    if !loader.is_empty() {
        if version.is_empty() || !supported_loader(loader) {
            return Err(invalid(
                "加载器筛选无效，请填写游戏版本并选择受支持的加载器。",
            ));
        }
        facets.push(vec![format!("categories:{loader}")]);
    }
    let mut url = api_url("search");
    url.query_pairs_mut()
        .append_pair("query", query)
        .append_pair("facets", &serde_json::to_string(&facets)?)
        .append_pair(
            "index",
            if query.is_empty() {
                "downloads"
            } else {
                "relevance"
            },
        )
        .append_pair("limit", &PAGE_SIZE.to_string())
        .append_pair("offset", &offset.to_string());
    Ok(url)
}

async fn filters(client: &Client) -> Result<Value> {
    let categories: Vec<Value> = fetch(client.get(api_url("tag/category"))).await?;
    let versions: Vec<Value> = fetch(client.get(api_url("tag/game_version"))).await?;
    Ok(json!({
        "categories": categories.iter().filter(|item| item["project_type"] == "mod")
            .filter_map(|item| item["name"].as_str()).collect::<Vec<_>>(),
        "gameVersions": versions.iter().filter(|item| item["version_type"] == "release")
            .filter_map(|item| item["version"].as_str()).collect::<Vec<_>>(),
    }))
}

#[derive(Deserialize)]
struct Target {
    #[serde(rename = "gameVersion")]
    game_version: String,
    loader: String,
}
impl Target {
    fn from_params(params: &Value) -> Result<Self> {
        let target: Self = serde_json::from_value(params.clone())?;
        if !supported_loader(&target.loader)
            || target.game_version.is_empty()
            || target.game_version.len() > 128
        {
            return Err(invalid("目标实例的游戏版本或加载器无效。"));
        }
        Ok(target)
    }
    fn compatible(&self, version: &Version) -> bool {
        version.game_versions.contains(&self.game_version) && version.loaders.contains(&self.loader)
    }
}

#[derive(Deserialize)]
struct Version {
    id: String,
    project_id: String,
    version_number: String,
    version_type: String,
    date_published: String,
    game_versions: Vec<String>,
    loaders: Vec<String>,
    files: Vec<VersionFile>,
    dependencies: Vec<Dependency>,
}

#[derive(Clone, Deserialize)]
struct Dependency {
    dependency_type: String,
    project_id: Option<String>,
    version_id: Option<String>,
    file_name: Option<String>,
}

#[derive(Deserialize)]
struct VersionFile {
    filename: String,
    url: String,
    size: u64,
    primary: bool,
    hashes: FileHashes,
}

#[derive(Deserialize)]
struct FileHashes {
    sha1: String,
}

impl VersionFile {
    fn valid(&self) -> bool {
        self.size > 0
            && self.filename.to_ascii_lowercase().ends_with(".jar")
            && !self.filename.contains('/')
            && storage::safe_relative(&self.filename)
            && storage::valid_hash(&self.hashes.sha1)
            && Url::parse(&self.url).is_ok_and(|url| {
                url.scheme() == "https"
                    && url.host_str() == Some("cdn.modrinth.com")
                    && url.port().is_none()
                    && url.username().is_empty()
                    && url.password().is_none()
            })
    }
}

impl Version {
    fn jar(&self) -> Option<&VersionFile> {
        self.files
            .iter()
            .filter(|file| file.valid())
            .max_by_key(|file| file.primary)
    }

    fn required(&self) -> impl DoubleEndedIterator<Item = &Dependency> {
        self.dependencies
            .iter()
            .filter(|item| item.dependency_type == "required")
    }
}

async fn project_versions(client: &Client, project: &str, target: &Target) -> Result<Vec<Version>> {
    let mut url = api_url(&format!("project/{project}/version"));
    url.query_pairs_mut()
        .append_pair("loaders", &serde_json::to_string(&[&target.loader])?)
        .append_pair(
            "game_versions",
            &serde_json::to_string(&[&target.game_version])?,
        )
        .append_pair("include_changelog", "false");
    let mut versions: Vec<Version> = fetch(client.get(url)).await?;
    versions.retain(|version| {
        version.project_id == project && target.compatible(version) && valid_id(&version.id).is_ok()
    });
    versions.sort_by(|a, b| b.date_published.cmp(&a.date_published));
    Ok(versions)
}

async fn version(client: &Client, id: &str) -> Result<Version> {
    let version: Version =
        fetch(client.get(api_url(&format!("version/{id}?include_changelog=false")))).await?;
    if version.id != id {
        return Err(invalid("Modrinth 返回了错误的版本标识。"));
    }
    Ok(version)
}

async fn versions(client: &Client, project: &str, target: Target) -> Result<Value> {
    let versions = project_versions(client, project, &target).await?;
    Ok(json!(
        versions
            .iter()
            .filter_map(|version| version.jar().map(|file| json!({
                "id": version.id, "number": version.version_number, "channel": version.version_type,
                "published": version.date_published, "filename": file.filename, "size": file.size,
                "requiredDependencies": version.required().count()
            })))
            .collect::<Vec<_>>()
    ))
}

fn is_update(installed: &Version, latest: &Version, target: &Target) -> bool {
    installed.project_id == latest.project_id
        && installed.id != latest.id
        && latest.date_published.trim_end_matches('Z')
            > installed.date_published.trim_end_matches('Z')
        && latest.version_type == "release"
        && target.compatible(latest)
        && latest.jar().is_some()
}

async fn latest_updates(
    client: &Client,
    target: &Target,
    mut hashes: Vec<String>,
    job: &Job,
) -> Result<BTreeMap<String, Version>> {
    if hashes.len() > 10_000 || !hashes.iter().all(|hash| storage::valid_hash(hash)) {
        return Err(invalid("模组文件指纹无效。"));
    }
    hashes
        .iter_mut()
        .for_each(|hash| hash.make_ascii_lowercase());
    hashes.sort_unstable();
    hashes.dedup();
    let mut available = BTreeMap::new();
    for batch in hashes.chunks(100) {
        job.check()?;
        let installed: BTreeMap<String, Version> = fetch(
            client
                .post(api_url("version_files"))
                .json(&json!({"algorithm": "sha1", "hashes": batch})),
        )
        .await?;
        let known: Vec<_> = batch
            .iter()
            .filter(|hash| {
                installed.get(*hash).is_some_and(|version| {
                    version
                        .files
                        .iter()
                        .any(|file| file.hashes.sha1.eq_ignore_ascii_case(hash))
                })
            })
            .collect();
        if known.is_empty() {
            continue;
        }
        let mut latest: BTreeMap<String, Version> =
            fetch(client.post(api_url("version_files/update")).json(&json!({
                "algorithm": "sha1", "hashes": known, "loaders": [&target.loader],
                "game_versions": [&target.game_version], "version_types": ["release"],
            })))
            .await?;
        for hash in known {
            if let Some(candidate) = latest.remove(hash)
                && is_update(&installed[hash], &candidate, target)
                && !candidate
                    .files
                    .iter()
                    .any(|file| file.hashes.sha1.eq_ignore_ascii_case(hash))
            {
                available.insert(hash.clone(), candidate);
            }
        }
    }
    Ok(available)
}

async fn updates(client: &Client, params: &Value, job: &Job) -> Result<Value> {
    let target = Target::from_params(params)?;
    let hashes = serde_json::from_value(params["hashes"].clone())?;
    let available = latest_updates(client, &target, hashes, job).await?;
    Ok(json!(
        available
            .into_iter()
            .map(|(hash, version)| (hash, version.version_number))
            .collect::<BTreeMap<_, _>>()
    ))
}

async fn download_updates(client: &Client, params: &Value, job: &Job) -> Result<Value> {
    let target = Target::from_params(params)?;
    let game = Path::new(text(params, "game")?);
    let root = Path::new(text(params, "root")?);
    let mut files: Vec<UpdateSource> = serde_json::from_value(params["files"].clone())?;
    if files.is_empty() || files.len() > 1000 {
        return Err(invalid("一次最多更新 1000 个模组文件。"));
    }
    job.phase("resolving", "正在获取模组版本信息…");
    let mut replacing = BTreeSet::new();
    for file in &mut files {
        job.check()?;
        let name = file.file_name.to_lowercase();
        if file.file_name.contains('/')
            || !storage::safe_relative(&file.file_name)
            || !(name.ends_with(".jar") || name.ends_with(".jar.disabled"))
            || !storage::valid_hash(&file.sha1)
            || !replacing.insert(name)
        {
            return Err(invalid("所选模组文件无效或重复。"));
        }
        file.sha1.make_ascii_lowercase();
        let path = storage::within(game, &format!("mods/{}", file.file_name))?;
        if !path.is_file() || storage::hash(&path).await? != file.sha1 {
            return Err(invalid("模组文件已变化，请刷新列表后重试。"));
        }
    }
    let hashes = files.iter().map(|file| file.sha1.clone()).collect();
    let updates: BTreeMap<_, _> = latest_updates(client, &target, hashes, job)
        .await?
        .into_iter()
        .map(|(hash, version)| (hash, Arc::new(version)))
        .collect();
    let mut sources = BTreeMap::new();
    let mut roots = Vec::new();
    for file in files {
        let version = updates
            .get(&file.sha1)
            .ok_or_else(|| invalid("所选模组的更新已变化，请刷新列表后重试。"))?;
        if sources.contains_key(&version.project_id) {
            return Err(invalid("所选文件中包含同一模组的多个副本，请分别处理。"));
        }
        let enabled = file.file_name.to_ascii_lowercase().ends_with(".jar");
        sources.insert(version.project_id.clone(), file);
        roots.push(dependencies::PlannedMod {
            version: version.clone(),
            enabled,
        });
    }
    let plan = dependencies::download_plan(client, &target, game, roots, &replacing, job).await?;
    let paths = download_files(client, root, &plan, job).await?;
    Ok(json!(
        plan.iter()
            .zip(paths)
            .map(|(mod_, path)| json!({
                "path": path, "sha1": mod_.version.jar().unwrap().hashes.sha1,
                "old": sources.get(&mod_.version.project_id), "enabled": mod_.enabled,
            }))
            .collect::<Vec<_>>()
    ))
}

async fn download(client: &Client, params: &Value, job: &Job) -> Result<Value> {
    let project = project_id(params, "project")?;
    let id = project_id(params, "versionId")?;
    let target = Target::from_params(params)?;
    let game = Path::new(text(params, "game")?);
    let root = Path::new(text(params, "root")?);
    job.phase("resolving", "正在获取模组版本信息…");
    let version = version(client, &id).await?;
    if version.project_id != project || !target.compatible(&version) {
        return Err(invalid("此模组版本与目标实例不兼容。"));
    }
    if version.jar().is_none() {
        return Err(invalid("此版本没有有效的模组 JAR 文件。"));
    }
    let requested = dependencies::PlannedMod {
        version: Arc::new(version),
        enabled: true,
    };
    let plan = dependencies::download_plan(
        client,
        &target,
        game,
        vec![requested],
        &BTreeSet::new(),
        job,
    )
    .await?;
    for mod_ in &plan {
        let file = mod_.version.jar().unwrap();
        let disabled = storage::within(game, &format!("mods/{}.disabled", file.filename))?;
        if disabled.exists() {
            return Err(invalid("同名模组已停用，请在模组管理中处理。"));
        }
    }
    Ok(json!(download_files(client, root, &plan, job).await?))
}

async fn download_files(
    client: &Client,
    root: &Path,
    plan: &[dependencies::PlannedMod],
    job: &Job,
) -> Result<Vec<PathBuf>> {
    let mut filenames = BTreeSet::new();
    let mut files = Vec::with_capacity(plan.len());
    for mod_ in plan {
        let file = mod_
            .version
            .jar()
            .ok_or_else(|| invalid("此版本没有有效的模组 JAR 文件。"))?;
        if !filenames.insert(file.filename.to_lowercase()) {
            return Err(invalid("下载计划包含同名的模组文件。"));
        }
        let cache = storage::within(
            root,
            &format!("cache/mod-downloads/{}", valid_id(&mod_.version.id)?),
        )?;
        files.push((cache, file));
    }
    job.phase(
        "downloading",
        if files.len() > 1 {
            "正在下载模组及必需依赖…"
        } else {
            "正在下载模组…"
        },
    );
    job.begin_download(
        files
            .iter()
            .map(|(_, file)| (file.filename.clone(), Some(file.size))),
        true,
    );
    let mut paths: Vec<PathBuf> = Vec::with_capacity(files.len());
    for (cache, file) in files {
        job.check()?;
        paths.push(
            net::ensure(
                client,
                &cache,
                &file.filename,
                &file.url,
                &file.hashes.sha1,
                Some(file.size),
                false,
                true,
                job,
            )
            .await?,
        );
    }
    Ok(paths)
}

impl State {
    fn call(&mut self, request: &Value) -> Result<Value> {
        let params = &request["params"];
        let method = request["method"].as_str().unwrap_or("");
        match method {
            "plugin.describe" => return Ok(json!({"id":"minecraft.resources","abiVersion":1})),
            "plugin.stop" => {
                self.jobs.clear();
                return Ok(Value::Null);
            }
            "poll" => return self.jobs.poll(text(params, "handle")?),
            "cancel" => {
                self.jobs.cancel(text(params, "handle")?);
                return Ok(Value::Null);
            }
            "filters" | "search" | "versions" | "dependencies" | "updates" | "download"
            | "downloadUpdates" => {}
            _ => return Err(invalid("未知资源下载服务方法。")),
        }
        let client = Client::new(&PluginHost, &Settings::default())?;
        let job = Arc::new(Job::default());
        let progress = job.clone();
        let method = method.to_owned();
        let params = params.clone();
        self.jobs.begin(progress, async move {
            if params["source"] == "curseforge" {
                return curse::call(&client, &method, &params, &job).await;
            }
            match method.as_str() {
                "filters" => filters(&client).await,
                "search" => Ok(serde_json::to_value(
                    fetch::<SearchPage>(client.get(search_url(&params)?)).await?,
                )?),
                "versions" => {
                    versions(
                        &client,
                        &project_id(&params, "project")?,
                        Target::from_params(&params)?,
                    )
                    .await
                }
                "dependencies" => {
                    dependencies::describe(
                        &client,
                        &project_id(&params, "project")?,
                        params["versionId"].as_str().map(valid_id).transpose()?,
                    )
                    .await
                }
                "updates" => updates(&client, &params, &job).await,
                "downloadUpdates" => download_updates(&client, &params, &job).await,
                _ => download(&client, &params, &job).await,
            }
        })
    }
}

fn dispatch(state: &mut State, request: &mut Value) -> hydrogen_plugin_sdk::Result {
    state.call(request).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_encodes_filters_separately_from_user_input() {
        let url = search_url(
            &json!({"query":"铜 & 锌","gameVersion":"1.21.1","category":"technology","loader":"neoforge","offset":20}),
        )
        .unwrap();
        let pairs: std::collections::BTreeMap<_, _> = url.query_pairs().collect();
        assert_eq!(pairs["query"], "铜 & 锌");
        assert_eq!(
            pairs["facets"],
            r#"[["project_type:mod"],["versions:1.21.1"],["categories:technology"],["categories:neoforge"]]"#
        );
        assert_eq!(pairs["offset"], "20");
    }

    #[test]
    fn loader_filter_is_optional_and_requires_a_game_version() {
        let mut params = json!({"query":"","gameVersion":" ","category":"","offset":0});
        assert!(search_url(&params).is_ok());
        params["loader"] = json!("fabric");
        assert!(search_url(&params).is_err());
        params["gameVersion"] = json!("1.21.1");
        params["loader"] = json!("unsupported");
        assert!(search_url(&params).is_err());
    }

    #[test]
    fn update_must_be_newer_compatible_and_from_the_same_project() {
        let mut data = json!({
            "id": "Version1", "project_id": "Project1", "version_number": "1.0",
            "version_type": "release", "date_published": "2025-01-01T00:00:00Z",
            "game_versions": ["1.21.1"], "loaders": ["fabric"], "dependencies": [],
            "files": [{ "filename": "mod.jar", "size": 42, "primary": true,
                "url": "https://cdn.modrinth.com/data/Project1/versions/Version1/mod.jar",
                "hashes": { "sha1": "0123456789abcdef0123456789abcdef01234567" } }],
        });
        let installed: Version = serde_json::from_value(data.clone()).unwrap();
        let target = Target {
            game_version: "1.21.1".into(),
            loader: "fabric".into(),
        };
        let check = |data: &Value| {
            is_update(
                &installed,
                &serde_json::from_value(data.clone()).unwrap(),
                &target,
            )
        };
        assert!(!check(&data));
        data["id"] = json!("Version2");
        assert!(!check(&data));
        data["date_published"] = json!("2025-01-02T00:00:00Z");
        assert!(check(&data));
        for (key, value) in [
            ("date_published", json!("2024-12-31T00:00:00Z")),
            ("version_type", json!("beta")),
            ("loaders", json!(["forge"])),
            ("game_versions", json!(["1.21.2"])),
            ("project_id", json!("Project2")),
            ("files", json!([])),
        ] {
            let mut candidate = data.clone();
            candidate[key] = value;
            assert!(!check(&candidate), "invalid update accepted for {key}");
        }
    }
}
