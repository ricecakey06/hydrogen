use crate::{net, storage};
use base64::{Engine, engine::general_purpose::STANDARD};
use hydrogen_plugin_sdk::contracts::{ApiError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModInfo {
    pub id: String,
    pub english_name: String,
    pub chinese_name: Option<String>,
    pub version: String,
    pub summary: String,
    pub kind: String,
    mod_ids: Vec<String>,
    pub file_name: Option<String>,
    pub sha1: Option<String>,
    pub enabled: bool,
    #[serde(default)]
    pub duplicate: bool,
    pub icon: Option<String>,
    pub problem: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModRef {
    pub file_name: String,
    pub sha1: String,
}
fn failure(message: &str) -> ApiError {
    ApiError::new("MOD_OPERATION", message)
}
pub fn filename(name: &str) -> Result<&str> {
    let lower = name.to_ascii_lowercase();
    if name.len() > 240
        || name.contains(['/', '\\', ':'])
        || name.chars().any(char::is_control)
        || !storage::safe_relative(name)
        || !(lower.ends_with(".jar") || lower.ends_with(".jar.disabled"))
    {
        return Err(failure("模组文件名无效。"));
    }
    Ok(name)
}
fn archive_bytes(
    archive: &mut zip::ZipArchive<fs::File>,
    name: &str,
    limit: u64,
) -> Option<Vec<u8>> {
    let entry = archive.by_name(name).ok()?;
    if entry.size() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    entry.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}
pub fn read_archive(path: &Path, sha1: &str) -> ModInfo {
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let enabled = !filename.to_ascii_lowercase().ends_with(".disabled");
    let fallback = filename
        .trim_end_matches(".disabled")
        .trim_end_matches(".jar");
    let mut info = ModInfo {
        id: fallback.into(),
        english_name: fallback.into(),
        chinese_name: None,
        version: "未知".into(),
        summary: "未提供简介".into(),
        kind: "未知".into(),
        mod_ids: Vec::new(),
        file_name: Some(filename),
        sha1: Some(sha1.into()),
        enabled,
        duplicate: false,
        icon: None,
        problem: None,
    };
    let result = (|| -> Result<()> {
        let mut archive =
            zip::ZipArchive::new(fs::File::open(path)?).map_err(|_| failure("模组压缩包损坏。"))?;
        let mut icon = None;
        let mut apply = |value: &Value, kind: &str| {
            for (key, target) in [
                ("id", &mut info.id),
                ("name", &mut info.english_name),
                ("version", &mut info.version),
                ("description", &mut info.summary),
            ] {
                if let Some(value) = value[key].as_str() {
                    *target = value.chars().take(4096).collect();
                }
            }
            info.kind = kind.into();
        };
        if let Some(bytes) = archive_bytes(&mut archive, "fabric.mod.json", 1024 * 1024) {
            let value: Value = serde_json::from_slice(&bytes)?;
            apply(&value, "Fabric");
            info.mod_ids.extend(value["id"].as_str().map(str::to_owned));
            info.kind = format!(
                "Fabric · {}",
                match value["environment"].as_str() {
                    Some("client") => "客户端",
                    Some("server") => "服务端",
                    _ => "通用",
                }
            );
            icon = value["icon"].as_str().map(String::from).or_else(|| {
                value["icon"]
                    .as_object()?
                    .iter()
                    .max_by_key(|(k, _)| k.parse::<u32>().unwrap_or(0))?
                    .1
                    .as_str()
                    .map(String::from)
            });
        } else if let Some(bytes) = archive_bytes(&mut archive, "quilt.mod.json", 1024 * 1024) {
            let value: Value = serde_json::from_slice(&bytes)?;
            let loader = &value["quilt_loader"];
            apply(&loader["metadata"], "Quilt");
            if let Some(id) = loader["id"].as_str() {
                info.id = id.into();
                info.mod_ids.push(id.into());
            }
            if let Some(version) = loader["version"].as_str() {
                info.version = version.into();
            }
            icon = loader["metadata"]["icon"].as_str().map(String::from);
        } else if let Some((bytes, kind)) =
            archive_bytes(&mut archive, "META-INF/neoforge.mods.toml", 1024 * 1024)
                .map(|v| (v, "NeoForge"))
                .or_else(|| {
                    archive_bytes(&mut archive, "META-INF/mods.toml", 1024 * 1024)
                        .map(|v| (v, "Forge"))
                })
        {
            let value: toml::Value = toml::from_str(&String::from_utf8_lossy(&bytes))
                .map_err(|_| failure("模组 TOML 元数据无法读取。"))?;
            let mods = value
                .get("mods")
                .and_then(toml::Value::as_array)
                .filter(|mods| !mods.is_empty())
                .ok_or_else(|| failure("模组缺少 mods 字段。"))?;
            let item = &mods[0];
            info.mod_ids.extend(mods.iter().filter_map(|item| {
                item.get("modId")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
            }));
            info.kind = kind.into();
            for (key, target) in [
                ("modId", &mut info.id),
                ("displayName", &mut info.english_name),
                ("version", &mut info.version),
                ("description", &mut info.summary),
            ] {
                if let Some(value) = item.get(key).and_then(toml::Value::as_str) {
                    *target = value.chars().take(4096).collect();
                }
            }
            icon = item
                .get("logoFile")
                .or_else(|| value.get("logoFile"))
                .and_then(toml::Value::as_str)
                .map(String::from);
            if info.version.contains("${")
                && let Some(manifest) =
                    archive_bytes(&mut archive, "META-INF/MANIFEST.MF", 1024 * 1024)
            {
                info.version = String::from_utf8_lossy(&manifest)
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("Implementation-Version:")
                            .map(|s| s.trim().to_string())
                    })
                    .unwrap_or_else(|| "未知".into());
            }
        } else if let Some(bytes) = archive_bytes(&mut archive, "mcmod.info", 1024 * 1024) {
            let value: Value = serde_json::from_slice(&bytes)?;
            let mods = value
                .as_array()
                .or_else(|| value["modList"].as_array())
                .filter(|mods| !mods.is_empty())
                .ok_or_else(|| failure("旧版模组元数据无效。"))?;
            let item = &mods[0];
            apply(item, "Forge");
            info.mod_ids.extend(
                mods.iter()
                    .filter_map(|item| item["modid"].as_str().map(str::to_owned)),
            );
            if let Some(id) = item["modid"].as_str() {
                info.id = id.into();
            }
            icon = item["logoFile"].as_str().map(String::from);
        } else {
            info.problem = Some("未找到可识别的模组元数据。".into());
        }
        if let Some(name) = icon
            && let Some(bytes) =
                archive_bytes(&mut archive, name.trim_start_matches('/'), 2 * 1024 * 1024)
            && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        {
            info.icon = Some(format!("data:image/png;base64,{}", STANDARD.encode(bytes)));
        }
        if info
            .english_name
            .chars()
            .any(|c| ('\u{3400}'..='\u{9fff}').contains(&c))
        {
            info.chinese_name = Some(info.english_name.clone());
        }
        Ok(())
    })();
    if let Err(error) = result {
        info.problem = Some(error.message);
    }
    info.mod_ids.retain(|id| !id.is_empty() && id.len() <= 128);
    info.mod_ids.sort_unstable();
    info.mod_ids.dedup();
    info
}

fn mark_duplicates(items: &mut [ModInfo]) {
    let mut ids = BTreeMap::new();
    let mut hashes = BTreeMap::new();
    for item in items.iter() {
        for id in &item.mod_ids {
            *ids.entry(id.clone()).or_insert(0) += 1;
        }
        if let Some(hash) = &item.sha1 {
            *hashes.entry(hash.clone()).or_insert(0) += 1;
        }
    }
    for item in items {
        item.duplicate = item.mod_ids.iter().any(|id| ids[id] > 1)
            || item.sha1.as_ref().is_some_and(|hash| hashes[hash] > 1);
    }
}
pub async fn inventory(root: &Path, game: &Path) -> Result<Vec<ModInfo>> {
    let directory = storage::within(game, "mods")?;
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    let mut icon_budget = 4 * 1024 * 1024;
    let index_path = storage::within(
        root,
        &format!("cache/mod-files/{:x}.json", {
            use sha1::{Digest, Sha1};
            Sha1::digest(game.to_string_lossy().as_bytes())
        }),
    )?;
    let previous: Value = storage::read_json(&index_path).unwrap_or_default();
    let mut index = json!({"schemaVersion":1,"files":{}});
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if filename(&name).is_err() {
            continue;
        }
        let path = storage::within(game, &format!("mods/{name}"))?;
        if !path.is_file() {
            continue;
        }
        if result.len() >= 10000 {
            return Err(failure("实例超过 10000 个模组文件，请先整理目录。"));
        }
        let metadata = path.metadata()?;
        let length = metadata.len();
        let modified = metadata
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .min(u64::MAX as u128) as u64;
        let cached = &previous["files"][&name];
        let hash = if previous["schemaVersion"] == 1
            && cached["length"].as_u64() == Some(length)
            && cached["modified"].as_u64() == Some(modified)
            && cached["hash"].as_str().is_some_and(net::valid_hash)
        {
            cached["hash"].as_str().unwrap().to_string()
        } else {
            net::hash(&path).await?
        };
        index["files"][&name] = json!({"length":length,"modified":modified,"hash":hash});
        let cache = storage::within(root, &format!("cache/mods/{hash}.json"))?;
        let cached = storage::read_json::<ModInfo>(&cache)
            .ok()
            .filter(|i| i.sha1.as_deref() == Some(&hash));
        let mut item = cached.unwrap_or_else(|| {
            let item = read_archive(&path, &hash);
            let _ = storage::write_json(&cache, &item);
            item
        });
        item.file_name = Some(name.clone());
        item.enabled = !name.to_ascii_lowercase().ends_with(".disabled");
        if let Some(icon) = &item.icon {
            if icon.len() > icon_budget {
                item.icon = None;
            } else {
                icon_budget -= icon.len();
            }
        }
        result.push(item);
    }
    mark_duplicates(&mut result);
    result.sort_by_key(|m| m.english_name.to_lowercase());
    if index != previous {
        let _ = storage::write_json(&index_path, &index);
    }
    Ok(result)
}
fn names(root: &Path) -> BTreeMap<String, String> {
    let Ok(path) = storage::within(root, "cache/mod-names.csv") else {
        return BTreeMap::new();
    };
    if !path.metadata().is_ok_and(|m| m.len() <= 8 * 1024 * 1024) {
        return BTreeMap::new();
    }
    let csv = fs::read_to_string(path).unwrap_or_default();
    let mut result = BTreeMap::new();
    for line in csv.lines().filter(|s| !s.starts_with('#')) {
        let fields: Vec<_> = line.trim_end_matches('\r').split(',').collect();
        if fields.len() < 7 || fields[3].trim().is_empty() {
            continue;
        }
        for key in [fields[1], fields[2], fields[4]]
            .into_iter()
            .chain(fields[6].split(';'))
        {
            if !key.trim().is_empty() {
                result
                    .entry(key.to_lowercase())
                    .or_insert_with(|| fields[3].into());
            }
        }
    }
    result
}
pub fn enrich(root: &Path, mods: &mut [ModInfo]) -> Result<()> {
    let dictionary = names(root);
    let path = storage::within(root, "mod-aliases.json")?;
    let aliases = if path.exists() {
        storage::read_json::<Value>(&path)?
    } else {
        Value::Null
    };
    for item in mods {
        item.chinese_name = aliases["names"][&item.id]
            .as_str()
            .map(String::from)
            .or_else(|| {
                dictionary
                    .get(&item.id.to_lowercase())
                    .or_else(|| dictionary.get(&item.english_name.to_lowercase()))
                    .cloned()
            });
    }
    Ok(())
}

pub async fn checked(game: &Path, reference: &ModRef) -> Result<PathBuf> {
    filename(&reference.file_name)?;
    if !net::valid_hash(&reference.sha1) {
        return Err(failure("文件校验标识无效，请刷新列表。"));
    }
    let path = storage::within(game, &format!("mods/{}", reference.file_name))?;
    if !path.is_file()
        || !net::hash(&path)
            .await?
            .eq_ignore_ascii_case(&reference.sha1)
    {
        return Err(failure("模组文件已变化，请刷新列表后重试。"));
    }
    Ok(path)
}
pub async fn set_enabled(game: &Path, files: &[ModRef], enabled: bool) -> Result<()> {
    if files.is_empty() || files.len() > 10000 {
        return Err(failure("请选择有效的模组。"));
    }
    let mut plan = Vec::new();
    let mut seen = BTreeSet::new();
    for reference in files {
        let source = checked(game, reference).await?;
        if !seen.insert(source.clone()) {
            return Err(failure("选择中包含重复文件。"));
        }
        let current = !reference
            .file_name
            .to_ascii_lowercase()
            .ends_with(".disabled");
        if current == enabled {
            continue;
        }
        let name = if enabled {
            reference.file_name[..reference.file_name.len() - 9].to_owned()
        } else {
            format!("{}.disabled", reference.file_name)
        };
        let target = storage::within(game, &format!("mods/{name}"))?;
        if target.exists() {
            return Err(failure("目标文件已存在，批量操作没有执行。"));
        }
        plan.push((source, target));
    }
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (source, target) in plan {
        if let Err(error) = storage::move_new(&source, &target) {
            for (source, target) in done.iter().rev() {
                let _ = storage::move_new(target, source);
            }
            return Err(error);
        }
        done.push((source, target));
    }
    Ok(())
}
pub async fn remove(game: &Path, files: &[ModRef]) -> Result<()> {
    if files.is_empty() || files.len() > 10000 {
        return Err(failure("请选择有效的模组。"));
    }
    let stage = storage::within(
        game,
        &format!(".hydrogen/removed/{}", uuid::Uuid::new_v4().simple()),
    )?;
    fs::create_dir_all(&stage)?;
    // Preserve moved user files if a rollback cannot complete.
    let mut paths = Vec::new();
    let mut seen = BTreeSet::new();
    for reference in files {
        let path = checked(game, reference).await?;
        if !seen.insert(path.clone()) {
            return Err(failure("选择中包含重复文件。"));
        }
        paths.push((path, storage::within(&stage, &reference.file_name)?));
    }
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (source, target) in paths {
        if let Err(error) = storage::move_new(&source, &target) {
            for (source, target) in done.iter().rev() {
                let _ = storage::move_new(target, source);
            }
            return Err(error);
        }
        done.push((source, target));
    }
    for (_, path) in done {
        fs::remove_file(path)?;
    }
    let _ = fs::remove_dir(stage);
    Ok(())
}

pub async fn name_mod(root: &Path, game: &Path, file: &ModRef, name: &str) -> Result<()> {
    if name.chars().count() > 100 || name.chars().any(char::is_control) {
        return Err(failure("中文名称过长或包含控制字符。"));
    }
    let path = checked(game, file).await?;
    let info = read_archive(&path, &file.sha1);
    let path = storage::within(root, "mod-aliases.json")?;
    let mut aliases: Value = if path.exists() {
        let value: Value = storage::read_json(&path)?;
        if value["schemaVersion"] != 1 {
            return Err(failure("中文名称配置不兼容，原文件已保留。"));
        }
        value
    } else {
        json!({"schemaVersion":1,"names":{}})
    };
    if name.trim().is_empty() {
        aliases["names"]
            .as_object_mut()
            .ok_or_else(|| failure("中文名配置损坏。"))?
            .remove(&info.id);
    } else {
        aliases["names"][info.id] = name.trim().into();
    }
    storage::write_json(&path, &aliases)
}
