use hydrogen_plugin_services::contracts::{ApiError, Result};
use hydrogen_plugin_services::storage;
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Version {
    pub name: String,
    pub manifest_file: String,
    pub metadata: Value,
    pub game_version: String,
    pub loader: String,
    pub loader_version: Option<String>,
}
fn error(message: impl Into<String>) -> ApiError {
    ApiError::new("GAME_REPOSITORY", message)
}
pub fn valid_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name != "."
        && name != ".."
        && name.chars().count() <= 255
        && !name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        && !name.ends_with(['.', ' '])
}
pub fn instance_id(root: &Path, name: &str) -> String {
    let root = root.to_string_lossy();
    let root = if cfg!(windows) {
        root.to_lowercase()
    } else {
        root.into_owned()
    };
    format!("{:x}", Sha1::digest(format!("{root}\0{name}").as_bytes()))[..32].into()
}
fn meaningful(value: &Value) -> bool {
    value["mainClass"]
        .as_str()
        .is_some_and(|s| !s.trim().is_empty())
        || value["inheritsFrom"].is_string()
        || value["patches"].as_array().is_some_and(|a| !a.is_empty())
}
pub fn manifest_file(root: &Path, name: &str) -> Result<Option<String>> {
    if !valid_name(name) {
        return Err(error("版本目录名无效。"));
    }
    let conventional = format!("versions/{name}/{name}.json");
    if storage::within(root, &conventional)?.is_file() {
        return Ok(Some(conventional));
    }
    let directory = storage::within(root, &format!("versions/{name}"))?;
    if !directory.is_dir() {
        return Ok(None);
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        {
            let filename = entry.file_name().to_string_lossy().into_owned();
            let relative = format!("versions/{name}/{filename}");
            storage::within(root, &relative)?;
            files.push(relative);
        }
    }
    files.sort();
    if files.len() == 1 {
        return Ok(files.pop());
    }
    Ok(files.into_iter().find(|file| {
        storage::read_json::<Value>(&root.join(file)).is_ok_and(|value| meaningful(&value))
    }))
}
fn combine(base: &Value, child: &Value, patch: bool) -> Value {
    let mut result = base.clone();
    if !result.is_object() {
        result = json!({});
    }
    if let Some(fields) = child.as_object() {
        for (key, value) in fields {
            if value.is_null()
                || ["id", "inheritsFrom", "patches", "priority", "version"].contains(&key.as_str())
                || (patch && ["jar", "root", "hidden"].contains(&key.as_str()))
            {
                continue;
            }
            if key == "arguments" {
                for kind in ["game", "jvm"] {
                    let mut args = result[key][kind].as_array().cloned().unwrap_or_default();
                    if let Some(extra) = value[kind].as_array() {
                        args.extend(extra.clone());
                    }
                    if !result[key].is_object() {
                        result[key] = json!({});
                    }
                    result[key][kind] = json!(args);
                }
            } else if key == "libraries" {
                let mut libraries = if patch {
                    result[key].as_array().cloned().unwrap_or_default()
                } else {
                    value.as_array().cloned().unwrap_or_default()
                };
                libraries.extend(if patch {
                    value.as_array().cloned().unwrap_or_default()
                } else {
                    result[key].as_array().cloned().unwrap_or_default()
                });
                result[key] = json!(libraries);
            } else if key == "minimumLauncherVersion" {
                result[key] = json!(
                    result[key]
                        .as_u64()
                        .unwrap_or(0)
                        .max(value.as_u64().unwrap_or(0))
                );
            } else {
                result[key] = value.clone();
            }
        }
    }
    result
}
fn resolve(
    root: &Path,
    name: &str,
    visiting: &mut BTreeSet<String>,
    resolved: &mut BTreeMap<String, Version>,
) -> Result<Version> {
    if let Some(version) = resolved.get(name) {
        return Ok(version.clone());
    }
    if visiting.len() >= 32 || !visiting.insert(name.into()) {
        return Err(error("版本继承存在循环或层数过多。"));
    }
    let file =
        manifest_file(root, name)?.ok_or_else(|| error(format!("找不到版本 {name} 的 JSON。")))?;
    let mut raw: Value = storage::read_json(&storage::within(root, &file)?)?;
    if !raw.is_object() || !meaningful(&raw) {
        return Err(error("版本 JSON 缺少启动入口或组件信息。"));
    }
    raw["id"] = name.into();
    let parent = if let Some(parent) = raw["inheritsFrom"].as_str() {
        if !valid_name(parent) {
            return Err(error("继承的版本目录无效。"));
        }
        Some(resolve(root, parent, visiting, resolved)?)
    } else {
        None
    };
    let patches = raw["patches"].as_array().cloned().unwrap_or_default();
    let mut metadata = if let Some(parent) = &parent {
        combine(&parent.metadata, &raw, false)
    } else if raw["root"].as_bool().unwrap_or(false) && !patches.is_empty() {
        json!({})
    } else {
        raw.clone()
    };
    let mut ordered = patches.clone();
    ordered.sort_by_key(|patch| patch["priority"].as_i64().unwrap_or(0));
    for patch in &ordered {
        metadata = combine(&metadata, patch, true);
    }
    metadata["id"] = name.into();
    let own_jar = Path::new(&file)
        .with_extension("jar")
        .to_string_lossy()
        .replace('\\', "/");
    let jar = if let Some(jar) = raw["jar"].as_str().or_else(|| {
        parent
            .as_ref()
            .and_then(|parent| parent.metadata["jar"].as_str())
    }) {
        if !valid_name(jar) {
            return Err(error("客户端 JAR 目录无效。"));
        }
        if jar == name {
            own_jar
        } else {
            manifest_file(root, jar)?
                .map(|file| {
                    Path::new(&file)
                        .with_extension("jar")
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .unwrap_or_else(|| format!("versions/{jar}/{jar}.jar"))
        }
    } else if let Some(parent) = &parent {
        parent.metadata["_hydrogenJar"].as_str().unwrap().into()
    } else {
        own_jar
    };
    storage::within(root, &jar)?;
    metadata["_hydrogenJar"] = jar.clone().into();
    metadata["_hydrogenVersionRoot"] = format!("versions/{name}").into();
    metadata.as_object_mut().unwrap().remove("inheritsFrom");
    metadata.as_object_mut().unwrap().remove("patches");
    let game_version = patches
        .iter()
        .find(|patch| ["game", "minecraft"].contains(&patch["id"].as_str().unwrap_or("")))
        .and_then(|patch| patch["version"].as_str())
        .map(String::from)
        .or_else(|| parent.as_ref().map(|p| p.game_version.clone()))
        .or_else(|| raw["clientVersion"].as_str().map(String::from))
        .or_else(|| jar_version(&root.join(&jar)))
        .unwrap_or_else(|| name.to_string());
    let mut loader = String::new();
    let mut loader_version = None;
    for library in metadata["libraries"].as_array().into_iter().flatten() {
        let descriptor = library["name"].as_str().unwrap_or("");
        for (prefix, label) in [
            ("net.fabricmc:fabric-loader:", "Fabric"),
            ("org.quiltmc:quilt-loader:", "Quilt"),
            ("net.neoforged:neoforge:", "NeoForge"),
            ("net.neoforged:forge:", "NeoForge"),
            ("net.minecraftforge:forge:", "Forge"),
            ("optifine:OptiFine:", "OptiFine"),
        ] {
            if let Some(version) = descriptor.strip_prefix(prefix) {
                loader = label.into();
                loader_version = Some(version.into());
            }
        }
    }
    let version = Version {
        name: name.into(),
        manifest_file: file,
        metadata,
        game_version,
        loader,
        loader_version,
    };
    visiting.remove(name);
    resolved.insert(name.into(), version.clone());
    Ok(version)
}
fn jar_version(path: &Path) -> Option<String> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).ok()?).ok()?;
    let file = zip.by_name("version.json").ok()?;
    if file.size() > 256 * 1024 {
        return None;
    }
    let value: Value = serde_json::from_reader(file).ok()?;
    value["id"]
        .as_str()
        .or_else(|| value["name"].as_str())
        .map(String::from)
}
pub fn normalize(metadata: &mut Value, game_version: &str) -> Result<()> {
    if !metadata["arguments"].is_object() {
        metadata["arguments"] = json!({});
    }
    if let Some(legacy) = metadata["minecraftArguments"].as_str() {
        let mut arguments: Vec<Value> = split_arguments(legacy)?
            .into_iter()
            .map(Value::String)
            .collect();
        arguments.extend(
            metadata["arguments"]["game"]
                .as_array()
                .into_iter()
                .flatten()
                .cloned(),
        );
        metadata["arguments"]["game"] = json!(arguments);
        metadata
            .as_object_mut()
            .unwrap()
            .remove("minecraftArguments");
    } else if !metadata["arguments"]["game"].is_array() {
        metadata["arguments"]["game"] = json!([]);
    }
    if !metadata["arguments"]["jvm"].is_array()
        || metadata["arguments"]["jvm"]
            .as_array()
            .is_some_and(Vec::is_empty)
    {
        metadata["arguments"]["jvm"] = json!([
            "-Djava.library.path=${natives_directory}",
            "-cp",
            "${classpath}"
        ]);
    }
    if !metadata["libraries"].is_array() {
        metadata["libraries"] = json!([]);
    }
    if !metadata["javaVersion"]["majorVersion"].is_number() {
        let version = regex::Regex::new(r"^1\.(\d+)(?:\.(\d+))?")
            .unwrap()
            .captures(game_version)
            .map(|c| {
                (
                    c[1].parse::<u32>().unwrap_or(0),
                    c.get(2)
                        .and_then(|v| v.as_str().parse::<u32>().ok())
                        .unwrap_or(0),
                )
            });
        let major = match version {
            Some((minor, patch)) if minor > 20 || minor == 20 && patch >= 5 => 21,
            Some((minor, _)) if minor >= 18 => 17,
            Some((17, _)) => 16,
            _ => 8,
        };
        metadata["javaVersion"] = json!({"majorVersion":major});
    }
    if !metadata["type"].is_string() {
        metadata["type"] = "release".into();
    }
    if !metadata["assetIndex"].is_object() {
        let id = metadata["assets"].as_str().unwrap_or("legacy").to_string();
        if !valid_name(&id) {
            return Err(error("资源索引名无效。"));
        }
        metadata["assetIndex"] = json!({"id":id});
    }
    Ok(())
}
pub fn split_arguments(input: &str) -> Result<Vec<String>> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut chars = input.chars().peekable();
    let mut started = false;
    while let Some(c) = chars.next() {
        if Some(c) == quote {
            quote = None;
            started = true;
        } else if quote.is_none() && ['\'', '"'].contains(&c) {
            quote = Some(c);
            started = true;
        } else if c == '\\'
            && chars
                .peek()
                .is_some_and(|next| Some(*next) == quote || *next == '\\')
        {
            current.push(chars.next().unwrap());
            started = true;
        } else if c.is_whitespace() && quote.is_none() {
            if started {
                result.push(std::mem::take(&mut current));
                started = false;
            }
        } else {
            current.push(c);
            started = true;
        }
    }
    if quote.is_some() {
        return Err(error("旧版启动参数的引号未闭合。"));
    }
    if started {
        result.push(current);
    }
    Ok(result)
}
pub fn load(root: &Path, name: &str) -> Result<Version> {
    let mut version = resolve(root, name, &mut BTreeSet::new(), &mut BTreeMap::new())?;
    normalize(&mut version.metadata, &version.game_version)?;
    Ok(version)
}
pub fn scan(root: &Path) -> Result<(Vec<Version>, Vec<String>)> {
    let directory = storage::within(root, "versions")?;
    if !directory.exists() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut versions = Vec::new();
    let mut issues = Vec::new();
    let mut resolved = BTreeMap::new();
    for entry in fs::read_dir(directory)?.take(10_000) {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !valid_name(&name)
            || name.starts_with(".hydrogen-delete-")
            || !entry.file_type()?.is_dir()
        {
            continue;
        }
        match resolve(root, &name, &mut BTreeSet::new(), &mut resolved) {
            Ok(mut version) if version.metadata["hidden"] != true => {
                if normalize(&mut version.metadata, &version.game_version).is_ok() {
                    versions.push(version);
                } else {
                    issues.push(name);
                }
            }
            Ok(_) => {}
            Err(_) => {
                if !matches!(manifest_file(root, &name), Ok(None)) {
                    issues.push(name);
                }
            }
        }
    }
    versions.sort_by_key(|version| version.name.to_lowercase());
    Ok((versions, issues))
}
pub fn run_directory(root: &Path, name: &str, isolate: Option<bool>) -> Result<PathBuf> {
    if !valid_name(name) {
        return Err(error("版本目录名无效。"));
    }
    let version = storage::within(root, &format!("versions/{name}"))?;
    let isolate = isolate.unwrap_or_else(|| {
        [
            "mods",
            "config",
            "saves",
            "resourcepacks",
            "shaderpacks",
            "options.txt",
            "servers.dat",
        ]
        .iter()
        .any(|entry| version.join(entry).exists())
            || load(root, name).is_ok_and(|version| !version.loader.is_empty())
    });
    Ok(if isolate { version } else { root.into() })
}
pub fn native_directory(root: &Path, name: &str, architecture: &str) -> Result<PathBuf> {
    if !valid_name(name) {
        return Err(error("版本目录名无效。"));
    }
    storage::within(
        root,
        &format!(
            "versions/{name}/natives-{}-{}",
            if cfg!(windows) {
                "windows"
            } else if cfg!(target_os = "macos") {
                "macos"
            } else {
                "linux"
            },
            match architecture {
                "x64" => "x86_64",
                "x86" => "x86",
                other => other,
            }
        ),
    )
}

pub fn publish(
    root: &Path,
    name: &str,
    game_version: &str,
    base: &Value,
    complete: &Value,
    loader: Option<&str>,
    loader_kind: &str,
) -> Result<()> {
    if !valid_name(name) {
        return Err(error("版本目录名无效。"));
    }
    let clean = |value: &Value| {
        let mut value = value.clone();
        if let Some(fields) = value.as_object_mut() {
            fields.retain(|key, _| !key.starts_with("_hydrogen"));
            fields.remove("patches");
        }
        value
    };
    let mut patch = clean(base);
    patch["id"] = "game".into();
    patch["version"] = game_version.into();
    patch["priority"] = 0.into();
    let mut patches = vec![patch];
    if let Some(loader) = loader {
        let base_names: BTreeSet<_> = base["libraries"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|library| library["name"].as_str())
            .collect();
        let libraries: Vec<_> = complete["libraries"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|library| !base_names.contains(library["name"].as_str().unwrap_or("")))
            .cloned()
            .collect();
        let mut arguments = json!({});
        for kind in ["game", "jvm"] {
            let previous = base["arguments"][kind].as_array().map_or(0, Vec::len);
            arguments[kind] = json!(
                complete["arguments"][kind]
                    .as_array()
                    .map(|arguments| arguments.iter().skip(previous).cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
            );
        }
        let id = if loader_kind == "quilt" {
            "quilt"
        } else {
            "fabric"
        };
        patches.push(json!({"id":id, "version":loader,"priority":30000,"mainClass":complete["mainClass"],"arguments":arguments,"libraries":libraries}));
    }
    let mut manifest = clean(complete);
    manifest["id"] = name.into();
    manifest["root"] = true.into();
    manifest["jar"] = name.into();
    manifest["patches"] = json!(patches);
    if matches!(loader_kind, "forge" | "neoforge") {
        // Forge may replace both legacy game arguments and vanilla library versions.
        // Persist the fully merged manifest instead of an append-only Fabric patch.
        manifest.as_object_mut().unwrap().remove("patches");
        manifest.as_object_mut().unwrap().remove("root");
        manifest["clientVersion"] = game_version.into();
    }
    let destination = storage::within(root, &format!("versions/{name}/{name}.json"))?;
    if destination.exists() {
        return Err(error("版本 JSON 已存在，拒绝覆盖。请刷新后重试。"));
    }
    let temporary =
        destination.with_file_name(format!(".{}.pending", uuid::Uuid::new_v4().simple()));
    storage::write_json(&temporary, &manifest)?;
    let result = storage::move_new(&temporary, &destination);
    if temporary.exists() {
        let _ = fs::remove_file(temporary);
    }
    result
}
