use super::{
    accounts::{AccountView, Accounts},
    java::{self, Java},
    minecraft, net, process,
    storage::{self, Components, Instance, Overrides, Settings, Store},
    tasks::{Task, Tasks},
};
use crate::contracts::{ApiError, Result};
use serde::Serialize;
use serde_json::Value;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub instances: Vec<Instance>,
    pub instance_icons: std::collections::BTreeMap<String, String>,
    pub selected_instance_id: String,
    pub settings: Settings,
    pub java_runtimes: Vec<Java>,
    pub account: AccountView,
    pub tasks: Vec<Task>,
    pub data_directory: String,
    pub game_directory: String,
    pub notice: Option<String>,
    pub read_only: bool,
    pub memory: storage::MemoryAdvice,
}
pub struct RootLease {
    pub root: PathBuf,
    pub game_root: PathBuf,
    pub settings: Settings,
    usage: Arc<AtomicUsize>,
}
impl Drop for RootLease {
    fn drop(&mut self) {
        self.usage.fetch_sub(1, Ordering::AcqRel);
    }
}
pub struct Launcher {
    pub store: Mutex<Store>,
    pub tasks: Tasks,
    pub java: Mutex<Vec<Java>>,
    instance_icons: Mutex<super::instance_icons::Icons>,
    pub accounts: Accounts,
    pub(crate) identity: Mutex<()>,
    pub plugins: crate::plugins::host::PluginHost,
    pub client: net::Client,
    usage: Arc<AtomicUsize>,
    pub location: PathBuf,
    default_root: PathBuf,
    default_game_root: Option<PathBuf>,
}
pub fn game_root(root: &Path, default_root: &Path, default_game_root: Option<&Path>) -> PathBuf {
    let same = if cfg!(windows) {
        root.to_string_lossy()
            .eq_ignore_ascii_case(&default_root.to_string_lossy())
    } else {
        root == default_root
    };
    if same && let Some(game) = default_game_root {
        game.to_owned()
    } else {
        root.join(".minecraft")
    }
}
impl Launcher {
    pub fn open(requested: Option<PathBuf>) -> Result<Arc<Self>> {
        let executable = std::env::current_exe()?;
        let default = executable
            .parent()
            .ok_or_else(|| ApiError::new("DATA_ROOT", "无法确定程序目录。"))?
            .join("HydrogenTauriData");
        let location = default.join("data-location.json");
        let default_game_root = requested
            .is_none()
            .then(|| executable.parent().unwrap().join(".minecraft"));
        let root = if let Some(root) = requested {
            root
        } else if location.exists() {
            let value: Value = storage::read_json(&location)?;
            if value["schemaVersion"] != 1 || value["directory"].as_str().is_none() {
                return Err(ApiError::new(
                    "DATA_LOCATION",
                    "数据位置配置不兼容，原文件已保留。",
                ));
            }
            PathBuf::from(value["directory"].as_str().unwrap())
        } else {
            default.clone()
        };
        let game_root = game_root(&root, &default, default_game_root.as_deref());
        let store = Store::open_at(root, Some(game_root))?;
        let plugins = store.plugins.clone();
        let client = net::client(&plugins, &store.settings)?;
        Ok(Arc::new(Self {
            store: Mutex::new(store),
            tasks: Tasks::default(),
            java: Mutex::new(Vec::new()),
            instance_icons: Mutex::new(super::instance_icons::Icons::default()),
            accounts: Accounts::new(plugins.clone()),
            identity: Mutex::new(()),
            plugins,
            client,
            usage: Arc::new(AtomicUsize::new(0)),
            location,
            default_root: default,
            default_game_root,
        }))
    }
    pub fn lease(&self) -> RootLease {
        let store = self.store.lock().unwrap();
        self.usage.fetch_add(1, Ordering::AcqRel);
        RootLease {
            root: store.root.clone(),
            game_root: store.game_root.clone(),
            settings: store.settings.clone(),
            usage: self.usage.clone(),
        }
    }
    pub fn view(&self) -> Result<View> {
        // Snapshot only in-memory state under the global lock. Plugin calls can
        // wait for Java, disk/network work, or another provider's worker queue.
        let (lease, state, selected, notice, read_only) = {
            let store = self.store.lock().unwrap();
            self.usage.fetch_add(1, Ordering::AcqRel);
            let lease = RootLease {
                root: store.root.clone(),
                game_root: store.game_root.clone(),
                settings: store.settings.clone(),
                usage: self.usage.clone(),
            };
            (
                lease,
                store.state(),
                store.selected.clone().unwrap_or_default(),
                store.notice.clone(),
                store.read_only,
            )
        };
        let (instances, issues): (Vec<Instance>, Vec<String>) =
            serde_json::from_value(self.plugins.call_service(
                &lease.settings,
                "minecraft.instances",
                "minecraft.instances",
                "records.list",
                serde_json::json!({"state":state}),
            )?)?;
        let tasks = self.tasks.list();
        let notice = notice.or_else(|| {
            (!issues.is_empty()).then(|| {
                format!(
                    "{} 个实例的配置损坏或版本不兼容，已保留原文件。",
                    issues.len()
                )
            })
        });
        let java_runtimes = self.java.lock().unwrap().clone();
        let account = self.accounts.view(
            &lease.root,
            &lease.settings,
            tasks
                .iter()
                .any(|t| t.kind == "login" && t.active() && t.phase != "restoring"),
        );
        let memory = storage::memory_advice(&self.client)?;
        let instance_icons = {
            let mut icons = self.instance_icons.lock().unwrap();
            instances
                .iter()
                .filter_map(|instance| {
                    let directory = instance.directory_name.as_deref()?;
                    Some((
                        instance.info.id.clone(),
                        icons.load(&lease.game_root, directory)?,
                    ))
                })
                .collect()
        };
        Ok(View {
            instances,
            instance_icons,
            selected_instance_id: selected,
            settings: lease.settings.clone(),
            java_runtimes,
            account,
            tasks,
            data_directory: lease.root.to_string_lossy().into(),
            game_directory: lease.game_root.to_string_lossy().into(),
            notice,
            read_only,
            memory,
        })
    }
    pub fn instance_lock(&self, id: &str) -> Result<File> {
        let store = self.store.lock().unwrap();
        store.writable()?;
        let _ = store.instance(id)?;
        let marker = storage::within(&store.root, &format!("instances/{id}/process.json"))?;
        if marker.exists() {
            let value: Value = storage::read_json(&marker)?;
            if let (Some(pid), Some(stamp)) = (value["pid"].as_u64(), value["stamp"].as_u64())
                && process::stamp(&self.plugins, &store.settings, pid as u32)? == Some(stamp)
            {
                return Err(ApiError::new(
                    "INSTANCE_RUNNING",
                    "此实例的游戏仍在运行，不能修改其目录。",
                ));
            }
        }
        let directory = storage::within(&store.root, &format!("instances/{id}"))?;
        std::fs::create_dir_all(directory)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(storage::within(
                &store.root,
                &format!("instances/{id}/instance.lock"),
            )?)?;
        file.try_lock()
            .map_err(|_| ApiError::new("INSTANCE_BUSY", "实例正用于安装、启动或文件操作。"))?;
        Ok(file)
    }
    pub fn save_settings(&self, settings: Settings) -> Result<()> {
        let lease = self.lease();
        if settings.locale != lease.settings.locale
            && self.plugins.available(&lease.settings, "i18n")
        {
            self.plugins.call_service(
                &lease.settings,
                "i18n",
                "i18n",
                "bundle",
                serde_json::json!({"root":lease.root,"locale":settings.locale}),
            )?;
        }
        drop(lease);
        let _identity = self.identity.lock().unwrap();
        let settings: Settings = serde_json::from_value(self.client.service(
            "storage",
            "validate",
            serde_json::json!({"settings":settings}),
        )?)?;
        let mut store = self.store.lock().unwrap();
        if settings.modules != store.settings.modules && self.tasks.busy(None) {
            return Err(ApiError::new(
                "PLUGIN_BUSY",
                "请等待当前任务和游戏结束后更改模块状态。",
            ));
        }
        let identity_changed = store.settings.microsoft_client_id != settings.microsoft_client_id
            || store.settings.enabled("account.offline") != settings.enabled("account.offline")
            || store.settings.account_mode != settings.account_mode
            || store.settings.offline_player_name != settings.offline_player_name;
        if identity_changed && self.tasks.busy(None) {
            return Err(ApiError::new(
                "ACCOUNT_BUSY",
                "请等待登录、安装或游戏结束后切换账号配置。",
            ));
        }
        let directory_changed = store.settings.game_directory != settings.game_directory;
        let isolation_changed =
            store.settings.isolate_game_directory != settings.isolate_game_directory;
        if directory_changed || isolation_changed {
            if self.usage.load(Ordering::Acquire) > 0 || self.tasks.busy(None) {
                return Err(ApiError::new(
                    "DATA_IN_USE",
                    "请等待安装、文件操作或游戏结束后更改游戏目录或隔离设置。",
                ));
            }
            for instance in store.instances()? {
                let marker = storage::within(
                    &store.root,
                    &format!("instances/{}/process.json", instance.info.id),
                )?;
                if marker.is_file() {
                    let marker: Value = storage::read_json(&marker)?;
                    if let Some((pid, stamp)) = marker["pid"].as_u64().zip(marker["stamp"].as_u64())
                        && process::stamp(&self.plugins, &store.settings, pid as u32)?
                            == Some(stamp)
                    {
                        return Err(ApiError::new(
                            "GAME_RUNNING",
                            "已有游戏使用当前目录，请退出后再更改游戏目录或隔离设置。",
                        ));
                    }
                }
            }
        }
        let client_changed = store.settings.microsoft_client_id != settings.microsoft_client_id;
        let previous = store.selected.clone();
        if directory_changed {
            store.selected = None;
        }
        if let Err(error) = store.save(settings) {
            store.selected = previous;
            return Err(error);
        }
        if directory_changed {
            store.selected = store
                .instances()?
                .into_iter()
                .find(|instance| instance.info.is_installed)
                .map(|instance| instance.info.id);
            self.tasks.forget_downloads(None);
        }
        self.client.set_settings(&store.settings);
        if client_changed {
            self.accounts.reset();
        }
        Ok(())
    }
    pub fn change_game_root(&self, directory: PathBuf) -> Result<()> {
        let directory =
            if directory.join(".minecraft").is_dir() && !directory.join("versions").exists() {
                directory.join(".minecraft")
            } else {
                directory
            };
        let directory = storage::checked_game_root(&directory)?;
        let mut settings = self.store.lock().unwrap().settings.clone();
        settings.game_directory = Some(directory.to_string_lossy().into());
        self.save_settings(settings)
    }
    pub fn change_root(&self, root: PathBuf) -> Result<()> {
        let mut store = self.store.lock().unwrap();
        if self.usage.load(Ordering::Acquire) > 0 || self.tasks.busy(None) {
            return Err(ApiError::new(
                "DATA_IN_USE",
                "请等待当前登录、安装、文件操作或游戏结束后更换数据目录。",
            ));
        }
        if store.root == root {
            return Ok(());
        }
        for instance in store.instances()? {
            let marker = storage::within(
                &store.root,
                &format!("instances/{}/process.json", instance.info.id),
            )?;
            if marker.exists() {
                let marker: Value = storage::read_json(&marker)?;
                if let Some((pid, stamp)) = marker["pid"].as_u64().zip(marker["stamp"].as_u64())
                    && process::stamp(&self.plugins, &store.settings, pid as u32)? == Some(stamp)
                {
                    return Err(ApiError::new("GAME_RUNNING", "已有游戏使用当前数据目录。"));
                }
            }
        }
        let game_root = game_root(&root, &self.default_root, self.default_game_root.as_deref());
        let mut next = Store::open_at(root.clone(), Some(game_root))?;
        next.writable()?;
        crate::plugins::manager::initialize(&next.root)?;
        storage::write_json(
            &self.location,
            &serde_json::json!({"schemaVersion":1,"directory":root}),
        )?;
        self.plugins
            .set_plugins(crate::plugins::manager::load(&next.root, &[])?);
        next.rebind_plugins(self.plugins.clone())?;
        *store = next;
        self.client.set_settings(&store.settings);
        self.tasks.forget_downloads(None);
        self.accounts.reset();
        self.java.lock().unwrap().clear();
        Ok(())
    }
    pub fn select(&self, id: &str) -> Result<()> {
        self.store.lock().unwrap().select(id)
    }
    pub fn require(&self, id: &str) -> Result<()> {
        if self.store.lock().unwrap().settings.enabled(id) {
            Ok(())
        } else {
            Err(ApiError::new(
                "PLUGIN_DISABLED",
                format!("{id} 模块已停用，请在插件管理中启用。"),
            ))
        }
    }
    pub fn rename(&self, id: &str, name: &str) -> Result<()> {
        let _lease = self.instance_lock(id)?;
        self.store.lock().unwrap().rename(id, name)
    }
    pub fn overrides(&self, id: &str, overrides: Overrides) -> Result<()> {
        let _lease = self.instance_lock(id)?;
        let store = self.store.lock().unwrap();
        store.settings.effective(&self.client, &overrides)?;
        let mut instance = store.instance(id)?;
        instance.overrides = overrides;
        store.write_instance(&instance)
    }
    pub fn delete(&self, id: &str, confirmation: &str) -> Result<()> {
        let lease = self.instance_lock(id)?;
        let mut store = self.store.lock().unwrap();
        drop(lease);
        store.delete(id, confirmation)?;
        self.tasks.forget_downloads(Some(id));
        Ok(())
    }
    pub fn scan_java(self: &Arc<Self>) -> Result<String> {
        self.require("java.runtime")?;
        let job = self.tasks.start("javaScan", None)?;
        let id = job.state.lock().unwrap().id.clone();
        let this = self.clone();
        let root = self.lease();
        tauri::async_runtime::spawn(async move {
            job.phase("scanning", "正在探测已安装的 Java…");
            let mut saved = Vec::new();
            if let Some(path) = root.settings.java.clone() {
                saved.push(path);
            }
            if let Ok(instances) = this.store.lock().unwrap().instances() {
                for instance in instances {
                    if let Some(path) = instance.overrides.java {
                        saved.push(path);
                    }
                }
            }
            let found = match java::scan(&this.plugins, &root.settings, saved).await {
                Ok(found) => found,
                Err(error) => {
                    job.finish(Err(error));
                    return;
                }
            };
            let count = found.len();
            *this.java.lock().unwrap() = found;
            job.phase("completed", format!("找到 {count} 个 Java 运行时"));
        });
        Ok(id)
    }
    pub async fn versions(&self) -> Result<Vec<minecraft::Version>> {
        let lease = self.lease();
        minecraft::versions(&self.client, &lease.root, &lease.settings).await
    }
    pub async fn fabric_options(&self, version: &str) -> Result<Value> {
        self.require("minecraft.fabric")?;
        let _lease = self.lease();
        minecraft::fabric_options(&self.client, version).await
    }
    pub async fn forge_options(&self, version: &str, kind: &str) -> Result<Value> {
        self.require("minecraft.forge")?;
        let lease = self.lease();
        net::task(
            &self.client,
            "minecraft.forge",
            "options",
            serde_json::json!({"version":version,"kind":kind,"settings":lease.settings}),
            None,
        )
        .await
    }
    pub async fn create(
        self: &Arc<Self>,
        version: String,
        name: String,
        components: Option<Components>,
    ) -> Result<String> {
        if let Some(component) = &components {
            if component.kind.contains(':') {
                self.plugins
                    .provider(&self.lease().settings, "loaderProvider", &component.kind)?;
            } else {
                self.require(if matches!(component.kind.as_str(), "forge" | "neoforge") {
                    "minecraft.forge"
                } else {
                    "minecraft.fabric"
                })?;
            }
        }
        let _lease = self.lease();
        let state = self.store.lock().unwrap().state();
        let instance:Instance=serde_json::from_value(net::task(&self.client,"minecraft.install","create",serde_json::json!({"state":state,"version":version,"name":name,"components":components}),None).await?)?;
        self.install(instance.info.id)
    }
    pub fn install(self: &Arc<Self>, id: String) -> Result<String> {
        self.plugins.hooks(
            &self.lease().settings,
            "beforeInstall",
            serde_json::json!({"instanceId":id}),
        )?;
        let lease = self.instance_lock(&id)?;
        let root = self.lease();
        let instance = self.store.lock().unwrap().instance(&id)?;
        let job = self.tasks.start("install", Some(&id))?;
        {
            let mut state = job.state.lock().unwrap();
            state.instance_name = Some(instance.info.name.clone());
            state.game_version = Some(instance.info.game_version.clone());
        }
        let task_id = job.state.lock().unwrap().id.clone();
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let work = this.install_instance(&instance, &job);
            let result = tokio::select! {_=job.cancel.cancelled()=>Err(ApiError::new("CANCELLED","安装已取消，可重试以复用缓存。")),result=work=>result};
            drop(lease);
            drop(root);
            job.finish(result.map_err(|mut error: ApiError| {
                error.message = match error.code.as_str() {
                    "NETWORK" => "下载连接中断，请检查网络或切换下载源后重试。".into(),
                    code if code.starts_with("DOWNLOAD_HTTP_") => {
                        "下载连接中断，请检查网络或切换下载源后重试。".into()
                    }
                    "INVALID_JSON" | "INVALID_METADATA" | "HASH_MISMATCH" | "DOWNLOAD_SIZE"
                    | "INVALID_RANGE" => "安装组件元数据无效或校验失败，请重试。".into(),
                    "IO_ERROR" => "文件无法保存，请检查目录权限和剩余空间。".into(),
                    _ => error.message,
                };
                error
            }));
        });
        Ok(task_id)
    }
    pub(super) async fn install_instance(
        &self,
        instance: &Instance,
        job: &super::tasks::Job,
    ) -> Result<()> {
        let state = self.store.lock().unwrap().state();
        let result = net::task(
            &self.client,
            "minecraft.install",
            "install",
            serde_json::json!({"state":state,"instance":instance}),
            Some(job),
        )
        .await?;
        let mut store = self.store.lock().unwrap();
        store.selected = serde_json::from_value(result["selected"].clone())?;
        store.invalidate_repository();
        Ok(())
    }
    pub fn login(self: &Arc<Self>, restore: bool) -> Result<String> {
        self.login_inner(restore, false)
    }
    pub fn add_microsoft_profile(self: &Arc<Self>) -> Result<String> {
        self.login_inner(false, true)
    }
    fn login_inner(self: &Arc<Self>, restore: bool, adding: bool) -> Result<String> {
        let _identity = self.identity.lock().unwrap();
        self.store.lock().unwrap().writable()?;
        let mut root = self.lease();
        if self
            .tasks
            .list()
            .iter()
            .any(|t| t.active() && ["login", "launch"].contains(&t.kind.as_str()))
        {
            return Err(ApiError::new(
                "ACCOUNT_BUSY",
                "请等待登录或游戏结束后更改档案。",
            ));
        }
        if adding {
            root.settings.account_mode = "microsoft".into();
        }
        let job = self.tasks.start("login", None)?;
        // Publish the background phase before scheduling, so the first snapshot cannot show browser login.
        job.phase(
            if restore { "restoring" } else { "authorizing" },
            if restore {
                "正在恢复账号…"
            } else {
                "正在登录…"
            },
        );
        let id = job.state.lock().unwrap().id.clone();
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let authenticate = async {
                if adding {
                    this.accounts
                        .add_microsoft(&root.root, &root.settings, &job)
                        .await
                } else {
                    this.accounts
                        .sign_in(&this.client, &root.root, &root.settings, &job, restore)
                        .await
                }
            };
            let mut result = tokio::select! {_=job.cancel.cancelled()=>Err(ApiError::new("CANCELLED","登录已取消。")),result=authenticate=>result};
            if result.is_ok() && adding {
                let mut store = this.store.lock().unwrap();
                let mut settings = store.settings.clone();
                settings.account_mode = "microsoft".into();
                result = store.save(settings);
                if result.is_ok() {
                    this.client.set_settings(&store.settings);
                }
            }
            if let Err(error) = &result {
                if !adding {
                    this.accounts
                        .failure(&root.root, &root.settings, &error.message);
                }
            }
            job.finish(result);
        });
        Ok(id)
    }
    pub fn restore(self: &Arc<Self>) {
        let lease = self.lease();
        let restore = if lease.settings.account_mode.contains(':') {
            self.plugins
                .provider(
                    &lease.settings,
                    "accountProvider",
                    &lease.settings.account_mode,
                )
                .is_ok()
                && self
                    .accounts
                    .view(&lease.root, &lease.settings, false)
                    .status
                    != "signedIn"
        } else {
            !lease.settings.offline_selected()
                && lease.settings.enabled("account.microsoft")
                && !lease.settings.microsoft_client_id.is_empty()
                && has_saved_credentials(&self.accounts, &lease.root, &lease.settings)
                && self
                    .accounts
                    .view(&lease.root, &lease.settings, false)
                    .needs_restore()
        };
        if restore {
            let _ = self.login(true);
        }
    }
    pub fn profile_change(&self, method: &str, fields: Value) -> Result<Value> {
        let _identity = self.identity.lock().unwrap();
        let store = self.store.lock().unwrap();
        store.writable()?;
        if self
            .tasks
            .list()
            .iter()
            .any(|t| t.active() && ["login", "launch"].contains(&t.kind.as_str()))
        {
            return Err(ApiError::new(
                "ACCOUNT_BUSY",
                "请等待登录或游戏结束后更改档案。",
            ));
        }
        self.accounts
            .profiles(&store.root, &store.settings, method, fields)
    }
    pub fn select_profile(self: &Arc<Self>, id: &str) -> Result<()> {
        {
            let _identity = self.identity.lock().unwrap();
            let mut store = self.store.lock().unwrap();
            store.writable()?;
            if self
                .tasks
                .list()
                .iter()
                .any(|t| t.active() && ["login", "launch"].contains(&t.kind.as_str()))
            {
                return Err(ApiError::new(
                    "ACCOUNT_BUSY",
                    "请等待登录或游戏结束后切换档案。",
                ));
            }
            let list = self.accounts.profiles(
                &store.root,
                &store.settings,
                "profiles.list",
                serde_json::json!({}),
            )?;
            let profiles: Vec<super::accounts::Profile> =
                serde_json::from_value(list["profiles"].clone())?;
            let profile = profiles
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(|| ApiError::new("PROFILE_NOT_FOUND", "此档案已不存在。"))?;
            let previous = store.settings.clone();
            let mut settings = previous.clone();
            settings.account_mode = profile.kind.clone();
            if profile.kind == "offline" {
                settings.offline_player_name = profile.name.clone();
            }
            store.save(settings)?;
            if let Err(error) = self.accounts.profiles(
                &store.root,
                &store.settings,
                "profiles.select",
                serde_json::json!({"id":id}),
            ) {
                store.save(previous)?;
                return Err(error);
            }
            self.client.set_settings(&store.settings);
        }
        self.restore();
        Ok(())
    }
    pub fn delete_profile(self: &Arc<Self>, id: &str, confirmation: &str) -> Result<()> {
        self.profile_change(
            "profiles.delete",
            serde_json::json!({"id":id,"confirmation":confirmation}),
        )?;
        let lease = self.lease();
        let view = self.accounts.view(&lease.root, &lease.settings, false);
        if view.selected_profile_id.is_none() {
            if let Some(profile) = view
                .profiles
                .iter()
                .find(|p| p.kind == "microsoft" || view.offline_available)
            {
                self.select_profile(&profile.id)?;
            }
        } else {
            self.restore();
        }
        Ok(())
    }
    pub fn sign_out(&self) -> Result<()> {
        let _identity = self.identity.lock().unwrap();
        if self
            .tasks
            .list()
            .iter()
            .any(|t| t.active() && ["login", "launch"].contains(&t.kind.as_str()))
        {
            return Err(ApiError::new(
                "ACCOUNT_BUSY",
                "请等待当前登录或游戏结束后退出账号。",
            ));
        }
        let lease = self.lease();
        self.accounts.sign_out(&lease.root, &lease.settings)
    }
}

fn has_saved_credentials(accounts: &Accounts, root: &Path, settings: &Settings) -> bool {
    accounts
        .profiles(
            root,
            settings,
            "profiles.hasCredentials",
            serde_json::json!({}),
        )
        .is_ok_and(|v| v == true)
}
