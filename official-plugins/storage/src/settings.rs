use hydrogen_plugin_services::contracts::{ApiError, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub theme: hydrogen_plugin_sdk::contracts::ThemeChoice,
    #[serde(default = "default_locale")]
    pub locale: String,
    #[serde(default)]
    pub game_directory: Option<String>,
    #[serde(default)]
    pub isolate_game_directory: Option<bool>,
    pub java: Option<String>,
    pub max_memory_mib: u32,
    #[serde(default = "default_gc_mode")]
    pub gc_mode: String,
    pub window_mode: String,
    pub window_width: u32,
    pub window_height: u32,
    pub after_launch: String,
    pub jvm_arguments: String,
    pub download_source: String,
    #[serde(default = "default_mod_mirror")]
    pub use_mod_mirror: bool,
    // Retained for existing configuration files; availability follows the plugin state.
    pub offline_enabled: bool,
    pub offline_player_name: String,
    pub account_mode: String,
    pub microsoft_client_id: String,
    pub modules: BTreeMap<String, bool>,
}
fn default_locale() -> String {
    "zh-CN".into()
}
fn default_mod_mirror() -> bool {
    true
}
fn default_gc_mode() -> String {
    "g1".into()
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Default::default(),
            locale: default_locale(),
            game_directory: None,
            isolate_game_directory: None,
            java: None,
            max_memory_mib: memory_advice().recommended_mib,
            gc_mode: default_gc_mode(),
            window_mode: "default".into(),
            window_width: 854,
            window_height: 480,
            after_launch: "minimize".into(),
            jvm_arguments: String::new(),
            download_source: "official".into(),
            use_mod_mirror: true,
            offline_enabled: true,
            offline_player_name: "Player".into(),
            account_mode: "microsoft".into(),
            microsoft_client_id: std::env::var("HYDROGEN_MICROSOFT_CLIENT_ID")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| option_env!("HYDROGEN_MICROSOFT_CLIENT_ID").map(str::to_owned))
                .unwrap_or_default(),
            modules: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryAdvice {
    pub total_mib: u32,
    pub recommended_mib: u32,
}
pub fn memory_advice() -> MemoryAdvice {
    static MEMORY: std::sync::OnceLock<MemoryAdvice> = std::sync::OnceLock::new();
    *MEMORY.get_or_init(|| {
        let total = {
            #[cfg(windows)]
            {
                #[repr(C)]
                struct Memory {
                    length: u32,
                    load: u32,
                    total: u64,
                    available: u64,
                    page: u64,
                    page_available: u64,
                    virtual_total: u64,
                    virtual_available: u64,
                    extended: u64,
                }
                #[link(name = "kernel32")]
                unsafe extern "system" {
                    fn GlobalMemoryStatusEx(memory: *mut Memory) -> i32;
                }
                let mut memory = Memory {
                    length: 64,
                    load: 0,
                    total: 0,
                    available: 0,
                    page: 0,
                    page_available: 0,
                    virtual_total: 0,
                    virtual_available: 0,
                    extended: 0,
                };
                if unsafe { GlobalMemoryStatusEx(&mut memory) } != 0 {
                    (memory.total / (1024 * 1024)).clamp(512, u32::MAX as u64) as u32
                } else {
                    65536
                }
            }
            #[cfg(not(windows))]
            {
                65536
            }
        };
        let recommended = (total / 4 / 256 * 256)
            .clamp(2048, 8192)
            .min((total / 2 / 256 * 256).max(512));
        MemoryAdvice {
            total_mib: total,
            recommended_mib: recommended,
        }
    })
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Overrides {
    pub java: Option<String>,
    pub max_memory_mib: Option<u32>,
    pub gc_mode: Option<String>,
    pub jvm_arguments: Option<String>,
}
pub fn jvm_arguments(text: &str) -> Result<Vec<String>> {
    let invalid = || {
        ApiError::new(
            "INVALID_JVM",
            "自定义 JVM 参数不能覆盖内存、身份、入口、类路径或启动器管理的目录；请检查引号和选项值。",
        )
    };
    if text.len() > 32768 || text.chars().any(|c| c.is_control() && !c.is_whitespace()) {
        return Err(invalid());
    }
    let mut args = Vec::new();
    let mut value = String::new();
    let mut quote = None;
    let mut started = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && chars
                .peek()
                .is_some_and(|next| Some(*next) == quote || *next == '"')
        {
            value.push(chars.next().unwrap());
            started = true;
        } else if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                value.push(c);
            }
        } else if c == '\'' || c == '"' {
            quote = Some(c);
            started = true;
        } else if c.is_whitespace() {
            if started {
                args.push(std::mem::take(&mut value));
                started = false;
            }
        } else {
            value.push(c);
            started = true;
        }
    }
    if quote.is_some() {
        return Err(invalid());
    }
    if started {
        args.push(value);
    }
    let mut needs_value = false;
    for arg in &args {
        if needs_value {
            if arg.is_empty() || arg.starts_with(['-', '@']) {
                return Err(invalid());
            }
            needs_value = false;
            continue;
        }
        let key = arg.split('=').next().unwrap_or("");
        if !arg.starts_with('-')
            || [
                "-cp",
                "-classpath",
                "--class-path",
                "-jar",
                "-m",
                "--module",
                "-p",
                "--module-path",
                "--",
                "--source",
                "-Djava.class.path",
                "-Djava.library.path",
                "-Duser.dir",
                "-Djna.tmpdir",
                "-Dorg.lwjgl.system.SharedLibraryExtractPath",
                "-Dio.netty.native.workdir",
                "-XX:MaxHeapSize",
                "-XX:InitialHeapSize",
                "-XX:MinHeapSize",
                "-XX:SoftMaxHeapSize",
                "-XX:MaxRAMPercentage",
                "-XX:InitialRAMPercentage",
                "-XX:MinRAMPercentage",
                "-XX:Flags",
                "-XX:VMOptionsFile",
            ]
            .contains(&key)
            || [
                "-Xmx",
                "-Xms",
                "-Xmn",
                "-Xbootclasspath",
                "-Dminecraft.",
                "-Dlog4j",
            ]
            .iter()
            .any(|s| arg.starts_with(s))
        {
            return Err(invalid());
        }
        needs_value = [
            "--add-opens",
            "--add-exports",
            "--add-reads",
            "--add-modules",
            "--limit-modules",
        ]
        .contains(&arg.as_str());
    }
    if needs_value {
        return Err(invalid());
    }
    Ok(args)
}
impl Settings {
    pub(crate) fn sync_account_mode(&mut self) {
        self.offline_enabled = self.enabled("account.offline");
        if !self.offline_enabled && self.account_mode == "offline" {
            self.account_mode = "microsoft".into();
        }
    }

    pub fn validate(&self) -> Result<()> {
        if !self.theme.is_valid() {
            return Err(ApiError::new("THEME_INVALID", "主题选择无效。"));
        }
        if self.locale.len() > 35
            || self.locale.is_empty()
            || !self
                .locale
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(ApiError::new("INVALID_LOCALE", "语言标识无效。"));
        }
        if !(512..=memory_advice().total_mib).contains(&self.max_memory_mib)
            || !["preferZgc", "preferGenerationalZgc", "g1", "tunedG1"]
                .contains(&self.gc_mode.as_str())
            || !["default", "custom", "fullscreen"].contains(&self.window_mode.as_str())
            || !(320..=16384).contains(&self.window_width)
            || !(240..=16384).contains(&self.window_height)
            || !["minimize", "keep"].contains(&self.after_launch.as_str())
            || (!["official", "bmclapi"].contains(&self.download_source.as_str())
                && !provider_id(&self.download_source))
            || (!["microsoft", "offline"].contains(&self.account_mode.as_str())
                && !provider_id(&self.account_mode))
        {
            return Err(ApiError::new(
                "INVALID_SETTINGS",
                "内存、窗口尺寸、账号模式或下载来源无效。",
            ));
        }
        if self.offline_player_name.len() < 3
            || self.offline_player_name.len() > 16
            || !self
                .offline_player_name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            return Err(ApiError::new(
                "INVALID_OFFLINE_NAME",
                "离线名称应为 3–16 位英文字母、数字或下划线。",
            ));
        }
        if self
            .java
            .as_ref()
            .is_some_and(|p| !Path::new(p).is_absolute())
        {
            return Err(ApiError::new(
                "INVALID_JAVA",
                "请选择 Java 可执行文件的完整路径。",
            ));
        }
        if !self.microsoft_client_id.is_empty()
            && uuid::Uuid::parse_str(&self.microsoft_client_id).is_err()
        {
            return Err(ApiError::new(
                "INVALID_CLIENT_ID",
                "Microsoft Client ID 应为注册应用的 UUID。",
            ));
        }
        jvm_arguments(&self.jvm_arguments)?;
        Ok(())
    }
    pub fn effective(&self, overrides: &Overrides) -> Result<Self> {
        let mut settings = self.clone();
        if let Some(java) = &overrides.java {
            settings.java = (java != "automatic").then(|| java.clone());
        }
        if let Some(memory) = overrides.max_memory_mib {
            settings.max_memory_mib = memory;
        }
        if let Some(mode) = &overrides.gc_mode {
            settings.gc_mode = mode.clone();
        }
        if let Some(jvm) = &overrides.jvm_arguments {
            settings.jvm_arguments = jvm.clone();
        }
        settings.validate()?;
        Ok(settings)
    }
    pub fn enabled(&self, id: &str) -> bool {
        self.modules.get(id).copied().unwrap_or(true)
    }
}

fn provider_id(value: &str) -> bool {
    value.len() <= 201
        && value.split_once(':').is_some_and(|(a, b)| {
            !a.is_empty()
                && !b.is_empty()
                && a.bytes()
                    .chain(b.bytes())
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gc_settings_default_inherit_override_and_validate() {
        let mut old = serde_json::to_value(Settings::default()).unwrap();
        old.as_object_mut().unwrap().remove("gcMode");
        let mut settings: Settings = serde_json::from_value(old).unwrap();
        assert_eq!(settings.gc_mode, "g1");
        let inherited: Overrides = serde_json::from_value(serde_json::json!({})).unwrap();
        for mode in ["preferZgc", "preferGenerationalZgc", "g1", "tunedG1"] {
            settings.gc_mode = mode.into();
            assert_eq!(settings.effective(&inherited).unwrap().gc_mode, mode);
            let overrides: Overrides =
                serde_json::from_value(serde_json::json!({"gcMode": "tunedG1"})).unwrap();
            let effective = settings.effective(&overrides).unwrap();
            assert_eq!(effective.gc_mode, "tunedG1");
            assert_eq!(effective.max_memory_mib, settings.max_memory_mib);
            assert_eq!(
                serde_json::to_value(&overrides).unwrap()["gcMode"],
                "tunedG1"
            );
        }
        let overrides = Overrides {
            gc_mode: Some("unknown".into()),
            ..Default::default()
        };
        assert!(settings.effective(&overrides).is_err());
        settings.gc_mode = "unknown".into();
        assert!(settings.validate().is_err());
    }
}
