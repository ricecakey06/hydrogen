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
    job.check()?;
    let id = service(host, settings, "importBegin", json!({"game":game})).await?;
    let handle = Handle {
        host: host.clone(),
        settings: settings.clone(),
        id: id.as_str().ok_or_else(|| failure("无效导入句柄。"))?.into(),
        close: "importAbort",
    };
    for (index, path) in paths.iter().enumerate() {
        job.check()?;
        service(
            host,
            settings,
            "importAdd",
            json!({"handle":handle.id,"path":path}),
        )
        .await?;
        job.progress(
            index as u64 + 1,
            Some(paths.len() as u64),
            &path.file_name().unwrap_or_default().to_string_lossy(),
        );
    }
    job.check()?;
    job.phase("committing", "正在提交模组文件…");
    service(host, settings, "importCommit", json!({"handle":handle.id})).await?;
    Ok(())
}
#[cfg(feature = "desktop")]
impl super::core::Launcher {
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
        let job = self.tasks.start("modInstall", Some(&id))?;
        let task_id = job.state.lock().unwrap().id.clone();
        let host = self.plugins.clone();
        tauri::async_runtime::spawn(async move {
            let _lock = lock;
            let _lease = lease;
            let result = import(&host, &_lease.settings, &game, &paths, &job).await;
            job.finish(result.map(|_| ()));
        });
        Ok(task_id)
    }
}
