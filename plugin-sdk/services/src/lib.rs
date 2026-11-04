pub mod contracts;
pub mod http;
pub mod jobs;
pub mod net;
pub mod storage;
use contracts::{ApiError, Result};
use serde_json::Value;
#[derive(Clone, Default)]
pub struct PluginHost;
impl PluginHost {
    pub fn call_service(
        &self,
        _settings: &Settings,
        provider: &str,
        service: &str,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        invoke(provider, service, method, params)
    }
    pub fn available(&self, settings: &Settings, id: &str) -> bool {
        settings.enabled(id) && invoke(id, "", "__available", Value::Null).is_ok_and(|v| v == true)
    }
}
pub fn invoke(provider: &str, service: &str, method: &str, params: Value) -> Result<Value> {
    hydrogen_plugin_sdk::call_service(provider, service, method, params).map_err(|error| {
        let mut e = ApiError::new(&error.code, error.message);
        e.retryable = e.code == "NETWORK";
        e
    })
}
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub download_source: String,
    #[serde(default)]
    pub microsoft_client_id: String,
    #[serde(default)]
    pub account_mode: String,
    #[serde(default)]
    pub offline_player_name: String,
    #[serde(default)]
    pub modules: std::collections::BTreeMap<String, bool>,
}
impl Settings {
    pub fn enabled(&self, id: &str) -> bool {
        self.modules.get(id).copied().unwrap_or(true)
    }
    pub fn offline_selected(&self) -> bool {
        self.account_mode == "offline" && self.enabled("account.offline")
    }
}
pub fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| ApiError::new("INVALID_METADATA", format!("服务返回的 {key} 字段无效。")))
}
