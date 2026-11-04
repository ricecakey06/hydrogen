use super::settings::Settings;
use hydrogen_plugin_services::{
    contracts::{ApiError, Result},
    storage::*,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Config {
    schema_version: u32,
    format: String,
    settings: Settings,
    selected_instance_id: Option<String>,
}

pub fn checked_game_root(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() || path.ancestors().any(linked) || (path.exists() && !path.is_dir()) {
        return Err(ApiError::new(
            "INVALID_GAME_ROOT",
            "请选择不含目录链接的完整游戏目录。",
        ));
    }
    Ok(if path.exists() {
        dunce::canonicalize(path)?
    } else {
        path.to_owned()
    })
}
#[derive(Serialize, Deserialize)]
pub struct Store {
    pub root: PathBuf,
    pub game_root: PathBuf,
    pub default_game_root: PathBuf,
    pub settings: Settings,
    pub selected: Option<String>,
    pub notice: Option<String>,
    pub read_only: bool,
}
impl Store {
    pub fn open(root: PathBuf, game_root: PathBuf) -> Result<Self> {
        let mut store = Self {
            root,
            default_game_root: game_root.clone(),
            game_root,
            settings: Settings::default(),
            selected: None,
            notice: None,
            read_only: false,
        };
        let config = within(&store.root, "settings.json")?;
        if config.exists() {
            let result = read_json::<Config>(&config).and_then(|mut file| {
                if file.schema_version != 1 || file.format != "hydrogen-tauri" {
                    return Err(ApiError::new(
                        "CONFIG_VERSION",
                        "配置版本不兼容，原文件已保留。",
                    ));
                }
                if file.settings.microsoft_client_id.is_empty()
                    || file.settings.microsoft_client_id == "c0643bbe-d204-4e4d-a8a2-e57c2497589f"
                {
                    file.settings.microsoft_client_id = store.settings.microsoft_client_id.clone();
                }
                file.settings.sync_account_mode();
                file.settings.validate()?;
                Ok(file)
            });
            match result {
                Ok(file) => {
                    store.settings = file.settings;
                    store.selected = file.selected_instance_id;
                }
                Err(error) => {
                    store.notice = Some(error.message);
                    store.read_only = true;
                }
            }
        }
        if let Some(directory) = &store.settings.game_directory {
            store.game_root = checked_game_root(Path::new(directory))?;
        }
        Ok(store)
    }
    pub fn writable(&self) -> Result<()> {
        if self.read_only {
            Err(ApiError::new(
                "CONFIG_READ_ONLY",
                self.notice.as_deref().unwrap_or("配置不可写。"),
            ))
        } else {
            Ok(())
        }
    }
    pub fn save(&mut self, mut settings: Settings) -> Result<()> {
        self.writable()?;
        settings.sync_account_mode();
        settings.validate()?;
        let game_root = if let Some(directory) = &settings.game_directory {
            checked_game_root(Path::new(directory))?
        } else {
            self.default_game_root.clone()
        };
        let path = within(&self.root, "settings.json")?;
        if path.exists() {
            let current: Config = read_json(&path)?;
            if current.schema_version != 1 || current.format != "hydrogen-tauri" {
                return Err(ApiError::new(
                    "CONFIG_VERSION",
                    "配置已由其他版本更改，拒绝覆盖。",
                ));
            }
        }
        write_json(
            &path,
            &Config {
                schema_version: 1,
                format: "hydrogen-tauri".into(),
                settings: settings.clone(),
                selected_instance_id: self.selected.clone(),
            },
        )?;
        if self.game_root != game_root {
            self.game_root = game_root;
        }

        self.settings = settings;
        Ok(())
    }
}
