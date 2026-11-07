use super::{
    migration_model::{self as storage, Components, Instance, Overrides, Settings},
    records::Store,
};
use hydrogen_plugin_services::{
    contracts::{ApiError, InstanceInfo, Result},
    jobs::Job,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Entry {
    source: PathBuf,
    destination: String,
    length: u64,
    modified: u128,
}
#[derive(Clone, Serialize, Deserialize)]
struct Item {
    old_id: String,
    instance: Instance,
    files: Vec<Entry>,
    record: Entry,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Plan {
    root: PathBuf,
    source: PathBuf,
    game: PathBuf,
    items: Vec<Item>,
    shared: Vec<Entry>,
    settings: Option<Settings>,
    selected: Option<String>,
    digest: String,
    catalog: Option<Value>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub instances: usize,
    pub bytes: u64,
    pub settings: bool,
    pub source: String,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    schema_version: u32,
    source: String,
    mapping: BTreeMap<String, String>,
    complete: Vec<String>,
    settings_done: bool,
}
fn error(message: &str) -> ApiError {
    ApiError::new("LEGACY_IMPORT", message)
}
fn fingerprint(source: &Path, destination: String) -> Result<Entry> {
    if source.ancestors().any(storage::linked) {
        return Err(error("旧数据包含链接，拒绝复制。"));
    }
    let meta = fs::metadata(source)?;
    if !meta.is_file() {
        return Err(error("旧数据包含非普通文件。"));
    }
    Ok(Entry {
        source: source.into(),
        destination,
        length: meta.len(),
        modified: meta
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    })
}
fn walk(source: &Path, destination: &str, out: &mut Vec<Entry>, depth: usize) -> Result<()> {
    if depth > 64 || out.len() >= 500_000 {
        return Err(error("旧数据的目录层数或文件数量超过导入限制。"));
    }
    if source.ancestors().any(storage::linked) {
        return Err(error("旧数据包含链接，拒绝复制。"));
    }
    if !source.exists() {
        return Ok(());
    }
    for item in fs::read_dir(source)? {
        let item = item?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| error("旧数据文件名不是有效 Unicode。"))?;
        let relative = if destination.is_empty() {
            name
        } else {
            format!("{destination}/{name}")
        };
        if !storage::safe_relative(&relative) || storage::linked(&item.path()) {
            return Err(error("旧数据包含不安全的路径或链接。"));
        }
        if item.file_type()?.is_dir() {
            walk(&item.path(), &relative, out, depth + 1)?;
        } else {
            out.push(fingerprint(&item.path(), relative)?);
        }
    }
    Ok(())
}
fn legacy_settings(
    client: &hydrogen_plugin_services::net::Client,
    source: &Path,
) -> Result<Option<Settings>> {
    let path = storage::within(source, "settings.json")?;
    if !path.exists() {
        return Ok(None);
    }
    let value: Value = storage::read_json(&path)?;
    if value["schemaVersion"] != 1 {
        return Err(error("旧设置版本不兼容，未执行导入。"));
    }
    let old = &value["settings"];
    let mut settings = Settings {
        theme: Default::default(),
        java: old["java"]["executablePath"].as_str().map(String::from),
        max_memory_mib: old["maxMemoryMiB"]
            .as_u64()
            .and_then(|n| n.try_into().ok())
            .ok_or_else(|| error("旧内存设置无效。"))?,
        window_mode: match old["window"]["mode"].as_u64() {
            Some(0) => "default",
            Some(1) => "custom",
            Some(2) => "fullscreen",
            _ => return Err(error("旧窗口设置无效。")),
        }
        .into(),
        window_width: old["window"]["width"]
            .as_u64()
            .and_then(|n| n.try_into().ok())
            .ok_or_else(|| error("旧窗口宽度无效。"))?,
        window_height: old["window"]["height"]
            .as_u64()
            .and_then(|n| n.try_into().ok())
            .ok_or_else(|| error("旧窗口高度无效。"))?,
        after_launch: if old["afterLaunch"] == 0 {
            "minimize"
        } else if old["afterLaunch"] == 1 {
            "keep"
        } else {
            return Err(error("旧启动行为设置无效。"));
        }
        .into(),
        download_source: if old["downloadSource"].is_null() || old["downloadSource"] == 0 {
            "official"
        } else if old["downloadSource"] == 1 {
            "bmclapi"
        } else {
            return Err(error("旧下载源设置无效。"));
        }
        .into(),
        jvm_arguments: old["jvmArguments"].as_str().unwrap_or("").into(),
        ..Settings::defaults()?
    };
    // Java choices are machine-specific. Missing executables return to automatic discovery.
    if settings
        .java
        .as_ref()
        .is_some_and(|p| !Path::new(p).is_file())
    {
        settings.java = None;
    }
    settings.validate(client)?;
    Ok(Some(settings))
}
pub fn preview(
    client: &hydrogen_plugin_services::net::Client,
    root: &Path,
    chosen: &Path,
) -> Result<(Plan, Preview)> {
    if !chosen.is_absolute() || chosen.ancestors().any(storage::linked) {
        return Err(error("请选择不包含链接的旧启动器目录。"));
    }
    let canonical = dunce::canonicalize(chosen)?;
    let chosen = canonical.as_path();
    let source = if chosen.join("LauncherData/instances").is_dir() {
        chosen.join("LauncherData")
    } else {
        chosen.into()
    };
    if source == root || source.starts_with(root) || root.starts_with(&source) {
        return Err(error("导入来源和目标不能相同或互相嵌套。"));
    }
    let game = if source
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("LauncherData"))
        && source
            .parent()
            .is_some_and(|p| p.join(".minecraft").is_dir())
    {
        source.parent().unwrap().join(".minecraft")
    } else {
        source.join(".minecraft")
    };
    if game.starts_with(root) || root.starts_with(&game) {
        return Err(error("旧游戏目录与目标目录不能互相嵌套。"));
    }
    let instances = storage::within(&source, "instances")?;
    if !instances.is_dir() {
        return Err(error(
            "此目录没有旧版 instances 数据，请选择旧 LauncherData 或启动器所在目录。",
        ));
    }
    let mut items = Vec::new();
    for item in fs::read_dir(instances)? {
        let item = item?;
        let id = item.file_name().to_string_lossy().to_string();
        if !storage::id_valid(&id) || !item.file_type()?.is_dir() {
            continue;
        }
        let record = storage::within(&source, &format!("instances/{id}/instance.json"))?;
        let value: Value = storage::read_json(&record)?;
        if value["schemaVersion"] != 1
            || !value["instance"].is_object()
            || value["layoutVersion"].as_u64().unwrap_or(1) > 2
        {
            return Err(error("旧实例版本不兼容，原数据未修改。"));
        }
        let old = &value["instance"];
        if old["id"] != id {
            return Err(error("旧实例标识与目录不一致。"));
        }
        let version = old["gameVersion"]
            .as_str()
            .ok_or_else(|| error("旧实例缺少游戏版本。"))?;
        if version.is_empty()
            || version.len() > 100
            || !version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(error("旧实例游戏版本无效。"));
        }
        let folder = value["gameDirectoryName"].as_str().unwrap_or(version);
        if folder != id && folder != version {
            return Err(error("旧实例游戏目录无效。"));
        }
        let private = storage::within(&game, &format!("versions/{folder}"))?;
        let mut files = Vec::new();
        walk(&private, "game", &mut files, 0)?;
        let selection = &old["preInstall"];
        let components = if selection.is_null() {
            None
        } else {
            if selection["providerId"] != "minecraft.fabric" {
                return Err(error(
                    "旧实例使用了尚不支持的预安装提供者，拒绝降级为原版。",
                ));
            }
            let option = selection["optionId"].as_str().unwrap_or("");
            let loader = if ["loader", "loader-api"].contains(&option) {
                let manifest: Value =
                    storage::read_json(&storage::within(&private, ".hydrogen/fabric.json")?)?;
                let profile_id = manifest["profile"]["id"].as_str().unwrap_or("");
                profile_id
                    .strip_prefix("fabric-loader-")
                    .and_then(|s| s.strip_suffix(&format!("-{version}")))
                    .ok_or_else(|| error("无法恢复旧 Fabric 的固定版本。"))?
                    .to_string()
            } else {
                option.into()
            };
            if loader.is_empty()
                || !loader
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-+".contains(&b))
            {
                return Err(error("旧 Fabric 版本无效。"));
            }
            Some(Components {
                kind: String::new(),
                loader,
                fabric_api: selection["addonOptionId"].as_str().map(String::from),
            })
        };
        let overrides = Overrides {
            java: value["overrides"]["java"]["executablePath"]
                .as_str()
                .filter(|p| Path::new(p).is_file())
                .map(String::from),
            max_memory_mib: value["overrides"]["maxMemoryMiB"]
                .as_u64()
                .and_then(|n| n.try_into().ok()),
            jvm_arguments: value["overrides"]["jvmArguments"]
                .as_str()
                .map(String::from),
            gc_mode: None,
        };
        Settings::defaults()?.effective(client, &overrides)?;
        items.push(Item {
            old_id: id.clone(),
            instance: Instance {
                schema_version: 1,
                info: InstanceInfo {
                    id,
                    name: storage::validate_name(old["name"].as_str().unwrap_or(version))?,
                    game_version: version.into(),
                    is_installed: old["isInstalled"].as_bool().unwrap_or(false) && private.is_dir(),
                },
                overrides,
                components,
                directory_name: None,
                repository_root: None,
                loader_name: String::new(),
                isolated: false,
            },
            files,
            record: fingerprint(&record, String::new())?,
        });
    }
    if items.is_empty() {
        return Err(error("未找到可导入的旧版实例。"));
    }
    items.sort_by(|a, b| a.old_id.cmp(&b.old_id));
    let mut shared = Vec::new();
    for directory in ["assets", "libraries"] {
        walk(
            &storage::within(&game, directory)?,
            &format!(".minecraft/{directory}"),
            &mut shared,
            0,
        )?;
    }
    for item in &items {
        let version = &item.instance.info.game_version;
        for suffix in ["json", "jar"] {
            let name = format!("versions/{version}/{version}.{suffix}");
            let file = storage::within(&game, &name)?;
            if file.is_file() && !shared.iter().any(|e| e.source == file) {
                shared.push(fingerprint(&file, format!(".minecraft/{name}"))?);
            }
        }
    }
    let selected_path = storage::within(&source, "instances/index.json")?;
    let selected = if selected_path.is_file() {
        let v: Value = storage::read_json(&selected_path)?;
        v["selectedInstanceId"].as_str().map(String::from)
    } else {
        None
    };
    let settings = legacy_settings(client, &source)?;
    let catalog_path = storage::within(&source, "cache/manifests/catalog.json")?;
    let catalog = if catalog_path.is_file() {
        let value: Value = storage::read_json(&catalog_path)?;
        if value["schemaVersion"] != 1 || !value["manifest"]["versions"].is_array() {
            return Err(error("旧版本目录缓存不兼容。"));
        }
        Some(value["manifest"].clone())
    } else {
        None
    };
    if let Some(catalog) = &catalog {
        for item in &items {
            let version = &item.instance.info.game_version;
            let destination = format!(".minecraft/versions/{version}/{version}.json");
            if !shared.iter().any(|e| e.destination == destination)
                && let Some(hash) = catalog["versions"]
                    .as_array()
                    .and_then(|a| a.iter().find(|v| v["id"] == *version))
                    .and_then(|v| v["sha1"].as_str())
                    .filter(|h| hydrogen_plugin_services::net::valid_hash(h))
            {
                let path = storage::within(&source, &format!("cache/manifests/{hash}.json"))?;
                if path.is_file() {
                    shared.push(fingerprint(&path, destination)?);
                }
            }
        }
    }
    for item in &mut items {
        item.files.sort_by(|a, b| a.destination.cmp(&b.destination));
    }
    shared.sort_by(|a, b| a.destination.cmp(&b.destination));
    let mut plan = Plan {
        root: root.into(),
        source: source.clone(),
        game,
        items,
        shared,
        settings,
        selected,
        digest: String::new(),
        catalog,
    };
    let mut hash = Sha1::new();
    hash.update(serde_json::to_vec(&plan)?);
    plan.digest = format!("{:x}", hash.finalize());
    let bytes = plan
        .shared
        .iter()
        .chain(plan.items.iter().flat_map(|i| &i.files))
        .try_fold(0u64, |a, e| {
            a.checked_add(e.length)
                .ok_or_else(|| error("导入大小超过限制。"))
        })?;
    if bytes > 1024 * 1024 * 1024 * 1024 {
        return Err(error("单次导入超过 1 TiB。"));
    }
    let view = Preview {
        instances: plan.items.len(),
        bytes,
        settings: plan.settings.is_some() && !root.join("settings.json").exists(),
        source: source.to_string_lossy().into(),
    };
    Ok((plan, view))
}
fn check_source(plan: &Plan) -> Result<Vec<File>> {
    let mut locks = Vec::new();
    for item in &plan.items {
        if fingerprint(&item.record.source, String::new())? != item.record {
            return Err(error("旧实例配置已更改，请重新预览。"));
        }
        let path = storage::within(
            &plan.source,
            &format!("instances/{}/launch.lock", item.old_id),
        )?;
        if path.exists() {
            let file = File::open(&path)?;
            file.try_lock()
                .map_err(|_| error("旧版实例仍在使用，请先退出旧启动器和游戏。"))?;
            if file.metadata()?.len() > 0 {
                let marker: Value = storage::read_json(&path)?;
                if let (Some(pid), Some(ticks)) = (
                    marker["ProcessId"]
                        .as_u64()
                        .or_else(|| marker["processId"].as_u64()),
                    marker["StartTimeUtcTicks"]
                        .as_u64()
                        .or_else(|| marker["startTimeUtcTicks"].as_u64()),
                ) && hydrogen_plugin_services::invoke(
                    "process",
                    "process",
                    "stamp",
                    serde_json::json!({"pid":pid}),
                )?
                .as_u64()
                .is_some_and(|s| s.checked_add(504_911_232_000_000_000) == Some(ticks))
                {
                    return Err(error("旧实例的游戏仍在运行。"));
                }
            }
            locks.push(file);
        }
    }
    Ok(locks)
}
async fn hash(path: &Path, job: &Job) -> Result<String> {
    let mut input = File::open(path)?;
    let mut digest = Sha1::new();
    let mut buffer = vec![0; 256 * 1024];
    loop {
        tokio::task::yield_now().await;
        job.check()?;
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
async fn copy(
    entry: &Entry,
    root: &Path,
    job: &Job,
    completed: &mut u64,
    total: u64,
) -> Result<()> {
    job.check()?;
    if fingerprint(&entry.source, entry.destination.clone())? != *entry {
        return Err(error("旧文件已更改，请重新预览后导入。"));
    }
    let destination = storage::within(root, &entry.destination)?;
    if destination.exists() {
        if destination.metadata()?.len() == entry.length
            && hash(&destination, job).await? == hash(&entry.source, job).await?
        {
            *completed += entry.length;
            return Ok(());
        }
        return Err(error("目标中已有不同内容的文件，导入不会覆盖它。"));
    }
    fs::create_dir_all(destination.parent().unwrap())?;
    let part = destination.with_file_name(format!(".{}.part", uuid::Uuid::new_v4().simple()));
    struct Partial(std::path::PathBuf);
    impl Drop for Partial {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let _partial = Partial(part.clone());
    let result: Result<()> = async {
        let mut source = File::open(&entry.source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)?;
        let mut digest = Sha1::new();
        let mut buffer = vec![0; 256 * 1024];
        let mut copied = 0;
        loop {
            tokio::task::yield_now().await;
            job.check()?;
            let n = source.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            output.write_all(&buffer[..n])?;
            digest.update(&buffer[..n]);
            copied += n as u64;
            job.progress(*completed + copied, Some(total), "正在复制并校验旧数据");
        }
        output.sync_all()?;
        drop(output);
        if copied != entry.length
            || fingerprint(&entry.source, entry.destination.clone())? != *entry
            || format!("{:x}", digest.finalize()) != hash(&entry.source, job).await?
        {
            return Err(error("旧文件在复制期间发生变化，已保留原文件。"));
        }
        storage::move_new(&part, &destination)?;
        *completed += copied;
        Ok(())
    }
    .await;
    let _ = fs::remove_file(part);
    result
}
pub async fn execute(store: &mut Store, plan: Plan, job: &Job) -> Result<()> {
    let _locks = check_source(&plan)?;
    let game_root = store.game_root.clone();
    let journal_path = storage::within(&plan.root, &format!("imports/{}.json", plan.digest))?;
    let mut journal: Journal = if journal_path.exists() {
        storage::read_json(&journal_path)?
    } else {
        Journal {
            schema_version: 1,
            source: plan.source.to_string_lossy().into(),
            mapping: plan
                .items
                .iter()
                .map(|i| (i.old_id.clone(), uuid::Uuid::new_v4().simple().to_string()))
                .collect(),
            complete: Vec::new(),
            settings_done: false,
        }
    };
    if journal.schema_version != 1 || journal.source != plan.source.to_string_lossy() {
        return Err(error("导入恢复记录不兼容。"));
    }
    storage::write_json(&journal_path, &journal)?;
    let total = plan
        .shared
        .iter()
        .chain(plan.items.iter().flat_map(|i| &i.files))
        .map(|e| e.length)
        .sum();
    let mut completed = 0;
    job.phase("downloading", "正在复制并校验旧数据");
    for entry in &plan.shared {
        let mut entry = entry.clone();
        entry.destination = entry
            .destination
            .strip_prefix(".minecraft/")
            .ok_or_else(|| error("游戏目标路径无效。"))?
            .into();
        copy(&entry, &game_root, job, &mut completed, total).await?;
    }
    if let Some(catalog) = &plan.catalog {
        let path = storage::within(&plan.root, "cache/version-manifest.json")?;
        if !path.exists() {
            storage::write_json(&path, catalog)?;
        }
    }
    for item in &plan.items {
        tokio::task::yield_now().await;
        job.check()?;
        let id = journal
            .mapping
            .get(&item.old_id)
            .filter(|s| storage::id_valid(s))
            .ok_or_else(|| error("导入映射损坏。"))?;
        if journal.complete.contains(&item.old_id) {
            continue;
        }
        let stage = storage::within(&plan.root, &format!("imports/{}-{}", plan.digest, id))?;
        let destination = storage::within(&plan.root, &format!("instances/{id}"))?;
        if !destination.exists() {
            fs::create_dir_all(&stage)?;
            let game = storage::within(&game_root, &format!("versions/{id}"))?;
            for entry in &item.files {
                let mut entry = entry.clone();
                entry.destination = entry
                    .destination
                    .strip_prefix("game/")
                    .ok_or_else(|| error("实例目标路径无效。"))?
                    .into();
                copy(&entry, &game, job, &mut completed, total).await?;
            }
            let mut instance = item.instance.clone();
            instance.info.id = id.clone();
            instance.directory_name = Some(id.clone());
            instance.repository_root = Some(game_root.to_string_lossy().into());
            store.writable()?;
            let existing = store.instances()?;
            let original = instance.info.name.clone();
            let mut suffix = 1;
            while existing
                .iter()
                .any(|i| i.info.name.eq_ignore_ascii_case(&instance.info.name))
            {
                suffix += 1;
                instance.info.name = format!(
                    "{} (旧版 {suffix})",
                    original.chars().take(50).collect::<String>()
                );
            }
            storage::write_json(&stage.join("instance.json"), &instance)?;
            tokio::task::yield_now().await;
            job.check()?;
            job.phase("committing", "正在登记导入实例");
            fs::create_dir_all(destination.parent().unwrap())?;
            fs::rename(&stage, &destination)?;
        } else {
            let imported: Instance = storage::read_json(&destination.join("instance.json"))?;
            if imported.info.id != *id
                || imported.info.game_version != item.instance.info.game_version
            {
                return Err(error("恢复目标已被其他实例占用。"));
            }
        }
        journal.complete.push(item.old_id.clone());
        storage::write_json(&journal_path, &journal)?;
        job.phase("downloading", "继续复制旧数据");
    }
    job.phase("committing", "正在保存导入结果");
    store.writable()?;
    if !journal.settings_done && !store.root.join("settings.json").exists() {
        let settings = plan
            .settings
            .map(serde_json::to_value)
            .transpose()?
            .unwrap_or_else(|| store.settings.clone());
        let old = store.selected.clone();
        store.selected = plan
            .selected
            .and_then(|id| journal.mapping.get(&id).cloned())
            .filter(|id| store.instance(id).is_ok_and(|i| i.info.is_installed));
        if let Err(e) = store.save(settings) {
            store.selected = old;
            return Err(e);
        }
    }
    journal.settings_done = true;
    storage::write_json(&journal_path, &journal)?;
    Ok(())
}
