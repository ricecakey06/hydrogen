use super::manifest::{self, Plugin};
use crate::{
    contracts::{ApiError, Result},
    launcher::storage::{self, Settings},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub fn enabled(settings: &Settings, id: &str) -> bool {
    settings.enabled(id)
}

pub fn directory(root: &Path) -> Result<PathBuf> {
    storage::within(root, "plugins")
}

pub fn initialize(root: &Path) -> Result<()> {
    recover(root)?;
    fs::create_dir_all(directory(root)?)?;
    super::builtin::extract(root)?;
    Ok(())
}

pub fn load(root: &Path, development: &[Plugin]) -> Result<Vec<Plugin>> {
    discover(root, development)
}

pub struct Candidate {
    pub plugin: Plugin,
    pub data_root: PathBuf,
}
impl Drop for Candidate {
    fn drop(&mut self) {
        let _ = remove_tree(&self.plugin.root);
    }
}
pub fn remove_tree(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    fn check(path: &Path, depth: usize) -> Result<()> {
        if depth > 64 || path.ancestors().any(storage::linked) {
            return Err(ApiError::new(
                "LINKED_PATH",
                "插件目录包含链接，拒绝递归操作。",
            ));
        }
        if path.is_dir() {
            for entry in fs::read_dir(path)? {
                check(&entry?.path(), depth + 1)?;
            }
        }
        Ok(())
    }
    check(path, 0)?;
    fs::remove_dir_all(path)?;
    Ok(())
}
pub fn discover(root: &Path, development: &[Plugin]) -> Result<Vec<Plugin>> {
    let folder = directory(root)?;
    if development
        .iter()
        .map(|p| &p.manifest.id)
        .collect::<HashSet<_>>()
        .len()
        != development.len()
    {
        return Err(ApiError::new("DUPLICATE_PLUGIN", "开发插件 ID 重复。"));
    }
    let mut result = development.to_vec();
    let receipts = receipts(root)?;
    if folder.exists() {
        for entry in fs::read_dir(folder)? {
            let entry = entry?;
            if entry.file_type()?.is_dir()
                && let Ok(plugin) = manifest::load(&entry.path())
            {
                if receipts.get(&plugin.manifest.id) != Some(&plugin.digest) {
                    continue;
                }
                if result.iter().any(|p| p.manifest.id == plugin.manifest.id) {
                    // An explicit --dev-plugin selects the implementation for this session.
                    continue;
                }
                if entry.file_name().to_string_lossy() != plugin.manifest.id {
                    return Err(ApiError::new(
                        "PLUGIN_DIRECTORY",
                        "插件 ID 与安装目录不一致。",
                    ));
                }
                result.push(plugin);
            }
        }
    }
    result.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    Ok(result)
}
pub fn order(plugins: &[Plugin], settings: &Settings, id: &str) -> Result<Vec<Plugin>> {
    fn visit(
        plugins: &[Plugin],
        settings: &Settings,
        id: &str,
        visiting: &mut HashSet<String>,
        done: &mut HashSet<String>,
        out: &mut Vec<Plugin>,
    ) -> Result<()> {
        if !enabled(settings, id) {
            return Err(ApiError::new(
                "PLUGIN_DISABLED",
                format!("依赖 {id} 已停用。"),
            ));
        }
        if done.contains(id) {
            return Ok(());
        }
        if !visiting.insert(id.into()) {
            return Err(ApiError::new("DEPENDENCY_CYCLE", "插件依赖存在循环。"));
        }
        let plugin = plugins
            .iter()
            .find(|p| p.manifest.id == id)
            .ok_or_else(|| ApiError::new("MISSING_DEPENDENCY", format!("缺少插件依赖 {id}。")))?;
        for value in &plugin.manifest.dependencies {
            let (key, requirement) = manifest::dependency(value)?;
            if value["optional"] == true {
                continue;
            }
            let version = plugins
                .iter()
                .find(|p| p.manifest.id == key)
                .map(|p| p.manifest.version.as_str())
                .ok_or_else(|| {
                    ApiError::new("MISSING_DEPENDENCY", format!("缺少插件依赖 {key}。"))
                })?;
            if !semver::Version::parse(version).is_ok_and(|v| requirement.matches(&v)) {
                return Err(ApiError::new(
                    "DEPENDENCY_VERSION",
                    format!("依赖 {key} 的版本不兼容。"),
                ));
            }
            visit(plugins, settings, key, visiting, done, out)?;
        }
        visiting.remove(id);
        done.insert(id.into());
        out.push(plugin.clone());
        Ok(())
    }
    let mut out = Vec::new();
    visit(
        plugins,
        settings,
        id,
        &mut HashSet::new(),
        &mut HashSet::new(),
        &mut out,
    )?;
    Ok(out)
}
pub fn prepare(root: &Path, archive: &Path) -> Result<Candidate> {
    if !archive.is_absolute()
        || archive.ancestors().any(storage::linked)
        || !archive
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("hplugin"))
    {
        return Err(ApiError::new(
            "PLUGIN_PACKAGE",
            "请选择本地 .hplugin 文件。",
        ));
    }
    let input = File::open(archive)?;
    if input.metadata()?.len() > 256 * 1024 * 1024 {
        return Err(ApiError::new("PACKAGE_LIMIT", "插件包超过 256 MiB。"));
    }
    let mut zip = zip::ZipArchive::new(input)
        .map_err(|_| ApiError::new("PLUGIN_PACKAGE", "插件包不是有效的 ZIP 文件。"))?;
    if zip.len() > 8192 {
        return Err(ApiError::new("PACKAGE_LIMIT", "插件包条目过多。"));
    }
    let stage = storage::within(
        root,
        &format!("plugin-staging/{}", uuid::Uuid::new_v4().simple()),
    )?;
    fs::create_dir_all(&stage)?;
    let result = (|| -> Result<Candidate> {
        let mut names = HashSet::new();
        let mut total = 0u64;
        let mut count = 0;
        for index in 0..zip.len() {
            let mut entry = zip
                .by_index(index)
                .map_err(|_| ApiError::new("PLUGIN_PACKAGE", "插件包条目损坏。"))?;
            let name = entry.name().trim_end_matches('/').to_string();
            if !manifest::safe_relative(&name)
                || !names.insert(name.to_lowercase())
                || entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err(ApiError::new(
                    "INVALID_PATH",
                    "插件包包含链接、路径越界或重复条目。",
                ));
            }
            let path = storage::within(&stage, &name)?;
            if entry.is_dir() {
                fs::create_dir_all(path)?;
                continue;
            }
            total = total
                .checked_add(entry.size())
                .ok_or_else(|| ApiError::new("PACKAGE_LIMIT", "插件包超过大小限制。"))?;
            count += 1;
            if total > 256 * 1024 * 1024 || count > 4096 {
                return Err(ApiError::new(
                    "PACKAGE_LIMIT",
                    "插件包超过 256 MiB 或 4096 个文件。",
                ));
            }
            fs::create_dir_all(path.parent().unwrap())?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            let limit = entry.size();
            let written = std::io::copy(&mut entry.by_ref().take(limit + 1), &mut file)?;
            if written != limit {
                return Err(ApiError::new("PLUGIN_PACKAGE", "插件条目大小不匹配。"));
            }
            file.flush()?;
        }
        Ok(Candidate {
            plugin: manifest::load(&stage)?,
            data_root: root.into(),
        })
    })();
    if result.is_err() {
        let _ = remove_tree(&stage);
    }
    result
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Transaction {
    schema_version: u32,
    id: String,
    stage: String,
    backup: String,
    had_old: bool,
    committed: bool,
    digest: String,
}
fn receipts(root: &Path) -> Result<BTreeMap<String, String>> {
    let path = storage::within(root, "plugin-receipts.json")?;
    if path.exists() {
        storage::read_json(&path)
    } else {
        Ok(BTreeMap::new())
    }
}
pub fn recover(root: &Path) -> Result<()> {
    let path = storage::within(root, "plugin-transaction.json")?;
    if !path.exists() {
        return Ok(());
    }
    let tx: Transaction = storage::read_json(&path)?;
    if tx.schema_version != 1
        || !manifest::valid_id(&tx.id)
        || !tx.stage.starts_with("plugin-staging/")
        || !tx.backup.starts_with("plugin-backups/")
    {
        return Err(ApiError::new(
            "PLUGIN_RECOVERY",
            "插件恢复记录不兼容，原文件已保留。",
        ));
    }
    let destination = storage::within(root, &format!("plugins/{}", tx.id))?;
    let backup = storage::within(root, &tx.backup)?;
    let stage = storage::within(root, &tx.stage)?;
    if tx.committed {
        let plugin = manifest::load(&destination)?;
        if plugin.digest != tx.digest {
            return Err(ApiError::new(
                "PLUGIN_RECOVERY",
                "已提交插件的摘要不匹配，保留恢复文件。",
            ));
        }
        let mut receipts = receipts(root)?;
        receipts.insert(tx.id.clone(), tx.digest);
        storage::write_json(&storage::within(root, "plugin-receipts.json")?, &receipts)?;
        if backup.exists() {
            remove_tree(&backup)?;
        }
    } else if backup.exists() {
        if destination.exists() {
            remove_tree(&destination)?;
        }
        fs::rename(&backup, &destination)?;
    } else if !tx.had_old && destination.exists() && !stage.exists() {
        remove_tree(&destination)?;
    }
    if stage.exists() {
        remove_tree(&stage)?;
    }
    fs::remove_file(path)?;
    Ok(())
}
pub fn install(
    root: &Path,
    candidate: &Candidate,
    previous_digest: Option<&str>,
) -> Result<Plugin> {
    if candidate.data_root != root {
        return Err(ApiError::new(
            "PLUGIN_ROOT_CHANGED",
            "数据目录已切换，请重新选择插件包。",
        ));
    }
    recover(root)?;
    let checked = manifest::load(&candidate.plugin.root)?;
    if checked.digest != candidate.plugin.digest {
        return Err(ApiError::new(
            "PLUGIN_CHANGED",
            "插件预览后发生变化，请重新选择。",
        ));
    }
    let destination = storage::within(root, &format!("plugins/{}", checked.manifest.id))?;
    let old = current_digest(root, &checked.manifest.id)?;
    if old.as_deref() != previous_digest {
        return Err(ApiError::new(
            "PLUGIN_CHANGED",
            "当前安装版本已变化，请重新预览。",
        ));
    }
    let backup_name = format!("plugin-backups/{}", uuid::Uuid::new_v4().simple());
    let backup = storage::within(root, &backup_name)?;
    let stage = candidate
        .plugin
        .root
        .strip_prefix(root)
        .map_err(|_| ApiError::new("INVALID_PATH", "暂存目录无效。"))?
        .to_string_lossy()
        .replace('\\', "/");
    let mut tx = Transaction {
        schema_version: 1,
        id: checked.manifest.id.clone(),
        stage,
        backup: backup_name,
        had_old: old.is_some(),
        committed: false,
        digest: checked.digest.clone(),
    };
    let journal = storage::within(root, "plugin-transaction.json")?;
    storage::write_json(&journal, &tx)?;
    let result = (|| -> Result<Plugin> {
        fs::create_dir_all(backup.parent().unwrap())?;
        fs::create_dir_all(destination.parent().unwrap())?;
        if old.is_some() {
            fs::rename(&destination, &backup)?;
        }
        fs::rename(&checked.root, &destination)?;
        let installed = manifest::load(&destination)?;
        if installed.digest != checked.digest {
            return Err(ApiError::new(
                "PLUGIN_CHANGED",
                "安装后校验失败，正在恢复旧版。",
            ));
        }
        tx.committed = true;
        storage::write_json(&journal, &tx)?;
        Ok(installed)
    })();
    recover(root)?;
    result
}
/// Snapshot the actual destination, including a damaged package, for explicit replacement consent.
pub fn current_digest(root: &Path, id: &str) -> Result<Option<String>> {
    if !manifest::valid_id(id) {
        return Err(ApiError::new("INVALID_PLUGIN_ID", "插件 ID 无效。"));
    }
    let destination = storage::within(root, &format!("plugins/{id}"))?;
    if destination.exists() {
        Ok(Some(super::package::files(&destination)?.1))
    } else {
        Ok(None)
    }
}
pub fn uninstall(root: &Path, plugin: &Plugin) -> Result<()> {
    let destination = storage::within(root, &format!("plugins/{}", plugin.manifest.id))?;
    if destination != plugin.root {
        return Err(ApiError::new(
            "DEVELOPMENT_PLUGIN",
            "开发目录通过启动参数移除，不会删除外部源码。",
        ));
    }
    if manifest::load(&destination)?.digest != plugin.digest {
        return Err(ApiError::new(
            "PLUGIN_CHANGED",
            "插件文件已变化，请重新校验。",
        ));
    }
    let mut saved = receipts(root)?;
    saved.remove(&plugin.manifest.id);
    storage::write_json(&storage::within(root, "plugin-receipts.json")?, &saved)?;
    remove_tree(&destination)
}
pub fn statuses(plugins: &[Plugin], settings: &Settings) -> BTreeMap<String, String> {
    plugins
        .iter()
        .map(|p| {
            (
                p.manifest.id.clone(),
                order(plugins, settings, &p.manifest.id)
                    .err()
                    .map(|e| e.message)
                    .unwrap_or_default(),
            )
        })
        .collect()
}

// Include transitive consumers so none retain bindings to a replaced provider.
pub fn affected(plugins: &[Plugin], id: &str) -> Vec<String> {
    let mut ids = vec![id.to_owned()];
    loop {
        let before = ids.len();
        for plugin in plugins {
            if !ids.contains(&plugin.manifest.id)
                && plugin.manifest.dependencies.iter().any(|value| {
                    manifest::dependency(value).is_ok_and(|(key, _)| ids.iter().any(|id| id == key))
                })
            {
                ids.push(plugin.manifest.id.clone());
            }
        }
        if ids.len() == before {
            return ids;
        }
    }
}

pub fn reload_candidate(previous: &Plugin) -> Result<Plugin> {
    if previous.manifest.runtime != "javascript" && !previous.manifest.entry.starts_with("bin/") {
        return Err(ApiError::new(
            "NATIVE_PLUGIN",
            "原生插件的实现随宿主编译，更新后需要重启。",
        ));
    }
    let checked = manifest::load(&previous.root)?;
    if checked.manifest.id != previous.manifest.id {
        return Err(ApiError::new(
            "PLUGIN_CHANGED",
            "重载不能更改插件 ID，请重新安装。",
        ));
    }
    if checked.manifest.runtime != previous.manifest.runtime {
        return Err(ApiError::new(
            "PLUGIN_RUNTIME_CHANGED",
            "切换运行时需要重新安装插件包。",
        ));
    }
    if checked
        .manifest
        .permissions
        .iter()
        .any(|permission| !previous.manifest.permissions.contains(permission))
    {
        return Err(ApiError::new(
            "PLUGIN_PERMISSION_CHANGED",
            "插件请求了新权限，请通过安装插件包重新确认。",
        ));
    }
    Ok(checked)
}

pub fn approve_reload(root: &Path, previous: &Plugin, checked: &Plugin) -> Result<()> {
    let destination = storage::within(root, &format!("plugins/{}", previous.manifest.id))?;
    if previous.root != destination
        || checked.root != destination
        || reload_candidate(previous)?.digest != checked.digest
    {
        return Err(ApiError::new(
            "PLUGIN_CHANGED",
            "重载期间插件文件已变化，请重试。",
        ));
    }
    let mut saved = receipts(root)?;
    if saved.get(&previous.manifest.id) != Some(&previous.digest) {
        return Err(ApiError::new(
            "PLUGIN_CHANGED",
            "安装记录已变化，请重新安装插件包。",
        ));
    }
    saved.insert(checked.manifest.id.clone(), checked.digest.clone());
    storage::write_json(&storage::within(root, "plugin-receipts.json")?, &saved)
}
