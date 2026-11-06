use super::manifest::{self, Plugin};
use crate::contracts::{ApiError, Result};
use serde::Serialize;
use std::{collections::HashMap, fs, sync::Mutex};

pub const PANEL_CSP: &str = "sandbox allow-scripts; font-src 'self' http://hydrogen-plugin.localhost hydrogen-plugin:; default-src 'none'; script-src 'self' http://hydrogen-plugin.localhost hydrogen-plugin:; style-src 'self' http://hydrogen-plugin.localhost hydrogen-plugin:; img-src 'self' data: http://hydrogen-plugin.localhost hydrogen-plugin:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; navigate-to 'none'";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelSession {
    pub session_id: String,
    pub nonce: String,
    pub url: String,
    pub generation: u64,
    pub appearance: String,
}
struct BoundSession {
    plugin: Plugin,
    generation: u64,
    appearance: String,
    theme: Option<(u64, String)>,
    previous_theme: Option<String>,
}
#[derive(Default)]
pub struct Panels {
    sessions: Mutex<HashMap<String, BoundSession>>,
}

impl Panels {
    pub fn open(&self, plugin: Plugin, panel: &str, generation: u64) -> Result<PanelSession> {
        if !plugin.manifest.permissions.iter().any(|p| p == "ui.panel") {
            return Err(ApiError::new(
                "PERMISSION_DENIED",
                "插件未声明自定义面板能力。",
            ));
        }
        let panel = plugin
            .manifest
            .contributes
            .panels
            .iter()
            .find(|p| p.id == panel)
            .ok_or_else(|| ApiError::new("UNKNOWN_PANEL", "未找到面板。"))?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let prefix = if cfg!(any(target_os = "windows", target_os = "android")) {
            "http://hydrogen-plugin.localhost"
        } else {
            "hydrogen-plugin://localhost"
        };
        let session = PanelSession {
            session_id: id.clone(),
            nonce: uuid::Uuid::new_v4().to_string(),
            url: format!("{prefix}/{id}/{}", panel.entry.strip_prefix("ui/").unwrap()),
            generation,
            appearance: panel.appearance.clone(),
        };
        let mut sessions = self.sessions.lock().unwrap();
        if sessions.len() >= 16 {
            return Err(ApiError::new("PANEL_LIMIT", "最多同时挂载 16 个插件视图。"));
        }
        let appearance = panel.appearance.clone();
        sessions.insert(
            id,
            BoundSession {
                plugin,
                generation,
                appearance,
                theme: None,
                previous_theme: None,
            },
        );
        Ok(session)
    }
    pub fn plugin(&self, id: &str, generation: u64) -> Result<Plugin> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(id)
            .filter(|s| s.generation == generation)
            .ok_or_else(|| ApiError::new("STALE_PANEL", "面板会话已过期，请重新打开。"))?;
        let current = manifest::load(&session.plugin.root)?;
        if current.digest != session.plugin.digest {
            return Err(ApiError::new("PLUGIN_CHANGED", "插件内容已更改，请重载。"));
        }
        Ok(current)
    }
    pub fn close(&self, id: &str) {
        self.sessions.lock().unwrap().remove(id);
    }
    pub fn close_plugins(&self, ids: &[String]) {
        self.sessions
            .lock()
            .unwrap()
            .retain(|_, s| !ids.contains(&s.plugin.manifest.id));
    }
    pub fn clear(&self) {
        self.sessions.lock().unwrap().clear();
    }

    pub fn bind_theme(
        &self,
        id: &str,
        revision: u64,
        token: &str,
        themes: &super::themes::Themes,
        ready: bool,
    ) -> Result<()> {
        let state = themes.state();
        if state.revision != revision || !state.active.is_some_and(|v| v.token == token) {
            return Err(ApiError::new("THEME_STALE", "面板主题版本已过期。"));
        }
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get_mut(id)
            .filter(|s| s.appearance == "inherit")
            .ok_or_else(|| ApiError::new("STALE_PANEL", "继承主题面板不可用。"))?;
        if ready {
            if session
                .theme
                .as_ref()
                .is_none_or(|(r, t)| *r != revision || t != token)
            {
                return Err(ApiError::new("THEME_STALE", "面板确认与绑定不一致。"));
            }
            session.previous_theme = None;
        } else {
            if session.theme.as_ref().is_some_and(|(r, _)| *r > revision) {
                return Err(ApiError::new("THEME_STALE", "面板不能倒退版本。"));
            }
            session.previous_theme = session.theme.take().map(|(_, t)| t);
            session.theme = Some((revision, token.into()));
        }
        Ok(())
    }

    pub fn theme_resource(
        &self,
        path: &str,
        themes: &super::themes::Themes,
    ) -> Result<(Vec<u8>, &'static str)> {
        let (id, file) = path
            .trim_start_matches('/')
            .split_once('/')
            .ok_or_else(|| ApiError::new("INVALID_PATH", "无效面板路径。"))?;
        if file.starts_with("@theme/") {
            let sessions = self.sessions.lock().unwrap();
            let session = sessions
                .get(id)
                .ok_or_else(|| ApiError::new("STALE_PANEL", "面板已关闭。"))?;
            if session.appearance == "independent" {
                return Err(ApiError::new("PERMISSION_DENIED", "独立面板不继承主题。"));
            }
            let token = file
                .strip_prefix("@theme/")
                .and_then(|p| p.split_once('/'))
                .map(|(t, _)| t);
            if token.is_none()
                || !token.is_some_and(|t| {
                    session
                        .theme
                        .as_ref()
                        .is_some_and(|(_, current)| current == t)
                        || session.previous_theme.as_deref() == Some(t)
                })
            {
                return Err(ApiError::new("THEME_STALE", "主题租约没有绑定到此面板。"));
            }
            return themes.resource(file, true);
        }
        self.resource(path)
    }
    pub fn resource(&self, path: &str) -> Result<(Vec<u8>, &'static str)> {
        if path.contains(['%', '\\', '?', '#']) {
            return Err(ApiError::new("INVALID_PATH", "非法资源路径。"));
        }
        let (id, relative) = path
            .trim_start_matches('/')
            .split_once('/')
            .ok_or_else(|| ApiError::new("INVALID_PATH", "无效面板资源。"))?;
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(id)
            .ok_or_else(|| ApiError::new("STALE_PANEL", "面板已关闭。"))?;
        let path = manifest::within(&session.plugin.root, &format!("ui/{relative}"))?;
        let mime = match path.extension().and_then(|p| p.to_str()).unwrap_or("") {
            "html" => "text/html; charset=utf-8",
            "js" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "png" => "image/png",
            "svg" => "image/svg+xml",
            "woff2" => "font/woff2",
            _ => return Err(ApiError::new("RESOURCE_TYPE", "不支持的面板资源类型。")),
        };
        if fs::metadata(&path)?.len() > 1024 * 1024 {
            return Err(ApiError::new("RESOURCE_LIMIT", "单个面板资源超过 1 MiB。"));
        }
        Ok((fs::read(path)?, mime))
    }
}
