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
    scripts: ScriptHost,
    native: NativeHost,
    plugins: Arc<Mutex<Vec<Plugin>>>,
}
impl std::ops::Deref for PluginHost {
    type Target = HostInner;
    fn deref(&self) -> &HostInner {
        &self.inner
    }
}
impl PluginHost {
    pub fn set_main_executor(&self, dispatch: super::native::MainDispatch) {
        self.native.set_main_executor(dispatch);
    }
    pub fn stop_main(&self) -> Result<()> {
        self.native.stop_main(None)
    }

    pub fn new(handle: tokio::runtime::Handle, instances: Vec<InstanceInfo>) -> Self {
        Self {
            inner: Arc::new(HostInner {
                scripts: ScriptHost::new(handle, instances),
                native: NativeHost::default(),
                plugins: Arc::default(),
            }),
        }
    }
    pub fn set_plugins(&self, plugins: Vec<Plugin>) {
        *self.plugins.lock().unwrap() = plugins;
    }
    pub fn set_instances(&self, instances: Vec<InstanceInfo>) {
        self.scripts.set_instances(instances);
    }
    pub fn available(&self, settings: &Settings, id: &str) -> bool {
        self.checked(settings, id).is_ok()
    }
    pub(super) fn checked(&self, settings: &Settings, id: &str) -> Result<Plugin> {
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
        let Some(declared) = declared else {
            return Err(ApiError::new("UNKNOWN_SERVICE", "插件未声明此服务方法。"));
        };
        if !semver::VersionReq::parse("^1").unwrap().matches(
            &semver::Version::parse(&declared.version)
                .map_err(|_| ApiError::new("SERVICE_VERSION", "服务版本无效。"))?,
        ) {
            return Err(ApiError::new(
                "SERVICE_VERSION",
                "当前调用方需要此服务的 v1 契约。",
            ));
        }
        if plugin.manifest.runtime != "native" {
            return Err(ApiError::new(
                "UNSUPPORTED_SERVICE_RUNTIME",
                "此服务需要原生运行时。",
            ));
        }
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
        self.call_service_path(settings, provider, service, method, params, Vec::new())
    }
    fn call_service_path(
        &self,
        settings: &Settings,
        provider: &str,
        service: &str,
        method: &str,
        params: Value,
        mut path: Vec<String>,
    ) -> Result<Value> {
        let mut params = hydrogen_plugin_sdk::JsonBuffer(params);
        if path.len() >= 32 || path.iter().any(|id| id == provider) {
            return Err(ApiError::new(
                "SERVICE_CYCLE",
                "插件服务调用存在循环或超过深度限制。",
            ));
        }
        let plugin = self.service_plugin(settings, provider, service, method)?;
        path.push(provider.into());
        let caller = plugin.clone();
        let host = Arc::downgrade(&self.inner);
        let settings = settings.clone();
        let router = Arc::new(move |request: Value| -> Result<Value> {
            let host = PluginHost {
                inner: host
                    .upgrade()
                    .ok_or_else(|| ApiError::new("HOST_STOPPED", "插件宿主已停止。"))?,
            };
            let mut request = hydrogen_plugin_sdk::JsonBuffer(request);
            let value = &mut request.0;
            let provider = value["provider"]
                .as_str()
                .ok_or_else(|| ApiError::new("INVALID_SERVICE", "缺少服务提供者。"))?
                .to_owned();
            if !caller
                .manifest
                .dependencies
                .iter()
                .any(|dep| manifest::dependency(dep).is_ok_and(|(id, _)| id == provider))
            {
                return Err(ApiError::new(
                    "UNDECLARED_DEPENDENCY",
                    "插件未声明此服务提供者依赖。",
                ));
            }
            let service = value["service"]
                .as_str()
                .ok_or_else(|| ApiError::new("INVALID_SERVICE", "缺少服务标识。"))?
                .to_owned();
            let method = value["method"]
                .as_str()
                .ok_or_else(|| ApiError::new("INVALID_SERVICE", "缺少服务方法。"))?
                .to_owned();
            let availability = service.is_empty() && method == "__available";
            let target = match host.checked(&settings, &provider) {
                Ok(target) => target,
                Err(_) if availability => return Ok(Value::Bool(false)),
                Err(error) => return Err(error),
            };
            let requirement = caller
                .manifest
                .dependencies
                .iter()
                .filter_map(|dep| manifest::dependency(dep).ok())
                .find(|(id, _)| *id == provider)
                .unwrap()
                .1;
            if !semver::Version::parse(&target.manifest.version)
                .is_ok_and(|v| requirement.matches(&v))
            {
                if availability {
                    return Ok(Value::Bool(false));
                }
                return Err(ApiError::new(
                    "DEPENDENCY_VERSION",
                    "服务提供者版本不兼容。",
                ));
            }
            if availability {
                return Ok(Value::Bool(true));
            }
            host.call_service_path(
                &settings,
                &provider,
                &service,
                &method,
                value["params"].take(),
                path.clone(),
            )
        });
        self.native
            .call_routed(plugin, method.into(), params.0.take(), Some(router))
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
        let mut params = hydrogen_plugin_sdk::JsonBuffer(params);
        self.native.call_inline(
            self.service_plugin(settings, provider, service, method)?,
            method.into(),
            params.0.take(),
        )
    }
    pub async fn run(&self, plugin: Plugin, command: String, reload: bool) -> Result<RunReport> {
        if plugin.manifest.runtime == "javascript" {
            return self.scripts.run(plugin, command, reload).await;
        }
        if !plugin
            .manifest
            .contributes
            .commands
            .iter()
            .any(|c| c.id == command)
        {
            return Err(ApiError::new("UNKNOWN_COMMAND", "命令未在清单中声明。"));
        }
        let native = self.native.clone();
        let reported = plugin.clone();
        let value = tokio::task::spawn_blocking(move || {
            if reload {
                native.reload(plugin.clone(), || Ok(()))?;
            }
            native.call(plugin, command, Value::Null)
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
        if plugin.manifest.runtime == "javascript" {
            return self.scripts.activate(plugin).await;
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
        if plugin.manifest.runtime == "javascript" {
            return self.scripts.reload_with_commit(plugin, commit).await;
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
