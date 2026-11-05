use super::repository;
use hydrogen_plugin_services::{
    contracts::{ApiError, InstanceInfo, Result},
    storage::*,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Overrides {
    pub java: Option<String>,
    pub max_memory_mib: Option<u32>,
    pub jvm_arguments: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Components {
    #[serde(default)]
    pub kind: String,
    pub loader: String,
    pub fabric_api: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
    pub schema_version: u32,
    #[serde(flatten)]
    pub info: InstanceInfo,
    pub overrides: Overrides,
    pub components: Option<Components>,
    #[serde(default)]
    pub directory_name: Option<String>,
    #[serde(default)]
    pub repository_root: Option<String>,
    #[serde(default)]
    pub loader_name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub isolated: bool,
}
pub fn validate_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > 64
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || name.ends_with('.')
    {
        return Err(ApiError::new(
            "INVALID_NAME",
            "实例名称应为 1–64 个字符，不能包含路径或控制字符。",
        ));
    }
    Ok(name.into())
}
pub fn id_valid(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|c| c.is_ascii_hexdigit())
}
#[derive(Serialize, Deserialize)]
pub struct Store {
    pub root: PathBuf,
    pub game_root: PathBuf,
    pub default_game_root: PathBuf,
    pub settings: Value,
    pub selected: Option<String>,
    pub notice: Option<String>,
    pub read_only: bool,
}
impl Store {
    pub fn writable(&self) -> Result<()> {
        hydrogen_plugin_services::invoke(
            "storage",
            "storage",
            "writable",
            serde_json::json!({"state":self}),
        )?;
        Ok(())
    }
    fn repository(&self) -> Result<(Arc<Vec<repository::Version>>, Vec<String>)> {
        let (versions, issues) = repository::scan(&self.game_root)?;
        Ok((Arc::new(versions), issues))
    }
    fn discovered_instance(&self, version: &repository::Version) -> Instance {
        Instance {
            schema_version: 1,
            info: InstanceInfo {
                id: repository::instance_id(&self.game_root, &version.name),
                name: version.name.clone(),
                game_version: version.game_version.clone(),
                is_installed: true,
            },
            overrides: Overrides::default(),
            components: if version.loader == "Fabric" {
                version.loader_version.as_ref().map(|loader| Components {
                    kind: String::new(),
                    loader: loader.clone(),
                    fabric_api: None,
                })
            } else {
                None
            },
            directory_name: Some(version.name.clone()),
            repository_root: Some(self.game_root.to_string_lossy().into()),
            loader_name: version.loader.clone(),
            isolated: false,
        }
    }
    pub fn instances(&self) -> Result<Vec<Instance>> {
        Ok(self.instances_with_issues()?.0)
    }
    pub fn instances_with_issues(&self) -> Result<(Vec<Instance>, Vec<String>)> {
        let root = within(&self.root, "instances")?;
        let mut result = Vec::new();
        let (versions, mut issues) = self.repository()?;
        for entry in if root.exists() {
            Some(fs::read_dir(root)?)
        } else {
            None
        }
        .into_iter()
        .flatten()
        {
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().to_string();
            if id_valid(&id) && entry.path().is_dir() {
                let record = within(&self.root, &format!("instances/{id}/instance.json"))?;
                if read_json::<Instance>(&record).is_ok_and(|instance| {
                    instance.repository_root.as_deref()
                        != Some(self.game_root.to_string_lossy().as_ref())
                }) {
                    continue;
                }
                match self.instance(&id) {
                    Ok(instance) => result.push(instance),
                    Err(_) => issues.push(id),
                }
            }
        }
        for version in versions.iter() {
            if !result
                .iter()
                .any(|instance| instance.directory_name.as_deref() == Some(&version.name))
            {
                result.push(self.discovered_instance(version));
            }
        }
        result.sort_by_key(|i| i.info.name.to_lowercase());
        Ok((result, issues))
    }
    pub fn instance(&self, id: &str) -> Result<Instance> {
        if !id_valid(id) {
            return Err(ApiError::new("INVALID_ID", "实例标识无效。"));
        }
        let path = within(&self.root, &format!("instances/{id}/instance.json"))?;
        if !path.exists()
            || read_json::<Instance>(&path).is_ok_and(|instance| {
                instance.repository_root.as_deref()
                    != Some(self.game_root.to_string_lossy().as_ref())
            })
        {
            return self
                .repository()?
                .0
                .iter()
                .find(|version| repository::instance_id(&self.game_root, &version.name) == id)
                .map(|version| self.discovered_instance(version))
                .ok_or_else(|| ApiError::new("UNKNOWN_INSTANCE", "实例不存在。"));
        }
        let instance: Instance = read_json(&path)?;
        if instance.schema_version != 1
            || instance.info.id != id
            || (validate_name(&instance.info.name).is_err()
                && instance.directory_name.as_deref() != Some(&instance.info.name))
            || instance
                .directory_name
                .as_deref()
                .is_none_or(|name| !repository::valid_name(name))
        {
            return Err(ApiError::new(
                "INSTANCE_CONFIG",
                "实例配置损坏或版本不兼容，原文件已保留。",
            ));
        }
        hydrogen_plugin_services::invoke(
            "storage",
            "storage",
            "effective",
            serde_json::json!({"settings":self.settings,"overrides":instance.overrides}),
        )?;
        Ok(instance)
    }
    pub fn write_instance(&self, instance: &Instance) -> Result<()> {
        self.writable()?;
        if !id_valid(&instance.info.id) {
            return Err(ApiError::new("INVALID_ID", "实例标识无效。"));
        }
        write_json(
            &within(
                &self.root,
                &format!("instances/{}/instance.json", instance.info.id),
            )?,
            instance,
        )
    }
    pub fn create(
        &self,
        version: &str,
        name: &str,
        components: Option<Components>,
        pack_stage: Option<&str>,
    ) -> Result<Instance> {
        self.writable()?;
        if version.chars().count() > 100 || !repository::valid_name(version) {
            return Err(ApiError::new("INVALID_VERSION", "游戏版本无效。"));
        }
        if let Some(component) = &components
            && (!["", "fabric", "forge", "neoforge", "quilt"].contains(&component.kind.as_str())
                || component.loader.is_empty()
                || component.loader.len() > 100
                || !component
                    .loader
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
                || (!component.kind.is_empty()
                    && component.kind != "fabric"
                    && component.fabric_api.is_some()))
        {
            return Err(ApiError::new(
                "INVALID_COMPONENTS",
                "组件类型或版本无效，Fabric API 只能与 Fabric 一起安装。",
            ));
        }
        let name = validate_name(if name.trim().is_empty() {
            version
        } else {
            name
        })?;
        if let Some(existing) = self
            .instances()?
            .into_iter()
            .find(|i| i.info.name.eq_ignore_ascii_case(&name))
        {
            if pack_stage.is_none()
                && !existing.info.is_installed
                && existing.info.game_version == version
                && existing.components == components
            {
                return Ok(existing);
            }
            return Err(ApiError::new("DUPLICATE_NAME", "该实例名称已存在。"));
        }
        if within(&self.game_root, &format!("versions/{name}"))?.exists() {
            return Err(ApiError::new(
                "DUPLICATE_NAME",
                "同名版本目录已存在，请在版本列表中使用它，或选择其他名称。",
            ));
        }
        let instance = Instance {
            schema_version: 1,
            info: InstanceInfo {
                id: uuid::Uuid::new_v4().simple().to_string(),
                name: name.clone(),
                game_version: version.into(),
                is_installed: false,
            },
            overrides: Overrides::default(),
            components: components.clone(),
            directory_name: Some(name.clone()),
            repository_root: Some(self.game_root.to_string_lossy().into()),
            isolated: pack_stage.is_some(),
            loader_name: components
                .as_ref()
                .map(|c| {
                    match c.kind.as_str() {
                        "forge" => "Forge",
                        "neoforge" => "NeoForge",
                        "quilt" => "Quilt",
                        _ => "Fabric",
                    }
                    .into()
                })
                .unwrap_or_default(),
        };
        if let Some(stage) = pack_stage {
            if !id_valid(stage) {
                return Err(ApiError::new("INVALID_ID", "整合包暂存目录无效。"));
            }
            let stage = within(&self.game_root, &format!(".hydrogen-pack-{stage}"))?;
            let versions = within(&self.game_root, "versions")?;
            fs::create_dir_all(&versions)?;
            let destination = within(&versions, &name)?;
            if ["json", "jar"]
                .iter()
                .any(|suffix| stage.join(format!("{name}.{suffix}")).exists())
            {
                return Err(ApiError::new(
                    "INVALID_MODPACK",
                    "整合包覆盖文件与实例的游戏文件冲突。",
                ));
            }
            // Register the instance before publishing payloads so repository discovery
            // cannot mistake a pack's top-level config JSON for a game manifest.
            let record = within(
                &self.root,
                &format!("instances/{}/instance.json", instance.info.id),
            )?;
            self.write_instance(&instance)?;
            if let Err(error) = fs::rename(stage, destination) {
                fs::remove_file(&record)?;
                let _ = fs::remove_dir(record.parent().unwrap());
                return Err(error.into());
            }
        } else {
            self.write_instance(&instance)?;
            fs::create_dir_all(self.version_dir(&instance.info.id)?)?;
        }
        Ok(instance)
    }
    pub fn game_dir(&self, id: &str) -> Result<PathBuf> {
        if !id_valid(id) {
            return Err(ApiError::new("INVALID_ID", "实例标识无效。"));
        }
        let instance = self.instance(id)?;
        repository::run_directory(
            &self.game_root,
            instance.directory_name.as_deref().unwrap_or(id),
            instance
                .isolated
                .then_some(true)
                .or(self.settings["isolateGameDirectory"].as_bool())
                .or_else(|| instance.components.as_ref().map(|_| true)),
        )
    }
    pub fn version_dir(&self, id: &str) -> Result<PathBuf> {
        let instance = self.instance(id)?;
        let name = instance
            .directory_name
            .as_deref()
            .ok_or_else(|| ApiError::new("INSTANCE_CONFIG", "实例缺少版本目录。"))?;
        if !repository::valid_name(name) {
            return Err(ApiError::new("INVALID_PATH", "版本目录无效。"));
        }
        within(&self.game_root, &format!("versions/{name}"))
    }
    pub fn select(&mut self, id: &str) -> Result<()> {
        if !self.instance(id)?.info.is_installed {
            return Err(ApiError::new("NOT_INSTALLED", "此实例尚未安装完成。"));
        }
        self.selected = Some(id.into());
        Ok(())
    }
    pub fn rename(&self, id: &str, name: &str) -> Result<()> {
        let name = validate_name(name)?;
        if self
            .instances()?
            .iter()
            .any(|i| i.info.id != id && i.info.name.eq_ignore_ascii_case(&name))
        {
            return Err(ApiError::new("DUPLICATE_NAME", "该实例名称已存在。"));
        }
        let mut instance = self.instance(id)?;
        instance.info.name = name;
        self.write_instance(&instance)
    }
}

impl Store {
    pub fn save(&mut self, settings: Value) -> Result<()> {
        let updated: Self = serde_json::from_value(hydrogen_plugin_services::invoke(
            "storage",
            "storage",
            "save",
            serde_json::json!({"state":self,"settings":settings}),
        )?)?;
        *self = updated;
        Ok(())
    }
    pub fn delete(&mut self, id: &str, confirmation: &str) -> Result<()> {
        self.writable()?;
        let instance = self.instance(id)?;
        if confirmation != instance.info.name {
            return Err(ApiError::new(
                "CONFIRMATION_REQUIRED",
                "请确认要删除的实例名称。",
            ));
        }
        let source = within(&self.root, &format!("instances/{id}"))?;
        let game = self.version_dir(id)?;
        fn check(path: &Path) -> Result<()> {
            if linked(path) {
                return Err(ApiError::new(
                    "LINKED_PATH",
                    "实例目录包含链接，拒绝递归删除。",
                ));
            }
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                if linked(&entry.path()) {
                    return Err(ApiError::new("LINKED_PATH", "实例包含链接，拒绝删除。"));
                }
                if entry.file_type()?.is_dir() {
                    check(&entry.path())?;
                }
            }
            Ok(())
        }
        check(&source)?;
        if game.exists() {
            check(&game)?;
        }
        let trash = within(
            &self.root,
            &format!("trash/{}", uuid::Uuid::new_v4().simple()),
        )?;
        std::fs::create_dir_all(&trash)?;

        // Stage on the game volume: the repository may be on another drive.
        let private = game.with_file_name(format!(
            ".hydrogen-delete-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let metadata = trash.join("instance");
        if game.exists() {
            std::fs::rename(&game, &private)?;
        }
        if let Err(error) = std::fs::rename(&source, &metadata) {
            if private.exists() {
                let _ = std::fs::rename(&private, &game);
            }
            return Err(error.into());
        }
        let previous = self.selected.clone();
        if self.selected.as_deref() == Some(id) {
            self.selected = None;
        }
        let settings = self.settings.clone();
        if let Err(error) = self.save(settings) {
            let _ = std::fs::rename(&metadata, &source);
            if private.exists() {
                let _ = std::fs::rename(&private, &game);
            }
            self.selected = previous;
            return Err(error);
        }
        if (private.exists() && std::fs::remove_dir_all(&private).is_err())
            || std::fs::remove_dir_all(&trash).is_err()
        {
            self.notice = Some(
                "实例已移除，部分文件仍保留在版本目录的 .hydrogen-delete-* 或数据目录的 trash 中。"
                    .into(),
            );
        }
        Ok(())
    }
}
