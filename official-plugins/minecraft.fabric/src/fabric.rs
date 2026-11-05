use hydrogen_plugin_services::{
    Settings,
    contracts::{ApiError, Result},
    jobs::Job,
    net, storage, text,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Components {
    #[serde(default)]
    pub kind: String,
    pub loader: String,
    pub fabric_api: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
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
pub fn supported(client: &net::Client, id: &str) -> Result<bool> {
    Ok(client.service("minecraft.content", "supported", json!({"id":id}))? == true)
}
pub async fn fabric_options(client: &net::Client, version: &str) -> Result<Value> {
    if !supported(client, version)? {
        return Err(invalid());
    }
    let loaders = net::json(
        client,
        &format!("https://meta.fabricmc.net/v2/versions/loader/{version}"),
    )
    .await?;
    let mut url = reqwest::Url::parse("https://api.modrinth.com/v2/project/fabric-api/version")
        .map_err(|_| invalid())?;
    url.query_pairs_mut()
        .append_pair("game_versions", &json!([version]).to_string())
        .append_pair("loaders", "[\"fabric\"]");
    let api = net::json(client, url.as_str()).await?;
    Ok(
        json!({ "loaders": loaders.as_array().ok_or_else(invalid)?.iter().map(|v| json!({"id":v["loader"]["version"],"stable":v["loader"]["stable"]})).collect::<Vec<_>>(), "api": api }),
    )
}
#[allow(clippy::too_many_arguments)]
pub async fn fabric(
    client: &net::Client,
    data_root: &Path,
    game_root: &Path,
    game: &Path,
    settings: &Settings,
    version: &str,
    components: &Components,
    prepared: &mut Prepared,
    job: &Job,
    installing: bool,
) -> Result<()> {
    if components.loader.is_empty()
        || !components
            .loader
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
    {
        return Err(invalid());
    }
    let (kind, name, endpoint) = if components.kind == "quilt" {
        ("quilt", "Quilt", "https://meta.quiltmc.org/v3")
    } else {
        ("fabric", "Fabric", "https://meta.fabricmc.net/v2")
    };
    let message = format!("正在解析 {name}");
    job.begin_download([(message.clone(), None)], false);
    job.file_started(&message);
    let profile_id = format!("{kind}-loader-{}-{version}", components.loader);
    let relative = format!("cache/{kind}-{version}-{}.json", components.loader);
    let profile_path = storage::within(data_root, &relative)?;
    let legacy_path = storage::within(game, ".hydrogen/fabric.json")?;
    let legacy: Value = if kind == "fabric" && legacy_path.exists() {
        storage::read_json(&legacy_path)?
    } else {
        Value::Null
    };
    let profile: Value = if profile_path.exists() {
        storage::read_json(&profile_path)?
    } else if legacy["schemaVersion"] == 1
        && legacy["gameVersion"] == version
        && legacy["profile"]["id"] == profile_id
    {
        storage::write_json(&profile_path, &legacy["profile"])?;
        legacy["profile"].clone()
    } else {
        if !settings.enabled("minecraft.download") {
            return Err(ApiError::new(
                "DOWNLOADS_DISABLED",
                format!("缺少已缓存的 {name} 启动配置。"),
            ));
        }
        let profile = net::json(
            client,
            &format!(
                "{endpoint}/versions/loader/{version}/{}/profile/json",
                components.loader
            ),
        )
        .await?;
        storage::write_json(&profile_path, &profile)?;
        profile
    };
    if profile["id"] != profile_id || profile["inheritsFrom"] != version {
        return Err(ApiError::new(
            "FABRIC_PROFILE_MISMATCH",
            format!("{name} 启动配置与此实例不匹配。"),
        ));
    }
    let mut extra = Vec::new();
    let mut downloads = Vec::new();
    let mut manifest_libraries = Vec::new();
    for library in profile["libraries"].as_array().ok_or_else(invalid)? {
        let name = text(library, "name")?;
        let parts: Vec<_> = name.split(':').collect();
        if parts.len() != 3
            || parts.iter().any(|p| {
                p.is_empty()
                    || !p
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
            })
        {
            return Err(invalid());
        }
        let path = format!(
            "{}/{}/{}/{}-{}.jar",
            parts[0].replace('.', "/"),
            parts[1],
            parts[2],
            parts[1],
            parts[2]
        );
        let base = library["url"]
            .as_str()
            .unwrap_or("https://maven.fabricmc.net/");
        let url = format!("{}{path}", base.trim_end_matches('/').to_owned() + "/");
        let hash_path = storage::within(
            data_root,
            &format!("cache/fabric-hashes/{}.sha1", {
                use sha1::{Digest, Sha1};
                format!("{:x}", Sha1::digest(url.as_bytes()))
            }),
        )?;
        let hash = if hash_path.exists() {
            std::fs::read_to_string(&hash_path)?
        } else if let Some(hash) = legacy["libraries"]
            .as_array()
            .and_then(|a| a.iter().find(|file| file["url"] == url))
            .and_then(|file| file["sha1"].as_str())
            .filter(|s| net::valid_hash(s))
        {
            storage::atomic_write(&hash_path, hash.as_bytes())?;
            hash.to_string()
        } else {
            if !settings.enabled("minecraft.download") {
                return Err(ApiError::new(
                    "DOWNLOADS_DISABLED",
                    format!("{name} 依赖缺少缓存校验信息。"),
                ));
            }
            let bytes = net::bytes(client, &format!("{url}.sha1"), 1024).await?;
            let value = String::from_utf8(bytes)
                .map_err(|_| invalid())?
                .split_whitespace()
                .next()
                .ok_or_else(invalid)?
                .to_string();
            if !net::valid_hash(&value) {
                return Err(invalid());
            }
            storage::atomic_write(&hash_path, value.as_bytes())?;
            value
        };
        let relative = format!("libraries/{path}");
        extra.push(storage::within(game_root, &relative)?);
        manifest_libraries.push(json!({"name":name,"downloads":{"artifact":{"path":path,"url":url,"sha1":hash.trim()}}}));
        downloads.push(FileSpec {
            relative,
            url,
            hash: hash.trim().into(),
            size: None,
        });
    }
    if installing && let Some(api_id) = &components.fabric_api {
        if !api_id.bytes().all(|c| c.is_ascii_alphanumeric()) {
            return Err(invalid());
        }
        let api = net::json(
            client,
            &format!("https://api.modrinth.com/v2/version/{api_id}"),
        )
        .await?;
        if !api["game_versions"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == version))
            || !api["loaders"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v == "fabric"))
        {
            return Err(ApiError::new(
                "INCOMPATIBLE_MOD",
                "Fabric API 与此实例不兼容。",
            ));
        }
        let file = api["files"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .find(|v| v["primary"] == true)
            .or_else(|| api["files"].as_array()?.first())
            .ok_or_else(invalid)?;
        downloads.push(FileSpec {
            relative: format!("mods/{}", text(file, "filename")?),
            url: text(file, "url")?.into(),
            hash: text(&file["hashes"], "sha1")?.into(),
            size: file["size"].as_u64(),
        });
    }
    job.begin_download(
        downloads.iter().map(|file| (file.relative.clone(), None)),
        true,
    );
    for file in downloads {
        net::ensure(
            client,
            if file.relative.starts_with("libraries/") {
                game_root
            } else {
                game
            },
            &file.relative,
            &file.url,
            &file.hash,
            file.size,
            false,
            settings.enabled("minecraft.download"),
            job,
        )
        .await?;
    }
    prepared.classpath.splice(0..0, extra);
    prepared.metadata["libraries"]
        .as_array_mut()
        .ok_or_else(invalid)?
        .extend(manifest_libraries);
    prepared.metadata["mainClass"] = profile["mainClass"].clone();
    if let Some(args) = profile["arguments"].as_object() {
        for key in ["game", "jvm"] {
            if let Some(extra) = args.get(key).and_then(Value::as_array) {
                prepared.metadata["arguments"][key]
                    .as_array_mut()
                    .ok_or_else(invalid)?
                    .extend(extra.clone());
            }
        }
    }
    Ok(())
}
