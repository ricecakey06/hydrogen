use super::{
    manager,
    manifest::{self, Plugin},
    native::NativeHost,
    runtime::{RunReport, RuntimeStats, ScriptHost},
};
use crate::{
    contracts::{ApiError, InstanceInfo, Result},
    launcher::storage::Settings,
};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// One execution boundary for installed, preinstalled and explicitly authorized development packages.
#[derive(Clone)]
pub struct PluginHost {
    inner: Arc<HostInner>,
}
#[doc(hidden)]
pub struct HostInner {
    pub themes: super::themes::Themes,
    scripts: ScriptHost,
    native: NativeHost,
    plugins: Arc<Mutex<Vec<Plugin>>>,
    operations: Mutex<Option<super::capabilities::Operations>>,
    settings: Mutex<Settings>,
    usage: Arc<Mutex<std::collections::BTreeMap<String, usize>>>,
    diagnostics: Mutex<std::collections::BTreeMap<String, Vec<String>>>,
}
pub struct PluginUse {
    counts: Arc<Mutex<std::collections::BTreeMap<String, usize>>>,
    id: String,
}
impl Drop for PluginUse {
    fn drop(&mut self) {
        let mut counts = self.counts.lock().unwrap();
        if let Some(count) = counts.get_mut(&self.id) {
            *count -= 1;
            if *count == 0 {
                counts.remove(&self.id);
            }
        }
    }
}
impl std::ops::Deref for PluginHost {
    type Target = HostInner;
    fn deref(&self) -> &HostInner {
        &self.inner
    }
}
impl PluginHost {
    pub fn lease(&self, id: &str) -> PluginUse {
        *self.usage.lock().unwrap().entry(id.into()).or_default() += 1;
        PluginUse {
            counts: self.usage.clone(),
            id: id.into(),
        }
    }
    pub fn busy(&self) -> bool {
        !self.usage.lock().unwrap().is_empty()
    }
    pub fn busy_plugins(&self, ids: &[String]) -> bool {
        self.usage.lock().unwrap().keys().any(|id| ids.contains(id))
    }
    pub fn diagnostics(&self, id: &str) -> Vec<String> {
        self.diagnostics
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .unwrap_or_default()
    }
    pub fn hooks(&self, settings: &Settings, name: &str, params: Value) -> Result<Value> {
        let mut patch = serde_json::Map::new();
        let mut plugins = self.plugins.lock().unwrap().clone();
        plugins.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        for plugin in plugins {
            if plugin.manifest.runtime != "javascript"
                || !plugin.manifest.permissions.iter().any(|p| p == "hooks")
                || !self.available(settings, &plugin.manifest.id)
            {
                continue;
            }
            let bridge = self.bridge(
                plugin.clone(),
                settings.clone(),
                vec![plugin.manifest.id.clone()],
                None,
            );
            let _usage = self.lease(&plugin.manifest.id);
            let result = self
                .scripts
                .call_blocking(
                    plugin,
                    ("hook".into(), name.into(), params.clone()),
                    Some(bridge),
                )?
                .result;
            if result.is_null() {
                continue;
            }
            let object = result
                .as_object()
                .ok_or_else(|| ApiError::new("INVALID_HOOK", "Hook 只能返回对象或 null。"))?;
            if object.keys().any(|k| k != "deny" && k != "patch") {
                return Err(ApiError::new("INVALID_HOOK", "Hook 包含未授权的修改字段。"));
            }
            if let Some(reason) = object.get("deny") {
                let reason = reason
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 4096)
                    .ok_or_else(|| ApiError::new("INVALID_HOOK", "拒绝理由无效。"))?;
                return Err(ApiError::new("HOOK_DENIED", reason));
            }
            if let Some(value) = object.get("patch") {
                let fields = value
                    .as_object()
                    .filter(|_| name == "beforeLaunch")
                    .ok_or_else(|| {
                        ApiError::new("INVALID_HOOK", "只有 beforeLaunch 可以返回启动配置补丁。")
                    })?;
                for (key, value) in fields {
                    let valid = match key.as_str() {
                        "maxMemoryMib" => value
                            .as_u64()
                            .is_some_and(|n| (512..=1_048_576).contains(&n)),
                        "jvmArguments" => value.as_str().is_some_and(|s| s.len() <= 4096),
                        _ => false,
                    };
                    if !valid {
                        return Err(ApiError::new(
                            "INVALID_HOOK",
                            "启动补丁包含无效或未授权字段。",
                        ));
                    }
                    if patch.get(key).is_some_and(|previous| previous != value) {
                        return Err(ApiError::new(
                            "HOOK_CONFLICT",
                            format!("多个 Hook 对 {key} 提供了不同值。"),
                        ));
                    }
                    patch.insert(key.clone(), value.clone());
                }
            }
        }
        Ok(Value::Object(patch))
    }
    pub fn set_main_executor(&self, dispatch: super::native::MainDispatch) {
        self.native.set_main_executor(dispatch);
    }
    pub fn stop_main(&self) -> Result<()> {
        self.native.stop_main(None)
    }

    pub fn new(handle: tokio::runtime::Handle, instances: Vec<InstanceInfo>) -> Self {
        Self {
            inner: Arc::new(HostInner {
                themes: super::themes::Themes::default(),
                scripts: ScriptHost::new(handle, instances),
                native: NativeHost::default(),
                plugins: Arc::default(),
                operations: Mutex::default(),
                settings: Mutex::new(Settings::default()),
                usage: Arc::default(),
                diagnostics: Mutex::default(),
            }),
        }
    }
    pub fn set_plugins(&self, plugins: Vec<Plugin>) {
        *self.plugins.lock().unwrap() = plugins;
    }
    pub fn set_operations(&self, operations: super::capabilities::Operations) {
        *self.operations.lock().unwrap() = Some(operations);
    }
    pub fn set_settings(&self, settings: &Settings) {
        *self.settings.lock().unwrap() = settings.clone();
    }
    pub fn emit(&self, event: &str, params: Value) {
        self.scripts.emit(event, params);
    }
    pub fn set_instances(&self, instances: Vec<InstanceInfo>) {
        self.scripts.set_instances(instances);
    }
    pub fn available(&self, settings: &Settings, id: &str) -> bool {
        self.checked(settings, id).is_ok()
    }
    pub(crate) fn checked(&self, settings: &Settings, id: &str) -> Result<Plugin> {
        let plugin = {
            let plugins = self.plugins.lock().unwrap();
            manager::order(&plugins, settings, id)?;
            plugins
                .iter()
                .find(|p| p.manifest.id == id)
                .cloned()
                .ok_or_else(|| ApiError::new("MISSING_DEPENDENCY", format!("缺少插件 {id}。")))?
        };
        let checked = manifest::load_runtime(&plugin.root)?;
        if checked.digest != plugin.digest {
            return Err(ApiError::new(
                "PLUGIN_CHANGED",
                "插件文件已变化，请校验并重载。",
            ));
        }
        Ok(checked)
    }
    fn service_plugin(
        &self,
        settings: &Settings,
        provider: &str,
        service: &str,
        method: &str,
    ) -> Result<Plugin> {
        let plugin = self.checked(settings, provider)?;
        let declared = plugin
            .manifest
            .provides
            .iter()
            .find(|s| s.id == service && s.methods.iter().any(|m| m == method));
        let Some(_declared) = declared else {
            return Err(ApiError::new("UNKNOWN_SERVICE", "插件未声明此服务方法。"));
        };
        Ok(plugin)
    }
    pub fn call_service(
        &self,
        settings: &Settings,
        provider: &str,
        service: &str,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        self.require_v1(settings, provider, service, method)?;
        self.call_service_path(
            settings,
            provider,
            service,
            method,
            params,
            Vec::new(),
            None,
        )
    }
    fn require_v1(
        &self,
        settings: &Settings,
        provider: &str,
        service: &str,
        method: &str,
    ) -> Result<()> {
        let plugin = self.service_plugin(settings, provider, service, method)?;
        let declared = plugin
            .manifest
            .provides
            .iter()
            .find(|s| s.id == service)
            .unwrap();
        if !semver::VersionReq::parse("^1")
            .unwrap()
            .matches(&semver::Version::parse(&declared.version).unwrap())
        {
            return Err(ApiError::new(
                "SERVICE_VERSION",
                "此宿主业务入口需要 v1 服务契约。",
            ));
        }
        Ok(())
    }
    fn call_service_path(
        &self,
        settings: &Settings,
        provider: &str,
        service: &str,
        method: &str,
        params: Value,
        mut path: Vec<String>,
        origin: Option<Plugin>,
    ) -> Result<Value> {
        let _usage = self.lease(provider);
        let mut params = hydrogen_plugin_sdk::JsonBuffer(params);
        if path.len() >= 32 || path.iter().any(|id| id == provider) {
            return Err(ApiError::new(
                "SERVICE_CYCLE",
                "插件服务调用存在循环或超过深度限制。",
            ));
        }
        let plugin = self.service_plugin(settings, provider, service, method)?;
        path.push(provider.into());
        let mut effective_method = method;
        if matches!(provider, "network" | "minecraft.download") {
            params.0["modMirror"] = Value::Bool(settings.use_mod_mirror);
        }
        let mod_file = params.0["url"].as_str().is_some_and(|value| {
            reqwest::Url::parse(value).is_ok_and(|url| {
                matches!(
                    url.host_str(),
                    Some(
                        "cdn.modrinth.com"
                            | "edge.forgecdn.net"
                            | "mediafilez.forgecdn.net"
                            | "media.forgecdn.net"
                            | "mediafiles.forgecdn.net"
                            | "mod.mcimirror.top"
                    )
                )
            })
        });
        if provider == "minecraft.download" && !mod_file {
            if let Some(id) = settings.download_source.strip_prefix("minecraft.download:") {
                params.0["mirror"] = Value::Bool(id == "bmclapi");
            } else if settings.download_source.contains(':')
                && matches!(method, "sourceUrl" | "transfer" | "trusted")
            {
                let (source, service) =
                    self.provider(settings, "downloadSource", &settings.download_source)?;
                let url = super::capabilities::string(&params.0, "url")?.to_owned();
                if method == "trusted" {
                    let allowed = reqwest::Url::parse(&url).is_ok_and(|url| {
                        url.scheme() == "https"
                            && source
                                .manifest
                                .scopes
                                .http
                                .iter()
                                .any(|h| Some(h.as_str()) == url.host_str())
                    });
                    if allowed {
                        return Ok(Value::Bool(true));
                    }
                } else {
                    let response = self.call_service_path(
                        settings,
                        &source.manifest.id,
                        &service,
                        "resolve",
                        serde_json::json!({"url":url}),
                        path.clone(),
                        origin.clone(),
                    )?;
                    let url = super::capabilities::string(&response, "url")?;
                    let parsed = reqwest::Url::parse(url)
                        .map_err(|_| ApiError::new("INVALID_URL", "下载源返回无效地址。"))?;
                    if parsed.scheme() != "https"
                        || !parsed.username().is_empty()
                        || parsed.password().is_some()
                        || !source
                            .manifest
                            .scopes
                            .http
                            .iter()
                            .any(|h| Some(h.as_str()) == parsed.host_str())
                    {
                        return Err(ApiError::new(
                            "RESOURCE_SCOPE",
                            "下载源返回的域名未在清单授权。",
                        ));
                    }
                    if method == "sourceUrl" {
                        return Ok(Value::String(url.into()));
                    }
                    params.0["url"] = Value::String(url.into());
                    effective_method = "once";
                }
            }
        }
        if let Some(origin) = &origin {
            super::capabilities::service_access(origin, &plugin, service, method)?;
        }
        let bridge = self.bridge(plugin.clone(), settings.clone(), path, origin);
        if plugin.manifest.runtime == "javascript" {
            return Ok(self
                .scripts
                .call_blocking(
                    plugin,
                    (
                        "service".into(),
                        format!("{service}/{method}"),
                        params.0.take(),
                    ),
                    Some(bridge),
                )?
                .result);
        }
        let router = Arc::new(move |request| bridge(0, "services.call", request));
        self.native.call_routed(
            plugin,
            effective_method.into(),
            params.0.take(),
            Some(router),
        )
    }
    fn bridge(
        &self,
        caller: Plugin,
        settings: Settings,
        path: Vec<String>,
        origin: Option<Plugin>,
    ) -> super::runtime::Bridge {
        let host = Arc::downgrade(&self.inner);
        let origin =
            origin.or_else(|| (caller.manifest.runtime == "javascript").then(|| caller.clone()));
        Arc::new(move |generation, method, mut value| {
            let host = PluginHost {
                inner: host
                    .upgrade()
                    .ok_or_else(|| ApiError::new("HOST_STOPPED", "插件宿主已停止。"))?,
            };
            if method == "lifecycle.stop" {
                if let Some(operations) = host.operations.lock().unwrap().clone() {
                    return operations(&caller, generation, method, Value::Null);
                }
                return Ok(Value::Null);
            }
            if method == "lifecycle.diagnostic" {
                let message = super::capabilities::string(&value, "message")?;
                let mut diagnostics = host.diagnostics.lock().unwrap();
                let entries = diagnostics.entry(caller.manifest.id.clone()).or_default();
                if entries.len() == 32 {
                    entries.remove(0);
                }
                entries.push(message.into());
                return Ok(Value::Null);
            }
            if method == "services.call" {
                if caller.manifest.runtime == "javascript" {
                    super::capabilities::permission(&caller, "services.call")?;
                }
                let provider = super::capabilities::string(&value, "provider")?.to_owned();
                if provider == "host.providers" && caller.manifest.runtime == "native" {
                    let kind = super::capabilities::string(&value, "service")?;
                    let method = super::capabilities::string(&value, "method")?;
                    if method == "list" {
                        return Ok(host.providers(&settings, kind));
                    }
                    let id = super::capabilities::string(&value["params"], "id")?;
                    let target = host.provider(&settings, kind, id);
                    if method == "__available" {
                        return Ok(Value::Bool(target.is_ok()));
                    }
                    let (target, service) = target?;
                    return host.call_service_path(
                        &settings,
                        &target.manifest.id,
                        &service,
                        method,
                        value["params"]["params"].clone(),
                        path.clone(),
                        origin.clone(),
                    );
                }

                let service = super::capabilities::string(&value, "service")?.to_owned();
                let method = super::capabilities::string(&value, "method")?.to_owned();
                let requirement = caller
                    .manifest
                    .dependencies
                    .iter()
                    .filter_map(|dep| manifest::dependency(dep).ok())
                    .find(|(id, _)| *id == provider)
                    .ok_or_else(|| {
                        ApiError::new("UNDECLARED_DEPENDENCY", "插件未声明此服务提供者依赖。")
                    })?
                    .1;
                let available = service.is_empty() && method == "__available";
                let target = match host.checked(&settings, &provider) {
                    Ok(p) => p,
                    Err(_) if available => return Ok(Value::Bool(false)),
                    Err(e) => return Err(e),
                };
                if !semver::Version::parse(&target.manifest.version)
                    .is_ok_and(|v| requirement.matches(&v))
                {
                    if available {
                        return Ok(Value::Bool(false));
                    }
                    return Err(ApiError::new(
                        "DEPENDENCY_VERSION",
                        "服务提供者版本不兼容。",
                    ));
                }
                if available {
                    return Ok(Value::Bool(true));
                }
                let version = value["version"].as_str().unwrap_or("^1");
                let range = semver::VersionReq::parse(version)
                    .map_err(|_| ApiError::new("SERVICE_VERSION", "服务版本范围无效。"))?;
                let declaration = target
                    .manifest
                    .provides
                    .iter()
                    .find(|s| s.id == service)
                    .ok_or_else(|| ApiError::new("UNKNOWN_SERVICE", "服务不存在。"))?;
                if !range.matches(&semver::Version::parse(&declaration.version).unwrap()) {
                    return Err(ApiError::new("SERVICE_VERSION", "服务契约版本不兼容。"));
                }
                host.call_service_path(
                    &settings,
                    &provider,
                    &service,
                    &method,
                    value["params"].take(),
                    path.clone(),
                    origin.clone(),
                )
            } else if method == "services.list" {
                super::capabilities::permission(&caller, "services.call")?;
                Ok(host.service_catalog(&settings))
            } else {
                let _usage = host.lease(&caller.manifest.id);
                super::capabilities::authorize(&caller, method)?;
                let mut effective = caller.clone();
                if let Some(origin) = &origin {
                    super::capabilities::authorize(origin, method)?;
                    effective
                        .manifest
                        .scopes
                        .http
                        .retain(|host| origin.manifest.scopes.http.contains(host));
                    effective
                        .manifest
                        .scopes
                        .process
                        .retain(|program| origin.manifest.scopes.process.contains(program));
                }
                // Business adapters re-enter installed services. Reject a return to a native
                // provider already waiting for this JS handler before its worker queue can block.
                let provider = match method.split('.').next().unwrap_or("") {
                    "accounts" => "account.manager",
                    "instances" if method == "instances.repair" || method == "instances.create" => {
                        "minecraft.install"
                    }
                    "instances" if method == "instances.launch" => "minecraft.launch",
                    "instances" => "minecraft.instances",
                    "settings" => "storage",
                    "java" => "java.runtime",
                    "resources" => "minecraft.resources",
                    "mods" => "minecraft.mods",
                    "modpacks" => "minecraft.modpacks",
                    _ => "",
                };
                if !provider.is_empty() && path.iter().any(|id| id == provider) {
                    return Err(ApiError::new(
                        "SERVICE_CYCLE",
                        "业务 API 不能回调正在等待此服务的提供者。",
                    ));
                }
                let operations = host.operations.lock().unwrap().clone();
                if let Some(operations) = operations {
                    operations(&effective, generation, method, value)
                } else if method == "instances.list" {
                    Ok(serde_json::to_value(host.scripts.instances())?)
                } else {
                    Err(ApiError::new(
                        "CAPABILITY_UNAVAILABLE",
                        "此宿主尚未连接业务操作。",
                    ))
                }
            }
        })
    }
    pub fn provider(&self, settings: &Settings, kind: &str, id: &str) -> Result<(Plugin, String)> {
        let (plugin_id, provider_id) = id
            .split_once(':')
            .ok_or_else(|| ApiError::new("UNKNOWN_PROVIDER", "提供者 ID 无效。"))?;
        let plugin = self.checked(settings, plugin_id)?;
        let service = plugin
            .manifest
            .contributes
            .providers
            .iter()
            .find(|p| p.id == provider_id && p.kind == kind)
            .ok_or_else(|| ApiError::new("UNKNOWN_PROVIDER", "提供者不存在或类型不匹配。"))?
            .service
            .clone();
        Ok((plugin, service))
    }
    pub async fn provider_call(
        &self,
        settings: &Settings,
        kind: &str,
        id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        let (plugin_id, provider_id) = id
            .split_once(':')
            .ok_or_else(|| ApiError::new("UNKNOWN_PROVIDER", "提供者 ID 无效。"))?;
        let plugin = self.checked(settings, plugin_id)?;
        let provider = plugin
            .manifest
            .contributes
            .providers
            .iter()
            .find(|p| p.id == provider_id && p.kind == kind)
            .ok_or_else(|| ApiError::new("UNKNOWN_PROVIDER", "提供者不存在或契约类型不匹配。"))?;
        let host = self.clone();
        let settings = settings.clone();
        let plugin_id = plugin_id.to_owned();
        let service = provider.service.clone();
        let method = method.to_owned();
        tokio::task::spawn_blocking(move || {
            host.call_service(&settings, &plugin_id, &service, &method, params)
        })
        .await
        .map_err(|_| ApiError::new("PROVIDER_ERROR", "提供者执行失败。"))?
    }
    pub async fn panel_task(
        &self,
        plugin: Plugin,
        generation: u64,
        method: String,
        params: Value,
    ) -> Result<RunReport> {
        if !matches!(method.as_str(), "tasks.get" | "tasks.cancel") {
            return Err(ApiError::new(
                "UNKNOWN_CAPABILITY",
                "面板只能查询或取消自己的任务。",
            ));
        }
        let operations = self
            .operations
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| ApiError::new("CAPABILITY_UNAVAILABLE", "任务入口不可用。"))?;
        let reported = plugin.clone();
        let result = tokio::task::spawn_blocking(move || {
            super::capabilities::authorize(&plugin, &method)?;
            operations(&plugin, generation, &method, params)
        })
        .await
        .map_err(|_| ApiError::new("HOST_ERROR", "任务操作失败。"))??;
        Ok(RunReport {
            plugin_id: reported.manifest.id,
            generation,
            result,
            notifications: Vec::new(),
            heap_bytes: 0,
            registrations: 0,
        })
    }
    pub fn service_catalog(&self, settings: &Settings) -> Value {
        let plugins = self.plugins.lock().unwrap().clone();
        serde_json::json!(plugins.iter().filter(|p| manager::order(&plugins, settings, &p.manifest.id).is_ok())
            .flat_map(|p| p.manifest.provides.iter().flat_map(move |s| s.methods.iter().map(move |m|
                serde_json::json!({"provider":p.manifest.id,"service":s.id,"version":s.version,"method":m,
                    "access":super::capabilities::service_policy(p,s.id.as_str(),m),
                    "maxMessageBytes":manifest::MAX_MESSAGE,"timeoutMs":25000})
            ))).collect::<Vec<_>>())
    }
    pub fn providers(&self, settings: &Settings, kind: &str) -> Value {
        let plugins = self.plugins.lock().unwrap().clone();
        serde_json::json!(plugins.iter().filter(|p| manager::order(&plugins,settings,&p.manifest.id).is_ok())
            .flat_map(|p| p.manifest.contributes.providers.iter().filter(|c| c.kind == kind).map(move |c|
                serde_json::json!({"id":format!("{}:{}",p.manifest.id,c.id),"pluginId":p.manifest.id,
                    "service":c.service,"title":c.title,"version":c.version})
            )).collect::<Vec<_>>())
    }
    /// Calls which must execute on an OS UI thread use the same package validation boundary.
    pub fn call_service_inline(
        &self,
        settings: &Settings,
        provider: &str,
        service: &str,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        self.require_v1(settings, provider, service, method)?;
        let mut params = hydrogen_plugin_sdk::JsonBuffer(params);
        self.native.call_inline(
            self.service_plugin(settings, provider, service, method)?,
            method.into(),
            params.0.take(),
        )
    }
    pub async fn run(&self, plugin: Plugin, command: String, reload: bool) -> Result<RunReport> {
        self.run_params(plugin, command, Value::Null, reload).await
    }
    pub async fn run_params(
        &self,
        plugin: Plugin,
        command: String,
        params: Value,
        reload: bool,
    ) -> Result<RunReport> {
        let _usage = self.lease(&plugin.manifest.id);
        if !plugin
            .manifest
            .contributes
            .commands
            .iter()
            .any(|c| c.id == command)
        {
            return Err(ApiError::new("UNKNOWN_COMMAND", "命令未在清单中声明。"));
        }
        if serde_json::to_vec(&params)?.len() > manifest::MAX_MESSAGE {
            return Err(ApiError::new("MESSAGE_LIMIT", "命令参数超过限制。"));
        }
        let settings = self.settings.lock().unwrap().clone();
        let bridge = self.bridge(
            plugin.clone(),
            settings,
            vec![plugin.manifest.id.clone()],
            None,
        );
        if plugin.manifest.runtime == "javascript" {
            return self
                .scripts
                .dispatch(
                    plugin,
                    Some(("command".into(), command, params)),
                    reload,
                    None,
                    Some(bridge),
                )
                .await;
        }
        let native = self.native.clone();
        let reported = plugin.clone();
        let value = tokio::task::spawn_blocking(move || {
            if reload {
                native.reload(plugin.clone(), || Ok(()))?;
            }
            native.call_routed(
                plugin,
                command,
                params,
                Some(Arc::new(move |request| bridge(0, "services.call", request))),
            )
        })
        .await
        .map_err(|_| ApiError::new("NATIVE_STOPPED", "原生执行线程停止。"))??;
        Ok(self.report(&reported, value))
    }
    fn report(&self, plugin: &Plugin, result: Value) -> RunReport {
        RunReport {
            plugin_id: plugin.manifest.id.clone(),
            generation: self.native.generation(&plugin.manifest.id),
            result,
            notifications: Vec::new(),
            heap_bytes: 0,
            registrations: plugin.manifest.provides.len()
                + plugin.manifest.contributes.commands.len(),
        }
    }
    pub async fn activate(&self, plugin: Plugin) -> Result<RunReport> {
        if super::themes::standalone(&plugin) {
            return Ok(self.report(&plugin, Value::Null));
        }
        let _usage = self.lease(&plugin.manifest.id);
        if plugin.manifest.runtime == "javascript" {
            let settings = self.settings.lock().unwrap().clone();
            let bridge = self.bridge(
                plugin.clone(),
                settings,
                vec![plugin.manifest.id.clone()],
                None,
            );
            return self
                .scripts
                .dispatch(plugin, None, false, None, Some(bridge))
                .await;
        }
        let native = self.native.clone();
        let reported = plugin.clone();
        tokio::task::spawn_blocking(move || {
            native.call(plugin, "plugin.describe".into(), Value::Null)
        })
        .await
        .map_err(|_| ApiError::new("NATIVE_STOPPED", "原生执行线程停止。"))??;
        Ok(self.report(&reported, Value::Null))
    }
    pub async fn reload_with_commit(
        &self,
        plugin: Plugin,
        commit: impl FnOnce() -> Result<()> + Send + 'static,
    ) -> Result<RunReport> {
        if super::themes::standalone(&plugin) {
            super::themes::validate(&plugin)?;
            commit()?;
            return Ok(self.report(&plugin, Value::Null));
        }
        if plugin.manifest.runtime == "javascript" {
            let settings = self.settings.lock().unwrap().clone();
            let bridge = self.bridge(
                plugin.clone(),
                settings,
                vec![plugin.manifest.id.clone()],
                None,
            );
            return self
                .scripts
                .dispatch(plugin, None, true, Some(Box::new(commit)), Some(bridge))
                .await;
        }
        let native = self.native.clone();
        let reported = plugin.clone();
        tokio::task::spawn_blocking(move || native.reload(plugin, commit))
            .await
            .map_err(|_| ApiError::new("NATIVE_STOPPED", "原生执行线程停止。"))??;
        Ok(self.report(&reported, Value::Null))
    }
    pub async fn stop_all(&self) -> Result<RuntimeStats> {
        self.stop(None).await
    }
    pub async fn stop_plugins(&self, ids: Vec<String>) -> Result<RuntimeStats> {
        self.stop(Some(ids)).await
    }
    async fn stop(&self, ids: Option<Vec<String>>) -> Result<RuntimeStats> {
        let native = self.native.clone();
        let plugins = self.plugins.lock().unwrap().clone();
        fn visit(
            id: &str,
            plugins: &[Plugin],
            seen: &mut std::collections::BTreeSet<String>,
            ordered: &mut Vec<String>,
        ) {
            if !seen.insert(id.into()) {
                return;
            }
            if let Some(plugin) = plugins.iter().find(|p| p.manifest.id == id) {
                for dep in &plugin.manifest.dependencies {
                    if let Ok((id, _)) = manifest::dependency(dep) {
                        visit(id, plugins, seen, ordered);
                    }
                }
            }
            ordered.push(id.into());
        }
        let mut ordered = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for plugin in &plugins {
            visit(&plugin.manifest.id, &plugins, &mut seen, &mut ordered);
        }
        ordered.reverse();
        ordered.retain(|id| ids.as_ref().is_none_or(|ids| ids.contains(id)));
        let native_ids = ids.clone();
        tokio::task::spawn_blocking(move || {
            for id in ordered {
                let _ = native.shutdown(&id);
            }
            native.stop(native_ids.as_deref())
        })
        .await
        .map_err(|_| ApiError::new("NATIVE_STOPPED", "原生执行线程停止。"))??;
        let mut stats = if let Some(ids) = ids {
            self.scripts.stop_plugins(ids).await?
        } else {
            self.scripts.stop_all().await?
        };
        let native = self.native.stats();
        stats.active += native.active;
        stats.created += native.created;
        stats.destroyed += native.destroyed;
        stats.registrations += native.registrations;
        Ok(stats)
    }
}
