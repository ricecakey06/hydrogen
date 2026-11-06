//! Transport and launcher task adaptation only; all mod algorithms live in the installed plugin.
use super::storage::Settings;
use crate::{
    contracts::{ApiError, Result},
    plugins::host::PluginHost,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModInfo {
    pub id: String,
    pub english_name: String,
    pub chinese_name: Option<String>,
    pub version: String,
    pub summary: String,
    pub kind: String,
    pub file_name: Option<String>,
    pub sha1: Option<String>,
    pub enabled: bool,
    #[serde(default)]
    pub duplicate: bool,
    pub icon: Option<String>,
    pub problem: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModRef {
    pub file_name: String,
    pub sha1: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Inventory {
    pub items: Vec<ModInfo>,
    pub notice: Option<String>,
}

fn failure(message: &str) -> ApiError {
    ApiError::new("MOD_OPERATION", message)
}
fn call(host: &PluginHost, settings: &Settings, method: &str, params: Value) -> Result<Value> {
    host.call_service(settings, "minecraft.mods", "minecraft.mods", method, params)
}
pub async fn service(
    host: &PluginHost,
    settings: &Settings,
    method: &str,
    params: Value,
) -> Result<Value> {
    let host = host.clone();
    let settings = settings.clone();
    let method = method.to_owned();
    tokio::task::spawn_blocking(move || call(&host, &settings, &method, params))
        .await
        .map_err(|_| failure("模组服务线程已停止。"))?
}
struct Handle {
    host: PluginHost,
    settings: Settings,
    id: String,
    close: &'static str,
}
impl Drop for Handle {
    fn drop(&mut self) {
        let _ = call(
            &self.host,
            &self.settings,
            self.close,
            json!({"handle":self.id}),
        );
    }
}
async fn result<T: serde::de::DeserializeOwned>(
    host: &PluginHost,
    settings: &Settings,
    method: &str,
    params: Value,
) -> Result<T> {
    let descriptor = service(host, settings, method, params).await?;
    let handle = Handle {
        host: host.clone(),
        settings: settings.clone(),
        id: descriptor["handle"]
            .as_str()
            .ok_or_else(|| failure("无效结果句柄。"))?
            .into(),
        close: "close",
    };
    let length = descriptor["length"]
        .as_u64()
        .filter(|n| *n <= 32 * 1024 * 1024)
        .ok_or_else(|| failure("模组结果过大。"))? as usize;
    let mut bytes = Vec::with_capacity(length);
    while bytes.len() < length {
        let chunk = service(
            host,
            settings,
            "read",
            json!({"handle":handle.id,"offset":bytes.len()}),
        )
        .await?;
        let chunk = STANDARD
            .decode(chunk.as_str().ok_or_else(|| failure("无效模组结果。"))?)
            .map_err(|_| failure("无效模组结果。"))?;
        if chunk.is_empty() || bytes.len() + chunk.len() > length {
            return Err(failure("模组结果长度无效。"));
        }
        bytes.extend(chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}
pub async fn inventory(
    host: &PluginHost,
    settings: &Settings,
    root: &Path,
    game: &Path,
) -> Result<Inventory> {
    result(
        host,
        settings,
        "inventory",
        json!({"root":root,"game":game}),
    )
    .await
}
pub async fn read_archive(
    host: &PluginHost,
    settings: &Settings,
    path: &Path,
    sha1: &str,
) -> Result<ModInfo> {
    result(
        host,
        settings,
        "readArchive",
        json!({"path":path,"sha1":sha1}),
    )
    .await
}
pub async fn set_enabled(
    host: &PluginHost,
    settings: &Settings,
    game: &Path,
    files: &[ModRef],
    enabled: bool,
) -> Result<()> {
    service(
        host,
        settings,
        "enable",
        json!({"game":game,"files":files,"enabled":enabled}),
    )
    .await?;
    Ok(())
}
pub async fn remove(
    host: &PluginHost,
    settings: &Settings,
    game: &Path,
    files: &[ModRef],
) -> Result<()> {
    service(host, settings, "remove", json!({"game":game,"files":files})).await?;
    Ok(())
}
pub async fn import(
    host: &PluginHost,
    settings: &Settings,
    game: &Path,
    paths: &[PathBuf],
    job: &super::tasks::Job,
) -> Result<()> {
    if paths.is_empty() || paths.len() > 1000 {
        return Err(failure("一次最多导入 1000 个模组文件。"));
    }
    stage_files(
        host,
        settings,
        game,
        paths.iter().map(|path| (path.as_path(), Value::Null)),
        job,
    )
    .await
}

async fn stage_files<'a>(
    host: &PluginHost,
    settings: &Settings,
    game: &Path,
    files: impl ExactSizeIterator<Item = (&'a Path, Value)>,
    job: &super::tasks::Job,
) -> Result<()> {
    job.check()?;
    let id = service(host, settings, "importBegin", json!({"game":game})).await?;
    let handle = Handle {
        host: host.clone(),
        settings: settings.clone(),
        id: id.as_str().ok_or_else(|| failure("无效导入句柄。"))?.into(),
        close: "importAbort",
    };
    let count = files.len();
    for (index, (path, update)) in files.enumerate() {
        job.check()?;
        service(
            host,
            settings,
            "importAdd",
            json!({"handle":handle.id,"path":path,"update":update}),
        )
        .await?;
        job.progress(
            index as u64 + 1,
            Some(count as u64),
            &path.file_name().unwrap_or_default().to_string_lossy(),
        );
    }
    job.begin_commit("正在提交模组文件…")?;
    service(host, settings, "importCommit", json!({"handle":handle.id})).await?;
    Ok(())
}

#[cfg(feature = "desktop")]
#[derive(Deserialize)]
struct UpdateFile {
    path: PathBuf,
    sha1: String,
    old: Option<ModRef>,
    enabled: bool,
}

#[cfg(feature = "desktop")]
enum ResourceInstall {
    Download {
        project: String,
        version_id: String,
        source: Option<String>,
    },
    Update(Vec<ModRef>),
}

#[cfg(feature = "desktop")]
fn lock_mod_directory(game: &Path) -> Result<std::fs::File> {
    let path = super::storage::within(game, ".hydrogen/mods.lock")?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock()
        .map_err(|_| failure("模组目录正在使用，请等待当前操作结束。"))?;
    Ok(file)
}

#[cfg(feature = "desktop")]
impl super::core::Launcher {
    pub async fn resource_catalog(&self, method: &str, params: Value) -> Result<Value> {
        let lease = self.lease();
        let mut params = params;
        if let Some(source) = params["source"].as_str().map(String::from) {
            if let Some(id) = source.strip_prefix("minecraft.resources:") {
                params["source"] = json!(id);
            } else if source.contains(':') {
                return self
                    .plugins
                    .provider_call(&lease.settings, "resourceSource", &source, method, params)
                    .await;
            }
        }
        self.require("minecraft.resources")?;
        super::net::task(&self.client, "minecraft.resources", method, params, None).await
    }
    fn mod_target(instance: &super::storage::Instance) -> Result<Value> {
        let loader = match instance.components.as_ref().map(|c| c.kind.as_str()) {
            Some("forge") => "forge",
            Some("neoforge") => "neoforge",
            Some("quilt") => "quilt",
            Some("fabric" | "") => "fabric",
            _ => match instance.loader_name.to_ascii_lowercase().as_str() {
                "fabric" => "fabric",
                "forge" => "forge",
                "neoforge" => "neoforge",
                "quilt" => "quilt",
                _ => "",
            },
        };
        if !instance.info.is_installed || loader.is_empty() {
            return Err(failure("请选择已安装模组加载器的游戏实例。"));
        }
        Ok(json!({"gameVersion":instance.info.game_version,"loader":loader}))
    }
    pub async fn resource_versions(
        &self,
        id: &str,
        project: &str,
        source: Option<String>,
    ) -> Result<Value> {
        let _lease = self.lease();
        let instance = self.store.lock().unwrap().instance(id)?;
        let mut params = Self::mod_target(&instance)?;
        params["project"] = json!(project);
        params["source"] = json!(source);
        self.resource_catalog("versions", params).await
    }
    pub async fn mod_updates(&self, id: &str, hashes: Vec<String>) -> Result<Value> {
        let _lease = self.lease();
        let instance = self.store.lock().unwrap().instance(id)?;
        let mut params = Self::mod_target(&instance)?;
        params["hashes"] = json!(hashes);
        self.resource_catalog("updates", params).await
    }
    pub fn download_mod(
        self: &std::sync::Arc<Self>,
        id: String,
        project: String,
        version_id: String,
        source: Option<String>,
    ) -> Result<String> {
        self.install_resource(
            id,
            ResourceInstall::Download {
                project,
                version_id,
                source,
            },
        )
    }
    pub fn update_mods(
        self: &std::sync::Arc<Self>,
        id: String,
        files: Vec<ModRef>,
    ) -> Result<String> {
        if files.is_empty() || files.len() > 1000 {
            return Err(failure("一次最多更新 1000 个模组文件。"));
        }
        self.install_resource(id, ResourceInstall::Update(files))
    }
    fn install_resource(
        self: &std::sync::Arc<Self>,
        id: String,
        install: ResourceInstall,
    ) -> Result<String> {
        let lock = self.instance_lock(&id)?;
        self.require("minecraft.resources")?;
        self.require("minecraft.mods")?;
        let lease = self.lease();
        let (instance, game) = {
            let store = self.store.lock().unwrap();
            (store.instance(&id)?, store.game_dir(&id)?)
        };
        let mut params = Self::mod_target(&instance)?;
        let mod_lock = lock_mod_directory(&game)?;
        let (method, kind) = match install {
            ResourceInstall::Download {
                project,
                version_id,
                source,
            } => {
                params["project"] = json!(project);
                params["versionId"] = json!(version_id);
                params["source"] = json!(source);
                ("download", "modDownload")
            }
            ResourceInstall::Update(files) => {
                params["files"] = json!(files);
                ("downloadUpdates", "modUpdate")
            }
        };
        params["game"] = json!(game);
        params["root"] = json!(lease.root);
        let job = self.tasks.start(kind, Some(&id))?;
        let task_id = {
            let mut state = job.state.lock().unwrap();
            state.instance_name = Some(instance.info.name);
            state.game_version = Some(instance.info.game_version);
            state.id.clone()
        };
        let core = self.clone();
        tauri::async_runtime::spawn(async move {
            let _lock = lock;
            let _mod_lock = mod_lock;
            let result = async {
                let mut params = params;
                if method == "download" && let Some(source) = params["source"].as_str().map(String::from) {
                    if let Some(id) = source.strip_prefix("minecraft.resources:") { params["source"] = json!(id); }
                    else if source.contains(':') {
                        job.phase("resolving", "正在读取资源提供者的下载计划…");
                        // Pass only public target metadata; internal paths never reach JS.
                        let request = json!({"project":params["project"],"versionId":params["versionId"],"gameVersion":params["gameVersion"],"loader":params["loader"]});
                        let plan = core.plugins.provider_call(&lease.settings, "resourceSource", &source, "downloadPlan", request).await?;
                        let (provider,_) = core.plugins.provider(&lease.settings,"resourceSource",&source)?;
                        params["allowedHosts"] = json!(provider.manifest.scopes.http);
                        params["plan"] = plan;
                        params["source"] = Value::Null;
                    }
                }
                let operation = if params["plan"].is_null() { method } else { "downloadPlan" };
                let result = super::net::task(&core.client, "minecraft.resources", operation, params, Some(&job)).await?;
                if method == "download" {
                    let paths: Vec<PathBuf> = serde_json::from_value(result)?;
                    if paths.is_empty() {
                        return Ok(());
                    }
                    return import(&core.plugins, &lease.settings, &game, &paths, &job).await;
                }
                let files: Vec<UpdateFile> = serde_json::from_value(result)?;
                job.phase("staging", "正在准备替换模组文件…");
                stage_files(
                    &core.plugins,
                    &lease.settings,
                    &game,
                    files.iter().map(|file| {
                        (
                            file.path.as_path(),
                            json!({"sha1":file.sha1,"old":file.old,"enabled":file.enabled}),
                        )
                    }),
                    &job,
                )
                .await
            }
            .await;
            job.finish(result);
        });
        Ok(task_id)
    }
    pub async fn mods(&self, id: &str) -> Result<Inventory> {
        self.require("minecraft.mods")?;
        let lease = self.lease();
        let game = {
            let store = self.store.lock().unwrap();
            store.instance(id)?;
            store.game_dir(id)?
        };
        inventory(&self.plugins, &lease.settings, &lease.root, &game).await
    }
    pub async fn enable_mods(&self, id: &str, files: Vec<ModRef>, enabled: bool) -> Result<()> {
        let _lock = self.instance_lock(id)?;
        self.require("minecraft.mods")?;
        let _lease = self.lease();
        let game = self.store.lock().unwrap().game_dir(id)?;
        let _mod_lock = lock_mod_directory(&game)?;
        set_enabled(&self.plugins, &_lease.settings, &game, &files, enabled).await
    }
    pub async fn delete_mods(&self, id: &str, files: Vec<ModRef>, confirmed: bool) -> Result<()> {
        if !confirmed {
            return Err(failure("请先确认删除所选模组。"));
        }
        let _lock = self.instance_lock(id)?;
        self.require("minecraft.mods")?;
        let _lease = self.lease();
        let game = self.store.lock().unwrap().game_dir(id)?;
        let _mod_lock = lock_mod_directory(&game)?;
        remove(&self.plugins, &_lease.settings, &game, &files).await
    }
    pub async fn name_mod(&self, id: &str, file: ModRef, name: &str) -> Result<()> {
        let _lock = self.instance_lock(id)?;
        self.require("minecraft.mods")?;
        let lease = self.lease();
        let game = self.store.lock().unwrap().game_dir(id)?;
        service(
            &self.plugins,
            &lease.settings,
            "name",
            json!({"root":lease.root,"game":game,"file":file,"name":name}),
        )
        .await?;
        Ok(())
    }
    pub fn import_mods(
        self: &std::sync::Arc<Self>,
        id: String,
        paths: Vec<PathBuf>,
    ) -> Result<String> {
        let lock = self.instance_lock(&id)?;
        self.require("minecraft.mods")?;
        let lease = self.lease();
        let game = self.store.lock().unwrap().game_dir(&id)?;
        let mod_lock = lock_mod_directory(&game)?;
        let job = self.tasks.start("modInstall", Some(&id))?;
        let task_id = job.state.lock().unwrap().id.clone();
        let host = self.plugins.clone();
        tauri::async_runtime::spawn(async move {
            let _lock = lock;
            let _mod_lock = mod_lock;
            let _lease = lease;
            let result = import(&host, &_lease.settings, &game, &paths, &job).await;
            job.finish(result.map(|_| ()));
        });
        Ok(task_id)
    }
}
