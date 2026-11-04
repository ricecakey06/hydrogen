use crate::contracts::{ApiError, InstanceInfo, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
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
impl Default for Settings {
    fn default() -> Self {
        Self {
            game_directory: None,
            isolate_game_directory: None,
            java: None,
            max_memory_mib: 2048,
            window_mode: "default".into(),
            window_width: 854,
            window_height: 480,
            after_launch: "minimize".into(),
            jvm_arguments: String::new(),
            download_source: "official".into(),
            offline_enabled: true,
            offline_player_name: "Player".into(),
            account_mode: "microsoft".into(),
            microsoft_client_id: std::env::var("HYDROGEN_MICROSOFT_CLIENT_ID")
                .unwrap_or_else(|_| "9a5a637c-bca3-44d2-984c-cb49aac9f5b7".into()),
            modules: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryAdvice {
    pub total_mib: u32,
    pub recommended_mib: u32,
}
pub fn memory_advice(client: &super::net::Client) -> Result<MemoryAdvice> {
    Ok(serde_json::from_value(client.service(
        "storage",
        "memory",
        serde_json::Value::Null,
    )?)?)
}
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
}
pub fn id_valid(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|c| c.is_ascii_hexdigit())
}
pub fn linked(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            m.file_attributes() & 0x400 != 0
        }
        #[cfg(not(windows))]
        {
            m.file_type().is_symlink()
        }
    })
}
pub fn within(root: &Path, relative: &str) -> Result<PathBuf> {
    if !crate::plugins::manifest::safe_relative(relative) {
        return Err(ApiError::new("INVALID_PATH", "路径不能离开数据目录。"));
    }
    let mut path = root.to_path_buf();
    for part in relative.replace('\\', "/").split('/') {
        path.push(part);
        if linked(&path) {
            return Err(ApiError::new(
                "LINKED_PATH",
                "数据路径不能包含符号链接或目录联接。",
            ));
        }
    }
    if linked(root) {
        return Err(ApiError::new("LINKED_PATH", "数据目录不能是链接。"));
    }
    Ok(path)
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
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    if fs::metadata(path)?.len() > 16 * 1024 * 1024 {
        return Err(ApiError::new("CONFIG_LIMIT", "配置文件超过大小限制。"));
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| ApiError::new("INVALID_PATH", "写入位置无效。"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4().simple()));
    let result = (|| -> Result<()> {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        output.write_all(bytes)?;
        output.sync_all()?;
        drop(output);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if temporary.exists() {
        let _ = fs::remove_file(temporary);
    }
    result
}
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    atomic_write(path, &serde_json::to_vec_pretty(value)?)
}

pub fn move_new(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileW(source: *const u16, destination: *const u16) -> i32;
        }
        let source: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<_> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        if unsafe { MoveFileW(source.as_ptr(), destination.as_ptr()) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::hard_link(source, destination)?;
        if let Err(error) = fs::remove_file(source) {
            let _ = fs::remove_file(destination);
            return Err(error.into());
        }
        Ok(())
    }
}

pub fn jvm_arguments(client: &super::net::Client, text: &str) -> Result<Vec<String>> {
    Ok(serde_json::from_value(client.service(
        "storage",
        "jvmArguments",
        serde_json::json!({"text":text}),
    )?)?)
}
impl Settings {
    pub fn offline_selected(&self) -> bool {
        self.enabled("account.offline") && self.account_mode == "offline"
    }

    pub fn validate(&self, client: &super::net::Client) -> Result<()> {
        client.service("storage", "validate", serde_json::json!({"settings":self}))?;
        Ok(())
    }
    pub fn effective(&self, client: &super::net::Client, overrides: &Overrides) -> Result<Self> {
        Ok(serde_json::from_value(client.service(
            "storage",
            "effective",
            serde_json::json!({"settings":self,"overrides":overrides}),
        )?)?)
    }
    pub fn enabled(&self, id: &str) -> bool {
        self.modules.get(id).copied().unwrap_or(true)
    }
}

pub struct Store {
    pub plugins: crate::plugins::host::PluginHost,
    repository_client: super::net::Client,
    pub root: PathBuf,
    pub game_root: PathBuf,
    pub default_game_root: PathBuf,
    pub settings: Settings,
    pub selected: Option<String>,
    pub notice: Option<String>,
    pub read_only: bool,
    _lock: File,
}
impl Store {
    pub fn open(root: PathBuf) -> Result<Self> {
        Self::open_at(root, None)
    }
    pub fn open_at(root: PathBuf, game_root: Option<PathBuf>) -> Result<Self> {
        if !root.is_absolute() || root.ancestors().any(linked) {
            return Err(ApiError::new(
                "INVALID_DATA_ROOT",
                "数据目录必须是完整路径，且不能通过链接访问。",
            ));
        }
        fs::create_dir_all(&root)?;
        let root = dunce::canonicalize(root)?;
        let game_root = game_root.unwrap_or_else(|| root.join(".minecraft"));
        if !game_root.is_absolute() || game_root.ancestors().any(linked) {
            return Err(ApiError::new(
                "INVALID_GAME_ROOT",
                "游戏目录必须是完整路径，且不能通过链接访问。",
            ));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(within(&root, "launcher.lock")?)?;
        lock.try_lock()
            .map_err(|_| ApiError::new("DATA_IN_USE", "此数据目录正由另一个启动器使用。"))?;
        crate::plugins::manager::initialize(&root)?;
        let plugins = crate::plugins::host::PluginHost::new(plugin_runtime(), Vec::new());
        plugins.set_plugins(crate::plugins::manager::load(&root, &[])?);
        let repository_client = super::net::client(&plugins, &Settings::default())?;
        let state: StoreState = serde_json::from_value(repository_client.service(
            "storage",
            "open",
            serde_json::json!({"root":root,"gameRoot":game_root}),
        )?)?;
        let mut store = Self {
            plugins,
            repository_client,
            root: state.root,
            game_root: state.game_root,
            default_game_root: state.default_game_root,
            settings: state.settings,
            selected: state.selected,
            notice: state.notice,
            read_only: state.read_only,
            _lock: lock,
        };
        store.repository_client.set_settings(&store.settings);
        if store
            .selected
            .as_ref()
            .is_none_or(|id| store.instance(id).is_err())
        {
            store.selected = store
                .instances()?
                .into_iter()
                .find(|instance| instance.info.is_installed)
                .map(|instance| instance.info.id);
        }
        Ok(store)
    }
    pub fn state(&self) -> serde_json::Value {
        serde_json::json!({"root":self.root,"game_root":self.game_root,"default_game_root":self.default_game_root,"settings":self.settings,"selected":self.selected,"notice":self.notice,"read_only":self.read_only})
    }
    pub fn writable(&self) -> Result<()> {
        self.repository_client.service(
            "storage",
            "writable",
            serde_json::json!({"state":self.state()}),
        )?;
        Ok(())
    }
    pub fn save(&mut self, settings: Settings) -> Result<()> {
        let state: StoreState = serde_json::from_value(self.repository_client.service(
            "storage",
            "save",
            serde_json::json!({"state":self.state(),"settings":settings}),
        )?)?;
        if self.game_root != state.game_root {
            self.invalidate_repository();
        }
        self.game_root = state.game_root;
        self.settings = state.settings;
        self.repository_client.set_settings(&self.settings);
        Ok(())
    }
    pub fn rebind_plugins(&mut self, plugins: crate::plugins::host::PluginHost) -> Result<()> {
        self.repository_client = super::net::client(&plugins, &self.settings)?;
        self.plugins = plugins;
        Ok(())
    }
    pub fn invalidate_repository(&self) {}
    fn records(&self, method: &str, mut params: serde_json::Value) -> Result<serde_json::Value> {
        params["state"] = self.state();
        self.repository_client
            .service("minecraft.instances", method, params)
    }
    pub fn instances(&self) -> Result<Vec<Instance>> {
        Ok(self.instances_with_issues()?.0)
    }
    pub fn instances_with_issues(&self) -> Result<(Vec<Instance>, Vec<String>)> {
        Ok(serde_json::from_value(
            self.records("records.list", serde_json::json!({}))?,
        )?)
    }
    pub fn instance(&self, id: &str) -> Result<Instance> {
        Ok(serde_json::from_value(
            self.records("records.get", serde_json::json!({"id":id}))?,
        )?)
    }
    pub fn write_instance(&self, instance: &Instance) -> Result<()> {
        self.records("records.write", serde_json::json!({"instance":instance}))?;
        Ok(())
    }
    pub fn create(
        &self,
        version: &str,
        name: &str,
        components: Option<Components>,
    ) -> Result<Instance> {
        Ok(serde_json::from_value(self.records(
            "records.create",
            serde_json::json!({"version":version,"name":name,"components":components}),
        )?)?)
    }
    pub fn game_dir(&self, id: &str) -> Result<PathBuf> {
        Ok(serde_json::from_value(self.records(
            "records.gameDir",
            serde_json::json!({"id":id}),
        )?)?)
    }
    pub fn version_dir(&self, id: &str) -> Result<PathBuf> {
        Ok(serde_json::from_value(self.records(
            "records.versionDir",
            serde_json::json!({"id":id}),
        )?)?)
    }
    pub fn select(&mut self, id: &str) -> Result<()> {
        let selected =
            serde_json::from_value(self.records("records.select", serde_json::json!({"id":id}))?)?;
        let previous = std::mem::replace(&mut self.selected, selected);
        if let Err(error) = self.save(self.settings.clone()) {
            self.selected = previous;
            return Err(error);
        }
        Ok(())
    }
    pub fn delete(&mut self, id: &str, confirmation: &str) -> Result<()> {
        let state: StoreState = serde_json::from_value(self.records(
            "records.delete",
            serde_json::json!({"id":id,"confirmation":confirmation}),
        )?)?;
        self.selected = state.selected;
        self.notice = state.notice;
        Ok(())
    }
    pub fn rename(&self, id: &str, name: &str) -> Result<()> {
        self.records("records.rename", serde_json::json!({"id":id,"name":name}))?;
        Ok(())
    }
}

fn plugin_runtime() -> tokio::runtime::Handle {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    tokio::runtime::Handle::try_current().unwrap_or_else(|_| {
        RUNTIME
            .get_or_init(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("plugin runtime")
            })
            .handle()
            .clone()
    })
}

#[derive(Deserialize)]
struct StoreState {
    root: PathBuf,
    game_root: PathBuf,
    default_game_root: PathBuf,
    settings: Settings,
    selected: Option<String>,
    notice: Option<String>,
    read_only: bool,
}
