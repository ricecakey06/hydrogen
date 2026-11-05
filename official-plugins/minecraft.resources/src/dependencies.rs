use super::{
    Dependency, Target, Version, api_url, fetch, invalid, project_versions, valid_id, version,
};
use hydrogen_plugin_services::{
    contracts::{ApiError, Result},
    http::Client,
    jobs::Job,
    storage,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::Path,
    sync::Arc,
};

const UNRESOLVED: &str = "MOD_DEPENDENCY_UNRESOLVED";

#[derive(Deserialize)]
struct DependencyVersion {
    id: String,
    project_id: String,
    version_number: String,
}

#[derive(Deserialize)]
struct DependencyProject {
    id: String,
    title: String,
    icon_url: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    downloads: u64,
    updated: Option<String>,
}

async fn fetch_ids<T: serde::de::DeserializeOwned>(
    client: &Client,
    endpoint: &str,
    ids: BTreeSet<&str>,
) -> Result<Vec<T>> {
    let ids: Vec<_> = ids.into_iter().collect();
    let mut items = Vec::new();
    for batch in ids.chunks(50) {
        let mut url = api_url(endpoint);
        url.query_pairs_mut()
            .append_pair("ids", &serde_json::to_string(batch)?);
        items.extend(fetch::<Vec<T>>(client.get(url)).await?);
    }
    Ok(items)
}

fn dependency_project<'a>(
    dependency: &'a Dependency,
    versions: &'a BTreeMap<String, DependencyVersion>,
) -> Option<&'a str> {
    dependency
        .project_id
        .as_deref()
        .and_then(|id| valid_id(id).ok())
        .or_else(|| {
            let pinned = versions.get(dependency.version_id.as_deref()?)?;
            valid_id(&pinned.project_id).ok()
        })
}

fn dependency_item(
    dependency: &Dependency,
    versions: &BTreeMap<String, DependencyVersion>,
    projects: &BTreeMap<String, DependencyProject>,
) -> Value {
    let project = dependency_project(dependency, versions);
    let metadata = project.and_then(|id| projects.get(id));
    let pin = dependency.version_id.as_deref();
    let version = pin
        .and_then(|id| versions.get(id))
        .filter(|version| Some(version.project_id.as_str()) == project)
        .map(|version| version.version_number.as_str());
    let name = metadata
        .map(|project| project.title.as_str())
        .filter(|title| !title.trim().is_empty())
        .or(dependency
            .file_name
            .as_deref()
            .filter(|name| !name.trim().is_empty()))
        .or(project)
        .or(pin);
    let icon_url = metadata.and_then(|project| project.icon_url.as_deref());
    let details = metadata.map(|project| {
        json!({
            "project_id": project.id,
            "title": project.title,
            "description": project.description,
            "display_categories": project.categories,
            "downloads": project.downloads,
            "date_modified": project.updated.as_deref().unwrap_or_default(),
            "icon_url": project.icon_url,
        })
    });
    json!({"name": name, "version": version.or(pin), "iconUrl": icon_url, "project": details})
}

pub(super) async fn describe(client: &Client, project: &str, id: Option<&str>) -> Result<Value> {
    let selected = if let Some(id) = id {
        version(client, id).await?
    } else {
        let url = api_url(&format!(
            "project/{project}/version?include_changelog=false"
        ));
        let versions: Vec<Version> = fetch(client.get(url)).await?;
        versions
            .into_iter()
            .filter(|version| version.project_id == project && valid_id(&version.id).is_ok())
            .max_by(|a, b| a.date_published.cmp(&b.date_published))
            .ok_or_else(|| invalid("此模组暂无可查看的版本。"))?
    };
    if selected.project_id != project {
        return Err(invalid("此版本不属于当前模组。"));
    }
    let version_ids = selected
        .required()
        .filter_map(|dependency| valid_id(dependency.version_id.as_deref()?).ok())
        .collect();
    let versions: BTreeMap<_, _> = fetch_ids::<DependencyVersion>(client, "versions", version_ids)
        .await?
        .into_iter()
        .map(|version| (version.id.clone(), version))
        .collect();
    let project_ids = selected
        .required()
        .filter_map(|dep| dependency_project(dep, &versions))
        .collect();
    let projects: BTreeMap<_, _> = fetch_ids::<DependencyProject>(client, "projects", project_ids)
        .await?
        .into_iter()
        .map(|project| (project.id.clone(), project))
        .collect();
    let items: Vec<_> = selected
        .required()
        .map(|dependency| dependency_item(dependency, &versions, &projects))
        .collect();
    Ok(json!({"version": selected.version_number, "items": items}))
}

#[derive(Clone)]
struct Choice {
    version: Arc<Version>,
    installed: bool,
    enabled: bool,
}

pub(super) struct PlannedMod {
    pub version: Arc<Version>,
    pub enabled: bool,
}

struct LocalVersion {
    version: Arc<Version>,
    enabled: bool,
}

type Installed = BTreeMap<String, Vec<LocalVersion>>;
type Plan = BTreeMap<String, Choice>;

async fn installed_versions(
    client: &Client,
    game: &Path,
    replacing: &BTreeSet<String>,
    job: &Job,
) -> Result<Installed> {
    let dir = match fs::read_dir(storage::within(game, "mods")?) {
        Ok(dir) => dir,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error.into()),
    };
    let mut hashes = BTreeMap::<String, bool>::new();
    let mut files = 0;
    for entry in dir {
        job.check()?;
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let lower = name.to_ascii_lowercase();
        let enabled = lower.ends_with(".jar");
        if !(enabled || lower.ends_with(".jar.disabled"))
            || replacing.contains(&name.to_lowercase())
            || !storage::safe_relative(&name)
            || !entry.file_type()?.is_file()
        {
            continue;
        }
        files += 1;
        if files > 10_000 {
            return Err(invalid("实例超过 10000 个模组文件，请先整理目录。"));
        }
        let path = storage::within(game, &format!("mods/{name}"))?;
        let hash = storage::hash(&path).await?;
        hashes
            .entry(hash)
            .and_modify(|active| *active |= enabled)
            .or_insert(enabled);
    }

    let mut installed = Installed::new();
    let keys = hashes.keys().collect::<Vec<_>>();
    for batch in keys.chunks(100) {
        job.check()?;
        let versions: BTreeMap<String, Version> = fetch(
            client
                .post(api_url("version_files"))
                .json(&json!({"hashes": batch, "algorithm": "sha1"})),
        )
        .await?;
        for (hash, version) in versions {
            let Some(&enabled) = hashes.get(&hash) else {
                continue;
            };
            if valid_id(&version.id).is_ok()
                && valid_id(&version.project_id).is_ok()
                && version
                    .files
                    .iter()
                    .any(|file| file.hashes.sha1.eq_ignore_ascii_case(&hash))
            {
                installed
                    .entry(version.project_id.clone())
                    .or_default()
                    .push(LocalVersion {
                        version: Arc::new(version),
                        enabled,
                    });
            }
        }
    }
    Ok(installed)
}

struct Resolver<'a> {
    client: &'a Client,
    target: &'a Target,
    job: &'a Job,
    installed: Installed,
    exact: BTreeMap<String, Arc<Version>>,
    projects: BTreeMap<String, Vec<Arc<Version>>>,
    attempts: usize,
    limit: usize,
}

impl Resolver<'_> {
    async fn exact(&mut self, id: &str) -> Result<Arc<Version>> {
        if let Some(version) = self.exact.get(id) {
            return Ok(version.clone());
        }
        let version = Arc::new(version(self.client, id).await?);
        self.exact.insert(id.into(), version.clone());
        Ok(version)
    }

    async fn identify(&mut self, dependency: &Dependency) -> Result<(String, Option<String>)> {
        let pinned = dependency.version_id.as_deref().map(valid_id).transpose()?;
        let project = match dependency.project_id.as_deref() {
            Some(project) => valid_id(project)?.to_owned(),
            None => {
                let id = pinned.ok_or_else(|| {
                    ApiError::new(UNRESOLVED, "Modrinth 未提供某个必需依赖的项目或版本标识。")
                })?;
                valid_id(&self.exact(id).await?.project_id)?.to_owned()
            }
        };
        Ok((project, pinned.map(String::from)))
    }

    async fn candidates(
        &mut self,
        project: &str,
        pinned: Option<&str>,
        enabled: bool,
    ) -> Result<Vec<Choice>> {
        let matches = |version: &Version| {
            self.target.compatible(version) && pinned.is_none_or(|id| version.id == id)
        };
        if let Some(installed) = self.installed.get(project) {
            let compatible: Vec<_> = installed
                .iter()
                .filter(|local| (!enabled || local.enabled) && matches(&local.version))
                .map(|local| Choice {
                    version: local.version.clone(),
                    installed: true,
                    enabled: local.enabled,
                })
                .collect();
            if !compatible.is_empty() {
                return Ok(compatible);
            }
            if installed.iter().any(|local| local.enabled) {
                return Err(ApiError::new(
                    UNRESOLVED,
                    format!("已安装的依赖 {project} 与所需版本不兼容。"),
                ));
            }
            if installed.iter().any(|local| matches(&local.version)) {
                return Err(ApiError::new(
                    UNRESOLVED,
                    format!("必需依赖 {project} 已停用，请在模组管理中处理。"),
                ));
            }
        }

        let versions = if let Some(id) = pinned {
            vec![self.exact(id).await?]
        } else {
            if !self.projects.contains_key(project) {
                let mut versions = project_versions(self.client, project, self.target).await?;
                versions.sort_by_key(|version| version.version_type != "release");
                self.projects
                    .insert(project.into(), versions.into_iter().map(Arc::new).collect());
            }
            self.projects[project].clone()
        };
        let candidates: Vec<_> = versions
            .into_iter()
            .filter(|version| {
                version.project_id == project && matches(version) && version.jar().is_some()
            })
            .map(|version| Choice {
                version,
                installed: false,
                enabled,
            })
            .collect();
        if candidates.is_empty() {
            return Err(ApiError::new(
                UNRESOLVED,
                format!("必需依赖 {project} 没有与目标实例兼容的所需版本。"),
            ));
        }
        Ok(candidates)
    }

    async fn resolve(
        &mut self,
        mut plan: Plan,
        mut pending: Vec<(Dependency, bool)>,
    ) -> Result<Plan> {
        while let Some((dependency, enabled)) = pending.pop() {
            self.job.check()?;
            let (project, pinned) = self.identify(&dependency).await?;
            if let Some(chosen) = plan.get(&project) {
                if pinned.is_some_and(|id| id != chosen.version.id) {
                    return Err(ApiError::new(
                        UNRESOLVED,
                        format!("必需依赖 {project} 的版本要求冲突。"),
                    ));
                }
                if enabled && !chosen.enabled {
                    return Err(ApiError::new(
                        UNRESOLVED,
                        format!("必需依赖 {project} 已停用，请在模组管理中处理。"),
                    ));
                }
                continue;
            }
            if plan.len() >= self.limit {
                return Err(invalid("必需依赖数量过多，无法自动安装。"));
            }

            let mut candidates = self
                .candidates(&project, pinned.as_deref(), enabled)
                .await?
                .into_iter();
            let first = candidates
                .next()
                .expect("candidates returns a nonempty list");
            if candidates.len() == 0 {
                pending.extend(
                    first
                        .version
                        .required()
                        .rev()
                        .cloned()
                        .map(|dep| (dep, enabled)),
                );
                plan.insert(project, first);
                continue;
            }

            // A later required edge can pin a version that an earlier project-only edge left open.
            let mut failure = None;
            for choice in std::iter::once(first).chain(candidates) {
                self.attempts += 1;
                if self.attempts > 512 {
                    return Err(invalid("必需依赖的版本组合过多，无法自动安装。"));
                }
                let mut branch = plan.clone();
                let mut edges = pending.clone();
                edges.extend(
                    choice
                        .version
                        .required()
                        .rev()
                        .cloned()
                        .map(|dep| (dep, enabled)),
                );
                branch.insert(project.clone(), choice);
                match Box::pin(self.resolve(branch, edges)).await {
                    Err(error) if error.code == UNRESOLVED => {
                        failure = Some(error);
                    }
                    result => return result,
                }
            }
            return Err(failure.expect("at least one candidate was tried"));
        }
        Ok(plan)
    }
}

pub(super) async fn download_plan(
    client: &Client,
    target: &Target,
    game: &Path,
    roots: Vec<PlannedMod>,
    replacing: &BTreeSet<String>,
    job: &Job,
) -> Result<Vec<PlannedMod>> {
    let mut pending = Vec::new();
    // Resolve enabled roots first so a shared new dependency is enabled if any active mod needs it.
    for enabled in [false, true] {
        for root in roots.iter().filter(|root| root.enabled == enabled) {
            pending.extend(
                root.version
                    .required()
                    .rev()
                    .cloned()
                    .map(|dep| (dep, enabled)),
            );
        }
    }
    if pending.is_empty() && replacing.is_empty() {
        return Ok(roots);
    }
    job.phase("resolving", "正在匹配模组的必需依赖…");
    let installed = installed_versions(client, game, replacing, job).await?;
    for root in &roots {
        if root.enabled
            && !replacing.is_empty()
            && installed
                .get(&root.version.project_id)
                .is_some_and(|locals| locals.iter().any(|local| local.enabled))
        {
            return Err(invalid("此模组存在其他已启用的副本，请先处理重复模组。"));
        }
    }
    let exact = installed
        .values()
        .flatten()
        .map(|choice| (choice.version.id.clone(), choice.version.clone()))
        .collect();
    let mut resolver = Resolver {
        client,
        target,
        job,
        installed,
        exact,
        projects: BTreeMap::new(),
        attempts: 0,
        limit: roots.len() + 127,
    };
    let plan = roots
        .iter()
        .map(|root| {
            (
                root.version.project_id.clone(),
                Choice {
                    version: root.version.clone(),
                    installed: false,
                    enabled: root.enabled,
                },
            )
        })
        .collect();
    let mut plan = resolver.resolve(plan, pending).await?;
    for root in &roots {
        plan.remove(&root.version.project_id);
    }
    Ok(plan
        .into_values()
        .filter(|choice| !choice.installed)
        .map(|choice| PlannedMod {
            version: choice.version,
            enabled: choice.enabled,
        })
        .chain(roots)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hydrogen_plugin_services::{PluginHost, Settings, jobs::Jobs};
    use serde_json::Value;

    fn required(project: Option<&str>, version: Option<&str>) -> Dependency {
        Dependency {
            dependency_type: "required".into(),
            project_id: project.map(String::from),
            version_id: version.map(String::from),
            file_name: None,
        }
    }

    #[test]
    fn dependency_display_resolves_pinned_versions_and_preserves_missing_projects() {
        let versions = BTreeMap::from([(
            "PinVer01".into(),
            DependencyVersion {
                id: "PinVer01".into(),
                project_id: "ProjectA".into(),
                version_number: "2.1.0".into(),
            },
        )]);
        let icon = "https://cdn.modrinth.com/data/ProjectA/icon.png";
        let projects = BTreeMap::from([(
            "ProjectA".into(),
            DependencyProject {
                id: "ProjectA".into(),
                title: "Example Library".into(),
                icon_url: Some(icon.into()),
                description: "Example dependency".into(),
                categories: vec!["library".into()],
                downloads: 12,
                updated: Some("2026-01-01T00:00:00Z".into()),
            },
        )]);
        assert_eq!(
            dependency_item(&required(None, Some("PinVer01")), &versions, &projects),
            json!({
                "name":"Example Library","version":"2.1.0","iconUrl":icon,
                "project":{
                    "project_id":"ProjectA","title":"Example Library","description":"Example dependency",
                    "display_categories":["library"],"downloads":12,"date_modified":"2026-01-01T00:00:00Z","icon_url":icon
                }
            })
        );
        let mut external = required(Some("Missing1"), None);
        external.file_name = Some("external.jar".into());
        assert_eq!(
            dependency_item(&external, &versions, &projects),
            json!({"name":"external.jar","version":null,"iconUrl":null,"project":null})
        );
    }

    fn fixture(
        project: &str,
        id: &str,
        loader: &str,
        dependencies: Vec<Dependency>,
    ) -> Arc<Version> {
        let mut version: Version = serde_json::from_value(json!({
            "id": id, "project_id": project, "version_number": id, "version_type": "release",
            "date_published": "2026-01-01T00:00:00Z", "game_versions": ["1.21.1"], "loaders": [loader],
            "dependencies": [], "files": [{
                "filename": format!("{id}.jar"), "primary": true, "size": 12,
                "url": format!("https://cdn.modrinth.com/data/{project}/versions/{id}/{id}.jar"),
                "hashes": {"sha1": "0123456789abcdef0123456789abcdef01234567"},
            }]
        })).unwrap();
        version.dependencies = dependencies;
        Arc::new(version)
    }

    fn resolve(
        versions: Vec<Arc<Version>>,
        locals: Vec<LocalVersion>,
        mut edges: Vec<Dependency>,
    ) -> Result<Value> {
        let mut jobs = Jobs::default();
        let job = Arc::new(Job::default());
        let handle = jobs.begin(job.clone(), async move {
            let client = Client::new(&PluginHost, &Settings::default())?;
            let target = Target {
                game_version: "1.21.1".into(),
                loader: "fabric".into(),
            };
            let mut resolver = Resolver {
                client: &client,
                target: &target,
                job: &job,
                installed: Installed::new(),
                projects: BTreeMap::new(),
                exact: BTreeMap::new(),
                attempts: 0,
                limit: 128,
            };
            for version in versions {
                resolver.exact.insert(version.id.clone(), version.clone());
                resolver
                    .projects
                    .entry(version.project_id.clone())
                    .or_default()
                    .push(version);
            }
            for local in locals {
                resolver
                    .installed
                    .entry(local.version.project_id.clone())
                    .or_default()
                    .push(local);
            }
            edges.reverse();
            let plan = resolver
                .resolve(
                    Plan::new(),
                    edges.into_iter().map(|dep| (dep, true)).collect(),
                )
                .await?;
            Ok(json!(
                plan.into_values()
                    .filter(|choice| !choice.installed)
                    .map(|choice| choice.version.id.clone())
                    .collect::<Vec<_>>()
            ))
        })?;
        let result = jobs.poll(handle.as_str().unwrap())?;
        assert_eq!(result["ready"], true);
        Ok(result["result"].clone())
    }

    #[test]
    fn finds_later_pins_and_skips_installed_files_but_walks_their_dependencies() {
        let local = fixture(
            "LocalMod",
            "LocalV01",
            "fabric",
            vec![required(Some("LeafProj"), None)],
        );
        let versions = vec![
            fixture("ProjectA", "ANewest1", "fabric", vec![]),
            fixture("ProjectA", "APinned1", "fabric", vec![]),
            fixture(
                "ProjectB",
                "BPinned1",
                "fabric",
                vec![
                    required(Some("ProjectA"), Some("APinned1")),
                    required(Some("LeafProj"), None),
                ],
            ),
            fixture(
                "LeafProj",
                "LeafV001",
                "fabric",
                vec![required(Some("LocalMod"), None)],
            ),
        ];
        let paths = resolve(
            versions,
            vec![LocalVersion {
                version: local,
                enabled: true,
            }],
            vec![
                required(Some("ProjectA"), None),
                required(Some("LocalMod"), None),
                required(None, Some("BPinned1")),
            ],
        )
        .unwrap();
        assert_eq!(paths, json!(["LeafV001", "APinned1", "BPinned1"]));
    }

    #[test]
    fn refuses_incompatible_installed_or_disabled_dependencies() {
        let compatible = fixture("LocalMod", "Fabric01", "fabric", vec![]);
        let wrong_loader = fixture("LocalMod", "Forge001", "forge", vec![]);
        let dependency = required(Some("LocalMod"), None);
        let error = resolve(
            vec![compatible.clone()],
            vec![LocalVersion {
                version: wrong_loader,
                enabled: true,
            }],
            vec![dependency.clone()],
        )
        .unwrap_err();
        assert!(error.message.contains("已安装的依赖 LocalMod"));

        let error = resolve(
            vec![compatible.clone()],
            vec![LocalVersion {
                version: compatible,
                enabled: false,
            }],
            vec![dependency],
        )
        .unwrap_err();
        assert!(error.message.contains("LocalMod 已停用"));
    }
}
