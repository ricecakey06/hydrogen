//! The public JS boundary. Raw native paths, account tokens and process handles
//! never pass through this catalog; adapters derive them from validated IDs.
use super::manifest::Plugin;
use crate::contracts::{ApiError, Result};
use serde_json::Value;
use std::sync::Arc;
pub type Operations = Arc<dyn Fn(&Plugin, u64, &str, Value) -> Result<Value> + Send + Sync>;

pub fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| v.len() <= 4096)
        .ok_or_else(|| ApiError::new("INVALID_PARAMS", format!("缺少有效的 {key}。")))
}
pub fn permission(plugin: &Plugin, permission: &str) -> Result<()> {
    if plugin.manifest.permissions.iter().any(|p| p == permission) {
        Ok(())
    } else {
        Err(ApiError::new(
            "PERMISSION_DENIED",
            format!("插件未声明 {permission}。"),
        ))
    }
}
pub fn read_only(method: &str) -> bool {
    matches!(
        method,
        "instances.list"
            | "accounts.list"
            | "java.list"
            | "settings.get"
            | "storage.get"
            | "files.read"
            | "files.list"
            | "tasks.list"
            | "tasks.get"
            | "services.list"
            | "ui.theme"
    )
}
pub fn authorize(plugin: &Plugin, method: &str) -> Result<()> {
    let required = match method {
        "instances.list" => "instances.read",
        "instances.repair" => "instances.repair",
        "instances.launch" => "instances.launch",
        "instances.create"
        | "instances.rename"
        | "instances.configure"
        | "instances.delete"
        | "instances.select" => "instances.write",
        "accounts.list" => "accounts.read",
        "accounts.login"
        | "accounts.addOffline"
        | "accounts.addProvider"
        | "accounts.select"
        | "accounts.delete"
        | "accounts.signOut" => "accounts.write",
        "java.list" => "java.read",
        "java.scan" => "java.scan",
        "resources.sources"
        | "resources.filters"
        | "resources.search"
        | "resources.versions"
        | "resources.dependencies" => "resources.read",
        "resources.install" => "resources.install",
        "mods.list" | "mods.updates" => "mods.read",
        "mods.update" | "mods.enable" | "mods.delete" | "mods.rename" => "mods.write",
        "modpacks.directories" => "modpacks.read",
        "modpacks.import" | "modpacks.export" => "modpacks.write",
        "settings.get" => "settings.read",
        "settings.update" => "settings.write",
        "storage.get" | "storage.set" | "storage.delete" => "storage",
        "files.read" | "files.write" | "files.list" | "files.delete" => "files",
        "http.request" | "http.download" => "http",
        "process.run" | "process.cancel" => "process",
        "tasks.list" | "tasks.get" => "tasks.read",
        "tasks.cancel" => "tasks.cancel",
        "ui.update" | "ui.navigate" | "ui.theme" => "ui.contribute",
        "log.write" => "log",
        _ => return Err(ApiError::new("UNKNOWN_CAPABILITY", "宿主不支持此能力。")),
    };
    permission(plugin, required)
}
pub fn service_policy(target: &Plugin, service: &str, method: &str) -> &'static str {
    if target.manifest.runtime == "javascript" {
        return "direct";
    }
    match (service, method) {
        ("java.runtime", "parse") | ("account.offline", "session") => "direct",
        _ => "nativeOnly",
    }
}
pub fn service_access(caller: &Plugin, target: &Plugin, service: &str, method: &str) -> Result<()> {
    permission(caller, "services.call")?;
    if service_policy(target, service, method) == "nativeOnly" {
        return Err(ApiError::new(
            "NATIVE_ONLY_SERVICE",
            "此原生方法含宿主路径、凭据或内部句柄，请使用业务 API。",
        ));
    }
    match (service, method) {
        ("java.runtime", "parse") => permission(caller, "java.read"),
        ("account.offline", "session") => permission(caller, "accounts.read"),
        _ => Ok(()),
    }
}

#[cfg(feature = "desktop")]
pub mod desktop {
    use super::*;
    use crate::launcher::{core::Launcher, storage};
    use serde_json::json;
    use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};
    use tauri::Emitter;

    pub fn attach(core: &Arc<Launcher>, window: tauri::WebviewWindow) {
        let weak = Arc::downgrade(core);
        let handle = tauri::async_runtime::handle();
        // A task handle grants access only to its creator and generation.
        let owners = Arc::new(Mutex::new(BTreeMap::<String, (String, u64)>::new()));
        let stopped = Arc::new(Mutex::new(BTreeMap::<String, u64>::new()));
        core.plugins.set_operations(Arc::new(move |plugin, generation, method, params| {
            let core = weak.upgrade().ok_or_else(|| ApiError::new("HOST_STOPPED", "启动器已停止。"))?;
            if method == "lifecycle.stop" {
                let mut owners=owners.lock().unwrap();
                stopped.lock().unwrap().entry(plugin.manifest.id.clone()).and_modify(|last| *last = (*last).max(generation)).or_insert(generation);
                owners.retain(|id,owner| {
                    if owner.0==plugin.manifest.id && owner.1==generation { let _=core.tasks.cancel(id); false } else { true }
                });
                let _=window.emit("plugin-ui-stop",json!({"pluginId":plugin.manifest.id,"digest":plugin.digest,"generation":generation}));
                return Ok(Value::Null);
            }
            authorize(plugin, method)?;
            if stopped.lock().unwrap().get(&plugin.manifest.id).is_some_and(|last| generation <= *last) {
                return Err(ApiError::new("SCRIPT_STOPPED", "插件代次已停止。"));
            }
            let lease = core.lease();
            let private = storage::within(&lease.root, &format!("plugin-data/{}", plugin.manifest.id))?;
            let id = || string(&params, "id");
            let task = |task_id: String| -> Result<Value> {
                let mut owners = owners.lock().unwrap();
                if stopped.lock().unwrap().get(&plugin.manifest.id).is_some_and(|last| generation <= *last) {
                    let _ = core.tasks.cancel(&task_id);
                    return Err(ApiError::new("SCRIPT_STOPPED", "插件代次已停止。"));
                }
                owners.insert(task_id.clone(), (plugin.manifest.id.clone(), generation));
                Ok(json!({"id": task_id}))
            };
            let owns = |task_id: &str| -> Result<()> {
                if owners.lock().unwrap().get(task_id).is_some_and(|o| o.0 == plugin.manifest.id && o.1 == generation) { Ok(()) }
                else { Err(ApiError::new("RESOURCE_SCOPE", "此任务不属于当前插件代次。")) }
            };
            let value = match method {
                "instances.list" => serde_json::to_value(core.store.lock().unwrap().instances()?)?,
                "instances.select" => { core.select(id()?)?; Value::Null }
                "instances.rename" => { core.rename(id()?, string(&params, "name")?)?; Value::Null }
                "instances.configure" => { core.overrides(id()?, serde_json::from_value(params["overrides"].clone())?)?; Value::Null }
                "instances.delete" => { core.delete(id()?, string(&params, "confirmation")?)?; Value::Null }
                "instances.repair" => task(core.install(id()?.into())?)?,
                "instances.launch" => task(core.launch(id()?.into(), window.clone())?)?,
                "instances.create" => task(handle.block_on(core.create(string(&params,"version")?.into(),string(&params,"name")?.into(),serde_json::from_value(params["components"].clone())?))?)?,
                "accounts.list" => serde_json::to_value(core.accounts.view(&lease.root, &lease.settings, false))?,
                "accounts.login" => task(core.login(false)?)?,
                "accounts.addOffline" => core.profile_change("profiles.addOffline", json!({"name":string(&params,"name")?}))?,
                "accounts.addProvider" => core.profile_change("profiles.addProvider",params.clone())?,
                "accounts.select" => { core.select_profile(id()?)?; Value::Null }
                "accounts.delete" => { core.delete_profile(id()?,string(&params,"confirmation")?)?; Value::Null }
                "accounts.signOut" => { core.sign_out()?; Value::Null }
                "java.list" => serde_json::to_value(&*core.java.lock().unwrap())?,
                "java.scan" => task(core.scan_java()?)?,
                "resources.sources" => core.plugins.providers(&lease.settings, "resourceSource"),
                "resources.filters" | "resources.search" | "resources.dependencies" => handle.block_on(core.resource_catalog(method.split_once('.').unwrap().1,params.clone()))?,
                "resources.versions" => handle.block_on(core.resource_versions(id()?,string(&params,"project")?,params["source"].as_str().map(String::from)))?,
                "resources.install" => task(core.download_mod(id()?.into(),string(&params,"project")?.into(),string(&params,"versionId")?.into(),params["source"].as_str().map(String::from))?)?,
                "mods.list" => serde_json::to_value(handle.block_on(core.mods(id()?))?)?,
                "mods.updates" => handle.block_on(core.mod_updates(id()?,serde_json::from_value(params["hashes"].clone())?))?,
                "mods.update" => task(core.update_mods(id()?.into(),serde_json::from_value(params["files"].clone())?)?)?,
                "mods.enable" => { handle.block_on(core.enable_mods(id()?,serde_json::from_value(params["files"].clone())?,params["enabled"].as_bool().ok_or_else(|| ApiError::new("INVALID_PARAMS","enabled 必须为布尔值。"))?))?; Value::Null }
                "mods.delete" => { handle.block_on(core.delete_mods(id()?,serde_json::from_value(params["files"].clone())?,params["confirmed"] == true))?; Value::Null }
                "mods.rename" => { handle.block_on(core.name_mod(id()?,serde_json::from_value(params["file"].clone())?,string(&params,"name")?))?; Value::Null }
                "modpacks.directories" => handle.block_on(core.modpack_directories(id()?))?,
                "modpacks.import" => task(handle.block_on(core.import_modpack(private_path(&private,string(&params,"path")?)?))?)?,
                "modpacks.export" => task(core.export_modpack(id()?,private_path(&private,string(&params,"path")?)?,params["options"].clone())?)?,
                "settings.get" => {
                    let mut settings = serde_json::to_value(&lease.settings)?;
                    settings.as_object_mut().unwrap().remove("modules");
                    settings.as_object_mut().unwrap().remove("gameDirectory");
                    settings
                }
                "settings.update" => {
                    let patch = params.as_object().ok_or_else(|| ApiError::new("INVALID_PARAMS","设置补丁必须为对象。"))?;
                    let mut settings = serde_json::to_value(core.store.lock().unwrap().settings.clone())?;
                    for (key,value) in patch {
                        if !["locale","java","maxMemoryMib","windowMode","windowWidth","windowHeight","afterLaunch","jvmArguments","downloadSource"].contains(&key.as_str()) {
                            return Err(ApiError::new("SETTING_SCOPE","此设置字段不能由插件更改。"));
                        }
                        settings[key] = value.clone();
                    }
                    core.save_settings(serde_json::from_value(settings)?)?;
                    Value::Null
                }
                "storage.get" | "storage.set" | "storage.delete" => {
                    let key = string(&params,"key")?;
                    if !super::super::manifest::valid_id(key) { return Err(ApiError::new("INVALID_KEY","存储键必须为合法 ID。")); }
                    let path = private_path(&private,&format!("storage/{key}.json"))?;
                    match method {
                        "storage.get" => if path.exists() { storage::read_json(&path)? } else { Value::Null },
                        "storage.set" => { storage::write_json(&path,&params["value"])?; Value::Null }
                        _ => { if path.exists() { std::fs::remove_file(path)?; } Value::Null }
                    }
                }
                "files.read" | "files.write" | "files.list" | "files.delete" => {
                    let path = private_path(&private,string(&params,"path")?)?;
                    match method {
                        "files.read" => { if std::fs::metadata(&path)?.len() > 192 * 1024 { return Err(ApiError::new("MESSAGE_LIMIT","请分块读取大文件。")); } json!({"text":std::fs::read_to_string(path)?}) }
                        "files.write" => { storage::atomic_write(&path,string(&params,"text")?.as_bytes())?; Value::Null }
                        "files.list" => {
                            let rows = std::fs::read_dir(path)?.take(1000).map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned())).collect::<std::io::Result<Vec<_>>>()?;
                            json!(rows)
                        }
                        _ => { std::fs::remove_file(path)?; Value::Null }
                    }
                }
                "tasks.list" => json!(core.tasks.list().into_iter().filter(|t| owns(&t.id).is_ok()).collect::<Vec<_>>()),
                "tasks.get" => { owns(id()?)?; serde_json::to_value(core.tasks.list().into_iter().find(|t| t.id == id().unwrap()).ok_or_else(|| ApiError::new("UNKNOWN_TASK","任务不存在。"))?)? }
                "tasks.cancel" | "process.cancel" => { owns(id()?)?; core.tasks.cancel(id()?)?; Value::Null }
                "ui.theme" => { let mut appearance = core.plugins.themes.state().public_appearance(); appearance["locale"] = json!(lease.settings.locale); appearance },
                "ui.update" => {
                    let item = string(&params,"id")?;
                    if !plugin.manifest.contributes.views.iter().chain(&plugin.manifest.contributes.menus).any(|v| v.id == item) { return Err(ApiError::new("UNKNOWN_CONTRIBUTION","界面贡献不存在。")); }
                    let state = params["state"].as_object().ok_or_else(|| ApiError::new("INVALID_PARAMS","界面状态必须为对象。"))?;
                    if state.iter().any(|(k,v)| match k.as_str() { "title" | "badge" => v.as_str().is_none_or(|s| s.len()>200), "enabled" | "visible" => !v.is_boolean(), _ => true }) { return Err(ApiError::new("INVALID_UI_STATE","界面状态字段无效。")); }
                    let _ = window.emit("plugin-ui-update",json!({"pluginId":plugin.manifest.id,"digest":plugin.digest,"generation":generation,"id":item,"state":state}));
                    Value::Null
                }
                "ui.navigate" => {
                    if !plugin.manifest.contributes.views.iter().any(|v| v.id == id().unwrap_or("")) { return Err(ApiError::new("UNKNOWN_CONTRIBUTION","视图不存在。")); }
                    let _ = window.emit("plugin-navigate",json!({"pluginId":plugin.manifest.id,"id":id()?})); Value::Null
                }
                "log.write" => {
                    let message = string(&params,"message")?;
                    let _ = window.emit("plugin-log",json!({"pluginId":plugin.manifest.id,"level":string(&params,"level")?,"message":message}));
                    Value::Null
                }
                "http.request" => handle.block_on(http(plugin,&params))?,
                "http.download" => {
                    let path = private_path(&private,string(&params,"path")?)?;
                    let plugin = plugin.clone();
                    let job = core.tasks.start(&format!("pluginDownload:{}",plugin.manifest.id),None)?;
                    let result = task(job.state.lock().unwrap().id.clone())?;
                    handle.spawn(async move {
                        let result = tokio::select! {
                            _ = job.cancel.cancelled() => Err(ApiError::new("CANCELLED","下载已取消。")),
                            result = download(&plugin, &params, &path, &job) => result,
                        };
                        job.finish(result);
                    });
                    result
                }
                "process.run" => {
                    let executable = string(&params,"program")?;
                    if !plugin.manifest.scopes.process.iter().any(|p| p == executable) || !PathBuf::from(executable).is_absolute() {
                        return Err(ApiError::new("RESOURCE_SCOPE","进程不在清单授权范围内。"));
                    }
                    let arguments: Vec<String> = serde_json::from_value(params["args"].clone())?;
                    if arguments.len() > 128 { return Err(ApiError::new("INVALID_PARAMS","参数数量超过限制。")); }
                    std::fs::create_dir_all(&private)?;
                    let mut child = tokio::process::Command::new(executable).args(arguments).current_dir(private)
                        .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true).spawn()?;
                    let job = core.tasks.start(&format!("pluginProcess:{}",plugin.manifest.id),None)?;
                    let result = task(job.state.lock().unwrap().id.clone())?;
                    handle.spawn(async move {
                        let result = tokio::select! {
                            _ = job.cancel.cancelled() => { let _ = child.kill().await; Err(ApiError::new("CANCELLED","进程已取消。")) },
                            result = child.wait() => result.map_err(ApiError::from).and_then(|s| if s.success() { Ok(()) } else { Err(ApiError::new("PROCESS_EXIT","进程以非零状态退出。")) }),
                        };
                        job.finish(result);
                    });
                    result
                }
                _ => return Err(ApiError::new("UNKNOWN_CAPABILITY","能力未实现。")),
            };
            Ok(value)
        }));
    }
    fn private_path(root: &std::path::Path, relative: &str) -> Result<PathBuf> {
        storage::within(root, relative)
    }
    fn client(plugin: &Plugin, url: &str) -> Result<reqwest::Client> {
        let hosts = plugin.manifest.scopes.http.clone();
        let url =
            reqwest::Url::parse(url).map_err(|_| ApiError::new("INVALID_URL", "URL 无效。"))?;
        if !matches!(url.scheme(), "https" | "http")
            || !url.username().is_empty()
            || url.password().is_some()
            || !hosts.iter().any(|h| Some(h.as_str()) == url.host_str())
        {
            return Err(ApiError::new(
                "RESOURCE_SCOPE",
                "网络域名不在清单授权范围内。",
            ));
        }
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() < 5
                    && matches!(attempt.url().scheme(), "https" | "http")
                    && hosts
                        .iter()
                        .any(|h| Some(h.as_str()) == attempt.url().host_str())
                {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()
            .map_err(|_| ApiError::new("NETWORK", "无法创建网络客户端。"))
    }
    async fn http(plugin: &Plugin, params: &Value) -> Result<Value> {
        let url = string(params, "url")?;
        let client = client(plugin, url)?;
        let mut response = match params["method"].as_str().unwrap_or("GET") {
            "GET" => client.get(url),
            "POST" => client
                .post(url)
                .body(params["body"].as_str().unwrap_or("").to_owned()),
            _ => return Err(ApiError::new("INVALID_PARAMS", "仅支持 GET 和 POST。")),
        }
        .send()
        .await
        .map_err(|_| ApiError::new("NETWORK", "网络请求失败。"))?;
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ApiError::new("NETWORK", "读取响应失败。"))?
        {
            if bytes.len() + chunk.len() > 192 * 1024 {
                return Err(ApiError::new("MESSAGE_LIMIT", "响应过大，请使用下载任务。"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(
            json!({"status":status,"text":String::from_utf8(bytes).map_err(|_| ApiError::new("INVALID_TEXT","响应不是 UTF-8。"))?}),
        )
    }
    async fn download(
        plugin: &Plugin,
        params: &Value,
        path: &std::path::Path,
        job: &crate::launcher::tasks::Job,
    ) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        let url = string(params, "url")?;
        let mut response = client(plugin, url)?
            .get(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| ApiError::new("NETWORK", "下载请求失败。"))?;
        let temporary = path.with_extension(format!("{}.part", uuid::Uuid::new_v4()));
        let mut guard = DownloadFile(temporary.clone());
        tokio::fs::create_dir_all(path.parent().unwrap()).await?;
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await?;
        let mut done = 0;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ApiError::new("NETWORK", "下载连接中断。"))?
        {
            job.check()?;
            done += chunk.len() as u64;
            if done > 1024 * 1024 * 1024 {
                return Err(ApiError::new("DOWNLOAD_LIMIT", "单次下载超过 1 GiB。"));
            }
            file.write_all(&chunk).await?;
            job.progress(done, response.content_length(), "正在下载");
        }
        file.sync_all().await?;
        drop(file);
        job.begin_commit("正在保存文件")?;
        if path.exists() {
            return Err(ApiError::new("OUTPUT_EXISTS", "目标文件已存在。"));
        }
        tokio::fs::rename(&temporary, path).await?;
        guard.0 = PathBuf::new();
        Ok(())
    }
    struct DownloadFile(PathBuf);
    impl Drop for DownloadFile {
        fn drop(&mut self) {
            if !self.0.as_os_str().is_empty() {
                let _ = std::fs::remove_file(&self.0);
            }
        }
    }
}
