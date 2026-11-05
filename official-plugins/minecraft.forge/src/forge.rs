use base64::{Engine, engine::general_purpose::STANDARD};
use hydrogen_plugin_services::{
    Settings,
    contracts::{ApiError, Result},
    invoke,
    jobs::{Job, service_task},
    net::{self, Client},
    storage, text,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
};

fn invalid() -> ApiError {
    ApiError::new("FORGE_METADATA", "Forge / NeoForge 安装元数据无效。")
}
fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 150
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
}
fn repository(kind: &str, game: &str) -> Result<(&'static str, &'static str)> {
    match kind {
        "forge" => Ok((
            "https://maven.minecraftforge.net",
            "net.minecraftforge:forge",
        )),
        "neoforge" if game == "1.20.1" => Ok((
            "https://maven.neoforged.net/releases",
            "net.neoforged:forge",
        )),
        "neoforge" => Ok((
            "https://maven.neoforged.net/releases",
            "net.neoforged:neoforge",
        )),
        _ => Err(invalid()),
    }
}
fn neo_prefix(game: &str) -> Option<String> {
    let parts: Vec<_> = game.split('.').collect();
    let numbers: Vec<u32> = parts
        .iter()
        .map(|s| s.parse())
        .collect::<std::result::Result<_, _>>()
        .ok()?;
    match numbers.as_slice() {
        [1, minor, patch] if *minor >= 20 => Some(format!("{minor}.{patch}.")),
        [1, minor] if *minor >= 20 => Some(format!("{minor}.0.")),
        [year, minor] if *year >= 26 => Some(format!("{year}.{minor}.0.")),
        [year, minor, patch] if *year >= 26 => Some(format!("{year}.{minor}.{patch}.")),
        _ => None,
    }
}
async fn forge_catalog(client: &Client, game: &str) -> Result<Value> {
    if !token(game) {
        return Ok(json!([]));
    }
    net::json(
        client,
        &format!("https://bmclapi2.bangbang93.com/forge/minecraft/{game}"),
    )
    .await
}
fn forge_coordinate(entry: &Value) -> Result<String> {
    let mut coordinate = format!("{}-{}", text(entry, "mcversion")?, text(entry, "version")?);
    if let Some(branch) = entry["branch"].as_str().filter(|b| !b.is_empty()) {
        coordinate.push('-');
        coordinate.push_str(branch);
    }
    if !token(&coordinate) {
        return Err(invalid());
    }
    Ok(coordinate)
}
fn installer_file(entry: &Value) -> Option<&Value> {
    entry["files"]
        .as_array()?
        .iter()
        .find(|f| f["category"] == "installer" && f["format"] == "jar")
}
pub async fn options(
    client: &Client,
    settings: &Settings,
    game: &str,
    kind: &str,
) -> Result<Value> {
    let (repo, artifact) = repository(kind, game)?;
    let prefix = if artifact.ends_with(":neoforge") {
        let Some(prefix) = neo_prefix(game) else {
            return Ok(json!([]));
        };
        prefix
    } else {
        format!("{game}-")
    };
    let path = artifact.replace(['.', ':'], "/");
    let url = net::source_url(
        client,
        &format!("{repo}/{path}/maven-metadata.xml"),
        settings.download_source == "bmclapi",
    )?;
    let xml = if kind == "forge" && settings.download_source == "bmclapi" {
        None
    } else {
        net::bytes(client, &url, 8 * 1024 * 1024).await.ok()
    };
    let Some(xml) = xml else {
        if kind != "forge" {
            return Err(ApiError::new(
                "NETWORK",
                "无法读取 NeoForge 版本目录，请重试。",
            ));
        }
        let catalog = forge_catalog(client, game).await?;
        let mut versions = catalog
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .filter(|v| installer_file(v).is_some())
            .map(forge_coordinate)
            .collect::<Result<Vec<_>>>()?;
        let digits = regex::Regex::new(r"\d+").unwrap();
        versions.sort_by_cached_key(|v| {
            digits
                .find_iter(v)
                .map(|n| n.as_str().parse::<u32>().unwrap_or(0))
                .collect::<Vec<_>>()
        });
        return Ok(json!(
            versions
                .into_iter()
                .rev()
                .map(|id| json!({"id":id,"stable":true}))
                .collect::<Vec<_>>()
        ));
    };
    let xml = String::from_utf8(xml).map_err(|_| invalid())?;
    let mut versions: Vec<String> = regex::Regex::new(r"<version>\s*([^<]+?)\s*</version>")
        .unwrap()
        .captures_iter(&xml)
        .map(|c| c[1].to_string())
        .filter(|v| token(v) && v.starts_with(&prefix))
        .collect();
    let digits = regex::Regex::new(r"\d+").unwrap();
    versions.sort_by_cached_key(|v| {
        digits
            .find_iter(v)
            .map(|n| n.as_str().parse::<u32>().unwrap_or(0))
            .collect::<Vec<_>>()
    });
    versions.dedup();
    Ok(json!(
        versions
            .into_iter()
            .rev()
            .map(|id| json!({"stable":!id.contains("beta") && !id.contains("alpha"),"id":id}))
            .collect::<Vec<_>>()
    ))
}

fn maven_path(coordinate: &str) -> Result<String> {
    let (coordinate, ext) = coordinate.split_once('@').unwrap_or((coordinate, "jar"));
    let parts: Vec<_> = coordinate.split(':').collect();
    if !(3..=4).contains(&parts.len())
        || !token(ext)
        || parts.iter().any(|p| !token(p) || *p == "..")
    {
        return Err(invalid());
    }
    Ok(format!(
        "{}/{}/{}/{}-{}{}.{}",
        parts[0].replace('.', "/"),
        parts[1],
        parts[2],
        parts[1],
        parts[2],
        parts.get(3).map(|s| format!("-{s}")).unwrap_or_default(),
        ext
    ))
}
fn artifact(root: &Path, coordinate: &str) -> Result<PathBuf> {
    storage::within(root, &format!("libraries/{}", maven_path(coordinate)?))
}
fn archive_bytes(zip: &mut zip::ZipArchive<fs::File>, name: &str, limit: u64) -> Result<Vec<u8>> {
    let mut file = zip
        .by_name(name.trim_start_matches('/'))
        .map_err(|_| invalid())?;
    if file.size() > limit || file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.by_ref().take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid());
    }
    Ok(bytes)
}
fn unpack(zip: &mut zip::ZipArchive<fs::File>, root: &Path, legacy: &Value) -> Result<()> {
    if zip.len() > 20_000 {
        return Err(invalid());
    }
    let mut total = 0;
    for i in 0..zip.len() {
        let name = zip.by_index(i).map_err(|_| invalid())?.name().to_owned();
        if let Some(relative) = name.strip_prefix("maven/") {
            if name.ends_with('/') {
                continue;
            }
            let path = storage::within(root, &format!("libraries/{relative}"))?;
            let bytes = archive_bytes(zip, &name, 256 * 1024 * 1024)?;
            total += bytes.len();
            if total > 512 * 1024 * 1024 {
                return Err(invalid());
            }
            storage::atomic_write(&path, &bytes)?;
        }
    }
    if let (Some(file), Some(coordinate)) = (legacy["filePath"].as_str(), legacy["path"].as_str()) {
        let bytes = archive_bytes(zip, file, 256 * 1024 * 1024)?;
        storage::atomic_write(&artifact(root, coordinate)?, &bytes)?;
    }
    Ok(())
}
fn merge(base: &Value, profile: &Value, game: &str) -> Result<Value> {
    let mut result = base.clone();
    result["mainClass"] = profile["mainClass"].clone();
    if let Some(args) = profile["minecraftArguments"].as_str() {
        result["minecraftArguments"] = json!(args);
        result["arguments"]["game"] = json!([]);
    }
    for kind in ["game", "jvm"] {
        if let Some(args) = profile["arguments"][kind].as_array() {
            result["arguments"][kind]
                .as_array_mut()
                .ok_or_else(invalid)?
                .extend(args.clone());
        }
    }
    // Loader libraries replace vanilla versions of the same artifact/classifier.
    let key = |lib: &Value| {
        let parts: Vec<_> = lib["name"].as_str().unwrap_or("").split(':').collect();
        format!(
            "{}:{}:{}",
            parts.first().unwrap_or(&""),
            parts.get(1).unwrap_or(&""),
            parts.get(3).unwrap_or(&"")
        )
    };
    let extra = profile["libraries"].as_array().ok_or_else(invalid)?;
    let mut libraries = extra.clone();
    libraries.extend(
        base["libraries"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .filter(|lib| !extra.iter().any(|other| key(lib) == key(other)))
            .cloned(),
    );
    result["libraries"] = json!(libraries);
    result["clientVersion"] = json!(game);
    invoke(
        "minecraft.instances",
        "minecraft.instances",
        "normalize",
        json!({"metadata":result,"gameVersion":game}),
    )
}
async fn libraries(
    client: &Client,
    settings: &Settings,
    root: &Path,
    libraries: &[Value],
    job: &Job,
) -> Result<()> {
    for lib in libraries {
        job.check()?;
        if lib["clientreq"] == false || client.service("minecraft.content", "allows", json!({"owner":lib,"architecture":if cfg!(target_arch="aarch64"){"arm64"}else{"x64"},"customResolution":false}))? != true { continue; }
        let a = &lib["downloads"]["artifact"];
        let path = a["path"]
            .as_str()
            .map(String::from)
            .unwrap_or(maven_path(text(lib, "name")?)?);
        let relative = format!("libraries/{path}");
        let destination = storage::within(root, &relative)?;
        // An empty artifact URL denotes an output generated by the installer.
        if a["url"].as_str() == Some("") {
            continue;
        }
        let url = a["url"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| {
                format!(
                    "{}/{path}",
                    lib["url"]
                        .as_str()
                        .unwrap_or("https://libraries.minecraft.net")
                        .trim_end_matches('/')
                )
            })
            .replacen("http://", "https://", 1);
        let hash = a["sha1"]
            .as_str()
            .or_else(|| lib["checksums"].as_array()?.first()?.as_str());
        if destination.is_file() && hash.is_none() {
            continue;
        }
        let size = a["size"].as_u64().filter(|n| *n > 0);
        net::ensure_game_file(
            client,
            root,
            &relative,
            &url,
            hash,
            size,
            settings.download_source == "bmclapi",
            settings.enabled("minecraft.download"),
            job,
        )
        .await?;
    }
    Ok(())
}

fn expand(input: &str, data: &BTreeMap<String, String>, root: &Path) -> Result<String> {
    if let Some(c) = input.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return Ok(artifact(root, c)?.to_string_lossy().into());
    }
    let tokens = regex::Regex::new(r"\{([^{}]+)\}").unwrap();
    let mut output = input.to_owned();
    for c in tokens.captures_iter(input) {
        output = output.replace(&c[0], data.get(&c[1]).ok_or_else(invalid)?);
    }
    Ok(output)
}
fn main_class(path: &Path) -> Result<String> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path)?).map_err(|_| invalid())?;
    let bytes = archive_bytes(&mut zip, "META-INF/MANIFEST.MF", 1024 * 1024)?;
    let manifest = String::from_utf8(bytes)
        .map_err(|_| invalid())?
        .replace("\r\n", "\n")
        .replace("\n ", "");
    manifest
        .lines()
        .find_map(|line| {
            line.strip_prefix("Main-Class:")
                .map(|s| s.trim().to_owned())
        })
        .filter(|s| !s.is_empty())
        .ok_or_else(invalid)
}
struct Processor(String);
impl Drop for Processor {
    fn drop(&mut self) {
        let _ = invoke("process", "process", "kill", json!({"handle":self.0}));
    }
}
struct Work(PathBuf);
impl Drop for Work {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
async fn run(
    java: &str,
    args: Vec<String>,
    directory: &Path,
    log: &mut fs::File,
    job: &Job,
) -> Result<()> {
    let child = invoke(
        "process",
        "process",
        "spawn",
        json!({"executable":java,"args":args,"directory":directory}),
    )?;
    let child = Processor(text(&child, "handle")?.into());
    let mut written = 0;
    let start = std::time::Instant::now();
    loop {
        job.check()?;
        let poll = invoke("process", "process", "poll", json!({"handle":child.0}))?;
        for stream in ["stdout", "stderr"] {
            let bytes = STANDARD
                .decode(text(&poll, stream)?)
                .map_err(|_| invalid())?;
            if written < 8 * 1024 * 1024 {
                log.write_all(&bytes)?;
                written += bytes.len();
            }
        }
        if let Some(code) = poll["exitCode"].as_i64() {
            if code != 0 {
                return Err(ApiError::new(
                    "FORGE_PROCESSOR",
                    "Forge 安装处理失败，请查看数据目录 logs 中的安装日志。",
                ));
            }
            if poll["stdoutEof"] == true && poll["stderrEof"] == true {
                return Ok(());
            }
        }
        if start.elapsed() > std::time::Duration::from_secs(20 * 60) {
            return Err(ApiError::new(
                "FORGE_PROCESSOR_TIMEOUT",
                "Forge 安装处理超时。",
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
#[allow(clippy::too_many_arguments)]
async fn processors(
    client: &Client,
    settings: &Settings,
    p: &Value,
    installer: &Path,
    zip: &mut zip::ZipArchive<fs::File>,
    profile: &Value,
    work: &Path,
    job: &Job,
) -> Result<()> {
    let Some(processors) = profile["processors"].as_array() else {
        return Ok(());
    };
    if !processors.iter().any(|p| {
        p["sides"]
            .as_array()
            .is_none_or(|s| s.iter().any(|s| s == "client"))
    }) {
        return Ok(());
    }
    let root = Path::new(text(p, "gameRoot")?);
    let base = &p["prepared"]["metadata"];
    let required = base["javaVersion"]["majorVersion"].as_u64().unwrap_or(8);
    let java = if let Some(path) = p["settings"]["java"].as_str() {
        invoke(
            "java.runtime",
            "java.runtime",
            "inspect",
            json!({"path":path}),
        )?
    } else {
        invoke("java.runtime", "java.runtime", "scan", json!({"saved":[]}))?
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .filter(|j| j["major"].as_u64() == Some(required))
            .max_by_key(|j| j["architecture"] == "x64")
            .cloned()
            .ok_or_else(|| {
                ApiError::new(
                    "JAVA_MISSING",
                    format!("安装此版本需要 Java {required}，请在设置中选择 Java。"),
                )
            })?
    };
    if java["major"].as_u64() != Some(required) {
        return Err(ApiError::new(
            "JAVA_INCOMPATIBLE",
            format!("安装此版本需要 Java {required}。"),
        ));
    }
    let mut data = BTreeMap::new();
    if let Some(values) = profile["data"].as_object() {
        for (key, value) in values {
            let value = text(value, "client")?;
            let resolved = if let Some(literal) =
                value.strip_prefix('\'').and_then(|s| s.strip_suffix('\''))
            {
                literal.into()
            } else if value.starts_with('[') {
                expand(value, &data, root)?
            } else {
                let destination = storage::within(work, value.trim_start_matches('/'))?;
                storage::atomic_write(
                    &destination,
                    &archive_bytes(zip, value, 256 * 1024 * 1024)?,
                )?;
                destination.to_string_lossy().into_owned()
            };
            data.insert(key.clone(), resolved);
        }
    }
    let source = p["prepared"]["classpath"]
        .as_array()
        .and_then(|a| a.last())
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let minecraft = work.join("minecraft.jar");
    fs::copy(source, &minecraft)?;
    for (key, value) in [
        ("SIDE", "client".into()),
        ("MINECRAFT_VERSION", text(p, "version")?.into()),
        ("MINECRAFT_JAR", minecraft.to_string_lossy().into_owned()),
        ("ROOT", root.to_string_lossy().into_owned()),
        ("INSTALLER", installer.to_string_lossy().into_owned()),
        (
            "LIBRARY_DIR",
            root.join("libraries").to_string_lossy().into_owned(),
        ),
    ] {
        data.insert(key.into(), value);
    }
    let log_path = storage::within(
        Path::new(text(p, "root")?),
        &format!("logs/install-forge-{}.log", uuid::Uuid::new_v4().simple()),
    )?;
    fs::create_dir_all(log_path.parent().unwrap())?;
    let mut log = fs::File::create(log_path)?;
    for (index, processor) in processors.iter().enumerate() {
        if processor["sides"]
            .as_array()
            .is_some_and(|s| !s.iter().any(|s| s == "client"))
        {
            continue;
        }
        job.phase(
            "preparing",
            format!(
                "正在处理 Forge / NeoForge 安装步骤 {}/{}…",
                index + 1,
                processors.len()
            ),
        );
        let raw_args = processor["args"].as_array().ok_or_else(invalid)?;
        // Download mappings with the launcher's verification and mirror support.
        if raw_args.iter().any(|a| a == "DOWNLOAD_MOJMAPS") {
            let output = raw_args
                .windows(2)
                .find(|a| a[0] == "--output")
                .and_then(|a| a[1].as_str())
                .ok_or_else(invalid)?;
            let output = PathBuf::from(expand(output, &data, root)?);
            let relative = output
                .strip_prefix(root)
                .map_err(|_| invalid())?
                .to_string_lossy()
                .replace('\\', "/");
            let mapping = &base["downloads"]["client_mappings"];
            net::ensure_game_file(
                client,
                root,
                &relative,
                text(mapping, "url")?,
                mapping["sha1"].as_str(),
                mapping["size"].as_u64(),
                settings.download_source == "bmclapi",
                settings.enabled("minecraft.download"),
                job,
            )
            .await?;
            continue;
        }
        let jar = artifact(root, text(processor, "jar")?)?;
        let mut classpath = vec![jar.clone()];
        for c in processor["classpath"].as_array().ok_or_else(invalid)? {
            classpath.push(artifact(root, c.as_str().ok_or_else(invalid)?)?);
        }
        let cp = std::env::join_paths(classpath).map_err(|_| invalid())?;
        let mut args = vec![
            "-Xmx2G".into(),
            "-cp".into(),
            cp.to_string_lossy().into_owned(),
            main_class(&jar)?,
        ];
        for arg in raw_args {
            args.push(expand(arg.as_str().ok_or_else(invalid)?, &data, root)?);
        }
        writeln!(log, "Step {}: {}", index + 1, text(processor, "jar")?)?;
        run(text(&java, "path")?, args, work, &mut log, job).await?;
        if let Some(outputs) = processor["outputs"].as_object() {
            for (path, hash) in outputs {
                let path = PathBuf::from(expand(path, &data, root)?);
                let hash = expand(hash.as_str().ok_or_else(invalid)?, &data, root)?;
                if !path.is_file()
                    || !hash.is_empty()
                        && !net::hash(client, &path).await?.eq_ignore_ascii_case(&hash)
                {
                    return Err(ApiError::new(
                        "FORGE_OUTPUT",
                        "Forge 安装产物缺失或校验失败，可重试安装。",
                    ));
                }
            }
        }
    }
    Ok(())
}

pub async fn prepare(client: &Client, settings: &Settings, p: &Value, job: &Job) -> Result<Value> {
    // ponytail: serialize generated shared libraries; use per-coordinate locks if this becomes a bottleneck.
    static INSTALL: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    let _lock = INSTALL
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    job.check()?;
    let root = Path::new(text(p, "root")?);
    let game_root = Path::new(text(p, "gameRoot")?);
    let game = text(p, "version")?;
    let kind = text(&p["components"], "kind")?;
    let loader = text(&p["components"], "loader")?;
    if !token(loader) {
        return Err(invalid());
    }
    let (repo, artifact_name) = repository(kind, game)?;
    let coordinate = format!("{artifact_name}:{loader}:installer");
    let url = format!("{repo}/{}", maven_path(&coordinate)?);
    let hash_path = storage::within(root, &format!("cache/installers/{kind}-{loader}.jar.sha1"))?;
    let hash = if hash_path.is_file() {
        fs::read_to_string(&hash_path)?
    } else {
        if !settings.enabled("minecraft.download") {
            return Err(ApiError::new(
                "DOWNLOADS_DISABLED",
                "缺少已缓存的 Forge 安装器校验信息。",
            ));
        }
        let from_catalog = if kind == "forge" {
            forge_catalog(client, game).await.ok().and_then(|catalog| {
                catalog
                    .as_array()?
                    .iter()
                    .find(|v| forge_coordinate(v).is_ok_and(|v| v == loader))
                    .and_then(installer_file)
                    .and_then(|f| f["hash"].as_str())
                    .filter(|s| net::valid_hash(s))
                    .map(String::from)
            })
        } else {
            None
        };
        let hash = if let Some(hash) = from_catalog {
            hash
        } else {
            String::from_utf8(net::bytes(client, &format!("{url}.sha1"), 1024).await?)
                .map_err(|_| invalid())?
        };
        if !hash.split_whitespace().next().is_some_and(net::valid_hash) {
            return Err(invalid());
        }
        storage::atomic_write(&hash_path, hash.as_bytes())?;
        hash
    };
    let hash = hash.split_whitespace().next().ok_or_else(invalid)?;
    if !net::valid_hash(hash) {
        return Err(invalid());
    }
    let relative = format!("cache/installers/{kind}-{loader}.jar");
    let installer = net::ensure(
        client,
        root,
        &relative,
        &url,
        hash,
        None,
        settings.download_source == "bmclapi",
        settings.enabled("minecraft.download"),
        job,
    )
    .await?;
    let mut zip = zip::ZipArchive::new(fs::File::open(&installer)?).map_err(|_| invalid())?;
    let profile: Value = serde_json::from_slice(&archive_bytes(
        &mut zip,
        "install_profile.json",
        8 * 1024 * 1024,
    )?)?;
    let version: Value = if profile["versionInfo"].is_object() {
        profile["versionInfo"].clone()
    } else {
        serde_json::from_slice(&archive_bytes(
            &mut zip,
            text(&profile, "json")?,
            8 * 1024 * 1024,
        )?)?
    };
    if profile["minecraft"]
        .as_str()
        .or_else(|| profile["install"]["minecraft"].as_str())
        != Some(game)
        || version["inheritsFrom"]
            .as_str()
            .is_some_and(|id| id != game)
    {
        return Err(ApiError::new(
            "FORGE_VERSION_MISMATCH",
            "所选加载器与 Minecraft 版本不兼容。",
        ));
    }
    unpack(&mut zip, game_root, &profile["install"])?;
    let mut dependencies = profile["libraries"].as_array().cloned().unwrap_or_default();
    dependencies.extend(version["libraries"].as_array().ok_or_else(invalid)?.clone());
    libraries(client, settings, game_root, &dependencies, job).await?;
    let work = storage::within(
        root,
        &format!("cache/forge-work/{}", uuid::Uuid::new_v4().simple()),
    )?;
    fs::create_dir_all(&work)?;
    // Keep cancellation cleanup tied to the future, including a dropped service task.
    let work = Work(work);
    processors(
        client, settings, p, &installer, &mut zip, &profile, &work.0, job,
    )
    .await?;
    let metadata = merge(&p["prepared"]["metadata"], &version, game)?;
    job.check()?;
    service_task(
        "minecraft.preinstall",
        "prepare",
        json!({"gameRoot":game_root,"settings":settings,"metadata":metadata,
        "architecture":if cfg!(target_arch="aarch64"){"arm64"}else{"x64"}}),
        job,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coordinates_and_processor_substitutions() {
        let root = Path::new("repository");
        assert_eq!(
            maven_path("net.neoforged:neoform:1.21.1:mappings@txt").unwrap(),
            "net/neoforged/neoform/1.21.1/neoform-1.21.1-mappings.txt"
        );
        for bad in [
            "../group:lib:1",
            "group:lib:../1",
            "group:lib:1:../../x",
            "group:lib:1@../jar",
        ] {
            assert!(maven_path(bad).is_err());
        }
        assert_eq!(
            expand(
                "--side={SIDE}",
                &BTreeMap::from([("SIDE".into(), "client".into())]),
                root
            )
            .unwrap(),
            "--side=client"
        );
        assert!(expand("{UNKNOWN}", &BTreeMap::new(), root).is_err());
        assert_eq!(neo_prefix("1.21.1").as_deref(), Some("21.1."));
        assert_eq!(neo_prefix("1.21").as_deref(), Some("21.0."));
        assert_eq!(neo_prefix("26.1").as_deref(), Some("26.1.0."));
        assert!(neo_prefix("24w14a").is_none());
    }
}
