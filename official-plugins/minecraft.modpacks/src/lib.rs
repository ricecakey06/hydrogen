mod archive;
mod sources;

use archive::{
    Format, Index, Loader, Manifest, Minecraft, Pack, add_size, content_path, invalid, zip_error,
};
use hydrogen_plugin_services::{
    PluginHost, Settings,
    contracts::Result,
    curseforge,
    http::Client,
    jobs::{Job, Jobs},
    storage, text,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

struct Entry {
    path: String,
    size: u64,
}

fn enabled_mod(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path.starts_with("mods/") && path.ends_with(".jar")
}

fn recommended(path: &str) -> bool {
    matches!(
        path.to_ascii_lowercase().as_str(),
        "config"
            | "mods"
            | "defaultconfigs"
            | "kubejs"
            | "scripts"
            | "fancymenu_data"
            | "patchouli_books"
            | "tlm_custom_pack"
            | "global_packs"
            | "datapacks"
    )
}

fn parent_directories(path: &str) -> impl Iterator<Item = &str> {
    let parent = path
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or(".");
    std::iter::successors(Some(parent), |parent| {
        parent.rsplit_once('/').map(|(path, _)| path)
    })
}

#[derive(Serialize)]
struct Directory<'a> {
    path: &'a str,
    files: usize,
    size: u64,
    selected: bool,
}

fn directories(entries: &[Entry]) -> Vec<Directory<'_>> {
    let mut counts = BTreeMap::new();
    for entry in entries {
        for path in parent_directories(&entry.path) {
            let (files, size) = counts.entry(path).or_insert((0, 0_u64));
            *files += 1;
            *size = size.saturating_add(entry.size);
        }
    }
    counts
        .into_iter()
        .map(|(path, (files, size))| Directory {
            path,
            files,
            size,
            selected: recommended(path),
        })
        .collect()
}

async fn list(game: &Path, directory: &str, job: &Job) -> Result<Vec<Entry>> {
    let mut result = Vec::new();
    let mut pending = vec![game.to_owned()];
    let mut visited = 0;
    while let Some(dir) = pending.pop() {
        if !dir.exists() {
            continue;
        }
        for entry in fs::read_dir(dir)? {
            job.check()?;
            visited += 1;
            if visited > archive::MAX_FILES {
                return Err(invalid("实例目录超出可选择的文件数限制。"));
            }
            if visited % 128 == 0 {
                tokio::task::yield_now().await;
            }
            let entry = entry?;
            let path = entry.path();
            let relative = path
                .strip_prefix(game)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if !content_path(&relative, directory) || storage::linked(&path) {
                continue;
            }
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() {
                let size = entry.metadata()?.len();
                result.push(Entry {
                    path: relative,
                    size,
                });
            }
        }
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Export {
    format: Format,
    name: String,
    version: String,
    directories: HashSet<String>,
    bundle_unmatched: bool,
}

fn loader_id(components: &archive::Components, format: Format) -> String {
    if format == Format::Modrinth {
        match components.kind.as_str() {
            "fabric" | "quilt" => format!("{}-loader", components.kind),
            _ => components.kind.clone(),
        }
    } else {
        format!("{}-{}", components.kind, components.loader)
    }
}

struct ExportLocation {
    game: PathBuf,
    directory: String,
    destination: PathBuf,
    game_version: String,
    components: Option<archive::Components>,
}

async fn export(
    client: &Client,
    location: ExportLocation,
    options: Export,
    job: &Job,
) -> Result<Value> {
    let ExportLocation {
        game,
        directory,
        destination,
        game_version,
        components,
    } = location;
    let Export {
        format,
        name,
        version,
        directories,
        bundle_unmatched,
    } = options;
    let (name, version) = (name.trim(), version.trim());
    let extension = if format == Format::Modrinth {
        "mrpack"
    } else {
        "zip"
    };
    let filename = format!("{name}-{version}.{extension}");
    if name.is_empty()
        || version.is_empty()
        || name.chars().count() > 64
        || version.chars().count() > 64
        || !archive::portable_path(&filename)
        || filename.contains('/')
    {
        return Err(invalid(
            "整合包名称和版本应为 1–64 个字符，且不能含有路径字符。",
        ));
    }
    let output = storage::within(&destination, &filename)?;
    if output.exists() {
        return Err(invalid("目标整合包已存在，请更改名称、版本或保存目录。"));
    }
    if directories.len() > archive::MAX_FILES {
        return Err(invalid("选中的目录数量超出整合包限制。"));
    }
    for path in &directories {
        if path != "."
            && (!content_path(path, &directory) || !storage::within(&game, path)?.is_dir())
        {
            return Err(invalid(format!("选中的实例目录无效或已移动（{path}）。")));
        }
    }

    let api_key = if format == Format::Curseforge {
        curseforge::api_key()
    } else {
        None
    };
    let fingerprint = format == Format::Curseforge && api_key.is_some();
    let mut files = BTreeMap::new();
    let mut candidates = BTreeMap::new();
    let mut seen = HashSet::new();
    let mut total = 0;
    job.phase("resolving", "正在匹配整合包模组版本…");
    for Entry { path, size } in list(&game, &directory, job).await? {
        if !parent_directories(&path).any(|parent| directories.contains(parent)) {
            continue;
        }
        if !seen.insert(path.to_lowercase()) {
            return Err(invalid(format!("选中的实例文件路径冲突（{path}）。")));
        }
        let source = storage::within(&game, &path)?;
        add_size(&mut total, size)?;
        if enabled_mod(&path)
            && (format == Format::Modrinth || fingerprint && path.matches('/').count() == 1)
        {
            let sum = sources::checksums(&source, fingerprint, job).await?;
            if size != sum.size {
                return Err(invalid("实例文件已更改，请重新选择并导出。"));
            }
            candidates.insert(path.clone(), sum);
        }
        files.insert(path, (source, size));
        tokio::task::yield_now().await;
    }

    let manifest = match format {
        Format::Modrinth => {
            let matched = sources::modrinth_matches(client, &candidates).await?;
            for file in &matched {
                files.remove(&file.path);
            }
            let mut dependencies = BTreeMap::from([("minecraft".into(), game_version)]);
            if let Some(loader) = components {
                dependencies.insert(loader_id(&loader, format), loader.loader);
            }
            serde_json::to_vec_pretty(&Index {
                format_version: 1,
                game: "minecraft".into(),
                name: name.into(),
                version_id: version.into(),
                files: matched,
                dependencies,
            })?
        }
        Format::Curseforge => {
            let matched = if let Some(key) = api_key.as_deref() {
                sources::curse_matches(client, &candidates, key).await?
            } else {
                BTreeMap::new()
            };
            for path in matched.keys() {
                files.remove(path);
            }
            let mod_loaders = components
                .iter()
                .map(|loader| Loader {
                    id: loader_id(loader, format),
                    primary: true,
                })
                .collect();
            serde_json::to_vec_pretty(&Manifest {
                manifest_type: "minecraftModpack".into(),
                manifest_version: 1,
                name: name.into(),
                version: version.into(),
                author: String::new(),
                minecraft: Minecraft {
                    version: game_version,
                    mod_loaders,
                },
                files: matched.into_values().collect(),
                overrides: "overrides".into(),
            })?
        }
    };
    let unmatched: Vec<_> = files.keys().filter(|path| enabled_mod(path)).collect();
    if !bundle_unmatched && !unmatched.is_empty() {
        return Err(invalid(format!(
            "有 {} 个模组未匹配到所选平台。请取消选择，或允许将这些模组作为覆盖文件打包（{}）。",
            unmatched.len(),
            unmatched
                .iter()
                .take(3)
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join("、")
        )));
    }
    if manifest.len() as u64 > archive::MAX_MANIFEST {
        return Err(invalid("整合包清单超过大小限制。"));
    }

    job.phase("packing", "正在写入整合包…");
    let temp = storage::within(
        &destination,
        &format!(".hydrogen-pack-{}.tmp", uuid::Uuid::new_v4().simple()),
    )?;
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    struct Temporary(PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let _cleanup = Temporary(temp.clone());
    let mut zip = ZipWriter::new(file);
    let zip_options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file(format.manifest(), zip_options)
        .map_err(zip_error)?;
    zip.write_all(&manifest)?;
    zip.add_directory("overrides/", zip_options)
        .map_err(zip_error)?;
    for (relative, (path, size)) in files {
        job.check()?;
        let mut input = File::open(storage::within(&game, &relative)?)?;
        let stored = relative.to_ascii_lowercase().ends_with(".jar")
            || relative.to_ascii_lowercase().ends_with(".zip");
        let options = if stored {
            zip_options.compression_method(CompressionMethod::Stored)
        } else {
            zip_options
        };
        zip.start_file(format!("overrides/{relative}"), options)
            .map_err(zip_error)?;
        if input.metadata()?.len() != size
            || archive::copy(&mut input, &mut zip, size, job).await? != size
        {
            return Err(invalid(format!(
                "导出时实例文件已更改（{}）。",
                path.display()
            )));
        }
    }
    zip.finish().map_err(zip_error)?.sync_all()?;
    job.check()?;
    storage::move_new(&temp, &output)?;
    Ok(json!(output))
}

struct State {
    jobs: Jobs,
}
impl Default for State {
    fn default() -> Self {
        Self {
            jobs: Jobs::with_limit(4),
        }
    }
}
impl State {
    fn call(&mut self, request: &Value) -> Result<Value> {
        let params = &request["params"];
        let method = request["method"].as_str().unwrap_or_default();
        match method {
            "plugin.describe" => return Ok(json!({"id":"minecraft.modpacks","abiVersion":1})),
            "plugin.stop" => {
                self.jobs.clear();
                return Ok(Value::Null);
            }
            "poll" => return self.jobs.poll(text(params, "handle")?),
            "cancel" => {
                self.jobs.cancel(text(params, "handle")?);
                return Ok(Value::Null);
            }
            "preview" => {
                let (_, pack) = Pack::open(Path::new(text(params, "source")?))?;
                return Ok(serde_json::to_value(pack.preview()?)?);
            }
            "list" | "prepare" | "export" => {}
            _ => return Err(invalid("未知整合包服务方法。")),
        }
        let client = Client::new(&PluginHost, &Settings::default())?;
        let job = Arc::new(Job::default());
        let method = method.to_owned();
        let params = params.clone();
        self.jobs.begin(job.clone(), async move {
            let directory = text(&params, "directory")?;
            match method.as_str() {
                "list" => {
                    let entries = list(Path::new(text(&params, "game")?), directory, &job).await?;
                    Ok(serde_json::to_value(directories(&entries))?)
                }
                "prepare" => {
                    sources::prepare(&client, serde_json::from_value(params.clone())?, &job).await
                }
                _ => {
                    let location = ExportLocation {
                        game: PathBuf::from(text(&params, "game")?),
                        directory: directory.into(),
                        destination: PathBuf::from(text(&params, "destination")?),
                        game_version: text(&params, "gameVersion")?.into(),
                        components: serde_json::from_value(params["components"].clone())?,
                    };
                    export(
                        &client,
                        location,
                        serde_json::from_value(params["options"].clone())?,
                        &job,
                    )
                    .await
                }
            }
        })
    }
}
fn dispatch(state: &mut State, request: &mut Value) -> hydrogen_plugin_sdk::Result {
    state
        .call(request)
        .map_err(|error| (error.code, error.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_round_trip_selected_configs_and_keep_loader_version() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let game = root.join("game");
        let paths = [
            "config/settings.toml",
            "kubejs/startup_scripts/test.js",
            "fancymenu_data/layout.txt",
            "patchouli_books/book/book.json",
            "tlm_custom_pack/model.json",
            "custom/wanted/extra.txt",
        ];
        for path in
            paths
                .into_iter()
                .chain(["saves/world/level.dat", "custom/leave.txt", "root.txt"])
        {
            let file = game.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, path).unwrap();
        }
        let client = Client::new(&PluginHost, &Settings::default()).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            let job = Job::default();
            let available = list(&game, "game", &job).await.unwrap();
            assert_eq!(
                directories(&available)
                    .iter()
                    .filter(|folder| folder.selected)
                    .count(),
                5
            );
            for (format, extension) in [(Format::Modrinth, "mrpack"), (Format::Curseforge, "zip")] {
                let location = ExportLocation {
                    game: game.clone(),
                    directory: "game".into(),
                    destination: root.clone(),
                    game_version: "1.21.1".into(),
                    components: Some(archive::Components {
                        kind: "fabric".into(),
                        loader: "0.16.10".into(),
                    }),
                };
                let options = Export {
                    format,
                    name: "Test pack".into(),
                    version: "1.0".into(),
                    directories: [
                        "config",
                        "kubejs",
                        "fancymenu_data",
                        "patchouli_books",
                        "tlm_custom_pack",
                        "custom/wanted",
                        ".",
                    ]
                    .map(str::to_owned)
                    .into(),
                    bundle_unmatched: false,
                };
                export(&client, location, options, &job).await.unwrap();
                let source = root.join(format!("Test pack-1.0.{extension}"));
                let (_, pack) = Pack::open(&source).unwrap();
                let preview = serde_json::to_value(pack.preview().unwrap()).unwrap();
                assert_eq!(preview["components"]["loader"], "0.16.10");
                let stage = root.join(format!("stage-{extension}"));
                fs::create_dir(&stage).unwrap();
                let options = serde_json::from_value(json!({
                    "source":source,"stage":stage,"directory":"Imported","preview":preview,
                    "downloadsEnabled":true,"optional":false
                }))
                .unwrap();
                sources::prepare(&client, options, &job).await.unwrap();
                for path in paths {
                    assert_eq!(fs::read_to_string(stage.join(path)).unwrap(), path);
                }
                assert!(!stage.join("saves").exists());
                assert!(!stage.join("custom/leave.txt").exists());
                assert!(stage.join("root.txt").exists());
            }
        });
    }

    #[test]
    fn client_override_wins_and_archive_cannot_escape_the_instance() {
        let root = tempfile::tempdir().unwrap();
        for bad in [false, true] {
            let path = root.path().join("client.mrpack");
            let mut zip = ZipWriter::new(File::create(&path).unwrap());
            let options = SimpleFileOptions::default();
            zip.start_file(Format::Modrinth.manifest(), options)
                .unwrap();
            zip.write_all(br#"{"formatVersion":1,"game":"minecraft","name":"Pack","versionId":"1","files":[],"dependencies":{"minecraft":"1.21.1"}}"#).unwrap();
            zip.start_file("overrides/config/test.json", options)
                .unwrap();
            zip.start_file("server-overrides/config/server.json", options)
                .unwrap();
            zip.start_file("client-overrides/config/test.json", options)
                .unwrap();
            zip.write_all(b"client").unwrap();
            if bad {
                zip.start_file("overrides/../outside.txt", options).unwrap();
            }
            zip.finish().unwrap();
            let (mut zip, pack) = Pack::open(&path).unwrap();
            let overrides = pack.overrides(&mut zip, "Pack");
            if bad {
                assert!(overrides.is_err());
            } else {
                let entries = overrides.unwrap();
                assert_eq!(entries.len(), 1);
                assert_eq!(entries["config/test.json"].size, 6);
            }
        }
        assert!(!content_path("My Pack.JSON", "my pack"));
        assert!(!content_path("config/COM¹.txt", "my pack"));
    }
}
