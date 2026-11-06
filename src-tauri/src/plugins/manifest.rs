use crate::contracts::{ApiError, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
};

pub const MAX_MESSAGE: usize = 256 * 1024;
pub const MAX_SOURCE: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub api_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub version: String,
    pub runtime: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub execution_thread: String,
    pub entry: String,
    #[serde(default)]
    pub activation: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<serde_json::Value>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub scopes: Scopes,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provides: Vec<Service>,
    #[serde(default)]
    pub contributes: Contributions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontend: Option<Frontend>,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scopes {
    #[serde(default)]
    pub http: Vec<String>,
    #[serde(default)]
    pub process: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frontend {
    pub entry: String,
    #[serde(default)]
    pub styles: Vec<String>,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contributions {
    #[serde(default)]
    pub commands: Vec<Command>,
    #[serde(default)]
    pub panels: Vec<Panel>,
    #[serde(default)]
    pub views: Vec<UiContribution>,
    #[serde(default)]
    pub menus: Vec<UiContribution>,
    #[serde(default)]
    pub providers: Vec<Provider>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<super::themes::Contribution>,
}
pub const UI_LOCATIONS: &[&str] = &[
    "sidebar.main",
    "sidebar.footer",
    "home.widgets",
    "home.actions",
    "settings.sections",
    "instance.tabs",
    "instance.toolbar",
    "instance.contextMenu",
    "mods.toolbar",
    "mods.contextMenu",
    "resources.actions",
    "profiles.actions",
    "downloads.taskActions",
    "titlebar.actions",
    "status.items",
    "commands.palette",
];
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiContribution {
    #[serde(default)]
    pub id: String,
    pub location: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub panel: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub when: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub service: String,
    pub version: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    pub id: String,
    pub version: String,
    pub methods: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub id: String,
    pub title: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Panel {
    pub id: String,
    pub title: String,
    pub entry: String,
    #[serde(default = "inherit", skip_serializing_if = "is_inherit")]
    pub appearance: String,
}
fn inherit() -> String {
    "inherit".into()
}
fn is_inherit(value: &str) -> bool {
    value == "inherit"
}

#[derive(Debug, Clone)]
pub struct Plugin {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub digest: String,
}

pub fn dependency(value: &serde_json::Value) -> Result<(&str, semver::VersionReq)> {
    let (id, version) = if let Some(id) = value.as_str() {
        (id, "*")
    } else {
        let object = value.as_object().ok_or_else(|| {
            ApiError::new(
                "INVALID_DEPENDENCY",
                "插件依赖应是 ID 或包含 id、version 的对象。",
            )
        })?;
        if object
            .keys()
            .any(|k| k != "id" && k != "version" && k != "optional")
        {
            return Err(ApiError::new(
                "INVALID_DEPENDENCY",
                "插件依赖包含未知字段。",
            ));
        }
        if object
            .get("optional")
            .is_some_and(|value| !value.is_boolean())
        {
            return Err(ApiError::new(
                "INVALID_DEPENDENCY",
                "optional 必须为布尔值。",
            ));
        }
        (
            object.get("id").and_then(|v| v.as_str()).unwrap_or(""),
            object
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("*"),
        )
    };
    if !valid_id(id) {
        return Err(ApiError::new("INVALID_DEPENDENCY", "插件依赖 ID 无效。"));
    }
    Ok((
        id,
        semver::VersionReq::parse(version)
            .map_err(|_| ApiError::new("INVALID_DEPENDENCY", "插件依赖版本范围无效。"))?,
    ))
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id.as_bytes()[0].is_ascii_lowercase()
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
        && !id.contains("..")
        && !id.ends_with('.')
}

pub fn safe_relative(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 240
        && !name.contains(['\\', ':', '%', '\0'])
        && Path::new(name)
            .components()
            .all(|p| matches!(p, Component::Normal(_)))
        && name.split('/').all(|part| {
            let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !part
                    .chars()
                    .any(|c| c.is_control() || "<>\"|?*".contains(c))
                && !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.as_bytes()[3].is_ascii_digit())
        })
}

pub fn within(root: &Path, name: &str) -> Result<PathBuf> {
    if !safe_relative(name) {
        return Err(ApiError::new("INVALID_PATH", "仅允许包内的普通相对路径。"));
    }
    let root = dunce::canonicalize(root)?;
    let mut current = root.clone();
    for part in name.split('/') {
        current.push(part);
        let meta = fs::symlink_metadata(&current)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(ApiError::new("INVALID_PATH", "不允许符号链接或目录联接。"));
            }
        }
        if meta.file_type().is_symlink() {
            return Err(ApiError::new("INVALID_PATH", "不允许符号链接。"));
        }
    }
    let path = dunce::canonicalize(current)?;
    if !path.starts_with(root) {
        return Err(ApiError::new("INVALID_PATH", "路径越过插件目录。"));
    }
    Ok(path)
}

pub fn load(root: &Path) -> Result<Plugin> {
    let root = dunce::canonicalize(root)?;
    let path = within(&root, "plugin.json")?;
    if fs::metadata(&path)?.len() > 64 * 1024 {
        return Err(ApiError::new("MANIFEST_TOO_LARGE", "清单不能超过 64 KiB。"));
    }
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    let manifest: Manifest = serde_json::from_value(value)?;
    if !matches!(
        (
            manifest.runtime.as_str(),
            manifest.schema_version,
            manifest.api_version
        ),
        ("javascript", 3, 2) | ("native", 2 | 3, 1)
    ) {
        return Err(ApiError::new(
            "INCOMPATIBLE_PLUGIN",
            "JavaScript 插件需要 schemaVersion 3 / API 2；原生插件需要 schemaVersion 2 或 3 / ABI 1。",
        ));
    }
    if !valid_id(&manifest.id) {
        return Err(ApiError::new("INVALID_PLUGIN_ID", "插件 ID 非法。"));
    }
    let version: Vec<_> = manifest.version.split('.').collect();
    if version.len() != 3
        || version.iter().any(|n| n.parse::<u32>().is_err())
        || manifest.name.trim().is_empty()
        || manifest.name.len() > 200
    {
        return Err(ApiError::new(
            "INVALID_MANIFEST",
            "请提供名称与三段数字版本号。",
        ));
    }
    let mut dependencies = HashSet::new();
    for value in &manifest.dependencies {
        let (id, _) = dependency(value)?;
        if id == manifest.id || !dependencies.insert(id) {
            return Err(ApiError::new(
                "INVALID_DEPENDENCY",
                "插件依赖重复或指向自身。",
            ));
        }
    }
    let allowed = [
        "instances.read",
        "instances.write",
        "instances.repair",
        "instances.launch",
        "accounts.read",
        "accounts.write",
        "java.read",
        "java.scan",
        "resources.read",
        "resources.install",
        "mods.read",
        "mods.write",
        "modpacks.read",
        "modpacks.write",
        "settings.read",
        "settings.write",
        "storage",
        "http",
        "files",
        "process",
        "tasks.read",
        "tasks.cancel",
        "events",
        "hooks",
        "services.call",
        "log",
        "ui.contribute",
        "ui.panel",
        "native.code",
        "ui.host",
        "theme.contribute",
    ];
    if manifest
        .permissions
        .iter()
        .any(|p| !allowed.contains(&p.as_str()))
    {
        return Err(ApiError::new(
            "UNSUPPORTED_PERMISSION",
            "清单请求了宿主尚未提供的能力。",
        ));
    }
    if manifest.scopes.http.len() > 64
        || manifest.scopes.process.len() > 16
        || manifest.scopes.http.iter().any(|host| {
            host.is_empty()
                || host.len() > 253
                || host.contains(['/', ':', '@', '*', ' '])
                || !manifest.permissions.iter().any(|p| p == "http")
        })
        || manifest.scopes.process.iter().any(|path| {
            !Path::new(path).is_absolute()
                || path.len() > 4096
                || !manifest.permissions.iter().any(|p| p == "process")
        })
    {
        return Err(ApiError::new(
            "INVALID_SCOPE",
            "网络域名或进程范围声明无效。",
        ));
    }
    if manifest.runtime == "javascript"
        && manifest.activation.iter().any(|event| {
            event != "startup"
                && !["command:", "view:", "service:", "provider:", "hook:"]
                    .iter()
                    .any(|prefix| event.starts_with(prefix))
        })
    {
        return Err(ApiError::new("INVALID_ACTIVATION", "激活事件不受支持。"));
    }
    let mut ids = HashSet::new();
    for c in &manifest.contributes.commands {
        if !valid_id(&c.id) || !ids.insert(&c.id) || c.title.is_empty() || c.title.len() > 200 {
            return Err(ApiError::new(
                "INVALID_COMMAND",
                "命令 ID 必须合法且唯一，并提供标题。",
            ));
        }
    }
    for p in &manifest.contributes.panels {
        if !valid_id(&p.id) || !ids.insert(&p.id) || !p.entry.starts_with("ui/") {
            return Err(ApiError::new(
                "INVALID_PANEL",
                "面板 ID 必须唯一，入口必须在 ui/ 内。",
            ));
        }
        if !["inherit", "independent"].contains(&p.appearance.as_str()) {
            return Err(ApiError::new("INVALID_PANEL", "面板 appearance 无效。"));
        }
        within(&root, &p.entry)?;
    }
    for (index, item) in manifest
        .contributes
        .views
        .iter()
        .chain(&manifest.contributes.menus)
        .enumerate()
    {
        if !manifest.permissions.iter().any(|p| p == "ui.contribute")
            || !UI_LOCATIONS.contains(&item.location.as_str())
            || (!item.id.is_empty() && (!valid_id(&item.id) || !ids.insert(&item.id)))
            || (item.panel.is_some() == item.command.is_some())
            || (item.location == "instance.tabs" && item.panel.is_none())
            || item
                .panel
                .as_ref()
                .is_some_and(|id| !manifest.contributes.panels.iter().any(|p| &p.id == id))
            || item
                .command
                .as_ref()
                .is_some_and(|id| !manifest.contributes.commands.iter().any(|c| &c.id == id))
            || !matches!(
                item.when.as_deref(),
                None | Some("instanceSelected" | "modsSelected" | "taskActive")
            )
            || item.title.len() > 200
            || item.group.len() > 100
            || index >= 128
        {
            return Err(ApiError::new(
                "INVALID_UI_CONTRIBUTION",
                "界面贡献的位置、引用、条件或权限无效。",
            ));
        }
    }
    let mut provider_ids = HashSet::new();
    for provider in &manifest.contributes.providers {
        if !valid_id(&provider.id)
            || !provider_ids.insert(&provider.id)
            || ![
                "resourceSource",
                "accountProvider",
                "loaderProvider",
                "downloadSource",
            ]
            .contains(&provider.kind.as_str())
            || provider.title.is_empty()
            || provider.title.len() > 200
            || provider.version != "1.0.0"
            || !manifest.provides.iter().any(|s| s.id == provider.service)
        {
            return Err(ApiError::new(
                "INVALID_PROVIDER",
                "提供者必须引用已声明服务和受支持的契约。",
            ));
        }
    }
    if manifest.runtime == "javascript" {
        for provider in &manifest.contributes.providers {
            let required: &[&str] = match provider.kind.as_str() {
                "resourceSource" => &[
                    "filters",
                    "search",
                    "versions",
                    "dependencies",
                    "downloadPlan",
                ],
                "accountProvider" => &["view", "signIn", "session", "signOut"],
                "loaderProvider" => &["versions", "installPlan"],
                "downloadSource" => &["resolve"],
                _ => &[],
            };
            if !manifest.provides.iter().any(|s| {
                s.id == provider.service
                    && semver::Version::parse(&s.version)
                        .is_ok_and(|v| semver::VersionReq::parse("^1").unwrap().matches(&v))
                    && required
                        .iter()
                        .all(|method| s.methods.iter().any(|m| m == method))
            }) {
                return Err(ApiError::new(
                    "INVALID_PROVIDER",
                    "JS 提供者缺少契约要求的方法或服务版本不兼容。",
                ));
            }
        }
    }
    let mut services = HashSet::new();
    for service in &manifest.provides {
        if !valid_id(&service.id)
            || !services.insert(&service.id)
            || semver::Version::parse(&service.version).is_err()
            || service.methods.is_empty()
            || service.methods.len() > 64
        {
            return Err(ApiError::new(
                "INVALID_SERVICE",
                "服务标识、版本或方法声明无效。",
            ));
        }
        let mut methods = HashSet::new();
        for method in &service.methods {
            if method.is_empty()
                || method.len() > 100
                || !method
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.')
                || !methods.insert(method)
                || method.starts_with("plugin.")
            {
                return Err(ApiError::new("INVALID_SERVICE", "服务方法非法或重复。"));
            }
        }
    }
    if !["", "worker", "main"].contains(&manifest.execution_thread.as_str())
        || manifest.execution_thread == "main" && manifest.runtime != "native"
    {
        return Err(ApiError::new(
            "INVALID_EXECUTION_THREAD",
            "线程声明仅支持原生插件的 worker 或 main。",
        ));
    }
    if let Some(frontend) = &manifest.frontend {
        if !manifest.permissions.iter().any(|p| p == "ui.host")
            || !frontend.entry.starts_with("assets/")
            || !frontend.entry.ends_with(".js")
        {
            return Err(ApiError::new(
                "INVALID_FRONTEND",
                "宿主界面需要 ui.host 权限和包内 assets JavaScript 入口。",
            ));
        }
        for file in std::iter::once(&frontend.entry).chain(frontend.styles.iter()) {
            if !within(&root, file)?.is_file() {
                return Err(ApiError::new("MISSING_FRONTEND", "界面实现资源缺失。"));
            }
        }
    }
    let valid_entry = if manifest.runtime == "native" {
        manifest.permissions.iter().any(|p| p == "native.code")
            && manifest.entry.starts_with("bin/")
            && manifest.entry.ends_with(if cfg!(windows) {
                ".dll"
            } else if cfg!(target_os = "macos") {
                ".dylib"
            } else {
                ".so"
            })
    } else {
        manifest.entry.ends_with(".js") && !manifest.permissions.iter().any(|p| p == "native.code")
    };
    if !valid_entry {
        return Err(ApiError::new(
            "INVALID_ENTRY",
            "JS 入口必须为包内 .js；原生入口必须在 bin/ 下、匹配当前平台并声明 native.code。",
        ));
    }
    within(&root, &manifest.entry)?;
    let (_, digest) = super::package::files(&root)?;
    let plugin = Plugin {
        root,
        manifest,
        digest,
    };
    super::themes::validate(&plugin)?;
    Ok(plugin)
}

// Runtime calls execute already verified shadow libraries. Check the package tree
// for changes without reading every DLL again. Installation, activation, reload
// and frontend resource delivery deliberately continue to use load().
#[derive(PartialEq, Eq)]
struct FileStamp {
    path: PathBuf,
    len: u64,
    modified: std::time::SystemTime,
    created: Option<std::time::SystemTime>,
    directory: bool,
}

fn stamps(root: &Path) -> Result<Vec<FileStamp>> {
    fn walk(path: &Path, depth: usize, out: &mut Vec<FileStamp>) -> Result<()> {
        if depth > 32 || out.len() >= 8192 {
            return Err(ApiError::new("PACKAGE_LIMIT", "插件目录超过限制。"));
        }
        let meta = fs::symlink_metadata(path)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(ApiError::new("INVALID_PATH", "不允许目录联接。"));
            }
        }
        if meta.file_type().is_symlink() || (!meta.is_dir() && !meta.is_file()) {
            return Err(ApiError::new("INVALID_PATH", "仅允许普通插件文件。"));
        }
        out.push(FileStamp {
            path: path.to_owned(),
            len: meta.len(),
            modified: meta.modified()?,
            created: meta.created().ok(),
            directory: meta.is_dir(),
        });
        if meta.is_dir() {
            for entry in fs::read_dir(path)? {
                walk(&entry?.path(), depth + 1, out)?;
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, 0, &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

pub(super) fn load_runtime(root: &Path) -> Result<Plugin> {
    use std::sync::{Mutex, OnceLock};
    type Cache = std::collections::HashMap<PathBuf, (Vec<FileStamp>, Plugin)>;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let current = stamps(root)?;
    if let Some((previous, plugin)) = cache.lock().unwrap().get(root)
        && *previous == current
    {
        return Ok(plugin.clone());
    }
    let plugin = load(root)?;
    // Never associate a digest with metadata captured across a concurrent edit.
    if stamps(root)? != current {
        return Err(ApiError::new(
            "PLUGIN_CHANGED",
            "校验期间插件文件发生变化。",
        ));
    }
    let mut cache = cache.lock().unwrap();
    if cache.len() >= 128 {
        cache.clear();
    }
    cache.insert(root.to_owned(), (current, plugin.clone()));
    Ok(plugin)
}

#[cfg(test)]
mod runtime_cache_tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("plugin.json"), r#"{"schemaVersion":3,"apiVersion":2,"id":"cache.test","name":"Cache","version":"1.0.0","runtime":"javascript","entry":"main.js","permissions":[]}"#).unwrap();
        fs::write(dir.path().join("main.js"), "export const value = 1;").unwrap();
        dir
    }

    #[test]
    fn runtime_cache_detects_edits_additions_deletions_and_manifest_changes() {
        let dir = fixture();
        let root = dir.path();
        let original = load_runtime(root).unwrap();
        assert_eq!(load_runtime(root).unwrap().digest, original.digest);
        let entry = root.join("main.js");
        let modified = fs::metadata(&entry).unwrap().modified().unwrap();
        fs::write(&entry, "export const value = 2;").unwrap();
        fs::File::options()
            .write(true)
            .open(&entry)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(modified + std::time::Duration::from_secs(1)),
            )
            .unwrap();
        let edited = load_runtime(root).unwrap();
        assert_ne!(edited.digest, original.digest);
        fs::write(root.join("extra.txt"), "extra").unwrap();
        assert_ne!(load_runtime(root).unwrap().digest, edited.digest);
        fs::remove_file(root.join("extra.txt")).unwrap();
        assert_eq!(load_runtime(root).unwrap().digest, edited.digest);
        fs::write(root.join("plugin.json"), "{}").unwrap();
        assert!(load_runtime(root).is_err());
        fs::remove_file(entry).unwrap();
        assert!(load_runtime(root).is_err());
    }

    #[test]
    #[ignore = "manual package verification throughput benchmark"]
    fn runtime_cache_benchmark() {
        let dir = fixture();
        fs::write(dir.path().join("payload.bin"), vec![42u8; 16 * 1024 * 1024]).unwrap();
        let expected = load_runtime(dir.path()).unwrap().digest;
        let start = std::time::Instant::now();
        for _ in 0..50 {
            assert_eq!(load(dir.path()).unwrap().digest, expected);
        }
        let full = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..50 {
            assert_eq!(load_runtime(dir.path()).unwrap().digest, expected);
        }
        eprintln!(
            "50 checks of 16 MiB package: full={full:?}, runtime={:?}",
            start.elapsed()
        );
    }
}
