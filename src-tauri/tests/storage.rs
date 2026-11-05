use hydrogen::launcher::{java, storage::*};
use std::fs;

#[test]
fn microsoft_client_id_upgrades_previous_defaults_and_preserves_custom_clients() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path().to_path_buf()).unwrap();
    let configured_client = store.settings.microsoft_client_id.clone();
    assert_eq!(configured_client, Settings::default().microsoft_client_id);
    let mut settings = store.settings.clone();
    settings.microsoft_client_id.clear();
    settings.offline_player_name = "ExistingPlayer".into();
    store.save(settings).unwrap();
    drop(store);

    let mut store = Store::open(dir.path().to_path_buf()).unwrap();
    assert!(!store.read_only);
    assert_eq!(store.settings.microsoft_client_id, configured_client);
    assert_eq!(store.settings.offline_player_name, "ExistingPlayer");
    let mut settings = store.settings.clone();
    settings.microsoft_client_id = "8f901468-c7b9-4fe3-948e-a6752caa328a".into();
    store.save(settings.clone()).unwrap();
    drop(store);

    let store = Store::open(dir.path().to_path_buf()).unwrap();
    assert_eq!(store.settings, settings);
}

#[test]
fn persistent_settings_instances_and_protected_configs() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path().to_path_buf()).unwrap();
    assert!(
        Store::open(dir.path().to_path_buf()).is_err(),
        "another process must not share the writable root"
    );
    let mut settings = store.settings.clone();
    settings.max_memory_mib = 3072;
    store.save(settings.clone()).unwrap();
    let mut instance = store.create("1.21.1", "测试实例", None).unwrap();
    assert!(
        store.select(&instance.info.id).is_err(),
        "pending instances cannot be selected"
    );
    assert_eq!(
        store.create("1.21.1", "测试实例", None).unwrap().info.id,
        instance.info.id,
        "unfinished installation is resumed"
    );
    instance.info.is_installed = true;
    store.write_instance(&instance).unwrap();
    store.select(&instance.info.id).unwrap();
    store.rename(&instance.info.id, "新的名称").unwrap();
    assert_eq!(store.instances().unwrap()[0].info.name, "新的名称");
    drop(store);
    let store = Store::open(dir.path().to_path_buf()).unwrap();
    assert_eq!(store.settings, settings);
    assert_eq!(store.selected, Some(instance.info.id));
    drop(store);
    let incompatible = br#"{"schemaVersion":99,"format":"hydrogen-tauri","settings":{}}"#;
    fs::write(dir.path().join("settings.json"), incompatible).unwrap();
    let mut store = Store::open(dir.path().to_path_buf()).unwrap();
    assert!(store.save(Settings::default()).is_err());
    assert_eq!(
        fs::read(dir.path().join("settings.json")).unwrap(),
        incompatible
    );
}

#[tokio::test]
async fn boundaries_and_argument_grammar() {
    let services = tempfile::tempdir().unwrap();
    let store = Store::open(services.path().to_path_buf()).unwrap();
    let client = hydrogen::launcher::net::client(&store.plugins, &store.settings).unwrap();
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "../outside",
        "C:/outside",
        "/outside",
        "file:stream",
        "a/../../b",
    ] {
        assert!(within(dir.path(), path).is_err(), "{path}");
    }
    for args in [
        "-Xmx2G",
        "-cp secret.jar",
        "@flags",
        "-Duser.dir=C:\\",
        "--add-opens",
        "'unclosed",
        "-XX:VMOptionsFile=flags",
    ] {
        assert!(jvm_arguments(&client, args).is_err(), "{args}");
    }
    assert_eq!(
        jvm_arguments(
            &client,
            r#"-Dtest="C:\Program Files\Java" --add-opens java.base/java.lang=ALL-UNNAMED"#
        )
        .unwrap(),
        [
            "-Dtest=C:\\Program Files\\Java",
            "--add-opens",
            "java.base/java.lang=ALL-UNNAMED"
        ]
    );
    let root = tempfile::tempdir().unwrap();
    hydrogen::plugins::manager::initialize(root.path()).unwrap();
    let host =
        hydrogen::plugins::host::PluginHost::new(tokio::runtime::Handle::current(), Vec::new());
    host.set_plugins(hydrogen::plugins::manager::discover(root.path(), &[]).unwrap());
    let parsed = java::parse(
        &host,
        &Settings::default(),
        std::path::Path::new("C:/Java/bin/java.exe"),
        "java.version = 21.0.5\nos.arch = amd64",
    )
    .unwrap();
    let parsed = parsed.unwrap();
    assert_eq!(parsed.major, 21);
    assert_eq!(parsed.architecture, "x64");
    host.stop_all().await.unwrap();
}
