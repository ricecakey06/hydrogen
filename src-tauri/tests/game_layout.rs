mod common;
use hydrogen::launcher::{
    minecraft, net,
    storage::{self, Components, Settings, Store},
    tasks::Tasks,
};
use serde_json::json;
use sha1::{Digest, Sha1};
use std::{fs, path::Path};

fn write(root: &Path, relative: &str, bytes: &[u8]) {
    let path = storage::within(root, relative).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn sha1(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}

#[tokio::test]
async fn game_content_and_private_instances_use_the_game_root_instead_of_launcher_data() {
    let fixture = tempfile::tempdir().unwrap();
    let data = fixture.path().join("HydrogenTauriData");
    let game = fixture.path().join(".minecraft");
    let store = Store::open_at(data.clone(), Some(game.clone())).unwrap();
    let instance = store.create("1.20.1", "路径测试", None).unwrap();
    let private = store.game_dir(&instance.info.id).unwrap();
    assert_eq!(private, game);
    assert_eq!(
        store.version_dir(&instance.info.id).unwrap(),
        game.join("versions/路径测试")
    );
    write(&private, "mods/local.jar", b"local mod");
    write(&private, "saves/world/level.dat", b"private save");
    let asset = b"asset";
    let asset_hash = sha1(asset);
    let asset_path = format!("assets/objects/{}/{asset_hash}", &asset_hash[..2]);
    let index =
        serde_json::to_vec(&json!({"objects":{"test":{"hash":asset_hash,"size":asset.len()}}}))
            .unwrap();
    let library_path = "org/example/library/1.0/library-1.0.jar";
    let metadata = json!({"id":"1.20.1","type":"release","minimumLauncherVersion":21,"mainClass":"example.Main","javaVersion":{"majorVersion":17},"arguments":{"game":["--gameDir","${game_directory}","--assetsDir","${assets_root}"],"jvm":["-cp","${classpath}"]},
        "downloads":{"client":{"url":"https://launcher.mojang.com/client.jar","sha1":sha1(b"client"),"size":6}},
        "assetIndex":{"id":"5","url":"https://piston-meta.mojang.com/index.json","sha1":sha1(&index),"size":index.len()},
        "libraries":[{"name":"org.example:library:1.0","downloads":{"artifact":{"path":library_path,"url":"https://libraries.minecraft.net/library.jar","sha1":sha1(b"library"),"size":7}}}]});
    let version = serde_json::to_vec(&metadata).unwrap();
    write(&game, "versions/1.20.1/1.20.1.json", &version);
    write(&game, "versions/1.20.1/1.20.1.jar", b"client");
    write(&game, "assets/indexes/5.json", &index);
    write(&game, &asset_path, asset);
    write(&game, &format!("libraries/{library_path}"), b"library");
    storage::write_json(&data.join("cache/version-manifest.json"), &json!({"versions":[{"id":"1.20.1","type":"release","url":"https://piston-meta.mojang.com/version.json","sha1":sha1(&version)}]})).unwrap();
    let mut settings = Settings::default();
    settings.modules.insert("minecraft.download".into(), false);
    let client = net::client(
        &common::plugin_host(&fixture.path().join("data")),
        &Settings::default(),
    )
    .unwrap();
    let tasks = Tasks::default();
    let job = tasks.start("install", Some(&instance.info.id)).unwrap();
    let resolved = minecraft::resolve(&client, &data, &game, &settings, "1.20.1", &job)
        .await
        .unwrap();
    let mut prepared = minecraft::prepare(
        &common::plugin_host(&data),
        &client,
        &game,
        &settings,
        resolved,
        "x64",
        job.clone(),
    )
    .await
    .unwrap();
    assert_eq!(prepared.assets, game.join("assets"));
    assert!(
        prepared
            .classpath
            .iter()
            .all(|path| path.starts_with(&game))
    );
    assert!(
        prepared
            .classpath
            .contains(&game.join("versions/1.20.1/1.20.1.jar"))
    );
    let loader = "libraries/net/fabricmc/fabric-loader/0.16.10/fabric-loader-0.16.10.jar";
    let loader_url =
        "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.16.10/fabric-loader-0.16.10.jar";
    write(&game, loader, b"loader");
    write(
        &data,
        &format!("cache/fabric-hashes/{}.sha1", sha1(loader_url.as_bytes())),
        sha1(b"loader").as_bytes(),
    );
    storage::write_json(&data.join("cache/fabric-1.20.1-0.16.10.json"), &json!({"id":"fabric-loader-0.16.10-1.20.1","inheritsFrom":"1.20.1","mainClass":"example.FabricMain","libraries":[{"name":"net.fabricmc:fabric-loader:0.16.10","url":"https://maven.fabricmc.net/"}]})).unwrap();
    minecraft::fabric(
        &client,
        &data,
        &game,
        &private,
        &settings,
        "1.20.1",
        &Components {
            loader: "0.16.10".into(),
            fabric_api: None,
        },
        &mut prepared,
        &job,
        false,
    )
    .await
    .unwrap();
    assert!(prepared.classpath.contains(&game.join(loader)));
    #[cfg(feature = "desktop")]
    {
        use hydrogen::launcher::{accounts::Accounts, java, launch};
        let mut offline = settings.clone();
        offline.offline_enabled = true;
        offline.account_mode = "offline".into();
        offline.modules.insert("account.offline".into(), true);
        hydrogen::plugins::manager::initialize(&data).unwrap();
        let host =
            hydrogen::plugins::host::PluginHost::new(tokio::runtime::Handle::current(), Vec::new());
        host.set_plugins(hydrogen::plugins::manager::discover(&data, &[]).unwrap());
        let session = Accounts::new(host.clone())
            .session(&client, &data, &offline)
            .await
            .unwrap();
        let java = java::parse(
            &host,
            &offline,
            Path::new("C:/Java/bin/java.exe"),
            "java.version = 17.0.1\nos.arch = amd64",
        )
        .unwrap();
        let java = java.unwrap();
        let args = launch::arguments(
            &client,
            &prepared,
            &java,
            &session,
            &offline,
            &private,
            &game.join("versions/1.20.1/natives/windows-x64/session"),
        )
        .unwrap();
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--gameDir" && Path::new(&pair[1]) == private)
        );
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "--assetsDir" && Path::new(&pair[1]) == game.join("assets"))
        );
    }
    assert!(
        data.join(format!("instances/{}/instance.json", instance.info.id))
            .is_file()
    );
    assert!(!data.join(".minecraft").exists());
    assert!(
        !data
            .join(format!("instances/{}/game", instance.info.id))
            .exists()
    );
}

#[cfg(feature = "desktop")]
#[test]
fn default_custom_roots_and_deletion_match_the_original_layout() {
    use hydrogen::launcher::core::{Launcher, game_root};
    let fixture = tempfile::tempdir().unwrap();
    let default = fixture.path().join("HydrogenTauriData");
    let sibling = fixture.path().join(".minecraft");
    let custom = fixture.path().join("custom");
    assert_eq!(game_root(&default, &default, Some(&sibling)), sibling);
    assert_eq!(
        game_root(&custom, &default, Some(&sibling)),
        custom.join(".minecraft")
    );
    assert_eq!(
        game_root(&default, &default, None),
        default.join(".minecraft")
    );
    let core = Launcher::open(Some(custom.clone())).unwrap();
    let (first, second, game, first_game, second_game) = {
        let store = core.store.lock().unwrap();
        let first = store
            .create(
                "1.20.1",
                "第一个实例",
                Some(Components {
                    loader: "0.16.10".into(),
                    fabric_api: None,
                }),
            )
            .unwrap();
        let second = store
            .create(
                "1.20.1",
                "第二个实例",
                Some(Components {
                    loader: "0.16.10".into(),
                    fabric_api: None,
                }),
            )
            .unwrap();
        let first_game = store.game_dir(&first.info.id).unwrap();
        let second_game = store.game_dir(&second.info.id).unwrap();
        (
            first,
            second,
            store.game_root.clone(),
            first_game,
            second_game,
        )
    };
    write(&first_game, "saves/world/level.dat", b"first world");
    write(&second_game, "saves/world/level.dat", b"second world");
    write(&game, "versions/1.20.1/1.20.1.jar", b"shared client");
    assert_eq!(core.view().unwrap().game_directory, game.to_string_lossy());
    core.delete(&first.info.id, &first.info.name).unwrap();
    assert!(!first_game.exists());
    assert!(!custom.join(format!("instances/{}", first.info.id)).exists());
    assert_eq!(
        fs::read(game.join("versions/1.20.1/1.20.1.jar")).unwrap(),
        b"shared client"
    );
    assert_eq!(
        fs::read(second_game.join("saves/world/level.dat")).unwrap(),
        b"second world"
    );
    let incompatible = br#"{"schemaVersion":99,"format":"future"}"#;
    fs::write(custom.join("settings.json"), incompatible).unwrap();
    assert!(core.delete(&second.info.id, &second.info.name).is_err());
    assert_eq!(
        fs::read(second_game.join("saves/world/level.dat")).unwrap(),
        b"second world"
    );
    assert!(
        custom
            .join(format!("instances/{}/instance.json", second.info.id))
            .is_file()
    );
    assert_eq!(
        fs::read(custom.join("settings.json")).unwrap(),
        incompatible
    );
}
