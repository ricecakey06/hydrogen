//! DTO and transport only. Java discovery, parsing and probing live in java.runtime.
use super::storage::Settings;
use crate::{contracts::Result, plugins::host::PluginHost};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Java {
    pub path: String,
    pub version: String,
    pub major: u32,
    pub architecture: String,
}
pub fn parse(
    host: &PluginHost,
    settings: &Settings,
    path: &Path,
    output: &str,
) -> Result<Option<Java>> {
    Ok(serde_json::from_value(host.call_service(
        settings,
        "java.runtime",
        "java.runtime",
        "parse",
        serde_json::json!({"path":path,"output":output}),
    )?)?)
}
pub async fn inspect(host: &PluginHost, settings: &Settings, path: &Path) -> Result<Java> {
    let host = host.clone();
    let settings = settings.clone();
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        Ok(serde_json::from_value(host.call_service(
            &settings,
            "java.runtime",
            "java.runtime",
            "inspect",
            serde_json::json!({"path":path}),
        )?)?)
    })
    .await
    .map_err(|_| crate::contracts::ApiError::new("NATIVE_STOPPED", "Java 插件执行线程停止。"))?
}
pub async fn scan(host: &PluginHost, settings: &Settings, saved: Vec<String>) -> Result<Vec<Java>> {
    let host = host.clone();
    let settings = settings.clone();
    tokio::task::spawn_blocking(move || {
        Ok(serde_json::from_value(host.call_service(
            &settings,
            "java.runtime",
            "java.runtime",
            "scan",
            serde_json::json!({"saved":saved}),
        )?)?)
    })
    .await
    .map_err(|_| crate::contracts::ApiError::new("NATIVE_STOPPED", "Java 插件执行线程停止。"))?
}
