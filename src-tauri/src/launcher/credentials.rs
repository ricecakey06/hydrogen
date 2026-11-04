//! Credential transport; DPAPI and its buffers belong to the credentials plugin.
use super::storage::Settings;
use crate::{
    contracts::{ApiError, Result},
    plugins::host::PluginHost,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;
pub fn transform(
    host: &PluginHost,
    settings: &Settings,
    value: &mut [u8],
    protect: bool,
) -> Result<Vec<u8>> {
    let response = host.call_service(
        settings,
        "credentials",
        "credentials",
        if protect { "protect" } else { "unprotect" },
        json!({"data":STANDARD.encode(&*value)}),
    );
    value.fill(0);
    let response = hydrogen_plugin_sdk::JsonBuffer(response?);
    STANDARD
        .decode(
            response.0["data"].as_str().ok_or_else(|| {
                ApiError::new("CREDENTIAL_PROTECTION", "凭据服务返回了无效编码。")
            })?,
        )
        .map_err(|_| ApiError::new("CREDENTIAL_PROTECTION", "凭据服务返回了无效编码。"))
}
