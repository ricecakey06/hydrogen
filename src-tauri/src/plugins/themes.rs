//! Declarative theme snapshots, independent of JS generations and native material state.
use super::{
    host::PluginHost,
    manifest::{self, Plugin},
};
use crate::{
    contracts::{ApiError, Result},
    launcher::storage::Settings,
};
use image::{ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const CONTRACT: &str = include_str!("../../../theme-contract.json");
pub const PREVIEW_SECONDS: u64 = 20;
const MAX_DECODED: u64 = 64 * 1024 * 1024;
const MAX_PEAK: u64 = 128 * 1024 * 1024;

pub type Choice = hydrogen_plugin_sdk::contracts::ThemeChoice;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Contribution {
    pub id: String,
    pub title: String,
    pub theme_version: u32,
    pub variants: BTreeMap<String, String>,
    #[serde(default)]
    pub preview: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Variant {
    theme_version: u32,
    scheme: String,
    extends: String,
    styles: Vec<String>,
    #[serde(default)]
    panel_styles: Vec<String>,
    #[serde(default)]
    branding: Branding,
    #[serde(default)]
    layers: Vec<Layer>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Branding {
    #[serde(default)]
    logo: Option<Asset>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Asset {
    source: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Layer {
    id: String,
    slot: String,
    source: String,
}

pub fn public_value(attribute: &str, value: &str) -> bool {
    if attribute == "data-h-layer" {
        return manifest::valid_id(value);
    }
    let contract: serde_json::Value =
        serde_json::from_str(CONTRACT).expect("bundled theme contract");
    let key = match attribute {
        "data-h-part" => "parts",
        "data-h-variant" => "variants",
        "data-h-scope" => "scopes",
        "data-h-page" => "pages",
        "data-h-sidebar" => "sidebar",
        _ => return false,
    };
    contract[key]
        .as_array()
        .is_some_and(|a| a.iter().any(|s| s.as_str() == Some(value)))
}

// Resolve .. lexically, then reuse the package's link and canonical-path checks.
pub fn relative(base: &str, reference: &str) -> Result<String> {
    if reference.is_empty()
        || reference.starts_with('/')
        || reference.contains(['\\', ':', '%', '?', '#', '\0'])
    {
        return Err(ApiError::new(
            "THEME_RESOURCE",
            format!("{base}: 只允许本包相对资源 {reference}。"),
        ));
    }
    let mut parts: Vec<&str> = base
        .rsplit_once('/')
        .map(|v| v.0.split('/').collect())
        .unwrap_or_default();
    for part in reference.split('/') {
        match part {
            "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(ApiError::new("THEME_RESOURCE", "资源路径越过插件包。"));
                }
            }
            _ => parts.push(part),
        }
    }
    let path = parts.join("/");
    if !manifest::safe_relative(&path) {
        return Err(ApiError::new("THEME_RESOURCE", "主题路径非法。"));
    }
    Ok(path)
}

#[derive(Clone)]
struct Resource {
    bytes: Arc<Vec<u8>>,
    mime: &'static str,
}
struct Assets<'a> {
    plugin: &'a Plugin,
    token: &'a str,
    resources: BTreeMap<String, Resource>,
    paths: BTreeMap<String, String>,
    bytes: u64,
    decoded: u64,
    images: usize,
    fonts: usize,
}
impl<'a> Assets<'a> {
    fn read(&mut self, base: &str, reference: &str, small: bool) -> Result<String> {
        let path = relative(base, reference)?;
        if let Some(id) = self.paths.get(&path) {
            if small {
                validate_image(&self.resources[id].bytes, Path::new(&path), true)?;
            }
            return Ok(id.clone());
        }
        let file = manifest::within(&self.plugin.root, &path)?;
        let size = file.metadata()?.len();
        let font = path.ends_with(".woff2");
        if size
            > if font {
                4 * 1024 * 1024
            } else if small {
                2 * 1024 * 1024
            } else {
                8 * 1024 * 1024
            }
            || self.bytes + size > 32 * 1024 * 1024
        {
            return Err(ApiError::new(
                "THEME_RESOURCE",
                format!("{path}: 资源超过字节预算。"),
            ));
        }
        let bytes = std::fs::read(file)?;
        let mime = if font {
            self.fonts += 1;
            if self.fonts > 2
                || bytes.len() < 48
                || &bytes[..4] != b"wOF2"
                || u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize != bytes.len()
                || u32::from_be_bytes(bytes[16..20].try_into().unwrap()) > 32 * 1024 * 1024
            {
                return Err(ApiError::new(
                    "THEME_RESOURCE",
                    format!("{path}: WOFF2 头/长度或字体数量无效。"),
                ));
            }
            let tables = u16::from_be_bytes(bytes[12..14].try_into().unwrap());
            let compressed = u32::from_be_bytes(bytes[20..24].try_into().unwrap()) as usize;
            if tables == 0
                || tables > 256
                || bytes[14..16] != [0, 0]
                || compressed == 0
                || compressed > bytes.len() - 48
            {
                return Err(ApiError::new(
                    "THEME_RESOURCE",
                    format!("{path}: WOFF2 表目录或压缩长度无效。"),
                ));
            }
            for offset in [28, 40] {
                let start =
                    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
                let length =
                    u32::from_be_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
                if start
                    .checked_add(length)
                    .is_none_or(|end| end > bytes.len())
                    || (length > 0 && start < 48)
                {
                    return Err(ApiError::new(
                        "THEME_RESOURCE",
                        format!("{path}: WOFF2 附加数据越界。"),
                    ));
                }
            }
            "font/woff2"
        } else {
            let (mime, decoded) = validate_image(&bytes, Path::new(&path), small)?;
            self.images += 1;
            self.decoded += decoded;
            if self.images > 32 || self.decoded > MAX_DECODED {
                return Err(ApiError::new(
                    "THEME_RESOURCE",
                    "图片数量或解码预算超过限制。",
                ));
            }
            mime
        };
        self.bytes += size;
        let id = format!("asset-{}", self.resources.len());
        self.resources.insert(
            id.clone(),
            Resource {
                bytes: Arc::new(bytes),
                mime,
            },
        );
        self.paths.insert(path, id.clone());
        Ok(id)
    }
    fn url(&self, id: &str) -> String {
        format!("{}/@theme/{}/{}", ui_origin(), self.token, id)
    }
}
fn validate_image(bytes: &[u8], path: &Path, small: bool) -> Result<(&'static str, u64)> {
    let format = image::guess_format(bytes)
        .map_err(|_| ApiError::new("THEME_RESOURCE", "无法识别图片内容。"))?;
    let (extension, mime) = match format {
        ImageFormat::Png => ("png", "image/png"),
        ImageFormat::Jpeg => ("jpg", "image/jpeg"),
        ImageFormat::WebP => ("webp", "image/webp"),
        _ => {
            return Err(ApiError::new(
                "THEME_RESOURCE",
                "只支持静态 PNG/JPEG/WebP。",
            ));
        }
    };
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    if ext != extension && !(extension == "jpg" && ext == "jpeg") {
        return Err(ApiError::new("THEME_RESOURCE", "图片扩展名与内容不一致。"));
    }
    // Reject APNG/WebP animations through their container chunks, not byte-substring guesses.
    if format == ImageFormat::Png {
        let mut offset = 8;
        while offset + 12 <= bytes.len() {
            let len = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
            if bytes.get(offset + 4..offset + 8) == Some(b"acTL") {
                return Err(ApiError::new("THEME_RESOURCE", "不支持动画 PNG。"));
            }
            offset = offset
                .checked_add(len + 12)
                .ok_or_else(|| ApiError::new("THEME_RESOURCE", "PNG 长度无效。"))?;
        }
    }
    if format == ImageFormat::WebP {
        let mut offset = 12;
        while offset + 8 <= bytes.len() {
            let len =
                u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
            if matches!(bytes.get(offset..offset + 4), Some(b"ANIM" | b"ANMF")) {
                return Err(ApiError::new("THEME_RESOURCE", "不支持动画 WebP。"));
            }
            offset = offset
                .checked_add(8 + len + (len % 2))
                .ok_or_else(|| ApiError::new("THEME_RESOURCE", "WebP 长度无效。"))?;
        }
    }
    let reader = ImageReader::with_format(Cursor::new(bytes), format);
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| ApiError::new("THEME_RESOURCE", e.to_string()))?;
    if w == 0
        || h == 0
        || w.max(h) > if small { 1600 } else { 4096 }
        || u64::from(w) * u64::from(h) > 8_000_000
    {
        return Err(ApiError::new("THEME_RESOURCE", "图片解码尺寸超过预算。"));
    }
    ImageReader::with_format(Cursor::new(bytes), format)
        .decode()
        .map_err(|e| ApiError::new("THEME_RESOURCE", e.to_string()))?;
    Ok((mime, u64::from(w) * u64::from(h) * 4))
}
fn ui_origin() -> &'static str {
    if cfg!(windows) {
        "http://hydrogen-ui.localhost"
    } else {
        "hydrogen-ui://localhost"
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualLayer {
    pub id: String,
    pub slot: String,
    pub source: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prepared {
    pub token: String,
    pub choice: Choice,
    pub actual_id: String,
    pub scheme: String,
    pub high_contrast: bool,
    pub package_digest: Option<String>,
    pub fallback_reason: Option<String>,
    pub styles: Vec<String>,
    pub logo: Option<String>,
    pub layers: Vec<VisualLayer>,
    pub images: Vec<String>,
    pub fonts: Vec<String>,
    pub font_sources: Vec<String>,
    pub bytes: u64,
    pub decoded_bytes: u64,
}
struct Snapshot {
    visual: Prepared,
    resources: BTreeMap<String, Resource>,
    created: Instant,
    paths: BTreeSet<String>,
}

fn base_tokens(scheme: &str, scope: &str) -> String {
    let contract: serde_json::Value = serde_json::from_str(CONTRACT).unwrap();
    let tokens = contract["tokens"].as_object().unwrap();
    let declarations = tokens
        .iter()
        .map(|(k, v)| format!("{k}:{};", v[scheme].as_str().unwrap()))
        .collect::<String>();
    format!("{scope}{{{declarations}color-scheme:{scheme};}}")
}

fn build(
    plugin: Option<&Plugin>,
    contribution: Option<&Contribution>,
    choice: Choice,
    scheme: &str,
    high_contrast: bool,
    fallback: Option<String>,
) -> Result<Snapshot> {
    let token = uuid::Uuid::new_v4().simple().to_string();
    let scope = format!("[data-h-theme-root][data-h-session=\"{token}\"]");
    let panel_scope = format!("[data-h-panel-root][data-h-session=\"{token}\"]");
    let mut visual = Prepared {
        token: token.clone(),
        choice,
        actual_id: "builtin:default".into(),
        scheme: scheme.into(),
        high_contrast,
        package_digest: None,
        fallback_reason: fallback,
        styles: vec![format!("{}/@theme/{token}/main.css", ui_origin())],
        logo: None,
        layers: vec![],
        images: vec![],
        fonts: vec![],
        font_sources: vec![],
        bytes: 0,
        decoded_bytes: 0,
    };
    let mut main = base_tokens(scheme, &scope);
    let mut panels = String::new();
    let mut inherited = String::new();
    let mut resources = BTreeMap::new();
    let mut paths = BTreeSet::new();
    if let (Some(plugin), Some(contribution)) = (plugin, contribution) {
        let file = &contribution.variants[scheme];
        paths.insert(file.clone());
        let json = manifest::within(&plugin.root, file)?;
        if json.metadata()?.len() > 64 * 1024 {
            return Err(ApiError::new("THEME_RESOURCE", "主题 JSON 超过 64 KiB。"));
        }
        let variant: Variant = serde_json::from_slice(&std::fs::read(json)?)
            .map_err(|e| ApiError::new("THEME_INVALID", format!("{file}: {e}")))?;
        if variant.theme_version != 1
            || variant.scheme != scheme
            || variant.extends != format!("builtin.{scheme}.v1")
            || variant.styles.is_empty()
            || variant.styles.len() + variant.panel_styles.len() > 8
            || variant.layers.len() > 12
        {
            return Err(ApiError::new(
                "THEME_INVALID",
                format!("{file}: 协议、继承、明暗或文件/图层数无效。"),
            ));
        }
        let mut css_bytes = 0;
        let mut sources = |files: &[String]| -> Result<Vec<(String, String)>> {
            files
                .iter()
                .map(|name| {
                    let path = relative(file, name)?;
                    if !path.ends_with(".css") {
                        return Err(ApiError::new("THEME_CSS", "样式文件必须为 .css。"));
                    }
                    let checked = manifest::within(&plugin.root, &path)?;
                    css_bytes += checked.metadata()?.len();
                    if css_bytes > 512 * 1024 {
                        return Err(ApiError::new("THEME_RESOURCE", "CSS 源码超过 512 KiB。"));
                    }
                    Ok((path, std::fs::read_to_string(checked)?))
                })
                .collect()
        };
        let main_sources = sources(&variant.styles)?;
        let panel_sources = sources(&variant.panel_styles)?;
        paths.extend(
            main_sources
                .iter()
                .chain(&panel_sources)
                .map(|(p, _)| p.clone()),
        );
        let mut assets = Assets {
            plugin,
            token: &token,
            resources: BTreeMap::new(),
            paths: BTreeMap::new(),
            bytes: css_bytes,
            decoded: 0,
            images: 0,
            fonts: 0,
        };
        let (compiled, fonts) =
            super::theme_css::compile(&main_sources, &scope, &token, |base, url| {
                let id = assets.read(base, url, false)?;
                Ok(assets.url(&id))
            })?;
        main.push_str(&compiled);
        visual.fonts = fonts;
        let (compiled, _) =
            super::theme_css::compile(&main_sources, &panel_scope, &token, |base, url| {
                let id = assets.read(base, url, false)?;
                Ok(id)
            })?;
        inherited = super::theme_css::inherited_tokens(&compiled, &panel_scope)?;
        let (compiled, _) = super::theme_css::compile(
            &panel_sources,
            &panel_scope,
            &format!("{token}-panel"),
            |base, url| {
                let id = assets.read(base, url, false)?;
                Ok(id)
            },
        )?;
        panels.push_str(&compiled);
        if let Some(logo) = variant.branding.logo {
            let id = assets.read(file, &logo.source, true)?;
            if !assets.resources[&id].mime.starts_with("image/") {
                return Err(ApiError::new("THEME_RESOURCE", "logo 必须为图片。"));
            }
            visual.logo = Some(assets.url(&id));
        }
        let mut ids = BTreeSet::new();
        let contract: serde_json::Value = serde_json::from_str(CONTRACT).unwrap();
        for layer in variant.layers {
            if !manifest::valid_id(&layer.id)
                || !ids.insert(layer.id.clone())
                || !contract["slots"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v.as_str() == Some(&layer.slot))
            {
                return Err(ApiError::new(
                    "THEME_PART",
                    "图层 ID 重复/无效，或槽位未公开。",
                ));
            }
            let id = assets.read(file, &layer.source, false)?;
            if !assets.resources[&id].mime.starts_with("image/") {
                return Err(ApiError::new("THEME_RESOURCE", "图层必须为图片。"));
            }
            visual.layers.push(VisualLayer {
                id: layer.id,
                slot: layer.slot,
                source: assets.url(&id),
            });
        }
        if let Some(preview) = &contribution.preview {
            let id = assets.read("plugin.json", preview, true)?;
            if !assets.resources[&id].mime.starts_with("image/") {
                return Err(ApiError::new("THEME_RESOURCE", "预览必须为静态图片。"));
            }
        }
        visual.images = assets
            .resources
            .iter()
            .filter(|(_, r)| r.mime.starts_with("image/"))
            .map(|(id, _)| assets.url(id))
            .collect();
        visual.font_sources = assets
            .resources
            .iter()
            .filter(|(_, r)| r.mime == "font/woff2")
            .map(|(id, _)| assets.url(id))
            .collect();
        paths.extend(assets.paths.keys().cloned());
        visual.bytes = assets.bytes;
        visual.decoded_bytes = assets.decoded;
        visual.actual_id = format!("{}:{}", plugin.manifest.id, contribution.id);
        visual.package_digest = Some(plugin.digest.clone());
        resources = assets.resources;
        if super::package::files(&plugin.root)?.1 != plugin.digest {
            return Err(ApiError::new("THEME_STALE", "准备期间主题包发生变化。"));
        }
    }
    resources.insert(
        "main.css".into(),
        Resource {
            bytes: Arc::new(main.into_bytes()),
            mime: "text/css",
        },
    );
    resources.insert(
        "tokens.css".into(),
        Resource {
            bytes: Arc::new(
                format!("{}{inherited}", base_tokens(scheme, &panel_scope)).into_bytes(),
            ),
            mime: "text/css",
        },
    );
    resources.insert(
        "panels.css".into(),
        Resource {
            bytes: Arc::new(panels.into_bytes()),
            mime: "text/css",
        },
    );
    Ok(Snapshot {
        visual,
        resources,
        created: Instant::now(),
        paths,
    })
}

pub fn validate(plugin: &Plugin) -> Result<()> {
    let contributions = &plugin.manifest.contributes.themes;
    if contributions.len() > 16 {
        return Err(ApiError::new("THEME_INVALID", "每包最多 16 个主题。"));
    }
    if !contributions.is_empty()
        && !plugin
            .manifest
            .permissions
            .iter()
            .any(|p| p == "theme.contribute")
    {
        return Err(ApiError::new(
            "PERMISSION_DENIED",
            "主题需要 theme.contribute。",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for c in contributions {
        if !manifest::valid_id(&c.id)
            || !ids.insert(&c.id)
            || c.title.trim().is_empty()
            || c.title.len() > 200
            || c.theme_version != 1
            || c.variants.is_empty()
            || c.variants
                .keys()
                .any(|v| !["light", "dark"].contains(&v.as_str()))
        {
            return Err(ApiError::new(
                "THEME_INVALID",
                "主题声明的 ID、名称、版本或变体无效。",
            ));
        }
        for scheme in c.variants.keys() {
            // build() finishes by checking the package digest without recursively revalidating themes.
            let snapshot = build(
                Some(plugin),
                Some(c),
                Choice::default(),
                scheme,
                false,
                None,
            )?;
            paths.extend(snapshot.paths);
        }
    }
    let bytes = paths
        .iter()
        .map(|p| manifest::within(&plugin.root, p).and_then(|p| Ok(p.metadata()?.len())))
        .collect::<Result<Vec<_>>>()?
        .iter()
        .sum::<u64>();
    if bytes > 32 * 1024 * 1024 {
        return Err(ApiError::new(
            "THEME_RESOURCE",
            "每插件主题资源超过 32 MiB。",
        ));
    }
    Ok(())
}

pub fn standalone(plugin: &Plugin) -> bool {
    let m = &plugin.manifest;
    let c = &m.contributes;
    m.runtime == "javascript"
        && m.permissions == ["theme.contribute"]
        && !c.themes.is_empty()
        && m.activation.is_empty()
        && m.provides.is_empty()
        && m.frontend.is_none()
        && c.commands.is_empty()
        && c.panels.is_empty()
        && c.views.is_empty()
        && c.menus.is_empty()
        && c.providers.is_empty()
}

#[derive(Default)]
struct Runtime {
    revision: u64,
    active: Option<String>,
    confirmed: Option<String>,
    preview: Option<Instant>,
    snapshots: BTreeMap<String, Arc<Snapshot>>,
}
#[derive(Default)]
pub struct Themes {
    inner: Mutex<Runtime>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub revision: u64,
    pub active: Option<Prepared>,
    pub preview: bool,
    pub remaining_seconds: u64,
    pub resources: usize,
    pub snapshots: usize,
    pub status: String,
}
impl Themes {
    pub fn prepare(
        &self,
        host: &PluginHost,
        settings: &Settings,
        choice: Choice,
        system_dark: bool,
        high_contrast: bool,
        safe: bool,
    ) -> Result<Prepared> {
        if !choice.is_valid() {
            return Err(ApiError::new("THEME_INVALID", "主题选择、明暗或材质无效。"));
        }
        let scheme = if choice.mode == "dark" || choice.mode == "system" && system_dark {
            "dark"
        } else {
            "light"
        };
        let mut plugin = None;
        let mut contribution = None;
        let mut fallback = None;
        if safe {
            fallback = Some("安全主题模式".into());
        } else if high_contrast {
            fallback = Some("系统强制颜色".into());
        } else if choice.id != "builtin:default" {
            let (id, theme) = choice.id.split_once(':').unwrap();
            match host.checked(settings, id) {
                Ok(p) => {
                    if let Some(c) = p
                        .manifest
                        .contributes
                        .themes
                        .iter()
                        .find(|c| c.id == theme)
                        .cloned()
                    {
                        if c.variants.contains_key(scheme) {
                            contribution = Some(c);
                            plugin = Some(p);
                        } else {
                            fallback = Some("THEME_VARIANT_MISSING: 当前明暗变体缺失".into());
                        }
                    } else {
                        fallback = Some("THEME_UNAVAILABLE: 主题已移除".into());
                    }
                }
                Err(e) => fallback = Some(format!("THEME_UNAVAILABLE: {}", e.message)),
            }
        }
        let snapshot = match build(
            plugin.as_ref(),
            contribution.as_ref(),
            choice.clone(),
            scheme,
            high_contrast,
            fallback,
        ) {
            Ok(snapshot) => snapshot,
            Err(e) if choice.id == settings.theme.id => build(
                None,
                None,
                choice,
                scheme,
                high_contrast,
                Some(format!("{}: {}", e.code, e.message)),
            )?,
            Err(e) => return Err(e),
        };
        let visual = snapshot.visual.clone();
        let mut runtime = self.inner.lock().unwrap();
        let active = runtime.active.clone();
        let confirmed = runtime.confirmed.clone();
        runtime
            .snapshots
            .retain(|id, _| Some(id) == active.as_ref() || Some(id) == confirmed.as_ref());
        let peak = runtime
            .snapshots
            .values()
            .map(|s| s.visual.decoded_bytes)
            .sum::<u64>()
            + visual.decoded_bytes;
        if peak > MAX_PEAK {
            return Err(ApiError::new(
                "THEME_RESOURCE",
                "切换解码峰值超过 128 MiB。",
            ));
        }
        runtime
            .snapshots
            .insert(visual.token.clone(), Arc::new(snapshot));
        Ok(visual)
    }
    pub fn commit(&self, token: &str, preview: bool) -> Result<State> {
        let mut runtime = self.inner.lock().unwrap();
        let snapshot = runtime
            .snapshots
            .get(token)
            .ok_or_else(|| ApiError::new("THEME_STALE", "主题准备令牌已过期。"))?;
        if snapshot.created.elapsed() > Duration::from_secs(30)
            && runtime.active.as_deref() != Some(token)
        {
            return Err(ApiError::new("THEME_STALE", "主题准备已超时。"));
        }
        runtime.active = Some(token.into());
        runtime.revision += 1;
        runtime.preview = if preview { Some(Instant::now()) } else { None };
        if !preview {
            runtime.confirmed = Some(token.into());
        }
        Ok(state(&mut runtime))
    }
    pub fn confirm(&self, token: &str, save: impl FnOnce(Choice) -> Result<()>) -> Result<State> {
        let mut runtime = self.inner.lock().unwrap();
        expire(&mut runtime);
        if runtime.active.as_deref() != Some(token) || runtime.preview.is_none() {
            return Err(ApiError::new("THEME_PREVIEW_EXPIRED", "预览已过期。"));
        }
        let choice = runtime.snapshots[token].visual.choice.clone();
        if let Err(e) = save(choice) {
            rollback(&mut runtime);
            return Err(e);
        }
        runtime.confirmed = Some(token.into());
        runtime.preview = None;
        runtime.revision += 1;
        Ok(state(&mut runtime))
    }
    pub fn cancel(&self) -> State {
        let mut r = self.inner.lock().unwrap();
        rollback(&mut r);
        state(&mut r)
    }
    pub fn state(&self) -> State {
        state(&mut self.inner.lock().unwrap())
    }
    pub fn release(&self, token: &str) {
        let mut r = self.inner.lock().unwrap();
        if r.active.as_deref() != Some(token) && r.confirmed.as_deref() != Some(token) {
            r.snapshots.remove(token);
        }
    }
    pub fn clear(&self) {
        let mut r = self.inner.lock().unwrap();
        let revision = r.revision + 1;
        *r = Runtime {
            revision,
            ..Default::default()
        };
    }
    pub fn invalidate(&self, ids: &[String], reason: &str) -> Result<()> {
        let active = self.state().active;
        let Some(active) = active else {
            return Ok(());
        };
        if !active
            .actual_id
            .split_once(':')
            .is_some_and(|(id, _)| ids.iter().any(|s| s == id))
        {
            return Ok(());
        }
        let Some(active) = self.cancel().active else {
            return Ok(());
        };
        if !active
            .actual_id
            .split_once(':')
            .is_some_and(|(id, _)| ids.iter().any(|s| s == id))
        {
            return Ok(());
        }
        let fallback = build(
            None,
            None,
            active.choice,
            &active.scheme,
            active.high_contrast,
            Some(reason.into()),
        )?;
        let mut r = self.inner.lock().unwrap();
        let token = fallback.visual.token.clone();
        r.snapshots.insert(token.clone(), Arc::new(fallback));
        r.active = Some(token.clone());
        r.confirmed = Some(token);
        r.preview = None;
        r.revision += 1;
        Ok(())
    }
    pub fn restore_failed_invalidation(&self, previous: &State) {
        let Some(active) = &previous.active else {
            return;
        };
        let mut r = self.inner.lock().unwrap();
        if !r.snapshots.contains_key(&active.token) {
            return;
        }
        let failed = r.active.replace(active.token.clone());
        r.confirmed = Some(active.token.clone());
        r.preview = None;
        r.revision += 1;
        if let Some(failed) = failed.filter(|token| token != &active.token) {
            r.snapshots.remove(&failed);
        }
    }
    pub fn resource(&self, path: &str, panel: bool) -> Result<(Vec<u8>, &'static str)> {
        let (token, file) = path
            .trim_start_matches('/')
            .strip_prefix("@theme/")
            .and_then(|p| p.split_once('/'))
            .ok_or_else(|| ApiError::new("THEME_RESOURCE", "无效租约 URL。"))?;
        if !manifest::valid_id(&format!("t{token}"))
            || file.contains(['/', '%', '\\', '?', '#'])
            || !panel && matches!(file, "tokens.css" | "panels.css")
            || panel && file == "main.css"
        {
            return Err(ApiError::new("THEME_RESOURCE", "资源作用域无效。"));
        }
        let runtime = self.inner.lock().unwrap();
        let resource = runtime
            .snapshots
            .get(token)
            .and_then(|s| s.resources.get(file))
            .ok_or_else(|| ApiError::new("THEME_STALE", "资源租约已释放。"))?;
        Ok((resource.bytes.as_ref().clone(), resource.mime))
    }
}
fn rollback(r: &mut Runtime) {
    if r.preview.take().is_some() {
        r.active = r.confirmed.clone();
        r.revision += 1;
    }
}
fn expire(r: &mut Runtime) {
    if r.preview
        .is_some_and(|p| p.elapsed() >= Duration::from_secs(PREVIEW_SECONDS))
    {
        rollback(r);
    }
}
fn state(r: &mut Runtime) -> State {
    expire(r);
    State {
        revision: r.revision,
        active: r
            .active
            .as_ref()
            .and_then(|id| r.snapshots.get(id))
            .map(|s| s.visual.clone()),
        preview: r.preview.is_some(),
        remaining_seconds: r
            .preview
            .map(|p| PREVIEW_SECONDS.saturating_sub(p.elapsed().as_secs()))
            .unwrap_or(0),
        resources: r.snapshots.values().map(|s| s.resources.len()).sum(),
        snapshots: r.snapshots.len(),
        status: "ready".into(),
    }
}
pub fn catalog(host: &PluginHost, settings: &Settings, plugins: &[Plugin]) -> serde_json::Value {
    let mut entries = vec![
        serde_json::json!({"id":"builtin:default","title":"Hydrogen","pluginId":null,"variants":["light","dark"],"available":true}),
    ];
    for plugin in plugins {
        for theme in &plugin.manifest.contributes.themes {
            if entries.len() >= 256 {
                break;
            }
            let available = host.available(settings, &plugin.manifest.id);
            let preview = (available && theme.preview.is_some()).then(|| {
                format!(
                    "{}/@theme-preview/{}/{}/{}",
                    ui_origin(),
                    plugin.digest,
                    plugin.manifest.id,
                    theme.id
                )
            });
            entries.push(serde_json::json!({"id":format!("{}:{}",plugin.manifest.id,theme.id),"title":theme.title,"pluginId":plugin.manifest.id,"variants":theme.variants.keys().collect::<Vec<_>>(),"available":available,"preview":preview}));
        }
    }
    serde_json::json!(entries)
}

// Catalog artwork does not create an active visual lease. Resolve by contribution,
// verify the approved digest before and after reading, and decode within the small-image budget.
pub fn preview_resource(
    host: &PluginHost,
    settings: &Settings,
    url: &str,
) -> Result<(Vec<u8>, &'static str)> {
    let parts = url
        .strip_prefix("/@theme-preview/")
        .unwrap_or("")
        .split('/')
        .collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0].len() != 64
        || !parts[0].bytes().all(|c| c.is_ascii_hexdigit())
        || !manifest::valid_id(parts[1])
        || !manifest::valid_id(parts[2])
    {
        return Err(ApiError::new("THEME_RESOURCE", "无效预览资源 URL。"));
    }
    let plugin = host.checked(settings, parts[1])?;
    if plugin.digest != parts[0] {
        return Err(ApiError::new("THEME_STALE", "预览摘要已过期。"));
    }
    let reference = plugin
        .manifest
        .contributes
        .themes
        .iter()
        .find(|c| c.id == parts[2])
        .and_then(|c| c.preview.as_ref())
        .ok_or_else(|| ApiError::new("THEME_RESOURCE", "主题没有预览图。"))?;
    let path = manifest::within(&plugin.root, &relative("plugin.json", reference)?)?;
    if path.metadata()?.len() > 2 * 1024 * 1024 {
        return Err(ApiError::new("THEME_RESOURCE", "预览图超过 2 MiB。"));
    }
    let bytes = std::fs::read(&path)?;
    let (mime, _) = validate_image(&bytes, &path, true)?;
    if super::package::files(&plugin.root)?.1 != plugin.digest {
        return Err(ApiError::new("THEME_STALE", "读取期间预览包变化。"));
    }
    Ok((bytes, mime))
}

impl State {
    pub fn public_appearance(&self) -> serde_json::Value {
        serde_json::json!({"theme":self.active.as_ref().map(|v|v.scheme.as_str()).unwrap_or("light"),
            "id":self.active.as_ref().map(|v|v.actual_id.as_str()).unwrap_or("builtin:default"),
            "mode":self.active.as_ref().map(|v|v.choice.mode.as_str()).unwrap_or("system"),
            "revision":self.revision,"highContrast":self.active.as_ref().is_some_and(|v|v.high_contrast)})
    }
}

pub fn write_template(root: &Path) -> Result<()> {
    for (name, text) in [
        (
            "plugin.json",
            include_str!("../../../plugin-sdk/templates/theme/plugin.json"),
        ),
        (
            "main.js",
            include_str!("../../../plugin-sdk/templates/theme/main.js"),
        ),
        (
            "themes/light.json",
            include_str!("../../../plugin-sdk/templates/theme/themes/light.json"),
        ),
        (
            "themes/dark.json",
            include_str!("../../../plugin-sdk/templates/theme/themes/dark.json"),
        ),
        (
            "themes/light.css",
            include_str!("../../../plugin-sdk/templates/theme/themes/light.css"),
        ),
        (
            "themes/dark.css",
            include_str!("../../../plugin-sdk/templates/theme/themes/dark.css"),
        ),
    ] {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, text)?;
    }
    Ok(())
}
pub fn self_test(host: &PluginHost, plugin: &Plugin) -> Result<serde_json::Value> {
    validate(plugin)?;
    host.set_plugins(vec![plugin.clone()]);
    let settings = Settings::default();
    let builtin = host
        .themes
        .prepare(host, &settings, Choice::default(), false, false, false)?;
    host.themes.commit(&builtin.token, false)?;
    let mut peak = 0;
    let start = Instant::now();
    for i in 0..100 {
        let c = &plugin.manifest.contributes.themes[i % plugin.manifest.contributes.themes.len()];
        let scheme = if i % 2 == 1 && c.variants.contains_key("dark") {
            "dark"
        } else if c.variants.contains_key("light") {
            "light"
        } else {
            "dark"
        };
        let candidate = host.themes.prepare(
            host,
            &settings,
            Choice {
                id: format!("{}:{}", plugin.manifest.id, c.id),
                mode: scheme.into(),
                material: "auto".into(),
            },
            false,
            false,
            false,
        )?;
        host.themes.commit(&candidate.token, true)?;
        peak = peak.max(host.themes.state().resources);
        host.themes.cancel();
        host.themes.release(&candidate.token);
        if host.themes.state().snapshots != 1 {
            return Err(ApiError::new(
                "THEME_CLEANUP_INCOMPLETE",
                "切换后快照数不稳定。",
            ));
        }
        if host
            .themes
            .resource(&format!("@theme/{}/main.css", candidate.token), false)
            .is_ok()
        {
            return Err(ApiError::new(
                "THEME_CLEANUP_INCOMPLETE",
                "已释放租约仍可读取。",
            ));
        }
    }
    let final_state = host.themes.state();
    host.themes.clear();
    Ok(
        serde_json::json!({"passed":true,"themeVersion":1,"switches":100,"elapsedMs":start.elapsed().as_millis(),"peakResources":peak,"retainedSnapshots":final_state.snapshots,"resourcesAfterClear":host.themes.state().resources,"javascriptExecuted":false,"fixtures":"isolated snapshot preparation/commit/cancel/release; visual checks require Launcher"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn insert(themes: &Themes, scheme: &str) -> String {
        let snapshot = build(None, None, Choice::default(), scheme, false, None).unwrap();
        let token = snapshot.visual.token.clone();
        themes
            .inner
            .lock()
            .unwrap()
            .snapshots
            .insert(token.clone(), Arc::new(snapshot));
        token
    }
    #[test]
    fn failed_save_keeps_the_confirmed_visual_and_public_events_have_no_leases() {
        let themes = Themes::default();
        let confirmed = insert(&themes, "light");
        themes.commit(&confirmed, false).unwrap();
        let candidate = insert(&themes, "dark");
        themes.commit(&candidate, true).unwrap();
        let error = themes
            .confirm(&candidate, |_| {
                Err(ApiError::new("READ_ONLY", "fixture save failure"))
            })
            .unwrap_err();
        assert_eq!(error.code, "READ_ONLY");
        let state = themes.state();
        assert_eq!(state.active.as_ref().unwrap().token, confirmed);
        assert!(!state.preview);
        let public = state.public_appearance();
        assert!(public.get("active").is_none());
        assert!(public.get("token").is_none());
        assert_eq!(public["theme"], "light");
        themes.release(&candidate);
        assert_eq!(themes.state().snapshots, 1);
    }
    #[tokio::test]
    async fn standalone_never_evaluates_its_entry_and_bad_candidates_preserve_the_previous_theme() {
        let directory = tempfile::tempdir().unwrap();
        write_template(directory.path()).unwrap();
        std::fs::write(
            directory.path().join("main.js"),
            "throw new Error('theme JavaScript must never run');",
        )
        .unwrap();
        let plugin = manifest::load(directory.path()).unwrap();
        let host = PluginHost::new(tokio::runtime::Handle::current(), vec![]);
        host.set_plugins(vec![plugin.clone()]);
        host.activate(plugin.clone()).await.unwrap();
        host.reload_with_commit(plugin.clone(), || Ok(()))
            .await
            .unwrap();
        let settings = Settings::default();
        let choice = Choice {
            id: format!(
                "{}:{}",
                plugin.manifest.id, plugin.manifest.contributes.themes[0].id
            ),
            ..Default::default()
        };
        let confirmed = host
            .themes
            .prepare(&host, &settings, choice.clone(), false, false, false)
            .unwrap();
        host.themes.commit(&confirmed.token, false).unwrap();
        let previous = host.themes.cancel();
        host.themes
            .invalidate(std::slice::from_ref(&plugin.manifest.id), "fixture disable")
            .unwrap();
        assert_eq!(
            host.themes.state().active.unwrap().actual_id,
            "builtin:default"
        );
        host.themes.restore_failed_invalidation(&previous);
        assert_eq!(host.themes.state().active.unwrap().token, confirmed.token);
        std::fs::write(
            directory.path().join("themes/light.json"),
            r#"{"themeVersion":1,"unknown":true}"#,
        )
        .unwrap();
        assert!(manifest::load(directory.path()).is_err());
        let fallback = host
            .themes
            .prepare(&host, &settings, choice.clone(), false, false, false)
            .unwrap();
        assert_eq!(fallback.actual_id, "builtin:default");
        assert_eq!(fallback.choice, choice);
        assert!(fallback.fallback_reason.is_some());
        assert_eq!(
            host.stop_all().await.unwrap().created,
            0,
            "no QuickJS context was created"
        );
    }
    #[tokio::test]
    async fn preview_digest_and_panel_binding_reject_cross_session_reads() {
        let directory = tempfile::tempdir().unwrap();
        write_template(directory.path()).unwrap();
        image::DynamicImage::new_rgba8(2, 2)
            .save(directory.path().join("preview.png"))
            .unwrap();
        let file = directory.path().join("plugin.json");
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        json["contributes"]["themes"][0]["preview"] = "preview.png".into();
        std::fs::write(file, serde_json::to_vec(&json).unwrap()).unwrap();
        let plugin = manifest::load(directory.path()).unwrap();
        let host = PluginHost::new(tokio::runtime::Handle::current(), vec![]);
        host.set_plugins(vec![plugin.clone()]);
        let settings = Settings::default();
        let theme = &plugin.manifest.contributes.themes[0];
        let path = format!(
            "/@theme-preview/{}/{}/{}",
            plugin.digest, plugin.manifest.id, theme.id
        );
        assert!(preview_resource(&host, &settings, &path).is_ok());
        assert!(
            preview_resource(
                &host,
                &settings,
                &path.replace(&plugin.digest, &"0".repeat(64))
            )
            .is_err()
        );
        assert!(preview_resource(&host, &settings, "/@theme-preview/../preview.png").is_err());
        assert_eq!(
            host.themes.state().snapshots,
            0,
            "catalog previews allocate no snapshots"
        );
        let themes = &host.themes;
        let first = insert(themes, "light");
        let state = themes.commit(&first, false).unwrap();
        let panel_plugin = manifest::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/instance-tools"),
        )
        .unwrap();
        let panels = super::super::panel::Panels::default();
        let panel = panels.open(panel_plugin, "overview", 1).unwrap();
        let panel_path =
            |token: &str, file: &str| format!("/{}/@theme/{token}/{file}", panel.session_id);
        assert!(
            panels
                .theme_resource(&panel_path(&first, "tokens.css"), themes)
                .is_err()
        );
        panels
            .bind_theme(&panel.session_id, state.revision, &first, themes, false)
            .unwrap();
        assert!(
            panels
                .theme_resource(&panel_path(&first, "tokens.css"), themes)
                .is_ok()
        );
        assert!(
            panels
                .theme_resource(&panel_path(&first, "main.css"), themes)
                .is_err()
        );
        let second = insert(themes, "dark");
        let state = themes.commit(&second, true).unwrap();
        assert!(
            panels
                .theme_resource(&panel_path(&second, "tokens.css"), themes)
                .is_err()
        );
        panels
            .bind_theme(&panel.session_id, state.revision, &second, themes, false)
            .unwrap();
        assert!(
            panels
                .theme_resource(&panel_path(&first, "tokens.css"), themes)
                .is_ok()
        );
        panels
            .bind_theme(&panel.session_id, state.revision, &second, themes, true)
            .unwrap();
        assert!(
            panels
                .theme_resource(&panel_path(&first, "tokens.css"), themes)
                .is_err()
        );
        panels.close(&panel.session_id);
        assert!(
            panels
                .theme_resource(&panel_path(&second, "tokens.css"), themes)
                .is_err()
        );
        assert!(validate_image(b"not a PNG", Path::new("fake.png"), false).is_err());
        let bytes = std::fs::read(directory.path().join("preview.png")).unwrap();
        assert!(validate_image(&bytes, Path::new("fake.jpg"), false).is_err());
    }
    #[test]
    fn paths_settings_leases_and_revisions() {
        assert_eq!(
            relative("themes/styles/main.css", "../assets/bg.png").unwrap(),
            "themes/assets/bg.png"
        );
        for p in [
            "../../../x.png",
            "https://evil/a.png",
            "/a.png",
            "..\\a.png",
            "data:image/png,a",
            "a%2f.png",
        ] {
            assert!(relative("themes/main.css", p).is_err());
        }
        assert!(
            !Choice {
                mode: "unknown".into(),
                ..Default::default()
            }
            .is_valid()
        );
        let themes = Themes::default();
        let first = build(None, None, Choice::default(), "light", false, None).unwrap();
        let first_token = first.visual.token.clone();
        themes
            .inner
            .lock()
            .unwrap()
            .snapshots
            .insert(first_token.clone(), Arc::new(first));
        let a = themes.commit(&first_token, false).unwrap();
        let second = build(None, None, Choice::default(), "dark", false, None).unwrap();
        let token = second.visual.token.clone();
        themes
            .inner
            .lock()
            .unwrap()
            .snapshots
            .insert(token.clone(), Arc::new(second));
        themes.commit(&token, true).unwrap();
        themes.inner.lock().unwrap().preview = Some(Instant::now() - Duration::from_secs(21));
        let restored = themes.state();
        assert!(!restored.preview);
        assert_eq!(restored.active.unwrap().token, first_token);
        assert!(restored.revision > a.revision);
        assert!(themes.confirm(&token, |_| Ok(())).is_err());
        themes.release(&token);
        assert!(
            themes
                .resource(&format!("@theme/{token}/main.css"), false)
                .is_err()
        );
    }
}
