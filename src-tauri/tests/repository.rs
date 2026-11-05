mod common;
use hydrogen::launcher::{
    minecraft, net, repository,
    storage::{self, Settings, Store},
    tasks::Tasks,
};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use std::{fs, path::Path};

fn write(root: &Path, relative: &str, bytes: &[u8]) {
    let file = storage::within(root, relative).unwrap();
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, bytes).unwrap();
}
fn manifest(root: &Path, name: &str, value: &Value) {
    storage::write_json(&root.join(format!("versions/{name}/{name}.json")), value).unwrap();
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}

#[test]
fn repository_discovery_resolves_inheritance_patches_and_renamed_manifests_without_rewriting() {
    let fixture = tempfile::tempdir().unwrap();
    let service_store = Store::open(fixture.path().join("services")).unwrap();
    let service_client = net::client(&service_store.plugins, &Settings::default()).unwrap();
    let root = fixture.path().join(".minecraft");
    let base = json!({"id":"1.20.1", "mainClass":"net.minecraft.client.main.Main",
        "minecraftArguments":"--username ${auth_player_name} --gameDir \"${game_directory}\"",
        "libraries":[{"name":"org.example:base:1.0"}], "minimumLauncherVersion":21});
    manifest(&root, "1.20.1", &base);
    manifest(
        &root,
        "Fabric 实例",
        &json!({"id":"an-old-name","inheritsFrom":"1.20.1",
        "mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient",
        "libraries":[{"name":"net.fabricmc:fabric-loader:0.16.10"}],
        "arguments":{"game":[{"rules":[{"action":"allow"}],"value":"--extra"}]}}),
    );
    let source = root.join("versions/Fabric 实例/Fabric 实例.json");
    let original = fs::read(&source).unwrap();
    let fabric = repository::load(&service_client, &root, "Fabric 实例").unwrap();
    assert_eq!(fabric.metadata["id"], "Fabric 实例");
    assert_eq!(fabric.game_version, "1.20.1");
    assert_eq!(fabric.loader, "Fabric");
    assert_eq!(
        fabric.metadata["_hydrogenJar"],
        "versions/1.20.1/1.20.1.jar"
    );
    assert_eq!(
        fabric.metadata["libraries"][0]["name"],
        "net.fabricmc:fabric-loader:0.16.10"
    );
    assert!(fabric.metadata["arguments"]["game"][4].is_object());
    assert_eq!(fs::read(source).unwrap(), original);

    manifest(
        &root,
        "Patches",
        &json!({"root":true,"mainClass":"ignored.TopLevel",
        "patches":[
            {"id":"fabric","version":"0.16.10","priority":30000,"mainClass":"example.Loader","libraries":[{"name":"net.fabricmc:fabric-loader:0.16.10"}]},
            {"id":"game","version":"1.20.1","priority":0,"mainClass":"example.Base","libraries":[{"name":"org.example:base:1.0"}]}
        ]}),
    );
    let patches = repository::load(&service_client, &root, "Patches").unwrap();
    assert_eq!(patches.game_version, "1.20.1");
    assert_eq!(patches.metadata["mainClass"], "example.Loader");
    assert_eq!(patches.metadata["libraries"].as_array().unwrap().len(), 2);
    storage::write_json(&root.join("versions/改过名/old.json"), &base).unwrap();
    write(&root, "versions/改过名/old.jar", b"client");
    let renamed = repository::load(&service_client, &root, "改过名").unwrap();
    assert_eq!(renamed.metadata["id"], "改过名");
    assert_eq!(renamed.metadata["_hydrogenJar"], "versions/改过名/old.jar");
    assert!(!root.join("versions/改过名/改过名.json").exists());
    manifest(&root, "cycle-a", &json!({"inheritsFrom":"cycle-b"}));
    manifest(&root, "cycle-b", &json!({"inheritsFrom":"cycle-a"}));
    manifest(&root, "escape", &json!({"inheritsFrom":"../outside"}));
    let (versions, issues) = repository::scan(&service_client, &root).unwrap();
    assert_eq!(versions.len(), 4);
    assert_eq!(issues.len(), 3);
    let store = Store::open_at(fixture.path().join("launcher-data"), Some(root)).unwrap();
    assert_eq!(store.instances().unwrap().len(), 4);
    assert!(
        store
            .instances()
            .unwrap()
            .iter()
            .all(|i| i.info.is_installed)
    );
}

#[test]
fn shared_and_isolated_game_directories_use_only_version_content_and_explicit_overrides() {
    let fixture = tempfile::tempdir().unwrap();
    let service_store = Store::open(fixture.path().join("services")).unwrap();
    let service_client = net::client(&service_store.plugins, &Settings::default()).unwrap();
    let root = fixture.path().join(".minecraft");
    fs::create_dir_all(&root).unwrap();
    assert_eq!(
        repository::run_directory(&service_client, &root, "vanilla", None).unwrap(),
        root
    );
    manifest(
        &root,
        "fabric",
        &json!({"mainClass":"example.Main","libraries":[{"name":"net.fabricmc:fabric-loader:0.16.10"}]}),
    );
    assert_eq!(
        repository::run_directory(&service_client, &root, "fabric", None).unwrap(),
        root.join("versions/fabric")
    );
    assert_eq!(
        repository::run_directory(&service_client, &root, "fabric", Some(false)).unwrap(),
        root
    );
    assert_eq!(
        repository::run_directory(&service_client, &root, "vanilla", Some(true)).unwrap(),
        root.join("versions/vanilla")
    );
    write(&root, "versions/existing/saves/world/level.dat", b"world");
    assert_eq!(
        repository::run_directory(&service_client, &root, "existing", None).unwrap(),
        root.join("versions/existing")
    );
    let natives = repository::native_directory(&service_client, &root, "fabric", "x64").unwrap();
    #[cfg(windows)]
    assert_eq!(natives, root.join("versions/fabric/natives-windows-x86_64"));
    // Unrelated hidden configuration is never opened, parsed or created.
    write(
        &root,
        ".other-launcher/config/settings.json",
        b"not valid JSON",
    );
    write(
        &root,
        "versions/vanilla/.other-launcher/config/settings.json",
        b"also invalid",
    );
    assert_eq!(
        repository::run_directory(&service_client, &root, "vanilla", None).unwrap(),
        root
    );
    assert_eq!(
        repository::native_directory(&service_client, &root, "fabric", "x64").unwrap(),
        natives
    );
}

#[tokio::test]
async fn cached_repository_libraries_logging_and_legacy_assets_prepare_without_downloads() {
    let fixture = tempfile::tempdir().unwrap();
    let service_store = Store::open(fixture.path().join("services")).unwrap();
    let service_client = net::client(&service_store.plugins, &Settings::default()).unwrap();
    let root = fixture.path().join(".minecraft");
    let asset = b"legacy asset";
    let asset_hash = hash(asset);
    write(
        &root,
        &format!("assets/objects/{}/{asset_hash}", &asset_hash[..2]),
        asset,
    );
    storage::write_json(&root.join("assets/indexes/legacy.json"), &json!({"virtual":true,"map_to_resources":true,"objects":{"sound/test.ogg":{"hash":asset_hash,"size":asset.len()}}})).unwrap();
    write(
        &root,
        "versions/Legacy/old.jar",
        b"client without a declared hash",
    );
    write(
        &root,
        "libraries/org/example/shared/1.0/shared-1.0.jar",
        b"maven library",
    );
    write(
        &root,
        "versions/Legacy/libraries/local.jar",
        b"local library",
    );
    write(&root, "assets/log_configs/client.xml", b"logging");
    storage::write_json(&root.join("versions/Legacy/old.json"), &json!({"id":"old","mainClass":"example.Main",
        "minecraftArguments":"--gameDir ${game_directory} --assetsDir ${game_assets}", "assets":"legacy",
        "arguments":{"jvm":["-Dlibs=${library_directory}","-Djava.library.path=${natives_directory}","-cp","${classpath}"]},
        "libraries":[{"name":"org.example:shared:1.0"},{"name":"example:local:1.0","MMC-hint":"local","MMC-filename":"local.jar"}],
        "logging":{"client":{"argument":"-Dlog4j.configurationFile=${path}","file":{"id":"client.xml"}}}})).unwrap();
    write(&root, "versions/Legacy/config/local.txt", b"local config");
    let mut metadata = repository::load(&service_client, &root, "Legacy")
        .unwrap()
        .metadata;
    let directory = repository::run_directory(&service_client, &root, "Legacy", None).unwrap();
    metadata["_hydrogenRunDirectory"] = directory.to_string_lossy().into_owned().into();
    let mut settings = Settings::default();
    settings.modules.insert("minecraft.download".into(), false);
    let client = net::client(
        &common::plugin_host(&fixture.path().join("data")),
        &Settings::default(),
    )
    .unwrap();
    let tasks = Tasks::default();
    let game = minecraft::prepare(
        &common::plugin_host(&fixture.path().join("data")),
        &client,
        &root,
        &settings,
        metadata,
        "x64",
        tasks.start("prepare", None).unwrap(),
    )
    .await
    .unwrap();
    assert!(
        game.classpath
            .contains(&root.join("versions/Legacy/old.jar"))
    );
    assert!(
        game.classpath
            .contains(&root.join("versions/Legacy/libraries/local.jar"))
    );
    assert!(
        game.classpath
            .contains(&root.join("libraries/org/example/shared/1.0/shared-1.0.jar"))
    );
    assert_eq!(game.assets, root.join("assets/virtual/legacy"));
    assert_eq!(
        fs::read(directory.join("resources/sound/test.ogg")).unwrap(),
        asset
    );
    assert_eq!(fs::read(game.assets.join("sound/test.ogg")).unwrap(), asset);
    assert!(
        game.logging
            .as_ref()
            .unwrap()
            .ends_with("assets\\log_configs\\client.xml")
            || game
                .logging
                .as_ref()
                .unwrap()
                .ends_with("assets/log_configs/client.xml")
    );
    #[cfg(feature = "desktop")]
    {
        use hydrogen::launcher::{accounts::Accounts, java, launch};
        settings.account_mode = "offline".into();
        settings.offline_enabled = true;
        settings.modules.insert("account.offline".into(), true);
        hydrogen::plugins::manager::initialize(&fixture.path().join("data")).unwrap();
        let host =
            hydrogen::plugins::host::PluginHost::new(tokio::runtime::Handle::current(), Vec::new());
        host.set_plugins(
            hydrogen::plugins::manager::discover(&fixture.path().join("data"), &[]).unwrap(),
        );
        let session = Accounts::new(host.clone())
            .session(&client, &fixture.path().join("data"), &settings)
            .await
            .unwrap();
        let java = java::parse(
            &host,
            &settings,
            Path::new("C:/Java/bin/java.exe"),
            "java.version = 8.0.1\nos.arch = amd64",
        )
        .unwrap();
        let java = java.unwrap();
        let natives =
            repository::native_directory(&service_client, &root, "Legacy", "x64").unwrap();
        let args = launch::arguments(
            &client, &game, &java, &session, &settings, &directory, &natives,
        )
        .unwrap();
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--gameDir" && Path::new(&pair[1]) == directory)
        );
        assert!(args.contains(&format!("-Dlibs={}", root.join("libraries").display())));
        assert!(!args.iter().any(|arg| arg.contains("${")));
    }
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn new_install_publishes_repository_named_versions_shared_fabric_libraries_and_isolation() {
    use hydrogen::launcher::core::Launcher;
    let fixture = tempfile::tempdir().unwrap();
    let service_store = Store::open(fixture.path().join("services")).unwrap();
    let service_client = net::client(&service_store.plugins, &Settings::default()).unwrap();
    let data = fixture.path().join("data");
    let root = data.join(".minecraft");
    let core = Launcher::open(Some(data.clone())).unwrap();
    let client = b"verified vanilla client";
    let index = br#"{"objects":{}}"#;
    let base = json!({"id":"1.20.1","mainClass":"example.Main","type":"release","javaVersion":{"majorVersion":17},
        "arguments":{"game":["--gameDir","${game_directory}"],"jvm":["-cp","${classpath}"]},"libraries":[],
        "downloads":{"client":{"sha1":hash(client),"size":client.len()}},
        "assetIndex":{"id":"5","sha1":hash(index),"size":index.len()}});
    let bytes = serde_json::to_vec(&base).unwrap();
    storage::write_json(&data.join("cache/version-manifest.json"), &json!({"versions":[{"id":"1.20.1","type":"release","url":"https://piston-meta.mojang.com/version.json","sha1":hash(&bytes)}]})).unwrap();
    write(
        &data,
        &format!("cache/manifests/{}.json", hash(&bytes)),
        &bytes,
    );
    write(&root, "versions/Cache/Cache.jar", client);
    write(&root, "assets/indexes/5.json", index);
    let loader_url =
        "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.16.10/fabric-loader-0.16.10.jar";
    write(
        &root,
        "libraries/net/fabricmc/fabric-loader/0.16.10/fabric-loader-0.16.10.jar",
        b"loader",
    );
    write(
        &data,
        &format!("cache/fabric-hashes/{}.sha1", hash(loader_url.as_bytes())),
        hash(b"loader").as_bytes(),
    );
    storage::write_json(&data.join("cache/fabric-1.20.1-0.16.10.json"), &json!({"id":"fabric-loader-0.16.10-1.20.1","inheritsFrom":"1.20.1","mainClass":"example.FabricMain","libraries":[{"name":"net.fabricmc:fabric-loader:0.16.10","url":"https://maven.fabricmc.net/"}],"arguments":{"game":["--fabric"]}})).unwrap();
    let mut settings = core.store.lock().unwrap().settings.clone();
    settings.modules.insert("minecraft.download".into(), false);
    core.save_settings(settings).unwrap();
    for (name, components) in [
        ("原版新安装", None),
        (
            "Fabric 新安装",
            Some(storage::Components {
                kind: String::new(),
                loader: "0.16.10".into(),
                fabric_api: None,
            }),
        ),
    ] {
        let instance = core
            .store
            .lock()
            .unwrap()
            .create("1.20.1", name, components)
            .unwrap();
        let task = core.install(instance.info.id.clone()).unwrap();
        wait(&core, &task).await;
        assert_eq!(
            fs::read(root.join(format!("versions/{name}/{name}.jar"))).unwrap(),
            client
        );
        let written: Value =
            storage::read_json(&root.join(format!("versions/{name}/{name}.json"))).unwrap();
        assert_eq!(written["id"], name);
        assert_eq!(written["root"], true);
        assert_eq!(written["patches"][0]["id"], "game");
        let version = repository::load(&service_client, &root, name).unwrap();
        assert_eq!(version.game_version, "1.20.1");
        assert!(!root.join(format!("versions/{}", instance.info.id)).exists());
        if name.starts_with("Fabric") {
            assert_eq!(version.loader, "Fabric");
            assert_eq!(version.metadata["mainClass"], "example.FabricMain");
            assert_eq!(
                version.metadata["arguments"]["game"],
                json!(["--gameDir", "${game_directory}", "--fabric"])
            );
            assert_eq!(
                repository::run_directory(&service_client, &root, name, None).unwrap(),
                root.join(format!("versions/{name}"))
            );
        } else {
            assert_eq!(
                repository::run_directory(&service_client, &root, name, None).unwrap(),
                root
            );
        }
        let before = fs::read(root.join(format!("versions/{name}/{name}.json"))).unwrap();
        let repair = core.install(instance.info.id).unwrap();
        wait(&core, &repair).await;
        assert_eq!(
            fs::read(root.join(format!("versions/{name}/{name}.json"))).unwrap(),
            before
        );
    }
    assert!(!root.join("versions/1.20.1").exists());

    let quilt_jar = "org/quiltmc/quilt-loader/0.29.2/quilt-loader-0.29.2.jar";
    let quilt_url = "https://maven.quiltmc.org/repository/release/";
    write(&root, &format!("libraries/{quilt_jar}"), b"quilt");
    write(
        &data,
        &format!(
            "cache/fabric-hashes/{}.sha1",
            hash(format!("{quilt_url}{quilt_jar}").as_bytes())
        ),
        hash(b"quilt").as_bytes(),
    );
    storage::write_json(&data.join("cache/quilt-1.20.1-0.29.2.json"), &json!({
        "id":"quilt-loader-0.29.2-1.20.1","inheritsFrom":"1.20.1","mainClass":"example.QuiltMain",
        "libraries":[{"name":"org.quiltmc:quilt-loader:0.29.2","url":quilt_url}]
    })).unwrap();
    let mut settings = core.store.lock().unwrap().settings.clone();
    settings.isolate_game_directory = Some(false);
    core.save_settings(settings).unwrap();

    let source = fixture.path().join("test.mrpack");
    {
        use std::io::Write;
        let mut archive = zip::ZipWriter::new(fs::File::create(&source).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("modrinth.index.json", options).unwrap();
        archive.write_all(br#"{"formatVersion":1,"game":"minecraft","name":"Quilt Pack","versionId":"1","files":[],"dependencies":{"minecraft":"1.20.1","quilt-loader":"0.29.2"}}"#).unwrap();
        archive
            .start_file("overrides/custom.json", options)
            .unwrap();
        archive
            .write_all(br#"{"mainClass":"not.a.game.manifest"}"#)
            .unwrap();
        archive
            .start_file("overrides/kubejs/startup_scripts/pack.js", options)
            .unwrap();
        archive.write_all(b"pack script").unwrap();
        archive.finish().unwrap();
    }
    let task = core.import_modpack(source.clone()).await.unwrap();
    wait(&core, &task).await;
    let store = core.store.lock().unwrap();
    let instance = store
        .instances()
        .unwrap()
        .into_iter()
        .find(|instance| instance.info.name == "Quilt Pack")
        .unwrap();
    let directory = store.game_dir(&instance.info.id).unwrap();
    assert!(instance.info.is_installed && instance.isolated);
    assert_eq!(directory, root.join("versions/Quilt Pack"));
    assert_eq!(
        fs::read(directory.join("kubejs/startup_scripts/pack.js")).unwrap(),
        b"pack script"
    );
    assert!(!root.join("kubejs").exists());
    let version = repository::load(&service_client, &root, "Quilt Pack").unwrap();
    assert_eq!(version.metadata["mainClass"], "example.QuiltMain");
    drop(store);
    let second = core.import_modpack(source).await.unwrap();
    wait(&core, &second).await;
    assert!(
        root.join("versions/Quilt Pack (2)/Quilt Pack (2).json")
            .is_file()
    );
}

#[cfg(feature = "desktop")]
async fn wait(core: &hydrogen::launcher::core::Launcher, id: &str) {
    for _ in 0..500 {
        let task = core
            .tasks
            .list()
            .into_iter()
            .find(|task| task.id == id)
            .unwrap();
        if !task.active() {
            assert_eq!(task.phase, "completed", "{}", task.message);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("cached install did not finish");
}

#[cfg(feature = "desktop")]
#[test]
fn binding_an_existing_repository_persists_and_deleting_versions_preserves_shared_data() {
    use hydrogen::launcher::core::Launcher;
    let fixture = tempfile::tempdir().unwrap();
    let data = fixture.path().join("launcher-data");
    let root = fixture.path().join("existing/.minecraft");
    for name in ["Vanilla", "Other"] {
        manifest(&root, name, &json!({"id":name,"mainClass":"example.Main"}));
    }
    write(&root, "saves/shared/level.dat", b"shared world");
    write(&root, "mods/local.jar", b"shared mod");
    let core = Launcher::open(Some(data.clone())).unwrap();
    core.change_game_root(root.parent().unwrap().to_owned())
        .unwrap();
    let before = core.view().unwrap();
    assert_eq!(before.instances.len(), 2);
    let vanilla = before
        .instances
        .iter()
        .find(|instance| instance.info.name == "Vanilla")
        .unwrap();
    let mut settings = core.store.lock().unwrap().settings.clone();
    settings.isolate_game_directory = Some(true);
    core.save_settings(settings.clone()).unwrap();
    for instance in &before.instances {
        assert_eq!(
            core.store
                .lock()
                .unwrap()
                .game_dir(&instance.info.id)
                .unwrap(),
            root.join(format!("versions/{}", instance.info.name))
        );
    }
    let job = core.tasks.start("install", Some(&vanilla.info.id)).unwrap();
    settings.isolate_game_directory = Some(false);
    assert!(core.save_settings(settings).is_err());
    job.finish(Ok(()));
    let ids: Vec<_> = before.instances.iter().map(|i| i.info.id.clone()).collect();
    drop(core);
    let core = Launcher::open(Some(data)).unwrap();
    assert_eq!(core.store.lock().unwrap().game_root, root);
    assert_eq!(
        core.store.lock().unwrap().settings.isolate_game_directory,
        Some(true)
    );
    assert_eq!(
        core.view()
            .unwrap()
            .instances
            .iter()
            .map(|i| i.info.id.clone())
            .collect::<Vec<_>>(),
        ids
    );
    let mut settings = core.store.lock().unwrap().settings.clone();
    settings.isolate_game_directory = Some(false);
    core.save_settings(settings).unwrap();
    for instance in &before.instances {
        assert_eq!(
            core.store
                .lock()
                .unwrap()
                .game_dir(&instance.info.id)
                .unwrap(),
            root
        );
    }
    for instance in before.instances {
        core.delete(&instance.info.id, &instance.info.name).unwrap();
    }
    assert_eq!(
        fs::read(root.join("saves/shared/level.dat")).unwrap(),
        b"shared world"
    );
    assert_eq!(
        fs::read(root.join("mods/local.jar")).unwrap(),
        b"shared mod"
    );

    assert!(core.view().unwrap().instances.is_empty());
}
