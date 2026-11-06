use hydrogen_plugin_services::{
    contracts::{ApiError, Result},
    net::Client,
    storage,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
};
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub name: String,
    pub uuid: String,
    pub access_token: String,
    pub xuid: String,
    pub client_id: String,
    pub expires: u64,
    pub offline: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Java {
    pub path: String,
    pub version: String,
    pub major: u32,
    pub architecture: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Prepared {
    pub metadata: Value,
    pub classpath: Vec<PathBuf>,
    pub natives: Vec<(PathBuf, Vec<String>)>,
    pub assets: PathBuf,
    pub libraries: PathBuf,
    pub asset_index: String,
    pub logging: Option<String>,
}
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
    pub window_mode: String,
    pub window_width: u32,
    pub window_height: u32,
    pub after_launch: String,
    pub jvm_arguments: String,
    pub download_source: String,
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
fn allows(
    client: &Client,
    owner: &Value,
    architecture: &str,
    custom_resolution: bool,
) -> Result<bool> {
    Ok(client.service(
        "minecraft.content",
        "allows",
        json!({"owner":owner,"architecture":architecture,"customResolution":custom_resolution}),
    )? == true)
}
fn jvm_arguments(client: &Client, text: &str) -> Result<Vec<String>> {
    Ok(serde_json::from_value(client.service(
        "storage",
        "jvmArguments",
        json!({"text":text}),
    )?)?)
}
fn add_lwjgl_unsafe_agent(
    client: &Client,
    game: &Prepared,
    java: &Java,
    directory: &Path,
    args: &mut Vec<String>,
) -> Result<()> {
    if java.major < 25
        || args.iter().any(|arg| {
            arg.strip_prefix("-javaagent:")
                .and_then(|path| path.split('=').next())
                .and_then(|path| path.rsplit(['/', '\\']).next())
                .is_some_and(|name| {
                    name == "lwjgl-unsafe-agent.jar"
                        || (name.starts_with("lwjgl-unsafe-agent-") && name.ends_with(".jar"))
                })
        })
    {
        return Ok(());
    }
    for library in game.metadata["libraries"].as_array().into_iter().flatten() {
        let name = library["name"]
            .as_str()
            .map(|name| name.trim_end_matches("@jar"));
        if matches!(
            name,
            Some("org.lwjgl:lwjgl:3.4.0" | "org.lwjgl:lwjgl:3.4.1")
        ) && allows(client, library, &java.architecture, false)?
        {
            const JAR: &[u8] = include_bytes!("../assets/lwjgl-unsafe-agent-2.0.jar");
            // Resolve from the game's working directory so '=' in its path is safe.
            const RELATIVE: &str = ".hydrogen/lwjgl-unsafe-agent-2.0.jar";
            let path = storage::within(directory, RELATIVE)?;
            let cached = || {
                let mut bytes = Vec::new();
                File::open(&path)
                    .and_then(|file| file.take(JAR.len() as u64 + 1).read_to_end(&mut bytes))
                    .is_ok()
                    && bytes == JAR
            };
            if !cached()
                && let Err(error) = storage::atomic_write(&path, JAR)
                && !cached()
            {
                return Err(error);
            }
            args.push(format!("-javaagent:{RELATIVE}"));
            break;
        }
    }
    Ok(())
}
pub fn arguments(
    client: &hydrogen_plugin_services::net::Client,
    game: &Prepared,
    java: &Java,
    session: &Session,
    settings: &Settings,
    directory: &Path,
    natives: &Path,
) -> Result<Vec<String>> {
    let classpath = std::env::join_paths(&game.classpath)
        .map_err(|_| ApiError::new("INVALID_CLASSPATH", "类路径无法用于此系统。"))?
        .to_string_lossy()
        .into_owned();
    let owned = [
        ("auth_player_name", session.name.clone()),
        ("auth_uuid", session.uuid.clone()),
        (
            "auth_session",
            if session.offline {
                "0".into()
            } else {
                format!("token:{}:{}", session.access_token, session.uuid)
            },
        ),
        (
            "library_directory",
            game.libraries.clone().to_string_lossy().into(),
        ),
        (
            "libraries_directory",
            game.libraries.clone().to_string_lossy().into(),
        ),
        ("auth_access_token", session.access_token.clone()),
        ("auth_xuid", session.xuid.clone()),
        ("clientid", session.client_id.clone()),
        (
            "user_type",
            if session.offline { "legacy" } else { "msa" }.into(),
        ),
        ("user_properties", "{}".into()),
        (
            "version_name",
            hydrogen_plugin_services::text(&game.metadata, "id")?.into(),
        ),
        (
            "version_type",
            hydrogen_plugin_services::text(&game.metadata, "type")?.into(),
        ),
        ("game_directory", directory.to_string_lossy().into()),
        ("assets_root", game.assets.to_string_lossy().into()),
        ("game_assets", game.assets.to_string_lossy().into()),
        ("assets_index_name", game.asset_index.clone()),
        ("natives_directory", natives.to_string_lossy().into()),
        ("launcher_name", "Hydrogen".into()),
        ("launcher_version", env!("CARGO_PKG_VERSION").into()),
        ("classpath", classpath),
        (
            "classpath_separator",
            if cfg!(windows) { ";" } else { ":" }.into(),
        ),
        ("resolution_width", settings.window_width.to_string()),
        ("resolution_height", settings.window_height.to_string()),
    ];
    let values: BTreeMap<_, _> = owned.into_iter().collect();
    let placeholder = regex::Regex::new(r"\$\{([^}]+)\}").unwrap();
    let substitute = |argument: &str| -> Result<String> {
        let mut output = argument.to_string();
        for capture in placeholder.captures_iter(argument) {
            let value = values.get(&capture[1]).ok_or_else(|| {
                ApiError::new("UNSUPPORTED_ARGUMENT", "游戏包含尚未支持的启动参数。")
            })?;
            output = output.replace(&capture[0], value);
        }
        Ok(output)
    };
    let expand = |key: &str| -> Result<Vec<String>> {
        let mut args = Vec::new();
        for entry in game.metadata["arguments"][key]
            .as_array()
            .ok_or_else(|| ApiError::new("INVALID_ARGUMENTS", "游戏缺少启动参数。"))?
        {
            if let Some(value) = entry.as_str() {
                args.push(substitute(value)?);
            } else if allows(
                client,
                entry,
                &java.architecture,
                settings.window_mode == "custom",
            )? {
                if let Some(value) = entry["value"].as_str() {
                    args.push(substitute(value)?);
                } else {
                    for value in entry["value"]
                        .as_array()
                        .ok_or_else(|| ApiError::new("INVALID_ARGUMENTS", "启动参数格式无效。"))?
                    {
                        args.push(substitute(value.as_str().ok_or_else(|| {
                            ApiError::new("INVALID_ARGUMENTS", "启动参数必须是字符串。")
                        })?)?);
                    }
                }
            }
        }
        Ok(args)
    };
    let mut args = expand("jvm")?;
    args.push(format!("-Xmx{}M", settings.max_memory_mib));
    if let Some(logging) = &game.logging {
        args.push(logging.clone());
    }
    args.extend(jvm_arguments(client, &settings.jvm_arguments)?);
    add_lwjgl_unsafe_agent(client, game, java, directory, &mut args)?;
    let main = hydrogen_plugin_services::text(&game.metadata, "mainClass")?;
    if !regex::Regex::new(r"^[a-zA-Z_$][a-zA-Z0-9_.$]+$")
        .unwrap()
        .is_match(main)
    {
        return Err(ApiError::new("INVALID_MAIN_CLASS", "游戏入口类无效。"));
    }
    args.push(main.into());
    args.extend(expand("game")?);
    if settings.window_mode == "fullscreen" {
        args.push("--fullscreen".into());
    }
    Ok(args)
}
pub fn extract(game: &Prepared, directory: &Path) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let mut total = 0_u64;
    for (path, excludes) in &game.natives {
        let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?)
            .map_err(|_| ApiError::new("INVALID_NATIVES", "原生库压缩包损坏。"))?;
        for index in 0..archive.len() {
            let file = archive
                .by_index(index)
                .map_err(|_| ApiError::new("INVALID_NATIVES", "原生库条目无效。"))?;
            let name = file.name().replace('\\', "/");
            if file.is_dir()
                || name.to_uppercase().starts_with("META-INF/")
                || excludes.iter().any(|prefix| name.starts_with(prefix))
            {
                continue;
            }
            if file
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err(ApiError::new("LINKED_NATIVES", "原生库不能包含链接。"));
            }
            total += file.size();
            if total > 512 * 1024 * 1024 || index > 10000 {
                return Err(ApiError::new("NATIVES_LIMIT", "原生库解压超过限制。"));
            }
            let output = storage::within(directory, &name)?;
            std::fs::create_dir_all(output.parent().unwrap())?;
            let mut destination = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(output)?;
            std::io::copy(&mut file.take(512 * 1024 * 1024 + 1), &mut destination)?;
        }
    }
    Ok(())
}
use super::services::Log;
#[derive(Default)]
pub struct OutputLines {
    line: Vec<u8>,
    oversized: bool,
}
impl OutputLines {
    pub fn feed(&mut self, bytes: &[u8], log: &mut Log) {
        for byte in bytes {
            if *byte == b'\n' {
                if self.oversized {
                    log.line("[oversized output omitted]");
                } else {
                    log.line(&String::from_utf8_lossy(&self.line));
                }
                self.line.clear();
                self.oversized = false;
            } else if self.line.len() < 65536 {
                self.line.push(*byte);
            } else {
                self.oversized = true;
            }
        }
    }
    pub fn finish(&mut self, log: &mut Log) {
        if !self.line.is_empty() && !self.oversized {
            log.line(&String::from_utf8_lossy(&self.line));
        }
        self.line.clear();
    }
}
