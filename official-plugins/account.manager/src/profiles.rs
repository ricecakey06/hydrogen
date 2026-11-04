use hydrogen_plugin_services::{
    Settings,
    contracts::{ApiError, Result},
    storage,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub uuid: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profiles {
    schema_version: u32,
    pub profiles: Vec<Profile>,
    pub microsoft: Option<String>,
    pub offline: Option<String>,
}

pub fn valid_name(name: &str) -> bool {
    (3..=16).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub fn account_root(root: &Path, id: &str) -> Result<PathBuf> {
    if id == "legacy-microsoft" {
        // Keep the existing encrypted credential in place during migration.
        storage::within(root, "credentials.bin")?;
        return Ok(root.to_owned());
    }
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(ApiError::new("INVALID_PROFILE", "档案标识无效。"));
    }
    storage::within(root, &format!("accounts/{id}"))
}

impl Profiles {
    pub fn load(root: &Path, settings: &Settings) -> Result<Self> {
        let path = storage::within(root, "profiles.json")?;
        if path.exists() {
            let profiles: Self = storage::read_json(&path)?;
            let mut ids = BTreeSet::new();
            if profiles.schema_version != 1
                || profiles.profiles.len() > 256
                || profiles.profiles.iter().any(|p| {
                    !ids.insert(&p.id)
                        || !["microsoft", "offline"].contains(&p.kind.as_str())
                        || (p.kind == "offline" && !valid_name(&p.name))
                        || (p.id != "legacy-offline" && account_root(root, &p.id).is_err())
                })
                || [
                    (&profiles.microsoft, "microsoft"),
                    (&profiles.offline, "offline"),
                ]
                .iter()
                .any(|(id, kind)| {
                    id.as_ref().is_some_and(|id| {
                        !profiles
                            .profiles
                            .iter()
                            .any(|p| &p.id == id && p.kind == *kind)
                    })
                })
            {
                return Err(ApiError::new(
                    "PROFILE_CONFIG",
                    "档案文件损坏或版本不兼容，原文件已保留。",
                ));
            }
            return Ok(profiles);
        }
        let mut value = Self {
            schema_version: 1,
            profiles: vec![],
            microsoft: None,
            offline: None,
        };
        if storage::within(root, "credentials.bin")?.is_file() {
            value.profiles.push(Profile {
                id: "legacy-microsoft".into(),
                kind: "microsoft".into(),
                name: "微软账号".into(),
                uuid: None,
            });
            value.microsoft = Some("legacy-microsoft".into());
        }
        if valid_name(&settings.offline_player_name) {
            value.profiles.push(Profile {
                id: "legacy-offline".into(),
                kind: "offline".into(),
                name: settings.offline_player_name.clone(),
                uuid: None,
            });
            value.offline = Some("legacy-offline".into());
        }
        Ok(value)
    }
    pub fn save(&self, root: &Path) -> Result<()> {
        storage::write_json(&storage::within(root, "profiles.json")?, self)
    }
    pub fn get(&self, id: &str) -> Result<&Profile> {
        self.profiles
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| ApiError::new("PROFILE_NOT_FOUND", "此档案已不存在，请刷新后重试。"))
    }
    pub fn selected(&self, offline: bool) -> Option<&Profile> {
        let id = if offline {
            &self.offline
        } else {
            &self.microsoft
        };
        id.as_ref()
            .and_then(|id| self.profiles.iter().find(|p| &p.id == id))
    }
    pub fn select(&mut self, id: &str) -> Result<Profile> {
        let profile = self.get(id)?.clone();
        if profile.kind == "offline" {
            self.offline = Some(id.into());
        } else {
            self.microsoft = Some(id.into());
        }
        Ok(profile)
    }
    pub fn add_offline(&mut self, name: &str) -> Result<Profile> {
        let name = name.trim();
        if !valid_name(name) {
            return Err(ApiError::new(
                "INVALID_PLAYER_NAME",
                "游戏名需要 3–16 个英文字母、数字或下划线。",
            ));
        }
        if self
            .profiles
            .iter()
            .any(|p| p.kind == "offline" && p.name.eq_ignore_ascii_case(name))
        {
            return Err(ApiError::new("PROFILE_EXISTS", "此离线游戏名已有档案。"));
        }
        self.check_limit()?;
        let profile = Profile {
            id: uuid::Uuid::new_v4().to_string(),
            kind: "offline".into(),
            name: name.into(),
            uuid: None,
        };
        self.profiles.push(profile.clone());
        Ok(profile)
    }
    pub fn check_limit(&self) -> Result<()> {
        if self.profiles.len() >= 256 {
            return Err(ApiError::new("PROFILE_LIMIT", "档案数量已达上限。"));
        }
        Ok(())
    }
    pub fn remove(&mut self, id: &str) -> Result<Profile> {
        let profile = self.get(id)?.clone();
        self.profiles.retain(|p| p.id != id);
        for (selected, kind) in [
            (&mut self.microsoft, "microsoft"),
            (&mut self.offline, "offline"),
        ] {
            if selected.as_deref() == Some(id) {
                *selected = self
                    .profiles
                    .iter()
                    .find(|p| p.kind == kind)
                    .map(|p| p.id.clone());
            }
        }
        Ok(profile)
    }
}
