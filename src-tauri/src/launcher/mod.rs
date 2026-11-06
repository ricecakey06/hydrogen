#[cfg(feature = "desktop")]
pub mod accounts;
#[cfg(feature = "desktop")]
pub mod core;
pub mod credentials;
pub mod diagnostics;
pub mod environment;
pub mod http;
pub mod instance_icons;
pub mod java;
#[cfg(feature = "desktop")]
pub mod launch;
#[cfg(feature = "desktop")]
pub mod migration;
pub mod minecraft;
#[cfg(feature = "desktop")]
pub mod modpacks;
pub mod mods;
pub mod net;
pub mod process;
pub mod repository;
pub mod storage;
pub mod tasks;

pub fn text<'a>(value: &'a serde_json::Value, key: &str) -> crate::contracts::Result<&'a str> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            crate::contracts::ApiError::new(
                "INVALID_METADATA",
                format!("服务返回的 {key} 字段无效。"),
            )
        })
}
