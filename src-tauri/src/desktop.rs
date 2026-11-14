use crate::{
    contracts::{ApiError, Result},
    plugins::{
        host::PluginHost,
        manifest::{self, Plugin},
        panel::{PANEL_CSP, Panels},
        runtime::RunReport,
    },
};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};
use tauri::{Emitter, Manager, State, WebviewWindow};

mod about;

struct PendingPlugin {
    created: Instant,
    candidate: crate::plugins::manager::Candidate,
    previous: Option<String>,
}
struct AppState {
    host: PluginHost,
    safe_theme: bool,
    theme_announced: AtomicU64,
    plugins: Mutex<Vec<Plugin>>,
    development: Vec<Plugin>,
    plugin_mutation: tokio::sync::RwLock<()>,
    plugin_candidates: Mutex<std::collections::BTreeMap<String, PendingPlugin>>,
    core: Arc<crate::launcher::core::Launcher>,
    panels: Panels,
    generation: AtomicU64,
    started: Instant,
    ready_file: Option<PathBuf>,
    panel_navigation_guard: AtomicBool,
    appearance: Mutex<crate::window::Appearance>,
    dropped_files: Mutex<std::collections::BTreeMap<String, (Instant, Vec<PathBuf>)>>,
    event_sequences: Mutex<std::collections::BTreeMap<String, u64>>,
    legacy_imports:
        Mutex<std::collections::BTreeMap<String, (Instant, crate::launcher::migration::Plan)>>,
}

fn main_document(window: &WebviewWindow) -> Result<()> {
    let url = window
        .url()
        .map_err(|_| ApiError::new("UNTRUSTED_CALLER", "无法验证主窗口。"))?;
    let trusted = match url.scheme() {
        "tauri" => url.host_str() == Some("localhost"),
        "http" => {
            url.host_str() == Some("tauri.localhost")
                || (cfg!(debug_assertions)
                    && url.host_str() == Some("127.0.0.1")
                    && url.port() == Some(1420))
        }
        _ => false,
    };
    if window.label() != "main" || !trusted {
        return Err(ApiError::new(
            "UNTRUSTED_CALLER",
            "此文档没有原生调用权限。",
        ));
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginView {
    id: String,
    name: String,
    version: String,
    digest: String,
    commands: Vec<manifest::Command>,
    panels: Vec<manifest::Panel>,
    permissions: Vec<String>,
    description: String,
    origin: &'static str,
    runtime: String,
    required: bool,
    enabled: bool,
    problem: String,
    dependencies: Vec<serde_json::Value>,
    views: Vec<manifest::UiContribution>,
    menus: Vec<manifest::UiContribution>,
    providers: Vec<manifest::Provider>,
    diagnostics: Vec<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    stage: &'static str,
    version: &'static str,
    #[serde(flatten)]
    launcher: crate::launcher::core::View,
    plugins: Vec<PluginView>,
    webview_version: String,
    appearance: crate::window::Appearance,
}

#[tauri::command]
async fn app_snapshot(window: WebviewWindow, state: State<'_, AppState>) -> Result<Snapshot> {
    main_document(&window)?;
    let core = state.core.clone();
    let launcher = tauri::async_runtime::spawn_blocking(move || core.view())
        .await
        .map_err(|_| ApiError::new("SNAPSHOT_ERROR", "无法读取启动器状态。"))??;
    state
        .host
        .set_instances(launcher.instances.iter().map(|i| i.info.clone()).collect());
    {
        let mut seen = state.event_sequences.lock().unwrap();
        for task in &launcher.tasks {
            if seen.insert(task.id.clone(), task.sequence) != Some(task.sequence) {
                let event = serde_json::json!({"id":task.id,"instanceId":task.instance_id,"phase":task.phase,"completed":task.completed,"total":task.total,"sequence":task.sequence});
                state.host.emit("task.updated", event.clone());
                if task.kind == "launch" && ["failed", "exited"].contains(&task.phase.as_str()) {
                    state.host.emit("game.exited", event);
                }
            }
        }
        seen.retain(|id, _| launcher.tasks.iter().any(|t| &t.id == id));
    }
    let installed = state.plugins.lock().unwrap();
    let statuses = crate::plugins::manager::statuses(&installed, &launcher.settings);
    let plugins = installed
        .iter()
        .map(|p| PluginView {
            id: p.manifest.id.clone(),
            name: p.manifest.name.clone(),
            version: p.manifest.version.clone(),
            digest: p.digest.clone(),
            commands: p.manifest.contributes.commands.clone(),
            panels: p.manifest.contributes.panels.clone(),
            permissions: p.manifest.permissions.clone(),
            description: if p.manifest.description.is_empty() {
                "JavaScript 插件，按依赖顺序在独立运行时激活。".into()
            } else {
                p.manifest.description.clone()
            },
            origin: if state.development.iter().any(|d| d.root == p.root) {
                "development"
            } else if crate::plugins::builtin::preinstalled(
                &state.core.lease().root,
                &p.manifest.id,
                &p.digest,
            ) {
                "builtIn"
            } else {
                "installed"
            },
            runtime: p.manifest.runtime.clone(),
            required: false,
            enabled: crate::plugins::manager::enabled(&launcher.settings, &p.manifest.id),
            problem: statuses.get(&p.manifest.id).cloned().unwrap_or_default(),
            dependencies: p.manifest.dependencies.clone(),
            views: p.manifest.contributes.views.clone(),
            menus: p.manifest.contributes.menus.clone(),
            providers: p.manifest.contributes.providers.clone(),
            diagnostics: state.host.diagnostics(&p.manifest.id),
        })
        .collect();
    Ok(Snapshot {
        stage: "功能移植测试版",
        version: env!("CARGO_PKG_VERSION"),
        launcher,
        plugins,
        webview_version: tauri::webview_version().unwrap_or_default(),
        appearance: state.appearance.lock().unwrap().clone(),
    })
}
#[tauri::command]
fn instance_select(window: WebviewWindow, state: State<'_, AppState>, id: String) -> Result<()> {
    main_document(&window)?;
    state.core.select(&id)
}
#[derive(serde::Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum LauncherAction {
    CheckLauncherUpdate,
    AboutLicenses,
    OpenAboutLink {
        link: crate::launcher::environment::AboutLink,
    },
    PluginPreview,
    PluginInstall {
        token: String,
    },
    PluginToggle {
        id: String,
        enabled: bool,
    },
    PluginUninstall {
        id: String,
        digest: String,
        confirmed: bool,
    },
    PluginDirectory,
    PluginRefresh,
    ImportLegacyPreview,
    ImportLegacyCommit {
        token: String,
    },
    ModpackImport,
    ModpackDirectories {
        id: String,
    },
    ModpackExport {
        id: String,
        options: serde_json::Value,
    },
    OpenMods {
        id: String,
    },
    ModsList {
        id: String,
    },
    ModUpdates {
        id: String,
        hashes: Vec<String>,
    },
    ModsUpdate {
        id: String,
        files: Vec<crate::launcher::mods::ModRef>,
    },
    ModsImport {
        id: String,
    },
    ModsDropped {
        id: String,
        token: String,
    },
    ModsEnable {
        id: String,
        files: Vec<crate::launcher::mods::ModRef>,
        enabled: bool,
    },
    ModsDelete {
        id: String,
        files: Vec<crate::launcher::mods::ModRef>,
        confirmed: bool,
    },
    ModsName {
        id: String,
        file: crate::launcher::mods::ModRef,
        name: String,
    },
    ResourceFilters {
        source: Option<String>,
    },
    ResourceSearch {
        query: String,
        game_version: String,
        category: String,
        #[serde(default)]
        loader: String,
        offset: u64,
        source: Option<String>,
    },
    ResourceVersions {
        id: String,
        project: String,
        source: Option<String>,
    },
    ResourceDependencies {
        project: String,
        version_id: Option<String>,
        source: Option<String>,
    },
    ResourceDownload {
        id: String,
        project: String,
        version_id: String,
        source: Option<String>,
    },
    ThemeCatalog,
    ThemePrepare {
        choice: crate::plugins::themes::Choice,
        system_dark: bool,
        high_contrast: bool,
    },
    ThemeApply {
        token: String,
        preview: bool,
    },
    ThemeConfirm {
        token: String,
    },
    ThemeCancel,
    ThemeRelease {
        token: String,
    },
    ThemeState,
    ThemeRestore,
    SaveSettings {
        settings: crate::launcher::storage::Settings,
    },
    ChangeDataDirectory,
    ChangeGameDirectory,
    I18nBundle {
        locale: String,
    },
    Versions,
    ForgeOptions {
        version: String,
        kind: String,
    },
    LoaderOptions {
        provider: String,
        version: String,
    },
    FabricOptions {
        version: String,
    },
    CreateInstance {
        version: String,
        name: String,
        components: Option<crate::launcher::storage::Components>,
    },
    RenameInstance {
        id: String,
        name: String,
    },
    MoveInstance {
        id: String,
        target: String,
        after: bool,
    },
    SaveOverrides {
        id: String,
        overrides: crate::launcher::storage::Overrides,
    },
    DeleteInstance {
        id: String,
        confirmation: String,
    },
    JavaScan,
    JavaChoose,
    SignIn,
    SignInMicrosoft,
    SignOut,
    ProfileAddMicrosoft,
    ProfileAddProvider {
        provider: String,
        name: String,
    },
    ProfileAddOffline {
        name: String,
    },
    ProfileSelect {
        id: String,
    },
    ProfileMove {
        id: String,
        target: String,
        after: bool,
    },
    ProfileDelete {
        id: String,
        confirmation: String,
    },
    Launch {
        id: String,
    },
    Repair {
        id: String,
    },
    RetryInstallation {
        id: String,
    },
    CancelTask {
        id: String,
    },
    OpenDirectory {
        id: Option<String>,
    },
    OpenLogs {
        id: String,
    },
    ExportCrashReport {
        id: String,
    },
}
async fn choose(
    window: &WebviewWindow,
    core: &crate::launcher::core::Launcher,
    folder: bool,
    multiple: bool,
) -> Result<Vec<PathBuf>> {
    let owner = window
        .hwnd()
        .map_err(|_| ApiError::new("WINDOW_HANDLE", "无法打开文件选择器。"))?
        .0 as isize;
    let plugins = core.plugins.clone();
    let settings = core.lease().settings.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::launcher::environment::choose(&plugins, &settings, owner, folder, multiple)
    })
    .await
    .map_err(|_| ApiError::new("DIALOG_ERROR", "文件选择器执行失败。"))?
}
#[tauri::command]
async fn launcher_action(
    window: WebviewWindow,
    state: State<'_, AppState>,
    action: LauncherAction,
) -> Result<serde_json::Value> {
    main_document(&window)?;
    // Mutation holds the write gate only through validation and commit; JS calls carry usage leases.
    let _guard = if matches!(
        &action,
        LauncherAction::ResourceFilters { .. }
            | LauncherAction::ResourceSearch { .. }
            | LauncherAction::ResourceVersions { .. }
            | LauncherAction::ResourceDependencies { .. }
            | LauncherAction::ModUpdates { .. }
            | LauncherAction::CheckLauncherUpdate
            | LauncherAction::AboutLicenses
    ) {
        None
    } else {
        Some(state.plugin_mutation.write().await)
    };
    let affected =
        match &action {
            LauncherAction::PluginToggle { id, .. }
            | LauncherAction::PluginUninstall { id, .. } => Some(
                crate::plugins::manager::affected(&state.plugins.lock().unwrap(), id),
            ),
            LauncherAction::PluginInstall { token } => state
                .plugin_candidates
                .lock()
                .unwrap()
                .get(token)
                .map(|pending| {
                    crate::plugins::manager::affected(
                        &state.plugins.lock().unwrap(),
                        &pending.candidate.plugin.manifest.id,
                    )
                }),
            _ => None,
        };
    let busy = affected
        .as_ref()
        .map_or_else(|| state.host.busy(), |ids| state.host.busy_plugins(ids));
    if matches!(
        &action,
        LauncherAction::PluginInstall { .. }
            | LauncherAction::PluginToggle { .. }
            | LauncherAction::PluginUninstall { .. }
            | LauncherAction::PluginRefresh
            | LauncherAction::SaveSettings { .. }
            | LauncherAction::ChangeDataDirectory
            | LauncherAction::ChangeGameDirectory
    ) && busy
    {
        return Err(ApiError::new(
            "PLUGIN_BUSY",
            "插件仍有在途操作，请稍后重试。",
        ));
    }
    let core = state.core.clone();
    let value = match action {
        LauncherAction::CheckLauncherUpdate => {
            serde_json::to_value(about::check_update(&core.client).await?)?
        }
        LauncherAction::AboutLicenses => {
            serde_json::Value::from(include_str!("../../licenses/third-party.json"))
        }
        LauncherAction::OpenAboutLink { link } => {
            crate::launcher::environment::open_about_link(
                &core.plugins,
                &core.lease().settings,
                link,
            )?;
            serde_json::Value::Null
        }
        LauncherAction::PluginPreview => {
            if let Some(path) = choose(&window, &core, false, false)
                .await?
                .into_iter()
                .next()
            {
                let lease = core.lease();
                core.store.lock().unwrap().writable()?;
                let root = lease.root.clone();
                let candidate = tauri::async_runtime::spawn_blocking(move || {
                    let _lease = lease;
                    crate::plugins::manager::prepare(&root, &path)
                })
                .await
                .map_err(|_| ApiError::new("PLUGIN_PREVIEW", "无法读取插件包。"))??;
                if state
                    .development
                    .iter()
                    .any(|p| p.manifest.id == candidate.plugin.manifest.id)
                {
                    return Err(ApiError::new(
                        "DEVELOPMENT_CONFLICT",
                        "请先移除相同 ID 的开发目录启动参数。",
                    ));
                }
                let previous = state
                    .plugins
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|p| p.manifest.id == candidate.plugin.manifest.id)
                    .cloned();
                let token = uuid::Uuid::new_v4().simple().to_string();
                let previous_digest = crate::plugins::manager::current_digest(
                    &candidate.data_root,
                    &candidate.plugin.manifest.id,
                )?;
                let value = serde_json::json!({"token":token,"id":candidate.plugin.manifest.id,"name":candidate.plugin.manifest.name,"version":candidate.plugin.manifest.version,"permissions":candidate.plugin.manifest.permissions,"dependencies":candidate.plugin.manifest.dependencies,"previousVersion":previous.as_ref().map(|p|&p.manifest.version)});
                let mut candidates = state.plugin_candidates.lock().unwrap();
                candidates.retain(|_, grant| grant.created.elapsed().as_secs() < 900);
                if candidates.len() >= 4 {
                    candidates.clear();
                }
                candidates.insert(
                    token,
                    PendingPlugin {
                        created: Instant::now(),
                        candidate,
                        previous: previous_digest,
                    },
                );
                value
            } else {
                serde_json::Value::Null
            }
        }
        LauncherAction::PluginInstall { token } => {
            if core.tasks.busy(None) && !state.plugin_candidates.lock().unwrap().get(&token).is_some_and(|pending|crate::plugins::themes::standalone(&pending.candidate.plugin)) {
                return Err(ApiError::new(
                    "PLUGIN_BUSY",
                    "请等待任务和游戏结束后更新插件。",
                ));
            }
            let PendingPlugin {
                created: time,
                candidate,
                previous,
            } = state
                .plugin_candidates
                .lock()
                .unwrap()
                .remove(&token)
                .ok_or_else(|| {
                    ApiError::new("PLUGIN_PREVIEW_EXPIRED", "插件预览已过期，请重新选择。")
                })?;
            if time.elapsed().as_secs() > 900 {
                return Err(ApiError::new("PLUGIN_PREVIEW_EXPIRED", "插件预览已过期。"));
            }
            let lease = core.lease();
            core.store.lock().unwrap().writable()?;
            let id = candidate.plugin.manifest.id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _lease = lease;
                crate::plugins::manager::install(&_lease.root, &candidate, previous.as_deref())
            })
            .await
            .map_err(|_| ApiError::new("PLUGIN_INSTALL", "插件安装任务异常结束。"))??;
            invalidate_plugin(&state, &id).await?;
            refresh_plugins(&state)?;
            serde_json::Value::Null
        }
        LauncherAction::PluginToggle { id, enabled } => {
            let known = state
                .plugins
                .lock()
                .unwrap()
                .iter()
                .any(|p| p.manifest.id == id);
            if !known {
                return Err(ApiError::new("UNKNOWN_PLUGIN", "插件不存在。"));
            }
            if core.tasks.busy(None) && !state.plugins.lock().unwrap().iter().any(|p|p.manifest.id == id && crate::plugins::themes::standalone(p)) {
                return Err(ApiError::new(
                    "PLUGIN_BUSY",
                    "请等待当前任务和游戏结束后更改插件状态。",
                ));
            }
            let mut settings = core.store.lock().unwrap().settings.clone();
            settings.modules.insert(id.clone(), enabled);
            if enabled {
                crate::plugins::manager::order(&state.plugins.lock().unwrap(), &settings, &id)?;
            }
            let previous_theme = state.host.themes.cancel();
            if !enabled { state.host.themes.invalidate(std::slice::from_ref(&id), "THEME_UNAVAILABLE: 主题插件已停用")?; }
            if let Err(error) = core.save_settings(settings) { state.host.themes.restore_failed_invalidation(&previous_theme); return Err(error); }
            invalidate_plugin(&state, &id).await?;
            if id == "account.offline" && !enabled {
                core.restore();
            }
            serde_json::Value::Null
        }
        LauncherAction::PluginUninstall {
            id,
            digest,
            confirmed,
        } => {
            if core.tasks.busy(None) && !state.plugins.lock().unwrap().iter().any(|p|p.manifest.id == id && crate::plugins::themes::standalone(p)) {
                return Err(ApiError::new(
                    "PLUGIN_BUSY",
                    "请等待任务和游戏结束后卸载插件。",
                ));
            }
            if !confirmed {
                return Err(ApiError::new("CONFIRMATION_REQUIRED", "请确认卸载插件。"));
            }
            let lease = core.lease();
            core.store.lock().unwrap().writable()?;
            let plugin = state
                .plugins
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.manifest.id == id && p.digest == digest)
                .cloned()
                .ok_or_else(|| ApiError::new("PLUGIN_CHANGED", "插件已变化，请刷新后重试。"))?;
            let previous_theme = state.host.themes.cancel();
            state.host.themes.invalidate(std::slice::from_ref(&id), "THEME_UNAVAILABLE: 主题插件已卸载")?;
            if let Err(error) = crate::plugins::manager::uninstall(&lease.root, &plugin) { state.host.themes.restore_failed_invalidation(&previous_theme); return Err(error); }
            invalidate_plugin(&state, &id).await?;
            refresh_plugins(&state)?;
            serde_json::Value::Null
        }
        LauncherAction::PluginDirectory => {
            let lease = core.lease();
            let directory = crate::plugins::manager::directory(&lease.root)?;
            std::fs::create_dir_all(&directory)?;
            crate::launcher::environment::open_folder(
                &core.plugins,
                &core.lease().settings,
                &directory,
            )?;
            serde_json::Value::Null
        }
        LauncherAction::PluginRefresh => {
            if core.tasks.busy(None) {
                return Err(ApiError::new(
                    "PLUGIN_BUSY",
                    "请等待游戏和任务结束后刷新插件。",
                ));
            }
            let previous = state.plugins.lock().unwrap().clone();
            refresh_plugins(&state)?;
            let current = state.plugins.lock().unwrap().clone();
            let mut affected = Vec::new();
            for old in &previous {
                if !current
                    .iter()
                    .any(|p| p.manifest.id == old.manifest.id && p.digest == old.digest)
                {
                    affected.extend(crate::plugins::manager::affected(
                        &previous,
                        &old.manifest.id,
                    ));
                }
            }
            if !affected.is_empty() {
                state.panels.close_plugins(&affected);
                state.host.stop_plugins(affected).await?;
                state.generation.fetch_add(1, Ordering::SeqCst);
            }
            serde_json::Value::Null
        }
        LauncherAction::ImportLegacyPreview => {
            if let Some(path) = choose(&window, &core, true, false)
                .await?
                .into_iter()
                .next()
            {
                let lease = core.lease();
                let root = lease.root.clone();
                let client = core.client.clone();
                let (plan, preview) = tauri::async_runtime::spawn_blocking(move || {
                    let _lease = lease;
                    crate::launcher::migration::preview(&client, &root, &path)
                })
                .await
                .map_err(|_| ApiError::new("IMPORT_PREVIEW", "无法读取旧数据。"))??;
                let token = uuid::Uuid::new_v4().simple().to_string();
                let mut grants = state.legacy_imports.lock().unwrap();
                grants.retain(|_, (time, _)| time.elapsed().as_secs() < 900);
                if grants.len() >= 4 {
                    grants.clear();
                }
                grants.insert(token.clone(), (Instant::now(), plan));
                let mut value = serde_json::to_value(preview)?;
                value["token"] = token.into();
                value
            } else {
                serde_json::Value::Null
            }
        }
        LauncherAction::ModpackImport => {
            let Some(path) = choose(&window, &core, false, false).await?.into_iter().next() else {
                return Ok(serde_json::Value::Null);
            };
            serde_json::json!(core.import_modpack(path).await?)
        }
        LauncherAction::ModpackDirectories { id } => core.modpack_directories(&id).await?,
        LauncherAction::ModpackExport { id, options } => {
            let Some(path) = choose(&window, &core, true, false).await?.into_iter().next() else {
                return Ok(serde_json::Value::Null);
            };
            serde_json::json!(core.export_modpack(&id, path, options)?)
        }
        LauncherAction::ImportLegacyCommit { token } => {
            let (created, plan) = state
                .legacy_imports
                .lock()
                .unwrap()
                .remove(&token)
                .ok_or_else(|| ApiError::new("IMPORT_EXPIRED", "导入预览已过期，请重新选择。"))?;
            if created.elapsed().as_secs() > 900 {
                return Err(ApiError::new("IMPORT_EXPIRED", "导入预览已过期。"));
            }
            serde_json::to_value(core.import_legacy(plan)?)?
        }
        LauncherAction::OpenMods { id } => {
            let path = {
                let store = core.store.lock().unwrap();
                let game = store.game_dir(&id)?;
                crate::launcher::storage::within(&game, "mods")?
            };
            std::fs::create_dir_all(&path)?;
            crate::launcher::environment::open_folder(
                &core.plugins,
                &core.lease().settings,
                &path,
            )?;
            serde_json::Value::Null
        }
        LauncherAction::ModsList { id } => serde_json::to_value(core.mods(&id).await?)?,
        LauncherAction::ModUpdates { id, hashes } => core.mod_updates(&id, hashes).await?,
        LauncherAction::ModsUpdate { id, files } => serde_json::to_value(core.update_mods(id, files)?)?,
        LauncherAction::ModsImport { id } => {
            let files = choose(&window, &core, false, true).await?;
            if files.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::to_value(core.import_mods(id, files)?)?
            }
        }
        LauncherAction::ModsDropped { id, token } => {
            let (created, paths) = state
                .dropped_files
                .lock()
                .unwrap()
                .remove(&token)
                .ok_or_else(|| {
                    ApiError::new(
                        "INVALID_FILE_SELECTION",
                        "拖入文件的授权已过期，请重新拖入。",
                    )
                })?;
            if created.elapsed().as_secs() > 300 {
                return Err(ApiError::new("EXPIRED_SELECTION", "拖入文件的授权已过期。"));
            }
            serde_json::to_value(core.import_mods(id, paths)?)?
        }
        LauncherAction::ModsEnable { id, files, enabled } => {
            core.enable_mods(&id, files, enabled).await?;
            serde_json::Value::Null
        }
        LauncherAction::ModsDelete {
            id,
            files,
            confirmed,
        } => {
            core.delete_mods(&id, files, confirmed).await?;
            serde_json::Value::Null
        }
        LauncherAction::ModsName { id, file, name } => {
            core.name_mod(&id, file, &name).await?;
            serde_json::Value::Null
        }
        LauncherAction::ResourceFilters { source } => {
            core.resource_catalog("filters", serde_json::json!({"source":source})).await?
        }
        LauncherAction::ResourceSearch { query, game_version, category, loader, offset, source } => {
            core.resource_catalog("search", serde_json::json!({
                "query": query, "gameVersion": game_version, "category": category, "loader": loader, "offset": offset, "source":source,
            })).await?
        }
        LauncherAction::ResourceVersions { id, project, source } => core.resource_versions(&id, &project, source).await?,
        LauncherAction::ResourceDependencies { project, version_id, source } => {
            core.resource_catalog("dependencies", serde_json::json!({
                "project": project, "versionId": version_id, "source":source,
            })).await?
        }
        LauncherAction::ResourceDownload { id, project, version_id, source } => {
            serde_json::to_value(core.download_mod(id, project, version_id, source)?)?
        }
        LauncherAction::ThemeCatalog => crate::plugins::themes::catalog(&state.host, &core.lease().settings, &state.plugins.lock().unwrap()),
        LauncherAction::ThemePrepare { choice, system_dark, high_contrast } => {
            let host = state.host.clone(); let settings = core.lease().settings.clone(); let safe = state.safe_theme;
            let visual = tauri::async_runtime::spawn_blocking(move || host.themes.prepare(&host, &settings, choice, system_dark, high_contrast, safe)).await
                .map_err(|_| ApiError::new("THEME_INVALID", "主题准备任务异常结束。"))??;
            serde_json::to_value(visual)?
        }
        LauncherAction::ThemeApply { token, preview } => {
            let visual = state.host.themes.commit(&token, preview)?;
            let opaque = state.safe_theme || visual.active.as_ref().is_some_and(|v| v.high_contrast || v.choice.material == "opaque");
            match crate::window::apply_preferences(&window, &core, opaque) {
                Ok(appearance) => *state.appearance.lock().unwrap() = appearance,
                Err(e) => *state.appearance.lock().unwrap() = crate::window::Appearance {material:"solid".into(),reason:e.message,rounded:false},
            }
            state.theme_announced.store(visual.revision,Ordering::SeqCst);
            state.host.emit("theme.changed", visual.public_appearance());
            serde_json::to_value(visual)?
        }
        LauncherAction::ThemeConfirm { token } => {
            let active = state.host.themes.state().active.ok_or_else(|| ApiError::new("THEME_STALE", "无活动主题。"))?;
            if let Some(digest) = active.package_digest.as_ref() {
                let id = active.actual_id.split_once(':').unwrap().0;
                if state.host.checked(&core.lease().settings,id)?.digest != *digest { return Err(ApiError::new("THEME_STALE", "主题包已变化。")); }
            }
            let result = state.host.themes.confirm(&token, |choice| { let mut settings = core.lease().settings.clone(); settings.theme = choice; core.save_settings(settings) });
            let visual = result?;
            state.theme_announced.store(visual.revision,Ordering::SeqCst);
            state.host.emit("theme.changed", visual.public_appearance());
            serde_json::to_value(visual)?
        }
        LauncherAction::ThemeCancel => {
            let visual=state.host.themes.cancel();
            let opaque=state.safe_theme || visual.active.as_ref().is_some_and(|v|v.high_contrast || v.choice.material=="opaque");
            if let Ok(material)=crate::window::apply_preferences(&window,&core,opaque) {*state.appearance.lock().unwrap()=material;}
            state.theme_announced.store(visual.revision,Ordering::SeqCst);
            state.host.emit("theme.changed", visual.public_appearance());
            serde_json::to_value(visual)?
        },
        LauncherAction::ThemeRelease { token } => {state.host.themes.release(&token); serde_json::Value::Null},
        LauncherAction::ThemeState => {
            let visual=state.host.themes.state();
            if state.theme_announced.swap(visual.revision,Ordering::SeqCst)!=visual.revision {
                let opaque=state.safe_theme || visual.active.as_ref().is_some_and(|v|v.high_contrast || v.choice.material=="opaque");
                if let Ok(material)=crate::window::apply_preferences(&window,&core,opaque) {*state.appearance.lock().unwrap()=material;}
                state.host.emit("theme.changed",visual.public_appearance());
            }
            serde_json::to_value(visual)?
        },
        LauncherAction::ThemeRestore => {
            let mut settings = core.lease().settings.clone(); settings.theme = Default::default(); core.save_settings(settings)?;
            state.host.themes.cancel(); state.host.themes.clear(); serde_json::Value::Null
        }
        LauncherAction::SaveSettings { mut settings } => {
            settings.theme = core.lease().settings.theme.clone();
            let restore_account = {
                let store = core.store.lock().unwrap();
                store.settings.account_mode != settings.account_mode
                    || store.settings.microsoft_client_id != settings.microsoft_client_id
            };
            let changed = {
                let store = core.store.lock().unwrap();
                store.settings.modules != settings.modules
                    || store.settings.game_directory != settings.game_directory
                    || store.settings.isolate_game_directory != settings.isolate_game_directory
            };
            core.save_settings(settings)?;
            if changed {
                state.panels.clear();
                state.generation.fetch_add(1, Ordering::SeqCst);
                state.host.stop_all().await?;
            }
            if restore_account {
                core.restore();
            }
            serde_json::Value::Null
        }
        LauncherAction::ChangeDataDirectory => {
            if let Some(path) = choose(&window, &core, true, false)
                .await?
                .into_iter()
                .next()
            {
                state.host.themes.clear();
                core.change_root(path)?;
                state.panels.clear();
                state.generation.fetch_add(1, Ordering::SeqCst);
                state.host.stop_all().await?;
                refresh_plugins(&state)?;
            }
            serde_json::Value::Null
        }
        LauncherAction::ChangeGameDirectory => {
            if let Some(path) = choose(&window, &core, true, false)
                .await?
                .into_iter()
                .next()
            {
                core.change_game_root(path)?;
                state.panels.clear();
                state.generation.fetch_add(1, Ordering::SeqCst);
                state.host.stop_all().await?;
            }
            serde_json::Value::Null
        }
        LauncherAction::I18nBundle { locale } => {
            let lease = core.lease();
            core.plugins.call_service(
                &lease.settings,
                "i18n",
                "i18n",
                "bundle",
                serde_json::json!({"root":lease.root,"locale":locale}),
            )?
        }
        LauncherAction::Versions => serde_json::to_value(core.versions().await?)?,
        LauncherAction::ForgeOptions { version, kind } => {
            core.forge_options(&version, &kind).await?
        }
        LauncherAction::LoaderOptions { provider, version } => core.plugins.provider_call(&core.lease().settings,"loaderProvider",&provider,"versions",serde_json::json!({"gameVersion":version})).await?,
        LauncherAction::FabricOptions { version } => core.fabric_options(&version).await?,
        LauncherAction::CreateInstance {
            version,
            name,
            components,
        } => serde_json::to_value(core.create(version, name, components).await?)?,
        LauncherAction::RenameInstance { id, name } => {
            core.rename(&id, &name)?;
            serde_json::Value::Null
        }
        LauncherAction::MoveInstance { id, target, after } => {
            core.store.lock().unwrap().move_instance(&id, &target, after)?;
            serde_json::Value::Null
        }
        LauncherAction::SaveOverrides { id, overrides } => {
            core.overrides(&id, overrides)?;
            serde_json::Value::Null
        }
        LauncherAction::DeleteInstance { id, confirmation } => {
            core.delete(&id, &confirmation)?;
            serde_json::Value::Null
        }
        LauncherAction::JavaScan => serde_json::to_value(core.scan_java()?)?,
        LauncherAction::JavaChoose => {
            core.require("java.runtime")?;
            if let Some(path) = choose(&window, &core, false, false)
                .await?
                .into_iter()
                .next()
            {
                let java =
                    crate::launcher::java::inspect(&core.plugins, &core.lease().settings, &path)
                        .await?;
                let mut known = core.java.lock().unwrap();
                known.retain(|j| j.path != java.path);
                known.push(java.clone());
                serde_json::to_value(java)?
            } else {
                serde_json::Value::Null
            }
        }
        LauncherAction::ProfileAddProvider { provider, name } => core.profile_change("profiles.addProvider",serde_json::json!({"provider":provider,"name":name}))?,
        LauncherAction::ProfileAddMicrosoft => serde_json::to_value(core.add_microsoft_profile()?)?,
        LauncherAction::ProfileAddOffline { name } => {
            core.profile_change("profiles.addOffline", serde_json::json!({"name":name}))?
        }
        LauncherAction::ProfileSelect { id } => {
            core.select_profile(&id)?;
            serde_json::Value::Null
        }
        LauncherAction::ProfileMove { id, target, after } => core.profile_change("profiles.move",serde_json::json!({"id":id,"target":target,"after":after}))?,
        LauncherAction::ProfileDelete { id, confirmation } => {
            core.delete_profile(&id, &confirmation)?;
            serde_json::Value::Null
        }
        LauncherAction::SignIn => serde_json::to_value(core.login(false)?)?,
        LauncherAction::SignInMicrosoft => serde_json::to_value(core.login_microsoft()?)?,
        LauncherAction::SignOut => {
            core.sign_out()?;
            serde_json::Value::Null
        }
        LauncherAction::Launch { id } => {
            let task = tauri::async_runtime::spawn_blocking(move || core.launch(id, window))
                .await
                .map_err(|_| ApiError::new("LAUNCH_ERROR", "无法创建启动任务。"))??;
            serde_json::to_value(task)?
        }
        LauncherAction::Repair { id } => serde_json::to_value(core.install(id)?)?,
        LauncherAction::RetryInstallation { id } => {
            serde_json::to_value(core.retry_installation(&id).await?)?
        }
        LauncherAction::CancelTask { id } => {
            core.tasks.cancel(&id)?;
            serde_json::Value::Null
        }
        LauncherAction::OpenDirectory { id } => {
            let path = {
                let store = core.store.lock().unwrap();
                if let Some(id) = id {
                    store.game_dir(&id)?
                } else {
                    store.root.clone()
                }
            };
            crate::launcher::environment::open_folder(
                &core.plugins,
                &core.lease().settings,
                &path,
            )?;
            serde_json::Value::Null
        }
        LauncherAction::OpenLogs { id } => {
            let task = core
                .tasks
                .list()
                .into_iter()
                .find(|t| t.id == id)
                .ok_or_else(|| ApiError::new("UNKNOWN_TASK", "任务不存在。"))?;
            let path = task
                .crash_report
                .as_ref()
                .and_then(|report| report["logPath"].as_str())
                .map(String::from)
                .filter(|path| std::path::Path::new(path).is_file())
                .or(task.log_path)
                .ok_or_else(|| ApiError::new("LOG_MISSING", "此任务尚未生成日志。"))?;
            crate::launcher::environment::open_text(
                &core.plugins,
                &core.lease().settings,
                std::path::Path::new(&path),
            )?;
            serde_json::Value::Null
        }
        LauncherAction::ExportCrashReport { id } => {
            let task = core
                .tasks
                .list()
                .into_iter()
                .find(|task| task.id == id)
                .ok_or_else(|| ApiError::new("UNKNOWN_TASK", "任务不存在。"))?;
            if task.crash_report.is_none() {
                return Err(ApiError::new("REPORT_MISSING", "此任务尚未生成崩溃报告。"));
            }
            let lease = core.lease();
            core.require("minecraft.crash")?;
            let Some(destination) = choose(&window, &core, true, false)
                .await?
                .into_iter()
                .next()
            else {
                return Ok(serde_json::Value::Null);
            };
            tauri::async_runtime::spawn_blocking(move || {
                core.client.service(
                    "minecraft.crash",
                    "export",
                    serde_json::json!({"root":lease.root,"taskId":id,"destination":destination}),
                )
            })
            .await
            .map_err(|_| ApiError::new("REPORT_EXPORT", "导出崩溃报告失败。"))??
        }
    };
    Ok(value)
}
fn plugin(state: &AppState, id: &str) -> Result<Plugin> {
    let settings = state.core.store.lock().unwrap().settings.clone();
    state.host.set_settings(&settings);
    let plugins = state.plugins.lock().unwrap();
    let plugin = plugins
        .iter()
        .find(|p| p.manifest.id == id)
        .ok_or_else(|| ApiError::new("UNKNOWN_PLUGIN", "未找到已授权开发插件。"))?;
    crate::plugins::manager::order(&plugins, &settings, id)?;
    let checked = manifest::load(&plugin.root)?;
    if checked.digest != plugin.digest {
        return Err(ApiError::new(
            "PLUGIN_CHANGED",
            "插件文件已变化，请先校验并重载。",
        ));
    }
    Ok(checked)
}
fn refresh_plugins(state: &AppState) -> Result<()> {
    let lease = state.core.lease();
    crate::plugins::manager::recover(&lease.root)?;
    let read_only = state.core.store.lock().unwrap().read_only;
    let mut plugins = load_plugins(&lease.root, &state.development, read_only)?;
    // Keep changed packages visible for explicit reload; missing directories are removed.
    let previous = state.plugins.lock().unwrap().clone();
    for old in &previous {
        if (old.manifest.runtime == "javascript" || old.manifest.entry.starts_with("bin/"))
            && old.root.is_dir()
            && old.root
                == crate::launcher::storage::within(
                    &lease.root,
                    &format!("plugins/{}", old.manifest.id),
                )?
            && !plugins.iter().any(|p| p.manifest.id == old.manifest.id)
        {
            plugins.push(old.clone());
        }
    }
    // Development paths authorize the location, not a reset to the startup digest.
    for plugin in &mut plugins {
        if state.development.iter().any(|p| p.root == plugin.root)
            && let Some(old) = previous.iter().find(|p| p.root == plugin.root)
        {
            *plugin = old.clone();
        }
    }
    state.host.set_plugins(plugins.clone());
    *state.plugins.lock().unwrap() = plugins;
    Ok(())
}
fn load_plugins(
    root: &std::path::Path,
    development: &[Plugin],
    read_only: bool,
) -> Result<Vec<Plugin>> {
    crate::plugins::manager::load(root, development).or_else(|error| {
        if read_only {
            // Unknown configuration versions must still open without creating native manifests.
            crate::plugins::manager::discover(root, development)
        } else {
            Err(error)
        }
    })
}
async fn invalidate_plugin(state: &AppState, id: &str) -> Result<()> {
    let affected = crate::plugins::manager::affected(&state.plugins.lock().unwrap(), id);
    state.host.themes.cancel();
    state.panels.close_plugins(&affected);
    state.host.stop_plugins(affected).await?;
    state.generation.fetch_add(1, Ordering::SeqCst);
    Ok(())
}
async fn activate_dependencies(state: &AppState, id: &str) -> Result<()> {
    let order = {
        let settings = state.core.store.lock().unwrap().settings.clone();
        let plugins = state.plugins.lock().unwrap();
        crate::plugins::manager::order(&plugins, &settings, id)?
    };
    for dependency in order.into_iter().filter(|p| p.manifest.id != id) {
        state
            .host
            .activate(plugin(state, &dependency.manifest.id)?)
            .await?;
    }
    Ok(())
}
#[tauri::command]
async fn plugin_run(
    window: WebviewWindow,
    state: State<'_, AppState>,
    plugin_id: String,
    command: String,
    params: Option<serde_json::Value>,
) -> Result<RunReport> {
    main_document(&window)?;
    let _lease = state.core.lease();
    let _guard = state.plugin_mutation.read().await;
    let _usage = state.host.lease(&plugin_id);
    drop(_guard);
    activate_dependencies(&state, &plugin_id).await?;
    state
        .host
        .run_params(
            plugin(&state, &plugin_id)?,
            command,
            params.unwrap_or_default(),
            false,
        )
        .await
}
#[tauri::command]
async fn plugin_reload(
    window: WebviewWindow,
    state: State<'_, AppState>,
    plugin_id: String,
    command: Option<String>,
) -> Result<RunReport> {
    main_document(&window)?;
    let lease = state.core.lease();
    let _guard = state.plugin_mutation.write().await;
    let affected = crate::plugins::manager::affected(&state.plugins.lock().unwrap(), &plugin_id);
    if state.core.tasks.busy(None)
        && !state
            .plugins
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.manifest.id == plugin_id && crate::plugins::themes::standalone(p))
        || state.host.busy_plugins(&affected)
    {
        return Err(ApiError::new(
            "PLUGIN_BUSY",
            "请等待任务和游戏结束后重载插件。",
        ));
    }
    let previous = state
        .plugins
        .lock()
        .unwrap()
        .iter()
        .find(|p| p.manifest.id == plugin_id)
        .cloned()
        .ok_or_else(|| ApiError::new("UNKNOWN_PLUGIN", "未找到插件。"))?;
    let checked = crate::plugins::manager::reload_candidate(&previous)?;
    let mut plugins = state.plugins.lock().unwrap().clone();
    *plugins
        .iter_mut()
        .find(|p| p.manifest.id == plugin_id)
        .unwrap() = checked.clone();
    let settings = state.core.store.lock().unwrap().settings.clone();
    let order = crate::plugins::manager::order(&plugins, &settings, &plugin_id)?;
    for dependency in order.into_iter().filter(|p| p.manifest.id != plugin_id) {
        state
            .host
            .activate(plugin(&state, &dependency.manifest.id)?)
            .await?;
    }
    let development = state.development.iter().any(|p| p.root == previous.root);
    if !development {
        state.core.store.lock().unwrap().writable()?;
    }
    let root = lease.root.clone();
    let candidate = checked.clone();
    let report = state
        .host
        .reload_with_commit(checked.clone(), move || {
            if development {
                if crate::plugins::manager::reload_candidate(&previous)?.digest != candidate.digest
                {
                    return Err(ApiError::new("PLUGIN_CHANGED", "重载期间插件文件已变化。"));
                }
                Ok(())
            } else {
                crate::plugins::manager::approve_reload(&root, &previous, &candidate)
            }
        })
        .await?;
    let consumers = crate::plugins::manager::affected(&plugins, &plugin_id)
        .into_iter()
        .filter(|id| id != &plugin_id)
        .collect();
    state
        .panels
        .close_plugins(&crate::plugins::manager::affected(&plugins, &plugin_id));
    state.host.stop_plugins(consumers).await?;
    state.host.set_plugins(plugins.clone());
    *state.plugins.lock().unwrap() = plugins;
    state.generation.fetch_add(1, Ordering::SeqCst);
    state.host.themes.cancel();
    if let Some(command) = command {
        state.host.run(checked, command, false).await
    } else {
        Ok(report)
    }
}
#[tauri::command]
async fn panel_open(
    window: WebviewWindow,
    state: State<'_, AppState>,
    plugin_id: String,
    panel: String,
) -> Result<crate::plugins::panel::PanelSession> {
    main_document(&window)?;
    if !state.panel_navigation_guard.load(Ordering::Acquire) {
        return Err(ApiError::new(
            "PANEL_UNAVAILABLE",
            "当前平台的面板导航隔离未通过初始化。",
        ));
    }
    let _guard = state.plugin_mutation.read().await;
    let _usage = state.host.lease(&plugin_id);
    drop(_guard);
    activate_dependencies(&state, &plugin_id).await?;
    let checked = plugin(&state, &plugin_id)?;
    let report = state.host.activate(checked.clone()).await?;
    state.panels.open(checked, &panel, report.generation)
}
#[tauri::command]
async fn panel_call(
    window: WebviewWindow,
    state: State<'_, AppState>,
    session_id: String,
    generation: u64,
    command: String,
    params: Option<serde_json::Value>,
) -> Result<RunReport> {
    main_document(&window)?;
    let _lease = state.core.lease();
    let _guard = state.plugin_mutation.read().await;
    let plugin = state.panels.plugin(&session_id, generation)?;
    let checked = crate::desktop::plugin(&state, &plugin.manifest.id)?;
    if checked.digest != plugin.digest {
        return Err(ApiError::new("STALE_PANEL", "插件已更新，请重新打开面板。"));
    }
    let _usage = state.host.lease(&plugin.manifest.id);
    drop(_guard);
    activate_dependencies(&state, &plugin.manifest.id).await?;
    if let Some(method) = command.strip_prefix('$') {
        state
            .host
            .panel_task(
                plugin,
                generation,
                method.into(),
                params.unwrap_or_default(),
            )
            .await
    } else {
        state
            .host
            .run_params(plugin, command, params.unwrap_or_default(), false)
            .await
    }
}
#[tauri::command]
fn panel_theme(
    window: WebviewWindow,
    state: State<'_, AppState>,
    session_id: String,
    revision: u64,
    token: String,
    ready: bool,
) -> Result<()> {
    main_document(&window)?;
    state
        .panels
        .bind_theme(&session_id, revision, &token, &state.host.themes, ready)
}
#[tauri::command]
fn panel_close(
    window: WebviewWindow,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<()> {
    main_document(&window)?;
    state.panels.close(&session_id);
    Ok(())
}
#[tauri::command]
async fn ui_bootstrap(
    window: WebviewWindow,
    state: State<'_, AppState>,
    provider: String,
) -> Result<Vec<serde_json::Value>> {
    main_document(&window)?;
    let settings = state.core.store.lock().unwrap().settings.clone();
    let host = state.host.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::plugins::frontend::bootstrap(&host, &settings, &provider)
    })
    .await
    .map_err(|_| ApiError::new("BOOTSTRAP_ERROR", "无法读取界面插件。"))?
}
#[tauri::command]
fn ui_ready(window: WebviewWindow, state: State<'_, AppState>) -> Result<()> {
    main_document(&window)?;
    if let Some(path) = &state.ready_file {
        std::fs::write(
            path,
            serde_json::to_vec(
                &serde_json::json!({"elapsedMs":state.started.elapsed().as_millis(), "windowGeometry": crate::window::geometry(&window,&state.core)?}),
            )?,
        )?;
    }
    Ok(())
}

pub fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let args: Vec<_> = std::env::args().skip(1).collect();
    // This check does not create a window or touch launcher/game data.
    let version =
        tauri::webview_version().map_err(|_| "缺少兼容的 Microsoft Edge WebView2 Runtime。")?;
    let major: u32 = version.split('.').next().unwrap_or("").parse().unwrap_or(0);
    if major < 110 {
        return Err("WebView2 Runtime 版本过低，请升级至 110 或更新版本。".into());
    }
    if args == ["--check"] {
        return Ok(());
    }
    let mut directories = Vec::new();
    let mut ready_file = None;
    let mut webview_data = None;
    let mut data_root = None;
    let mut safe_theme = false;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "--safe-theme" => safe_theme = true,
            "--data-dir" => {
                data_root = Some(PathBuf::from(iter.next().ok_or("--data-dir 需要完整目录")?));
            }
            "--dev-plugin" => {
                directories.push(PathBuf::from(iter.next().ok_or("--dev-plugin 需要目录")?))
            }
            "--benchmark-ready-file" => {
                ready_file = Some(PathBuf::from(iter.next().ok_or("缺少基准结果路径")?))
            }
            "--benchmark-webview-data" => {
                webview_data = Some(PathBuf::from(iter.next().ok_or("缺少 WebView 数据目录")?))
            }
            _ => return Err("用法：Launcher.exe [--check] [--dev-plugin <directory>]".into()),
        }
    }
    let development = directories
        .iter()
        .map(|d| manifest::load(d))
        .collect::<Result<Vec<_>>>()?;
    let core = crate::launcher::core::Launcher::open(data_root)?;
    let plugins = {
        let store = core.store.lock().unwrap();
        crate::plugins::manager::recover(&store.root)?;
        load_plugins(&store.root, &development, store.read_only)?
    };
    core.plugins.set_plugins(plugins.clone());
    let unique: std::collections::HashSet<_> = plugins.iter().map(|p| &p.manifest.id).collect();
    if unique.len() != plugins.len() {
        return Err("开发插件 ID 重复。".into());
    }
    let builder = tauri::Builder::default()
        .plugin(
            tauri::plugin::Builder::<tauri::Wry>::new("navigation-guard")
                .on_navigation(|_, url| {
                    url.scheme() == "tauri" && url.host_str() == Some("localhost")
                        || url.scheme() == "http"
                            && (url.host_str() == Some("tauri.localhost")
                                || cfg!(debug_assertions)
                                    && url.host_str() == Some("127.0.0.1")
                                    && url.port() == Some(1420))
                })
                .build(),
        )
        .register_uri_scheme_protocol("hydrogen-ui", |context, request| {
            let state = context.app_handle().state::<AppState>();
            let settings = state.core.store.lock().unwrap().settings.clone();
            let result = if request.method() == "GET" {
                if request.uri().path().starts_with("/@theme/") {
                    state.host.themes.resource(request.uri().path(), false)
                } else if request.uri().path().starts_with("/@theme-preview/") {
                    crate::plugins::themes::preview_resource(
                        &state.host,
                        &settings,
                        request.uri().path(),
                    )
                } else {
                    crate::plugins::frontend::resource(&state.host, &settings, request.uri().path())
                }
            } else {
                Err(ApiError::new("METHOD_DENIED", "仅允许读取静态资源。"))
            };
            let (status, body, mime) = match result {
                Ok((body, mime)) => (200, body, mime),
                Err(_) => (403, b"Plugin resource unavailable".to_vec(), "text/plain"),
            };
            tauri::http::Response::builder()
                .status(status)
                .header("Content-Type", mime)
                .header("X-Content-Type-Options", "nosniff")
                .header("Cache-Control", "no-store")
                .header("Access-Control-Allow-Origin", "*")
                .body(body)
                .unwrap()
        })
        .register_uri_scheme_protocol("hydrogen-plugin", |context, request| {
            let state = context.app_handle().state::<AppState>();
            let result = if request.method() == "GET" {
                state
                    .panels
                    .theme_resource(request.uri().path(), &state.host.themes)
            } else {
                Err(ApiError::new("METHOD_DENIED", "仅允许读取静态资源。"))
            };
            let (status, body, mime) = match result {
                Ok((body, mime)) => (200, body, mime),
                Err(_) => (403, b"Resource denied".to_vec(), "text/plain"),
            };
            tauri::http::Response::builder()
                .status(status)
                .header("Content-Type", mime)
                .header("Content-Security-Policy", PANEL_CSP)
                .header("X-Content-Type-Options", "nosniff")
                .header("Cache-Control", "no-store")
                .header("Access-Control-Allow-Origin", "null")
                .body(body)
                .unwrap()
        })
        .invoke_handler(tauri::generate_handler![
            app_snapshot,
            instance_select,
            plugin_run,
            plugin_reload,
            panel_open,
            panel_call,
            panel_theme,
            panel_close,
            ui_ready,
            ui_bootstrap,
            launcher_action
        ])
        .setup(move |app| {
            let dispatcher = app.handle().clone();
            core.plugins
                .set_main_executor(std::sync::Arc::new(move |task| {
                    dispatcher
                        .run_on_main_thread(task)
                        .map_err(|_| ApiError::new("MAIN_THREAD_UNAVAILABLE", "主线程已停止。"))
                }));
            app.manage(AppState {
                host: core.plugins.clone(),
                safe_theme,
                theme_announced: AtomicU64::new(0),
                plugins: Mutex::new(plugins),
                development,
                plugin_mutation: tokio::sync::RwLock::new(()),
                plugin_candidates: Mutex::new(std::collections::BTreeMap::new()),
                core: core.clone(),
                panels: Panels::default(),
                generation: AtomicU64::new(1),
                started,
                ready_file,
                panel_navigation_guard: AtomicBool::new(false),
                appearance: Mutex::new(crate::window::Appearance {
                    material: "solid".into(),
                    reason: "正在初始化窗口材质".into(),
                    rounded: false,
                }),
                dropped_files: Mutex::new(std::collections::BTreeMap::new()),
                event_sequences: Mutex::default(),
                legacy_imports: Mutex::new(std::collections::BTreeMap::new()),
            });
            let window =
                tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                    .initialization_script(include_str!("frame-guard.js"))
                    .build()?;
            crate::plugins::capabilities::desktop::attach(&core, window.clone());
            let startup = app
                .state::<AppState>()
                .plugins
                .lock()
                .unwrap()
                .iter()
                .filter(|p| {
                    p.manifest.runtime == "javascript"
                        && p.manifest.activation.iter().any(|a| a == "startup")
                })
                .cloned()
                .collect::<Vec<_>>();
            let startup_host = core.plugins.clone();
            let startup_settings = core.lease().settings.clone();
            tauri::async_runtime::spawn(async move {
                for plugin in startup {
                    if startup_host.available(&startup_settings, &plugin.manifest.id) {
                        let _ = startup_host.activate(plugin).await;
                    }
                }
            });
            *app.state::<AppState>().appearance.lock().unwrap() = crate::window::apply_preferences(
                &window,
                &core,
                safe_theme || core.lease().settings.theme.material == "opaque",
            )?;
            crate::window::attach_resize(&window, core.clone());
            let drop_app = app.handle().clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) =
                    event
                {
                    if paths.is_empty() || paths.len() > 1000 {
                        return;
                    }
                    if let Some(path) = paths.iter().find(|path| {
                        path.extension().is_some_and(|ext| {
                            ext.eq_ignore_ascii_case("mrpack") || ext.eq_ignore_ascii_case("zip")
                        })
                    }) {
                        if paths.len() != 1 {
                            let error =
                                ApiError::new("INVALID_MODPACK", "每次请拖入一个整合包文件。");
                            let _ = drop_app.emit_to("main", "modpack-drop", Some(error));
                            return;
                        }
                        let (app, path) = (drop_app.clone(), path.clone());
                        tauri::async_runtime::spawn(async move {
                            let state = app.state::<AppState>();
                            let _guard = state.plugin_mutation.write().await;
                            let error = state.core.import_modpack(path).await.err();
                            let _ = app.emit_to("main", "modpack-drop", error);
                        });
                        return;
                    }
                    let state = drop_app.state::<AppState>();
                    let token = uuid::Uuid::new_v4().simple().to_string();
                    let mut grants = state.dropped_files.lock().unwrap();
                    grants.retain(|_, (created, _)| created.elapsed().as_secs() < 300);
                    if grants.len() >= 16 {
                        return;
                    }
                    grants.insert(token.clone(), (Instant::now(), paths.clone()));
                    drop(grants);
                    let _ = drop_app.emit_to(
                        "main",
                        "mod-files-dropped",
                        serde_json::json!({"token":token,"count":paths.len()}),
                    );
                }
            });
            core.restore();
            #[cfg(windows)]
            {
                let app_handle = app.handle().clone();
                window.with_webview(move |webview| unsafe {
                    // WebView2's top-level navigation callback does not cover iframe navigation.
                    // Restrict child frames before their remote request is issued, independently of CSP.
                    use webview2_com::{NavigationStartingEventHandler, take_pwstr};
                    let result = (|| -> windows::core::Result<()> {
                        let core = webview.controller().CoreWebView2()?;
                        let bound_app = app_handle.clone();
                        let handler =
                            NavigationStartingEventHandler::create(Box::new(move |_, args| {
                                if let Some(args) = args {
                                    let mut uri = windows::core::PWSTR::null();
                                    args.Uri(&mut uri)?;
                                    let uri = take_pwstr(uri);
                                    let allowed = tauri::Url::parse(&uri).is_ok_and(|url| {
                                        url.scheme() == "http"
                                            && url.host_str() == Some("hydrogen-plugin.localhost")
                                            && bound_app
                                                .state::<AppState>()
                                                .panels
                                                .resource(url.path())
                                                .is_ok()
                                    });
                                    args.SetCancel(!allowed)?;
                                }
                                Ok(())
                            }));
                        core.add_FrameNavigationStarting(&handler, &mut 0)?;
                        Ok(())
                    })();
                    app_handle
                        .state::<AppState>()
                        .panel_navigation_guard
                        .store(result.is_ok(), Ordering::Release);
                })?;
            }
            Ok(())
        });
    // Keep the WebView's disposable cache apart from real launcher data and from the old application.
    let mut context = tauri::generate_context!();
    if let Some(path) = webview_data {
        context.config_mut().app.windows[0].data_directory = Some(path);
    }
    builder.build(context)?.run(|app, event| {
        if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
            let state = app.state::<AppState>();
            state.core.tasks.stop_preparation();
            state.panels.clear();
            let _ = state.host.stop_main();
            let host = state.host.clone();
            tauri::async_runtime::spawn(async move {
                let _ = host.stop_all().await;
            });
        }
    });
    Ok(())
}
