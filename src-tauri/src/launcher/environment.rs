//! OS environment service transport. No native implementation is linked into the host.
use super::storage::Settings;
use crate::{contracts::Result, plugins::host::PluginHost};
#[cfg(feature = "desktop")]
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
const SERVICE: &str = "platform.windows.environment";

#[cfg(feature = "desktop")]
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AboutLink {
    Developer,
    Bmclapi,
    Releases,
    MiSansLicense,
}

pub fn choose(
    host: &PluginHost,
    settings: &Settings,
    owner: isize,
    folder: bool,
    multiple: bool,
) -> Result<Vec<PathBuf>> {
    Ok(serde_json::from_value(host.call_service_inline(
        settings,
        SERVICE,
        SERVICE,
        "choose",
        json!({"owner":owner,"folder":folder,"multiple":multiple}),
    )?)?)
}
pub fn browser(host: &PluginHost, settings: &Settings, url: &str) -> Result<()> {
    host.call_service(settings, SERVICE, SERVICE, "browser", json!({"url":url}))?;
    Ok(())
}
#[cfg(feature = "desktop")]
pub(crate) fn open_about_link(
    host: &PluginHost,
    settings: &Settings,
    link: AboutLink,
) -> Result<()> {
    host.call_service(
        settings,
        SERVICE,
        SERVICE,
        "openAboutLink",
        json!({ "link": link }),
    )?;
    Ok(())
}
pub fn open_folder(host: &PluginHost, settings: &Settings, path: &Path) -> Result<()> {
    host.call_service(
        settings,
        SERVICE,
        SERVICE,
        "openFolder",
        json!({"path":path}),
    )?;
    Ok(())
}
pub fn available_space(host: &PluginHost, settings: &Settings, path: &Path) -> Result<Option<u64>> {
    Ok(serde_json::from_value(host.call_service(
        settings,
        SERVICE,
        SERVICE,
        "availableSpace",
        json!({"path":path}),
    )?)?)
}
pub fn open_text(host: &PluginHost, settings: &Settings, path: &Path) -> Result<()> {
    host.call_service(settings, SERVICE, SERVICE, "openText", json!({"path":path}))?;
    Ok(())
}
