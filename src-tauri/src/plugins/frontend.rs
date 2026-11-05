//! Generic installed frontend resources. No UI source is embedded in the host.
use super::{host::PluginHost, manifest};
use crate::{
    contracts::{ApiError, Result},
    launcher::storage::Settings,
};
use serde_json::{Value, json};
pub fn bootstrap(host: &PluginHost, settings: &Settings, provider: &str) -> Result<Vec<Value>> {
    fn visit(
        host: &PluginHost,
        settings: &Settings,
        id: &str,
        seen: &mut std::collections::BTreeSet<String>,
        out: &mut Vec<Value>,
    ) -> Result<()> {
        if !seen.insert(id.into()) {
            return Ok(());
        }
        let plugin = host.checked(settings, id)?;
        let frontend = plugin
            .manifest
            .frontend
            .as_ref()
            .ok_or_else(|| ApiError::new("MISSING_FRONTEND", "插件未声明界面入口。"))?;
        if !plugin.manifest.permissions.iter().any(|p| p == "ui.host") {
            return Err(ApiError::new("PERMISSION_DENIED", "缺少 ui.host 权限。"));
        }
        for dependency in &plugin.manifest.dependencies {
            let (id, requirement) = manifest::dependency(dependency)?;
            let plugin = host.checked(settings, id)?;
            if !semver::Version::parse(&plugin.manifest.version)
                .is_ok_and(|v| requirement.matches(&v))
            {
                return Err(ApiError::new("DEPENDENCY_VERSION", "界面依赖版本不兼容。"));
            }
            if plugin.manifest.frontend.is_some() {
                visit(host, settings, id, seen, out)?;
            }
        }
        out.push(
            json!({"id":id,"digest":plugin.digest,"entry":frontend.entry,"styles":frontend.styles}),
        );
        Ok(())
    }
    let mut result = Vec::new();
    visit(
        host,
        settings,
        provider,
        &mut Default::default(),
        &mut result,
    )?;
    Ok(result)
}
pub fn resource(
    host: &PluginHost,
    settings: &Settings,
    path: &str,
) -> Result<(Vec<u8>, &'static str)> {
    let path = path.strip_prefix('/').unwrap_or(path);
    let (id, file) = path
        .split_once('/')
        .ok_or_else(|| ApiError::new("INVALID_PATH", "无效界面资源路径。"))?;
    let plugin = host.checked(settings, id)?;
    if plugin.manifest.frontend.is_none()
        || !plugin.manifest.permissions.iter().any(|p| p == "ui.host")
        || !file.starts_with("assets/")
    {
        return Err(ApiError::new("PERMISSION_DENIED", "未授权界面资源。"));
    }
    // Resources are read from the live package, not the verified native shadow.
    if manifest::load(&plugin.root)?.digest != plugin.digest {
        return Err(ApiError::new("PLUGIN_CHANGED", "界面插件文件已变化。"));
    }
    let path = manifest::within(&plugin.root, file)?;
    let mime = match path.extension().and_then(|v| v.to_str()) {
        Some("js") => "text/javascript",
        Some("css") => "text/css",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("svg") => "image/svg+xml",
        Some("woff2") => "font/woff2",
        _ => return Err(ApiError::new("INVALID_RESOURCE", "不支持此界面资源类型。")),
    };
    if path.metadata()?.len() > 16 * 1024 * 1024 {
        return Err(ApiError::new("RESOURCE_LIMIT", "界面资源超过限制。"));
    }
    Ok((std::fs::read(path)?, mime))
}
