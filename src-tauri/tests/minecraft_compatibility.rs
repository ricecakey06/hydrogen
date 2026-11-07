mod common;
use hydrogen::launcher::{
    minecraft, net, repository,
    storage::{self, Settings},
    tasks::Tasks,
};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use std::{fs, io::Write, path::Path};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}
fn write(root: &Path, path: &str, bytes: &[u8]) {
    let path = storage::within(root, path).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn base(id: &str, modern: bool) -> Value {
    let mut metadata = json!({"id":id,"type":if id.contains('w') || id.contains("-pre") {"snapshot"} else {"release"},
        "mainClass":"net.minecraft.client.main.Main","libraries":[],
        "downloads":{"client":{"sha1":hash(b"client"),"size":6}},
        "assetIndex":{"id":"test","sha1":hash(br#"{"objects":{}}"#)}});
    if modern {
        metadata["arguments"] =
            json!({"game":["--username","${auth_player_name}"],"jvm":["-cp","${classpath}"]});
        metadata["javaVersion"] = json!({"majorVersion":17});
    } else {
        metadata["minecraftArguments"] = json!(
            "--username ${auth_player_name} --session ${auth_session} --assetsDir ${game_assets}"
        );
    }
    metadata
}
fn launch_args(
    client: &net::Client,
    prepared: &Value,
    settings: &Settings,
    root: &Path,
    java_major: u32,
) -> Vec<String> {
    serde_json::from_value(client.service("minecraft.launch","arguments",json!({"game":prepared,"settings":settings,
        "java":{"path":"unused","version":java_major.to_string(),"major":java_major,"architecture":"x64"},
        "session":{"name":"Test Player","uuid":"00000000000000000000000000000000","access_token":"0","xuid":"","client_id":"","expires":0,"offline":true},
        "directory":root,"natives":root.join("natives")})).unwrap()).unwrap()
}

#[tokio::test]
async fn stack_shadow_pages_follows_the_game_version_and_preserves_existing_jvm_options() {
    let temp = tempfile::tempdir().unwrap();
    let host = common::plugin_host(&temp.path().join("data"));
    let mut settings = Settings::default();
    let client = net::client(&host, &settings).unwrap();
    let root = temp.path().join("game");
    let mut prepared = json!({"metadata":{},"classpath":[root.join("client.jar")],
        "natives":[],"assets":root.join("assets"),"libraries":root.join("libraries"),"asset_index":"test"});
    let option = "-XX:StackShadowPages=32";
    for (version, name, expected) in [
        ("1.21.11", "26.3", false),
        ("26.2", "26.3", false),
        ("26.2.9", "26.3", false),
        ("26.3", "Custom modpack", true),
        ("26.3-snapshot-1", "Custom modpack", true),
        ("26.3-snapshot-10", "Custom modpack", true),
        ("26.3-pre-1", "Custom modpack", true),
        ("26.3-rc-2", "Custom modpack", true),
        ("26.3.1", "Custom modpack", true),
        ("26.10", "Custom modpack", true),
        ("27.1", "Custom modpack", true),
        ("25w45a", "26.3", false),
        ("unknown", "Custom modpack", false),
    ] {
        prepared["metadata"] = client
            .service(
                "minecraft.instances",
                "normalize",
                json!({"metadata":base(name, true),"gameVersion":version}),
            )
            .unwrap();
        let args = launch_args(&client, &prepared, &settings, &root, 25);
        let main = args
            .iter()
            .position(|arg| arg == "net.minecraft.client.main.Main")
            .unwrap();
        assert_eq!(
            args[..main].iter().filter(|arg| *arg == option).count(),
            usize::from(expected),
            "{version}"
        );
    }
    prepared["metadata"] = base("26.3", true);
    assert!(launch_args(&client, &prepared, &settings, &root, 25).contains(&option.into()));
    prepared["metadata"]["arguments"]["jvm"]
        .as_array_mut()
        .unwrap()
        .push(json!(option));
    let args = launch_args(&client, &prepared, &settings, &root, 25);
    assert_eq!(args.iter().filter(|arg| *arg == option).count(), 1);
    prepared["metadata"]["arguments"]["jvm"]
        .as_array_mut()
        .unwrap()
        .pop();
    for value in [32, 40] {
        let custom = format!("-XX:StackShadowPages={value}");
        settings.jvm_arguments = format!("-Dcustom=true {custom}");
        let args = launch_args(&client, &prepared, &settings, &root, 25);
        assert_eq!(
            args.iter()
                .filter(|arg| arg.starts_with("-XX:StackShadowPages="))
                .collect::<Vec<_>>(),
            [&custom]
        );
        assert!(args.contains(&"-Dcustom=true".into()));
    }
}

#[tokio::test]
async fn lwjgl_unsafe_agent_is_added_only_to_compatible_launches() {
    let temp = tempfile::tempdir().unwrap();
    let host = common::plugin_host(&temp.path().join("data"));
    let mut settings = Settings::default();
    let client = net::client(&host, &settings).unwrap();
    let directory = temp.path().join("game = fixture");
    let mut prepared = json!({"metadata":base("26.1.2", true),"classpath":[directory.join("client.jar")],
        "natives":[],"assets":directory.join("assets"),"libraries":directory.join("libraries"),"asset_index":"test"});
    let agent = ".hydrogen/lwjgl-unsafe-agent-2.0.jar";
    let path = directory.join(agent);
    for (major, library, expected) in [
        (24, json!({"name":"org.lwjgl:lwjgl:3.4.1"}), false),
        (
            25,
            json!({"name":"org.lwjgl:lwjgl:3.4.1:natives-windows"}),
            false,
        ),
        (
            25,
            json!({"name":"org.lwjgl:lwjgl:3.4.1","rules":[{"action":"disallow"}]}),
            false,
        ),
        (25, json!({"name":"org.lwjgl:lwjgl:3.3.1"}), false),
        (25, json!({"name":"org.lwjgl:lwjgl:3.4.3"}), false),
        (25, json!({"name":"org.lwjgl:lwjgl:3.4.0"}), true),
        (26, json!({"name":"org.lwjgl:lwjgl:3.4.1"}), true),
    ] {
        prepared["metadata"]["libraries"] = json!([library]);
        let args = launch_args(&client, &prepared, &settings, &directory, major);
        assert_eq!(path.is_file(), expected);
        let main = args
            .iter()
            .position(|arg| arg == "net.minecraft.client.main.Main")
            .unwrap();
        assert_eq!(
            args[..main].contains(&format!("-javaagent:{agent}")),
            expected
        );
    }
    let bundled = fs::read(&path).unwrap();
    fs::write(&path, b"corrupt cached JAR").unwrap();
    launch_args(&client, &prepared, &settings, &directory, 25);
    assert_eq!(fs::read(&path).unwrap(), bundled);

    let custom = "-javaagent:custom/lwjgl-unsafe-agent.jar=options";
    settings.jvm_arguments = custom.into();
    let args = launch_args(&client, &prepared, &settings, &directory, 25);
    assert_eq!(
        args.iter()
            .filter(|arg| arg.starts_with("-javaagent:"))
            .collect::<Vec<_>>(),
        [custom]
    );
}

#[tokio::test]
async fn old_releases_and_snapshots_resolve_prepare_and_build_launch_arguments() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let game = temp.path().join("game");
    let host = common::plugin_host(&data);
    let settings = Settings::default();
    let client = net::client(&host, &settings).unwrap();
    let mut offline = settings.clone();
    offline.modules.insert("minecraft.download".into(), false);
    let cases = [
        ("1.0", false, 8),
        ("1.5.2", false, 8),
        ("1.7.10", false, 8),
        ("1.12.2", false, 8),
        ("1.16.5", true, 8),
        ("1.17.1", true, 16),
        ("17w13a", false, 8),
        ("1.18-pre1", true, 17),
        ("24w14a", true, 21),
    ];
    let mut entries = vec![];
    for (id, modern, major) in cases {
        let mut value = base(id, modern);
        if modern {
            value["javaVersion"]["majorVersion"] = json!(major);
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        write(&game, &format!("versions/{id}/{id}.json"), &bytes);
        write(&game, &format!("versions/{id}/{id}.jar"), b"client");
        entries.push(json!({"id":id,"type":value["type"],"sha1":hash(&bytes)}));
    }
    entries.push(json!({"id":"a1.2.6","type":"old_alpha"}));
    storage::write_json(
        &data.join("cache/version-manifest.json"),
        &json!({"versions":entries}),
    )
    .unwrap();
    write(&game, "assets/indexes/test.json", br#"{"objects":{}}"#);
    let catalog = minecraft::versions(&client, &data, &offline).await.unwrap();
    assert_eq!(catalog.len(), cases.len());
    assert!(
        catalog
            .iter()
            .any(|v| v.id == "17w13a" && v.version_type == "snapshot")
    );
    let tasks = Tasks::default();
    for (id, _, major) in cases {
        let job = Tasks::default().start("prepare", None).unwrap();
        let metadata = minecraft::resolve(&client, &data, &game, &offline, id, &job)
            .await
            .unwrap();
        assert_eq!(metadata["javaVersion"]["majorVersion"], major);
        let prepared = minecraft::prepare(&host, &client, &game, &offline, metadata, "x64", job)
            .await
            .unwrap();
        let args = launch_args(
            &client,
            &serde_json::to_value(prepared).unwrap(),
            &settings,
            &game,
            major,
        );
        assert!(args.contains(&"Test Player".into()));
        assert!(!args.iter().any(|a| a.contains("${")));
    }
    assert!(
        minecraft::resolve(
            &client,
            &data,
            &game,
            &offline,
            "made-up-version",
            &tasks.start("prepare", None).unwrap()
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn cached_legacy_and_modern_forge_installers_publish_launchable_manifests() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let game = temp.path().join("game");
    let host = common::plugin_host(&data);
    let mut settings = Settings::default();
    settings.modules.insert("minecraft.download".into(), false);
    let client = net::client(&host, &Settings::default()).unwrap();
    write(&game, "assets/indexes/test.json", br#"{"objects":{}}"#);
    for (kind, id, loader, legacy) in [
        ("forge", "1.12.2", "1.12.2-14.23.5.2860", true),
        ("forge", "1.20.1", "1.20.1-47.4.0", false),
        ("neoforge", "1.21.1", "21.1.172", false),
    ] {
        let name = format!("{kind}-{id}");
        let mut metadata = base(id, !legacy);
        metadata["id"] = json!(name);
        metadata = client
            .service(
                "minecraft.instances",
                "normalize",
                json!({"metadata":metadata,"gameVersion":id}),
            )
            .unwrap();
        write(&game, &format!("versions/{name}/{name}.jar"), b"client");
        let group = if kind == "forge" {
            "net.minecraftforge"
        } else {
            "net.neoforged"
        };
        let module = if kind == "forge" { "forge" } else { "neoforge" };
        let coordinate = format!("{group}:{module}:{loader}");
        let library_path = format!(
            "{}/{module}/{loader}/{module}-{loader}.jar",
            group.replace('.', "/")
        );
        let mut version = json!({"id":"loader-profile","inheritsFrom":id,"mainClass":"cpw.mods.bootstraplauncher.BootstrapLauncher",
            "libraries":[{"name":coordinate,"downloads":{"artifact":{"path":library_path,"url":"","sha1":hash(b"loader"),"size":6}}}],
            "arguments":{"game":["--launchTarget","forgeclient"],"jvm":["-DlibraryDirectory=${library_directory}"]}});
        if legacy {
            version["mainClass"] = json!("net.minecraft.launchwrapper.Launch");
            version["minecraftArguments"] = json!(
                "--username ${auth_player_name} --tweakClass net.minecraftforge.fml.common.launcher.FMLTweaker"
            );
            version.as_object_mut().unwrap().remove("arguments");
        }
        let profile = if legacy {
            json!({"install":{"minecraft":id,"path":coordinate,"filePath":"universal.jar"},"versionInfo":version})
        } else {
            json!({"minecraft":id,"json":"/version.json","processors":[{"sides":["server"]}],"libraries":[]})
        };
        let cursor = std::io::Cursor::new(Vec::new());
        let mut archive = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in [
            (
                "install_profile.json".to_string(),
                serde_json::to_vec(&profile).unwrap(),
            ),
            ("version.json".into(), serde_json::to_vec(&version).unwrap()),
            (
                if legacy {
                    "universal.jar".into()
                } else {
                    format!("maven/{library_path}")
                },
                b"loader".to_vec(),
            ),
        ] {
            archive.start_file(name, options).unwrap();
            archive.write_all(&bytes).unwrap();
        }
        let bytes = archive.finish().unwrap().into_inner();
        write(
            &data,
            &format!("cache/installers/{kind}-{loader}.jar"),
            &bytes,
        );
        write(
            &data,
            &format!("cache/installers/{kind}-{loader}.jar.sha1"),
            hash(&bytes).as_bytes(),
        );
        let job = Tasks::default().start("install", None).unwrap();
        let prepared = minecraft::prepare(
            &host,
            &client,
            &game,
            &settings,
            metadata.clone(),
            "x64",
            job.clone(),
        )
        .await
        .unwrap();
        let prepared = net::task(
            &client,
            "minecraft.forge",
            "prepare",
            json!({"root":data,"gameRoot":game,"version":id,"settings":settings,
            "components":{"kind":kind,"loader":loader},"prepared":prepared}),
            Some(&job),
        )
        .await
        .unwrap();
        assert_eq!(job.state.lock().unwrap().phase, "verifying");
        assert_eq!(job.state.lock().unwrap().message, "正在校验并补齐游戏文件…");
        client.service("minecraft.instances","publish",json!({"root":game,"name":name,"gameVersion":id,"base":metadata,"complete":prepared["metadata"],"loader":loader,"loaderKind":kind})).unwrap();
        let loaded = repository::load(&client, &game, &name).unwrap();
        assert_eq!(loaded.game_version, id);
        assert_eq!(
            loaded.loader,
            if kind == "forge" { "Forge" } else { "NeoForge" }
        );
        let prepared = minecraft::prepare(
            &host,
            &client,
            &game,
            &settings,
            loaded.metadata,
            "x64",
            job,
        )
        .await
        .unwrap();
        let args = launch_args(
            &client,
            &serde_json::to_value(prepared).unwrap(),
            &settings,
            &game,
            8,
        );
        assert_eq!(args.iter().filter(|a| *a == "--username").count(), 1);
        assert!(
            args.contains(
                &if legacy {
                    "--tweakClass"
                } else {
                    "--launchTarget"
                }
                .to_string()
            )
        );
        assert!(!args.iter().any(|a| a.contains("${")));
    }
}

#[tokio::test]
#[ignore = "requires HYDROGEN_TEST_JAVA_HOME pointing to a JDK 17 installation"]
async fn java_processors_generate_and_verify_client_outputs() {
    let java_home = std::path::PathBuf::from(std::env::var_os("HYDROGEN_TEST_JAVA_HOME").unwrap());
    let exe = |name: &str| {
        java_home.join("bin").join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.into()
        })
    };
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data with spaces");
    let game = temp.path().join("game with spaces");
    let tool_dir = temp.path().join("tool");
    fs::create_dir_all(&tool_dir).unwrap();
    fs::write(
        tool_dir.join("Processor.java"),
        r#"
        import java.nio.file.*;
        public class Processor {
            public static void main(String[] args) throws Exception {
                System.out.println("processing client"); System.err.println("processor stderr");
                Path output = Paths.get(args[1]); Files.createDirectories(output.getParent());
                Files.copy(Paths.get(args[0]),output,StandardCopyOption.REPLACE_EXISTING);
            }
        }
    "#,
    )
    .unwrap();
    assert!(
        std::process::Command::new(exe("javac"))
            .arg(tool_dir.join("Processor.java"))
            .status()
            .unwrap()
            .success()
    );
    let jar = tool_dir.join("processor.jar");
    assert!(
        std::process::Command::new(exe("jar"))
            .args(["--create", "--file"])
            .arg(&jar)
            .args(["--main-class", "Processor", "-C"])
            .arg(&tool_dir)
            .arg("Processor.class")
            .status()
            .unwrap()
            .success()
    );
    let host = common::plugin_host(&data);
    let client = net::client(&host, &Settings::default()).unwrap();
    let mut settings = Settings::default();
    settings.modules.insert("minecraft.download".into(), false);
    settings.java = Some(exe("java").to_string_lossy().into_owned());
    let mut metadata = base("1.20.1", true);
    metadata["id"] = json!("ProcessorTest");
    write(&game, "versions/ProcessorTest/ProcessorTest.jar", b"client");
    write(&game, "assets/indexes/test.json", br#"{"objects":{}}"#);
    let payload = b"generated client";
    let payload_hash = hash(payload);
    let generated = "net/minecraftforge/forge/1.20.1-test/forge-1.20.1-test-client.jar";
    let version = json!({"id":"forge-test","inheritsFrom":"1.20.1","mainClass":"example.Forge",
        "arguments":{"game":["--forge"]},"libraries":[{"name":"net.minecraftforge:forge:1.20.1-test:client",
            "downloads":{"artifact":{"path":generated,"url":"","sha1":payload_hash,"size":payload.len()}}}]});
    let profile = json!({"minecraft":"1.20.1","json":"/version.json","libraries":[],
        "data":{"PAYLOAD":{"client":"/data/client.bin"},"OUTPUT":{"client":"[net.minecraftforge:forge:1.20.1-test:client]"},"HASH":{"client":format!("'{payload_hash}'")}},
        "processors":[{"sides":["server"],"jar":"missing:server:1"},
            {"sides":["client"],"jar":"test:processor:1","classpath":[],"args":["{PAYLOAD}","{OUTPUT}"],"outputs":{"{OUTPUT}":"{HASH}"}}]});
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes) in [
        (
            "install_profile.json",
            serde_json::to_vec(&profile).unwrap(),
        ),
        ("version.json", serde_json::to_vec(&version).unwrap()),
        ("data/client.bin", payload.to_vec()),
        (
            "maven/test/processor/1/processor-1.jar",
            fs::read(jar).unwrap(),
        ),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&bytes).unwrap();
    }
    let installer = zip.finish().unwrap().into_inner();
    write(&data, "cache/installers/forge-1.20.1-test.jar", &installer);
    write(
        &data,
        "cache/installers/forge-1.20.1-test.jar.sha1",
        hash(&installer).as_bytes(),
    );
    let job = Tasks::default().start("install", None).unwrap();
    let prepared = minecraft::prepare(
        &host,
        &client,
        &game,
        &settings,
        metadata,
        "x64",
        job.clone(),
    )
    .await
    .unwrap();
    let result = net::task(
        &client,
        "minecraft.forge",
        "prepare",
        json!({"root":data,"gameRoot":game,"version":"1.20.1","settings":settings,
        "components":{"kind":"forge","loader":"1.20.1-test"},"prepared":prepared}),
        Some(&job),
    )
    .await
    .unwrap();
    assert_eq!(
        fs::read(game.join("libraries").join(generated)).unwrap(),
        payload
    );
    assert_eq!(result["metadata"]["mainClass"], "example.Forge");
    assert!(
        data.join("cache/forge-work")
            .read_dir()
            .unwrap()
            .next()
            .is_none()
    );
}

#[tokio::test]
#[ignore = "live Forge 1.20.1 download/processor check; requires HYDROGEN_FORGE_SMOKE_DIR and HYDROGEN_TEST_JAVA_HOME (JDK 17)"]
async fn forge_1_20_1_download_smoke() {
    let data = std::path::PathBuf::from(std::env::var_os("HYDROGEN_FORGE_SMOKE_DIR").unwrap());
    assert!(data.is_absolute());
    let game = data.join("game");
    let host = common::plugin_host(&data);
    let mut settings = Settings::default();
    settings.download_source = "bmclapi".into();
    settings.java = Some(
        std::path::PathBuf::from(std::env::var_os("HYDROGEN_TEST_JAVA_HOME").unwrap())
            .join("bin")
            .join(if cfg!(windows) { "java.exe" } else { "java" })
            .to_string_lossy()
            .into_owned(),
    );
    let client = net::client(&host, &settings).unwrap();
    let job = Tasks::default().start("install", None).unwrap();
    let mut metadata = minecraft::resolve(&client, &data, &game, &settings, "1.20.1", &job)
        .await
        .unwrap();
    // Exercise real installer dependencies and processors without downloading game assets.
    let index = br#"{"objects":{}}"#;
    write(&game, "assets/indexes/smoke.json", index);
    metadata["assetIndex"] = json!({"id":"smoke","sha1":hash(index),"size":index.len()});
    metadata["libraries"] = json!([]);
    metadata["id"] = json!("forge-smoke");
    let prepared = minecraft::prepare(
        &host,
        &client,
        &game,
        &settings,
        metadata,
        "x64",
        job.clone(),
    )
    .await
    .unwrap();
    let params = json!({"root":data,"gameRoot":game,"version":"1.20.1","settings":settings,
        "components":{"kind":"forge","loader":"1.20.1-47.4.0"},"prepared":prepared});
    let mut reported = std::collections::BTreeSet::new();
    let result = net::task_observed(
        &client,
        "minecraft.forge",
        "prepare",
        params,
        Some(&job),
        |p| {
            if let Some(files) = p["files"].as_object() {
                for (name, file) in files {
                    if file["done"] == true && reported.insert(name.clone()) {
                        eprintln!("verified: {name}");
                    }
                }
            }
            Ok(())
        },
    )
    .await;
    assert!(result.is_ok(), "Forge download failed: {:?}", result.err());
}
