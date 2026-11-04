mod common;
#[cfg(feature = "desktop")]
use hydrogen::launcher::{storage, tasks::Tasks};
use hydrogen::{
    launcher::{mods, net, storage::Settings},
    plugins::{manager, manifest, package},
};
use serde_json::json;
use std::{fs, io::Write, path::Path};

fn plugin(root: &Path, id: &str, version: &str, dependencies: serde_json::Value) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("plugin.json"),serde_json::to_vec(&json!({"schemaVersion":2,"apiVersion":1,"id":id,"name":id,"version":version,"runtime":"javascript","entry":"main.js","dependencies":dependencies,"permissions":[]})).unwrap()).unwrap();
    fs::write(root.join("main.js"), "export function activate(ctx) {}\n").unwrap();
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn offline_plugin_adds_a_login_choice_and_preserves_microsoft_credentials() {
    use hydrogen::launcher::core::Launcher;

    let test = tempfile::tempdir().unwrap();
    let root = test.path().join("data");
    let core = Launcher::open(Some(root.clone())).unwrap();
    let view = core.view().unwrap();
    assert!(manager::enabled(&view.settings, "account.offline"));
    assert!(view.account.offline_available);
    assert_eq!(view.account.mode, "microsoft");
    assert_eq!(view.account.status, "signedOut");

    let mut settings = view.settings;
    settings.offline_enabled = false; // The old settings checkbox cannot override the plugin.
    core.save_settings(settings).unwrap();
    assert!(core.view().unwrap().account.offline_available);

    let mut settings = core.view().unwrap().settings;
    settings.account_mode = "offline".into();
    settings.offline_player_name = "Notch".into();
    core.save_settings(settings.clone()).unwrap();
    let session = core
        .accounts
        .session(&core.client, &root, &settings)
        .await
        .unwrap();
    assert!(session.offline);
    assert_eq!(session.name, "Notch");
    assert_eq!(session.uuid, "b50ad385829d3141a2167e7d7539ba7f");
    fs::write(root.join("credentials.bin"), b"saved Microsoft credentials").unwrap();
    core.sign_out().unwrap();
    assert_eq!(core.view().unwrap().account.status, "signedOut");
    assert_eq!(
        fs::read(root.join("credentials.bin")).unwrap(),
        b"saved Microsoft credentials"
    );

    let tasks = Tasks::default();
    let job = tasks.start("login", None).unwrap();
    core.accounts
        .sign_in(&core.client, &root, &settings, &job, false)
        .await
        .unwrap();
    assert_eq!(core.view().unwrap().account.status, "signedIn");

    settings.account_mode = "microsoft".into();
    core.save_settings(settings.clone()).unwrap();
    assert_eq!(core.view().unwrap().account.mode, "microsoft");
    assert_eq!(core.view().unwrap().account.status, "signedOut");
    assert_eq!(
        core.accounts
            .session(&core.client, &root, &settings)
            .await
            .err()
            .unwrap()
            .code,
        "ACCOUNT_REQUIRED"
    );

    let busy = core.tasks.start("login", None).unwrap();
    settings.account_mode = "offline".into();
    assert_eq!(
        core.save_settings(settings.clone()).unwrap_err().code,
        "ACCOUNT_BUSY"
    );
    busy.finish(Ok(()));
    core.save_settings(settings.clone()).unwrap();
    settings.modules.insert("account.offline".into(), false);
    core.save_settings(settings).unwrap();
    let view = core.view().unwrap();
    assert!(!view.account.offline_available);
    assert_eq!(view.account.mode, "microsoft");
    assert!(!view.settings.offline_enabled);
    drop(core);

    let core = Launcher::open(Some(root)).unwrap();
    assert!(!core.view().unwrap().account.offline_available);
    let mut settings = core.view().unwrap().settings;
    settings.modules.insert("account.offline".into(), true);
    core.save_settings(settings).unwrap();
    let view = core.view().unwrap();
    assert!(view.account.offline_available);
    assert_eq!(
        view.account.mode, "microsoft",
        "enabling the plugin must not switch the selected account mode"
    );
}
#[test]
fn plugin_install_upgrade_consent_dependencies_and_traversal() {
    let test = tempfile::tempdir().unwrap();
    let data = test.path().join("data");
    fs::create_dir(&data).unwrap();
    let data = dunce::canonicalize(data).unwrap();
    let source = test.path().join("source");
    plugin(&source, "example.first", "1.0.0", json!([]));
    let archive = test.path().join("first.hplugin");
    package::pack(&source, &archive).unwrap();
    let candidate = manager::prepare(&data, &archive).unwrap();
    assert!(
        manager::discover(&data, &[]).unwrap().is_empty(),
        "preview must not install or activate"
    );
    let first = manager::install(&data, &candidate, None).unwrap();
    assert_eq!(manager::discover(&data, &[]).unwrap().len(), 1);
    plugin(
        &source,
        "example.first",
        "2.0.0",
        json!([{"id":"example.dependency","version":"^1"}]),
    );
    let archive = test.path().join("upgrade.hplugin");
    package::pack(&source, &archive).unwrap();
    let candidate = manager::prepare(&data, &archive).unwrap();
    assert!(manager::install(&data, &candidate, Some("stale-digest")).is_err());
    assert_eq!(
        manifest::load(&first.root).unwrap().manifest.version,
        "1.0.0"
    );
    let second = manager::install(&data, &candidate, Some(&first.digest)).unwrap();
    let mut plugins = manager::discover(&data, &[]).unwrap();
    assert_eq!(
        manager::order(&plugins, &Settings::default(), "example.first")
            .unwrap_err()
            .code,
        "MISSING_DEPENDENCY"
    );
    let dep = test.path().join("dependency");
    plugin(&dep, "example.dependency", "1.2.0", json!([]));
    plugins.push(manifest::load(&dep).unwrap());
    assert_eq!(
        manager::order(&plugins, &Settings::default(), "example.first")
            .unwrap()
            .iter()
            .map(|p| p.manifest.id.as_str())
            .collect::<Vec<_>>(),
        ["example.dependency", "example.first"]
    );
    plugin(
        &dep,
        "example.dependency",
        "1.2.0",
        json!(["example.first"]),
    );
    plugins[1] = manifest::load(&dep).unwrap();
    assert_eq!(
        manager::order(&plugins, &Settings::default(), "example.first")
            .unwrap_err()
            .code,
        "DEPENDENCY_CYCLE"
    );
    fs::write(second.root.join("main.js"), "tampered").unwrap();
    assert!(
        manager::discover(&data, &[]).unwrap().is_empty(),
        "unconfirmed changes must not execute"
    );
    let malicious = test.path().join("malicious.hplugin");
    let mut zip = zip::ZipWriter::new(fs::File::create(&malicious).unwrap());
    zip.start_file("../outside", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"escape").unwrap();
    zip.finish().unwrap();
    assert!(manager::prepare(&data, &malicious).is_err());
    assert!(!test.path().join("outside").exists());
}
#[tokio::test]
async fn mod_metadata_collision_and_stale_file_protection() {
    let test = tempfile::tempdir().unwrap();
    let host = common::plugin_host(test.path());
    let settings = Settings::default();
    let game = test.path().join("game");
    fs::create_dir_all(game.join("mods")).unwrap();
    let file = game.join("mods/example.jar");
    let mut zip = zip::ZipWriter::new(fs::File::create(&file).unwrap());
    zip.start_file("fabric.mod.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(br#"{"id":"example","name":"Example Mod","version":"1.0","description":"test"}"#)
        .unwrap();
    zip.finish().unwrap();
    let hash = net::hash(&net::client(&host, &settings).unwrap(), &file)
        .await
        .unwrap();
    let info = mods::read_archive(&host, &settings, &file, &hash)
        .await
        .unwrap();
    assert_eq!(info.id, "example");
    assert_eq!(info.english_name, "Example Mod");
    let reference = mods::ModRef {
        file_name: "example.jar".into(),
        sha1: hash.clone(),
    };
    fs::write(game.join("mods/example.jar.disabled"), b"existing disabled").unwrap();
    assert!(
        mods::set_enabled(
            &host,
            &settings,
            &game,
            std::slice::from_ref(&reference),
            false
        )
        .await
        .is_err()
    );
    assert!(file.exists());
    assert_eq!(
        fs::read(game.join("mods/example.jar.disabled")).unwrap(),
        b"existing disabled"
    );
    fs::remove_file(game.join("mods/example.jar.disabled")).unwrap();
    mods::set_enabled(
        &host,
        &settings,
        &game,
        std::slice::from_ref(&reference),
        false,
    )
    .await
    .unwrap();
    let disabled = mods::ModRef {
        file_name: "example.jar.disabled".into(),
        sha1: hash,
    };
    mods::set_enabled(&host, &settings, &game, &[disabled], true)
        .await
        .unwrap();
    fs::create_dir_all(test.path().join("cache")).unwrap();
    fs::write(
        test.path().join("cache/mod-names.csv"),
        "0,example,Example Mod,示例模组,example,,example\n",
    )
    .unwrap();
    let local = mods::inventory(&host, &settings, test.path(), &game)
        .await
        .unwrap()
        .items;
    assert_eq!(local[0].chinese_name.as_deref(), Some("示例模组"));
    fs::write(
        test.path().join("mod-aliases.json"),
        serde_json::to_vec(&json!({"schemaVersion":1,"names":{"example":"本地自定义名称"}}))
            .unwrap(),
    )
    .unwrap();
    let local = mods::inventory(&host, &settings, test.path(), &game)
        .await
        .unwrap()
        .items;
    assert_eq!(local[0].chinese_name.as_deref(), Some("本地自定义名称"));
    fs::write(&file, b"changed after selection").unwrap();
    assert!(
        mods::remove(&host, &settings, &game, &[reference])
            .await
            .is_err()
    );
    assert_eq!(fs::read(&file).unwrap(), b"changed after selection");
}
#[cfg(feature = "desktop")]
#[test]
fn legacy_copy_resume_and_source_preservation() {
    use hydrogen::launcher::{core::Launcher, migration};
    let test = tempfile::tempdir().unwrap();
    let source = test.path().join("old/LauncherData");
    let game = test.path().join("old/.minecraft");
    let id = "a".repeat(32);
    let old = source.join(format!("instances/{id}"));
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("launch.lock"), b"").unwrap();
    fs::create_dir_all(game.join(format!("versions/{id}/saves/world"))).unwrap();
    fs::create_dir_all(game.join("assets/objects/ab")).unwrap();
    let record = json!({"schemaVersion":1,"instance":{"id":id,"gameVersion":"1.20.1","name":"旧存档","isInstalled":true,"preInstall":null},"overrides":{},"layoutVersion":2,"gameDirectoryName":id});
    storage::write_json(&old.join("instance.json"), &record).unwrap();
    fs::write(
        game.join(format!("versions/{id}/saves/world/level.dat")),
        b"precious world",
    )
    .unwrap();
    fs::write(game.join("assets/objects/ab/cache"), b"shared").unwrap();
    fs::write(source.join("credentials.bin"), b"never migrate credentials").unwrap();
    let target = test.path().join("new");
    let core = Launcher::open(Some(target)).unwrap();
    let root = core.lease().root.clone();
    let (plan, preview) = migration::preview(&core.client, &root, &source).unwrap();
    assert_eq!(preview.instances, 1);
    let tasks = Tasks::default();
    let job = tasks.start("test", None).unwrap();
    migration::execute(&core, plan, &job).unwrap();
    let instances = core.store.lock().unwrap().instances().unwrap();
    assert_eq!(instances.len(), 1);
    let path = root.join(format!(
        ".minecraft/versions/{}/saves/world/level.dat",
        instances[0].info.id
    ));
    assert_eq!(fs::read(path).unwrap(), b"precious world");
    assert!(!root.join("credentials.bin").exists());
    assert_eq!(
        storage::read_json::<serde_json::Value>(&old.join("instance.json")).unwrap(),
        record
    );
    let (plan, _) = migration::preview(&core.client, &root, &source).unwrap();
    let job = tasks.start("retry", None).unwrap();
    migration::execute(&core, plan, &job).unwrap();
    assert_eq!(
        core.store.lock().unwrap().instances().unwrap().len(),
        1,
        "retry must reuse committed mapping"
    );
}
