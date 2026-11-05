use hydrogen::{
    launcher::storage::Settings,
    plugins::{builtin, host::PluginHost, manager, manifest, package},
};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn host(root: &Path) -> PluginHost {
    let host = PluginHost::new(tokio::runtime::Handle::current(), Vec::new());
    host.set_plugins(manager::discover(root, &[]).unwrap());
    host
}

#[tokio::test]
async fn real_bundled_services_have_no_missing_entry_fallback_or_automatic_resurrection() {
    let root = tempfile::tempdir().unwrap();
    manager::initialize(root.path()).unwrap();
    let host = host(root.path());
    let settings = Settings::default();
    let session = host
        .call_service(
            &settings,
            "account.offline",
            "account.offline",
            "session",
            json!({"name":"Notch"}),
        )
        .unwrap();
    assert_eq!(session["uuid"], "b50ad385829d3141a2167e7d7539ba7f");
    let java = host.call_service(&settings, "java.runtime", "java.runtime", "parse", json!({"path":"C:/Java/bin/java.exe","output":"java.version = 21.0.5\nos.arch = amd64"})).unwrap();
    assert_eq!(java["major"], 21);
    assert_eq!(
        host.call_service(
            &settings,
            "java.runtime",
            "java.runtime",
            "arbitrary",
            Value::Null
        )
        .unwrap_err()
        .code,
        "UNKNOWN_SERVICE"
    );
    let plugin = manager::discover(root.path(), &[])
        .unwrap()
        .into_iter()
        .find(|p| p.manifest.id == "account.offline")
        .unwrap();
    fs::remove_file(manifest::within(&plugin.root, &plugin.manifest.entry).unwrap()).unwrap();
    assert!(
        host.call_service(
            &settings,
            "account.offline",
            "account.offline",
            "session",
            json!({"name":"Notch"})
        )
        .is_err(),
        "an already active instance must not hide a missing disk entry"
    );
    manager::initialize(root.path()).unwrap();
    assert!(!plugin.root.join(&plugin.manifest.entry).exists());
    host.set_plugins(manager::load(root.path(), &[]).unwrap());
    assert!(!host.available(&settings, "account.offline"));
    // An explicit reinstall can repair the damaged directory; startup never does it silently.
    let archive = root.path().join("repair.hplugin");
    fs::write(
        &archive,
        builtin::PACKAGES
            .iter()
            .find(|(id, _)| *id == "account.offline")
            .unwrap()
            .1,
    )
    .unwrap();
    let candidate = manager::prepare(root.path(), &archive).unwrap();
    let damaged = manager::current_digest(root.path(), "account.offline").unwrap();
    manager::install(root.path(), &candidate, damaged.as_deref()).unwrap();
    host.stop_plugins(vec!["account.offline".into()])
        .await
        .unwrap();
    host.set_plugins(manager::load(root.path(), &[]).unwrap());
    assert!(
        host.call_service(
            &settings,
            "account.offline",
            "account.offline",
            "session",
            json!({"name":"Notch"})
        )
        .is_ok()
    );
    let stats = host.stop_all().await.unwrap();
    assert_eq!(stats.created, stats.destroyed);
    assert_eq!(stats.active, 0);
    let java = manager::discover(root.path(), &[])
        .unwrap()
        .into_iter()
        .find(|p| p.manifest.id == "java.runtime")
        .unwrap();
    manager::uninstall(root.path(), &java).unwrap();
    manager::initialize(root.path()).unwrap();
    assert!(
        !java.root.exists(),
        "uninstalled preinstalled packages must remain uninstalled"
    );
}

fn counter_source(root: &Path) {
    let os = std::env::consts::OS;
    let (prefix, suffix) = if os == "windows" {
        ("", "dll")
    } else if os == "macos" {
        ("lib", "dylib")
    } else {
        ("lib", "so")
    };
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../official-plugins");
    let mut descriptor: Value =
        serde_json::from_slice(&fs::read(project.join("example.counter/plugin.json")).unwrap())
            .unwrap();
    descriptor["entry"] = format!("bin/plugin.{suffix}").into();
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(
        root.join("plugin.json"),
        serde_json::to_vec_pretty(&descriptor).unwrap(),
    )
    .unwrap();
    fs::copy(
        project.join(format!(
            "target/debug/{prefix}hydrogen_example_counter.{suffix}"
        )),
        root.join(format!("bin/plugin.{suffix}")),
    )
    .unwrap();
}

#[tokio::test]
async fn application_distribution_replaces_updated_builtins_and_preserves_newer_plugins() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    counter_source(&source);
    let data = fixture.path().join("data");
    fs::create_dir(&data).unwrap();
    let first = fixture.path().join("first.hplugin");
    package::pack(&source, &first).unwrap();
    let first = fs::read(first).unwrap();
    builtin::deploy(&data, &[("example.counter", &first)]).unwrap();
    let installed = manifest::load(&data.join("plugins/example.counter")).unwrap();
    fs::write(
        installed.root.join("local-file.txt"),
        b"modified old package",
    )
    .unwrap();
    fs::remove_file(installed.root.join(&installed.manifest.entry)).unwrap();
    // The same application payload never acts as a missing-entry execution fallback.
    builtin::deploy(&data, &[("example.counter", &first)]).unwrap();
    assert!(!installed.root.join(&installed.manifest.entry).exists());
    let mut descriptor: Value =
        serde_json::from_slice(&fs::read(source.join("plugin.json")).unwrap()).unwrap();
    descriptor["version"] = "1.1.0".into();
    fs::write(
        source.join("plugin.json"),
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../official-plugins/target/debug/{}hydrogen_example_counter_v2{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        )),
        source.join(&installed.manifest.entry),
    )
    .unwrap();
    let second = fixture.path().join("second.hplugin");
    package::pack(&source, &second).unwrap();
    let second = fs::read(second).unwrap();
    let mut settings = Settings::default();
    settings.modules.insert("example.counter".into(), false);
    let saved_settings = serde_json::to_vec(&settings).unwrap();
    fs::write(data.join("settings.json"), &saved_settings).unwrap();
    builtin::deploy(&data, &[("example.counter", &second)]).unwrap();
    let updated = manifest::load(&installed.root).unwrap();
    assert_eq!(updated.manifest.version, "1.1.0");
    assert!(!updated.root.join("local-file.txt").exists());
    assert_eq!(
        fs::read(data.join("settings.json")).unwrap(),
        saved_settings
    );
    let runtime = host(&data);
    assert_eq!(
        runtime
            .run(updated.clone(), "increment".into(), false)
            .await
            .unwrap()
            .result,
        2
    );
    runtime.stop_all().await.unwrap();
    // A newer standalone update must not be downgraded by an older bundled version.
    descriptor["version"] = "2.0.0".into();
    fs::write(
        source.join("plugin.json"),
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    let newer = fixture.path().join("newer.hplugin");
    package::pack(&source, &newer).unwrap();
    let newer = manager::prepare(&data, &newer).unwrap();
    manager::install(&data, &newer, Some(&updated.digest)).unwrap();
    builtin::deploy(&data, &[("example.counter", &first)]).unwrap();
    assert_eq!(
        manifest::load(&updated.root).unwrap().manifest.version,
        "2.0.0"
    );
}

#[tokio::test]
async fn third_party_native_package_uses_the_same_install_update_reload_and_cleanup_pipeline() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    counter_source(&source);
    let data = fixture.path().join("data");
    fs::create_dir(&data).unwrap();
    let archive = fixture.path().join("counter.hplugin");
    package::pack(&source, &archive).unwrap();
    let candidate = manager::prepare(&data, &archive).unwrap();
    let installed = manager::install(&data, &candidate, None).unwrap();
    let host = host(&data);
    let before = host
        .run(installed.clone(), "increment".into(), false)
        .await
        .unwrap();
    assert_eq!(before.result, 1);
    assert!(before.generation > 0);
    assert_eq!(
        host.run(installed.clone(), "increment".into(), false)
            .await
            .unwrap()
            .result,
        2
    );
    let checked = manager::reload_candidate(&installed).unwrap();
    assert!(
        host.reload_with_commit(checked.clone(), || Err(hydrogen::contracts::ApiError::new(
            "TEST_COMMIT",
            "failed receipt"
        )))
        .await
        .is_err()
    );
    assert_eq!(
        host.run(installed.clone(), "increment".into(), false)
            .await
            .unwrap()
            .result,
        3
    );
    let previous = installed.clone();
    let replacement = checked.clone();
    let root = data.clone();
    host.reload_with_commit(checked.clone(), move || {
        manager::approve_reload(&root, &previous, &replacement)
    })
    .await
    .unwrap();
    let after = host
        .run(checked.clone(), "increment".into(), false)
        .await
        .unwrap();
    assert!(after.generation > before.generation);
    assert_eq!(after.result, 1);
    // Upgrading a package replaces real implementation files through the same transaction.
    let mut descriptor: Value =
        serde_json::from_slice(&fs::read(source.join("plugin.json")).unwrap()).unwrap();
    descriptor["version"] = "1.1.0".into();
    let suffix = std::env::consts::DLL_SUFFIX;
    let prefix = std::env::consts::DLL_PREFIX;
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../official-plugins/target/debug/{prefix}hydrogen_example_counter_v2{suffix}"
        )),
        source.join(descriptor["entry"].as_str().unwrap()),
    )
    .unwrap();
    fs::write(
        source.join("plugin.json"),
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    let update = fixture.path().join("update.hplugin");
    package::pack(&source, &update).unwrap();
    let candidate = manager::prepare(&data, &update).unwrap();
    let updated = manager::install(&data, &candidate, Some(&checked.digest)).unwrap();
    host.stop_plugins(vec!["example.counter".into()])
        .await
        .unwrap();
    host.set_plugins(manager::discover(&data, &[]).unwrap());
    assert_eq!(
        host.run(updated.clone(), "increment".into(), false)
            .await
            .unwrap()
            .result,
        2
    );
    assert_eq!(updated.manifest.version, "1.1.0");
    let development = manifest::load(&source).unwrap();
    let selected = manager::discover(&data, std::slice::from_ref(&development)).unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].root, development.root);
    assert_eq!(
        manager::discover(&data, &[development.clone(), development])
            .unwrap_err()
            .code,
        "DUPLICATE_PLUGIN"
    );
    descriptor["provides"][0]["version"] = "2.0.0".into();
    fs::write(
        source.join("plugin.json"),
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    host.set_plugins(vec![manifest::load(&source).unwrap()]);
    assert_eq!(
        host.call_service(
            &Settings::default(),
            "example.counter",
            "example.counter",
            "increment",
            Value::Null
        )
        .unwrap_err()
        .code,
        "SERVICE_VERSION"
    );
    host.stop_all().await.unwrap();
    manager::uninstall(&data, &updated).unwrap();
    assert!(manager::discover(&data, &[]).unwrap().is_empty());
}

#[test]
fn descriptor_migration_installs_real_payload_and_preserves_disabled_settings() {
    let root = tempfile::tempdir().unwrap();
    let old = root.path().join("plugins/account.offline");
    fs::create_dir_all(&old).unwrap();
    let (id, name, description) = ("account.offline", "离线启动", "旧版描述清单");
    fs::write(old.join("plugin.json"), serde_json::to_vec(&json!({"schemaVersion":2,"apiVersion":1,"id":id,"name":name,"description":description,"version":"0.1.0","runtime":"native","entry":id,"activation":[],"dependencies":[],"permissions":[],"contributes":{"commands":[],"panels":[]}})).unwrap()).unwrap();
    manager::initialize(root.path()).unwrap();
    let plugin = manifest::load(&old).unwrap();
    assert!(builtin::migrated(&plugin.manifest.id));
    let mut settings = Settings::default();
    settings.modules.insert("account.offline".into(), false);
    assert_eq!(
        manager::order(
            &manager::load(root.path(), &[]).unwrap(),
            &settings,
            "account.offline"
        )
        .unwrap_err()
        .code,
        "PLUGIN_DISABLED"
    );
}

#[test]
fn changed_distribution_reinstalls_removed_package_and_applies_same_version_revision() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    counter_source(&source);
    let data = fixture.path().join("data");
    fs::create_dir(&data).unwrap();
    let archive = fixture.path().join("first.hplugin");
    package::pack(&source, &archive).unwrap();
    let first = fs::read(&archive).unwrap();
    builtin::deploy(&data, &[("example.counter", &first)]).unwrap();
    let installed = manifest::load(&data.join("plugins/example.counter")).unwrap();
    manager::uninstall(&data, &installed).unwrap();
    builtin::deploy(&data, &[("example.counter", &first)]).unwrap();
    assert!(!installed.root.exists());
    fs::write(
        source.join("revision.txt"),
        b"new distribution, same version",
    )
    .unwrap();
    let revised = fixture.path().join("revised.hplugin");
    package::pack(&source, &revised).unwrap();
    let revised = fs::read(revised).unwrap();
    builtin::deploy(&data, &[("example.counter", &revised)]).unwrap();
    let reinstalled = manifest::load(&installed.root).unwrap();
    assert_eq!(reinstalled.manifest.version, installed.manifest.version);
    assert!(reinstalled.root.join("revision.txt").is_file());
    assert_ne!(reinstalled.digest, installed.digest);
    fs::write(source.join("revision.txt"), b"second revision").unwrap();
    let second = fixture.path().join("second-revision.hplugin");
    package::pack(&source, &second).unwrap();
    builtin::deploy(&data, &[("example.counter", &fs::read(second).unwrap())]).unwrap();
    assert_eq!(
        fs::read(installed.root.join("revision.txt")).unwrap(),
        b"second revision"
    );
}

#[test]
fn main_thread_native_instances_reload_cleanup_and_support_negotiated_large_messages() {
    use hydrogen::plugins::native::NativeHost;
    let fixture = tempfile::tempdir().unwrap();
    counter_source(fixture.path());
    let path = fixture.path().join("plugin.json");
    let mut definition: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    definition["executionThread"] = json!("main");
    fs::write(&path, serde_json::to_vec(&definition).unwrap()).unwrap();
    let plugin = manifest::load(fixture.path()).unwrap();
    let native = NativeHost::default();
    let worker = native.clone();
    let (queue, rx) = std::sync::mpsc::channel::<Box<dyn FnOnce() + Send>>();
    let (ready, started) = std::sync::mpsc::sync_channel(1);
    let owner = std::thread::spawn(move || {
        worker.set_main_executor(std::sync::Arc::new(move |task| {
            queue
                .send(task)
                .map_err(|_| hydrogen::contracts::ApiError::new("STOPPED", "executor stopped"))
        }));
        ready.send(()).unwrap();
        while let Ok(task) = rx.recv() {
            task();
        }
    });
    started.recv().unwrap();
    assert_eq!(
        native
            .call(plugin.clone(), "increment".into(), Value::Null)
            .unwrap(),
        1
    );
    let large = json!({"payload":"x".repeat(1024*1024)});
    assert_eq!(
        native
            .call(plugin.clone(), "echo".into(), large.clone())
            .unwrap(),
        large
    );
    assert!(
        native
            .reload(plugin.clone(), || Err(hydrogen::contracts::ApiError::new(
                "COMMIT_FAILED",
                "test"
            )))
            .is_err()
    );
    assert_eq!(
        native
            .call(plugin.clone(), "increment".into(), Value::Null)
            .unwrap(),
        2
    );
    native.reload(plugin.clone(), || Ok(())).unwrap();
    assert_eq!(
        native
            .call(plugin.clone(), "increment".into(), Value::Null)
            .unwrap(),
        1
    );
    fs::remove_file(plugin.root.join(&plugin.manifest.entry)).unwrap();
    assert!(
        native
            .call(plugin, "increment".into(), Value::Null)
            .is_err()
    );
    native.stop(None).unwrap();
    let stats = native.stats();
    assert_eq!(stats.active, 0);
    assert_eq!(stats.created, stats.destroyed);
    // Replace dispatch to release the original channel; the executor exits after queued work drains.
    native.set_main_executor(std::sync::Arc::new(|_| Ok(())));
    owner.join().unwrap();
}

#[tokio::test]
async fn every_distribution_has_payload_and_frontend_assets_use_installed_integrity() {
    let root = tempfile::tempdir().unwrap();
    manager::initialize(root.path()).unwrap();
    let plugins = manager::load(root.path(), &[]).unwrap();
    assert_eq!(plugins.len(), 26);
    assert_eq!(builtin::PACKAGES.len(), 26);
    let host = host(root.path());
    let settings = Settings::default();
    for plugin in &plugins {
        assert!(plugin.root.join(&plugin.manifest.entry).is_file());
        assert!(plugin.manifest.entry.starts_with("bin/"));
        let mut disabled = settings.clone();
        disabled.modules.insert(plugin.manifest.id.clone(), false);
        assert!(manager::order(&plugins, &disabled, &plugin.manifest.id).is_err());
    }
    let frontends = hydrogen::plugins::frontend::bootstrap(&host, &settings, "ui.shell").unwrap();
    assert_eq!(frontends.len(), 3);
    let (bytes, mime) =
        hydrogen::plugins::frontend::resource(&host, &settings, "/ui.shell/assets/entry.js")
            .unwrap();
    assert!(!bytes.is_empty());
    assert_eq!(mime, "text/javascript");
    fs::remove_file(root.path().join("plugins/ui.launcher/assets/launcher.js")).unwrap();
    assert!(hydrogen::plugins::frontend::bootstrap(&host, &settings, "ui.shell").is_err());
    manager::initialize(root.path()).unwrap();
    assert!(
        !root
            .path()
            .join("plugins/ui.launcher/assets/launcher.js")
            .exists()
    );
    host.stop_all().await.unwrap();
}
