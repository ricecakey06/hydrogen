use hydrogen_plugin_services::{
    Settings,
    contracts::{ApiError, Result},
    jobs::Job,
    net, storage, text,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
const MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub id: String,
    pub release_time: String,
    pub version_type: String,
}
fn invalid() -> ApiError {
    ApiError::new("INVALID_METADATA", "游戏元数据不完整或格式尚未支持。")
}
pub fn supported(id: &str) -> bool {
    // The official catalog is the authority; snapshots also have non-numeric IDs.
    !id.trim().is_empty()
        && id.chars().count() <= 100
        && id != "."
        && id != ".."
        && !id.ends_with(['.', ' '])
        && !id
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
}

pub async fn catalog(client: &net::Client, root: &Path, settings: &Settings) -> Result<Value> {
    use std::{
        sync::OnceLock,
        time::{Duration, Instant},
    };
    type CatalogCache = tokio::sync::Mutex<Option<(PathBuf, Instant, Value)>>;
    static CATALOG: OnceLock<CatalogCache> = OnceLock::new();
    let mut current = CATALOG
        .get_or_init(|| tokio::sync::Mutex::new(None))
        .lock()
        .await;
    if let Some((directory, loaded, value)) = &*current
        && directory == root
        && loaded.elapsed() < Duration::from_secs(15 * 60)
    {
        return Ok(value.clone());
    }
    let cache = storage::within(root, "cache/version-manifest.json")?;
    let mut remote_error = None;
    if settings.enabled("minecraft.download") {
        let fetch = async {
            for attempt in 0..3 {
                let result = async {
                    let url =
                        net::source_url(client, MANIFEST, settings.download_source == "bmclapi")?;
                    let bytes = match net::bytes(client, &url, 16 * 1024 * 1024).await {
                        Ok(bytes) => bytes,
                        Err(_) if url != MANIFEST => {
                            net::bytes(client, MANIFEST, 16 * 1024 * 1024).await?
                        }
                        Err(error) => return Err(error),
                    };
                    let value: Value = serde_json::from_slice(&bytes)?;
                    if !value["versions"]
                        .as_array()
                        .is_some_and(|versions| !versions.is_empty())
                    {
                        return Err(invalid());
                    }
                    Ok(value)
                }
                .await;
                match result {
                    Ok(value) => return Ok(value),
                    Err(error) if error.retryable && attempt < 2 => {
                        tokio::time::sleep(Duration::from_millis(300 * (attempt + 1))).await
                    }
                    Err(error) => return Err(error),
                }
            }
            unreachable!()
        };
        match tokio::time::timeout(Duration::from_secs(60), fetch).await {
            Ok(Ok(value)) => {
                storage::write_json(&cache, &value)?;
                *current = Some((root.to_owned(), Instant::now(), value.clone()));
                return Ok(value);
            }
            Ok(Err(error)) => remote_error = Some(error),
            Err(_) => {
                remote_error = Some(ApiError::new("NETWORK", "下载请求超时，请检查网络后重试。"))
            }
        }
    }
    if cache.exists() {
        let value: Value = storage::read_json(&cache)?;
        if value["versions"]
            .as_array()
            .is_some_and(|versions| !versions.is_empty())
        {
            *current = Some((root.to_owned(), Instant::now(), value.clone()));
            return Ok(value);
        }
    }
    Err(remote_error.unwrap_or_else(|| {
        ApiError::new(
            "CATALOG_UNAVAILABLE",
            "没有本地版本目录，请启用游戏下载并检查网络。",
        )
    }))
}

pub async fn versions(
    client: &net::Client,
    root: &Path,
    settings: &Settings,
) -> Result<Vec<Version>> {
    Ok(catalog(client, root, settings).await?["versions"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .filter(|v| {
            matches!(v["type"].as_str(), Some("release" | "snapshot"))
                && v["id"].as_str().is_some_and(supported)
        })
        .map(|v| Version {
            id: v["id"].as_str().unwrap().into(),
            release_time: v["releaseTime"].as_str().unwrap_or("").into(),
            version_type: v["type"].as_str().unwrap_or("release").into(),
        })
        .collect())
}
pub async fn resolve(
    client: &net::Client,
    data_root: &Path,
    game_root: &Path,
    settings: &Settings,
    id: &str,
    job: &Job,
) -> Result<Value> {
    if !supported(id) {
        return Err(ApiError::new("UNSUPPORTED_VERSION", "游戏版本标识无效。"));
    }
    let catalog = catalog(client, data_root, settings).await?;
    let entry = catalog["versions"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .find(|v| v["id"] == id && matches!(v["type"].as_str(), Some("release" | "snapshot")))
        .ok_or_else(invalid)?;
    let hash = text(entry, "sha1")?;
    let local = storage::within(game_root, &format!("versions/{id}/{id}.json"))?;
    let path = if local.is_file() && net::hash(client, &local).await?.eq_ignore_ascii_case(hash) {
        local
    } else {
        net::ensure(
            client,
            data_root,
            &format!("cache/manifests/{hash}.json"),
            text(entry, "url")?,
            hash,
            None,
            settings.download_source == "bmclapi",
            settings.enabled("minecraft.download"),
            job,
        )
        .await?
    };
    let mut metadata: Value = storage::read_json(&path)?;
    if metadata["id"] != id
        || !metadata["mainClass"].is_string()
        || !(metadata["arguments"].is_object() || metadata["minecraftArguments"].is_string())
        || metadata["minimumLauncherVersion"]
            .as_u64()
            .is_some_and(|n| n > 21)
    {
        return Err(invalid());
    }
    metadata = hydrogen_plugin_services::invoke(
        "minecraft.instances",
        "minecraft.instances",
        "normalize",
        serde_json::json!({"metadata": metadata, "gameVersion": id}),
    )?;
    Ok(metadata)
}
pub fn allows(owner: &Value, architecture: &str, custom_resolution: bool) -> Result<bool> {
    let Some(rules) = owner.get("rules") else {
        return Ok(true);
    };
    let rules = rules.as_array().ok_or_else(invalid)?;
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    };
    let java_arch = match architecture {
        "x64" => "x86_64",
        "arm64" => "aarch64",
        other => other,
    };
    let mut allowed = false;
    for rule in rules {
        let mut matches = true;
        if let Some(name) = rule["os"]["name"].as_str() {
            matches &= name == os;
        }
        if let Some(arch) = rule["os"]["arch"].as_str() {
            matches &= regex::Regex::new(&format!("^(?:{arch})$"))
                .map_err(|_| invalid())?
                .is_match(java_arch);
        }
        if let Some(version) = rule["os"]["version"].as_str() {
            matches &= regex::Regex::new(version)
                .map_err(|_| invalid())?
                .is_match(&super::os_version()?);
        }
        if let Some(range) = rule["os"].get("versionRange") {
            let parse = |v: &Value| {
                v.as_str()
                    .unwrap_or("")
                    .split('.')
                    .map(|n| n.parse::<u32>().unwrap_or(0))
                    .chain(std::iter::repeat(0))
                    .take(4)
                    .collect::<Vec<_>>()
            };
            let actual = parse(&Value::String(super::os_version()?));
            if range.get("min").is_some() {
                matches &= actual >= parse(&range["min"]);
            }
            if range.get("max").is_some() {
                matches &= actual < parse(&range["max"]);
            }
        }
        if let Some(features) = rule["features"].as_object() {
            for (key, value) in features {
                matches &= (key == "has_custom_resolution" && custom_resolution)
                    == value.as_bool().ok_or_else(invalid)?;
            }
        }
        if matches {
            allowed = match text(rule, "action")? {
                "allow" => true,
                "disallow" => false,
                _ => return Err(invalid()),
            };
        }
    }
    Ok(allowed)
}
