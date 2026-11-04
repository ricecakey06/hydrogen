use futures_util::StreamExt;
use hydrogen_plugin_services::{
    PluginHost, Settings,
    contracts::{ApiError, Result},
    jobs::Job,
    net, storage, text,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};
fn allows(
    client: &net::Client,
    owner: &Value,
    architecture: &str,
    custom_resolution: bool,
) -> Result<bool> {
    Ok(client.service(
        "minecraft.content",
        "allows",
        json!({"owner":owner,"architecture":architecture,"customResolution":custom_resolution}),
    )? == true)
}
fn available_space(host: &PluginHost, settings: &Settings, path: &Path) -> Result<Option<u64>> {
    Ok(serde_json::from_value(host.call_service(
        settings,
        "platform.windows.environment",
        "platform.windows.environment",
        "availableSpace",
        json!({"path":path}),
    )?)?)
}
#[derive(Clone)]
pub struct FileSpec {
    pub relative: String,
    pub url: String,
    pub hash: String,
    pub size: Option<u64>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Prepared {
    pub metadata: Value,
    pub classpath: Vec<PathBuf>,
    pub natives: Vec<(PathBuf, Vec<String>)>,
    pub assets: PathBuf,
    pub libraries: PathBuf,
    pub asset_index: String,
    pub logging: Option<String>,
}
fn invalid() -> ApiError {
    ApiError::new("INVALID_METADATA", "游戏元数据不完整或格式尚未支持。")
}
fn spec(value: &Value, relative: String) -> Result<FileSpec> {
    Ok(FileSpec {
        relative,
        url: value["url"].as_str().unwrap_or("").into(),
        hash: value["sha1"].as_str().unwrap_or("").into(),
        size: value["size"].as_u64(),
    })
}
fn library_spec(
    library: &Value,
    artifact: &Value,
    classifier: Option<&str>,
    owner: &str,
) -> Result<FileSpec> {
    let descriptor = text(library, "name")?;
    let (coordinate, extension) = descriptor.split_once('@').unwrap_or((descriptor, "jar"));
    let parts: Vec<_> = coordinate.split(':').collect();
    if !(3..=4).contains(&parts.len())
        || parts.iter().chain([extension].iter()).any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
        })
    {
        return Err(invalid());
    }
    let suffix = classifier
        .or_else(|| parts.get(3).copied())
        .map(|classifier| format!("-{classifier}"))
        .unwrap_or_default();
    let path = artifact["path"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| {
            format!(
                "{}/{}/{}/{}-{}{}.{}",
                parts[0].replace('.', "/"),
                parts[1],
                parts[2],
                parts[1],
                parts[2],
                suffix,
                extension
            )
        });
    let hint = library["hint"]
        .as_str()
        .or_else(|| library["MMC-hint"].as_str());
    let relative = if hint == Some("local") {
        let filename = library["filename"]
            .as_str()
            .or_else(|| library["MMC-filename"].as_str())
            .map(String::from)
            .unwrap_or_else(|| {
                Path::new(&path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            });
        format!("{owner}/libraries/{filename}")
    } else {
        format!("libraries/{path}")
    };
    let url = artifact["url"]
        .as_str()
        .filter(|url| !url.is_empty())
        .map(String::from)
        .unwrap_or_else(|| {
            format!(
                "{}{path}",
                library["url"]
                    .as_str()
                    .unwrap_or("https://libraries.minecraft.net/")
                    .trim_end_matches('/')
                    .to_string()
                    + "/"
            )
        });
    let integrity = if classifier.is_none() {
        library
    } else {
        &Value::Null
    };
    Ok(FileSpec {
        relative,
        url: if url.starts_with("http://") {
            url.replacen("http://", "https://", 1)
        } else {
            url
        },
        hash: artifact["sha1"]
            .as_str()
            .or_else(|| integrity["sha1"].as_str())
            .or_else(|| integrity["checksums"].as_array()?.first()?.as_str())
            .unwrap_or("")
            .into(),
        size: artifact["size"]
            .as_u64()
            .or_else(|| integrity["size"].as_u64())
            .filter(|size| *size > 0),
    })
}

pub async fn prepare(
    plugins: &hydrogen_plugin_services::PluginHost,
    client: &net::Client,
    game_root: &Path,
    settings: &Settings,
    metadata: Value,
    architecture: &str,
    job: Arc<Job>,
) -> Result<Prepared> {
    let id = text(&metadata, "id")?;
    let jar = metadata["_hydrogenJar"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| format!("versions/{id}/{id}.jar"));
    let destination = storage::within(game_root, &jar)?;
    let client_hash = metadata["downloads"]["client"]["sha1"]
        .as_str()
        .unwrap_or("");
    if !destination.exists() && net::valid_hash(client_hash) {
        let versions = storage::within(game_root, "versions")?;
        if versions.is_dir() {
            'search: for directory in std::fs::read_dir(versions)?.take(10_000) {
                let directory = directory?;
                if !directory.file_type()?.is_dir() || storage::linked(&directory.path()) {
                    continue;
                }
                for file in std::fs::read_dir(directory.path())?.take(100) {
                    job.check()?;
                    let file = file?;
                    if !file.file_type()?.is_file()
                        || storage::linked(&file.path())
                        || !file
                            .path()
                            .extension()
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("jar"))
                    {
                        continue;
                    }
                    if metadata["downloads"]["client"]["size"]
                        .as_u64()
                        .is_some_and(|size| {
                            file.metadata().is_ok_and(|metadata| metadata.len() != size)
                        })
                    {
                        continue;
                    }
                    if net::hash(client, &file.path())
                        .await?
                        .eq_ignore_ascii_case(client_hash)
                    {
                        tokio::fs::create_dir_all(destination.parent().unwrap()).await?;
                        let temporary = destination
                            .with_file_name(format!(".{}.reuse", uuid::Uuid::new_v4().simple()));
                        let result = async {
                            tokio::fs::copy(file.path(), &temporary).await?;
                            if !net::hash(client, &temporary)
                                .await?
                                .eq_ignore_ascii_case(client_hash)
                            {
                                return Err(invalid());
                            }
                            job.check()?;
                            storage::move_new(&temporary, &destination)
                        }
                        .await;
                        let _ = tokio::fs::remove_file(&temporary).await;
                        result?;
                        break 'search;
                    }
                }
            }
        }
    }
    let mut files = vec![spec(&metadata["downloads"]["client"], jar.clone())?];
    let mut classpath = Vec::new();
    let mut natives = Vec::new();
    let asset_index = text(&metadata["assetIndex"], "id")?.to_string();
    if asset_index.len() > 100
        || !asset_index
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    {
        return Err(invalid());
    }
    let index_file = spec(
        &metadata["assetIndex"],
        format!("assets/indexes/{asset_index}.json"),
    )?;
    job.begin_download([(index_file.relative.clone(), None)], false);
    let index_path = net::ensure_game_file(
        client,
        game_root,
        &index_file.relative,
        &index_file.url,
        (!index_file.hash.is_empty()).then_some(index_file.hash.as_str()),
        index_file.size,
        settings.download_source == "bmclapi",
        settings.enabled("minecraft.download"),
        &job,
    )
    .await?;
    files.push(index_file);
    let index: Value = storage::read_json(&index_path)?;
    for value in index["objects"].as_object().ok_or_else(invalid)?.values() {
        let hash = text(value, "hash")?;
        if !net::valid_hash(hash) {
            return Err(invalid());
        }
        files.push(FileSpec {
            relative: format!("assets/objects/{}/{hash}", &hash[..2]),
            url: format!(
                "https://resources.download.minecraft.net/{}/{hash}",
                &hash[..2]
            ),
            hash: hash.into(),
            size: value["size"].as_u64(),
        });
    }
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    };
    let native_suffix = format!(
        "natives-{}{}",
        if os == "osx" { "macos" } else { os },
        match architecture {
            "x64" => "",
            "arm64" => "-arm64",
            "x86" => "-x86",
            _ => "-unsupported",
        }
    );
    let owner = metadata["_hydrogenVersionRoot"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| format!("versions/{id}"));
    for library in metadata["libraries"].as_array().ok_or_else(invalid)? {
        if !allows(client, library, architecture, false)? {
            continue;
        }
        let name = text(library, "name")?;
        if let Some(classifier) = name.split(':').nth(3)
            && classifier.starts_with("natives-")
            && classifier != native_suffix
        {
            continue;
        }
        if library["downloads"]["artifact"].is_object() || !library["natives"].is_object() {
            let file = library_spec(library, &library["downloads"]["artifact"], None, &owner)?;
            classpath.push(storage::within(game_root, &file.relative)?);
            files.push(file);
        }
        if let Some(native_map) = library.get("natives") {
            if architecture == "arm64" {
                return Err(ApiError::new(
                    "UNSUPPORTED_NATIVES",
                    "此版本未提供 ARM64 原生库。",
                ));
            }
            let Some(native_name) = native_map[os].as_str() else {
                continue;
            };
            let classifier =
                native_name.replace("${arch}", if architecture == "x86" { "32" } else { "64" });
            let native = &library["downloads"]["classifiers"][&classifier];
            let file = library_spec(library, native, Some(&classifier), &owner)?;
            let excludes = library["extract"]["exclude"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            natives.push((storage::within(game_root, &file.relative)?, excludes));
            files.push(file);
        }
    }
    let logging = if let Some(logging) = metadata["logging"].get("client") {
        let filename = logging["file"]["id"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| {
                format!(
                    "{}.xml",
                    logging["file"]["sha1"].as_str().unwrap_or("logging")
                )
            });
        let relative = format!("assets/log_configs/{filename}");
        let path = storage::within(game_root, &relative)?;
        files.push(spec(&logging["file"], relative)?);
        Some(text(logging, "argument")?.replace("${path}", &path.to_string_lossy()))
    } else {
        None
    };
    classpath.push(storage::within(game_root, &jar)?);
    let mut seen = BTreeSet::new();
    let unique: Vec<_> = files
        .into_iter()
        .filter(|file| seen.insert(file.relative.clone()))
        .collect();
    if let Some(available) = available_space(plugins, settings, game_root)? {
        let missing = unique.iter().try_fold(0_u64, |sum, file| {
            let destination = storage::within(game_root, &file.relative)?;
            let cached = destination
                .metadata()
                .is_ok_and(|metadata| file.size.is_none_or(|size| metadata.len() == size));
            if cached {
                Ok(sum)
            } else {
                sum.checked_add(file.size.unwrap_or(0)).ok_or_else(invalid)
            }
        })?;
        if available < missing.saturating_add(64 * 1024 * 1024) {
            return Err(ApiError::new(
                "INSUFFICIENT_SPACE",
                format!("至少需要 {} MB 可用空间。", missing / (1024 * 1024) + 64),
            ));
        }
    }
    job.begin_download(
        unique.iter().map(|file| (file.relative.clone(), file.size)),
        true,
    );
    let limit = Arc::new(tokio::sync::Semaphore::new(32));
    let mut running = futures_util::stream::FuturesUnordered::new();
    for file in unique {
        let client = client.clone();
        let game_root = game_root.to_path_buf();
        let settings = settings.clone();
        let job = job.clone();
        let limit = limit.clone();
        running.push(async move {
            let _permit = limit.acquire().await.map_err(|_| invalid())?;
            net::ensure_game_file(
                &client,
                &game_root,
                &file.relative,
                &file.url,
                (!file.hash.is_empty()).then_some(file.hash.as_str()),
                file.size,
                settings.download_source == "bmclapi",
                settings.enabled("minecraft.download"),
                &job,
            )
            .await
            .map(|_| file.relative)
        });
    }
    while let Some(result) = running.next().await {
        result?;
    }
    let assets = if index["virtual"] == true {
        storage::within(game_root, &format!("assets/virtual/{asset_index}"))?
    } else {
        storage::within(game_root, "assets")?
    };
    if index["virtual"] == true || index["map_to_resources"] == true {
        let run_directory = metadata["_hydrogenRunDirectory"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| game_root.into());
        for (name, asset) in index["objects"].as_object().ok_or_else(invalid)? {
            let hash = text(asset, "hash")?;
            let source =
                storage::within(game_root, &format!("assets/objects/{}/{hash}", &hash[..2]))?;
            let mut destinations = Vec::new();
            if index["virtual"] == true {
                destinations.push(storage::within(&assets, name)?);
            }
            if index["map_to_resources"] == true {
                destinations.push(storage::within(
                    &run_directory,
                    &format!("resources/{name}"),
                )?);
            }
            for destination in destinations {
                job.check()?;
                if !destination.exists() {
                    tokio::fs::create_dir_all(destination.parent().unwrap()).await?;
                    tokio::fs::copy(&source, destination).await?;
                }
            }
        }
    }
    Ok(Prepared {
        metadata,
        classpath,
        natives,
        assets,
        libraries: storage::within(game_root, "libraries")?,
        asset_index,
        logging,
    })
}

#[cfg(test)]
mod library_tests {
    use super::*;

    #[test]
    fn fabric_library_integrity_fields_are_not_lost_without_downloads_artifact() {
        let library = json!({"name":"org.ow2.asm:asm:9.10.1", "url":"https://maven.fabricmc.net/",
            "sha1":"ada2141c0cc52ee8f5c48cd5fa4ce0e794f22236", "size":126151});
        let file = library_spec(&library, &Value::Null, None, "versions/26.1.2").unwrap();
        assert_eq!(file.hash, library["sha1"].as_str().unwrap());
        assert_eq!(file.size, Some(126151));
        assert_eq!(
            file.relative,
            "libraries/org/ow2/asm/asm/9.10.1/asm-9.10.1.jar"
        );
        let artifact = json!({"sha1":"1111111111111111111111111111111111111111", "size":42});
        let native = library_spec(
            &library,
            &artifact,
            Some("natives-windows"),
            "versions/26.1.2",
        )
        .unwrap();
        assert_eq!(native.hash, artifact["sha1"].as_str().unwrap());
        assert_eq!(native.size, Some(42));
        let native = library_spec(
            &library,
            &Value::Null,
            Some("natives-windows"),
            "versions/26.1.2",
        )
        .unwrap();
        assert!(
            native.hash.is_empty(),
            "a classifier must not inherit the main artifact's hash"
        );
        assert_eq!(native.size, None);
    }
}
