//! OS environment service transport. No native implementation is linked into the host.
use super::storage::Settings;
use crate::contracts::Result;
use hydrogen_plugin_services::PluginHost;
use serde_json::json;

const SERVICE: &str = "platform.windows.environment";
pub fn browser(host: &PluginHost, settings: &Settings, url: &str) -> Result<()> {
    host.call_service(settings, SERVICE, SERVICE, "browser", json!({"url":url}))?;
    Ok(())
}
