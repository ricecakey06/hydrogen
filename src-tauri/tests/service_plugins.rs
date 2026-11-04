mod common;
use hydrogen::{
    launcher::storage::Settings,
    plugins::{manager, manifest, package},
};
use serde_json::{Value, json};
use std::{fs, path::Path};
#[tokio::test]
async fn external_native_consumer_uses_declared_installed_services_without_host_business_fallback()
{
    let fixture = tempfile::tempdir().unwrap();
    let data = fixture.path().join("data");
    fs::create_dir(&data).unwrap();
    let host = common::plugin_host(&data);
    let source = fixture.path().join("consumer");
    fs::create_dir_all(source.join("bin")).unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).join("../official-plugins");
    let mut descriptor: Value =
        serde_json::from_slice(&fs::read(project.join("example.services/plugin.json")).unwrap())
            .unwrap();
    let entry = format!("bin/plugin{}", std::env::consts::DLL_SUFFIX);
    descriptor["entry"] = entry.clone().into();
    fs::write(
        source.join("plugin.json"),
        serde_json::to_vec(&descriptor).unwrap(),
    )
    .unwrap();
    fs::copy(
        project.join(format!(
            "target/debug/{}hydrogen_example_services{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        )),
        source.join(entry),
    )
    .unwrap();
    let archive = fixture.path().join("consumer.hplugin");
    package::pack(&source, &archive).unwrap();
    let candidate = manager::prepare(&data, &archive).unwrap();
    manager::install(&data, &candidate, None).unwrap();
    host.set_plugins(manager::discover(&data, &[]).unwrap());
    let mut settings = Settings::default();
    let session = host
        .call_service(
            &settings,
            "example.services",
            "example.services",
            "forward",
            json!({"params":{"name":"Notch"}}),
        )
        .unwrap();
    assert_eq!(session["uuid"], "b50ad385829d3141a2167e7d7539ba7f");
    let error = host
        .call_service(
            &settings,
            "example.services",
            "example.services",
            "forward",
            json!({"provider":"java.runtime","method":"parse"}),
        )
        .unwrap_err();
    assert_eq!(error.code, "UNDECLARED_DEPENDENCY");
    let error = host
        .call_service(
            &settings,
            "example.services",
            "example.services",
            "forward",
            json!({"method":"notDeclared"}),
        )
        .unwrap_err();
    assert_eq!(error.code, "UNKNOWN_SERVICE");
    settings.modules.insert("account.offline".into(), false);
    assert!(
        host.call_service(
            &settings,
            "example.services",
            "example.services",
            "forward",
            json!({"params":{"name":"Notch"}})
        )
        .is_err()
    );
    settings.modules.insert("account.offline".into(), true);
    let provider = manager::discover(&data, &[])
        .unwrap()
        .into_iter()
        .find(|p| p.manifest.id == "account.offline")
        .unwrap();
    fs::remove_file(manifest::within(&provider.root, &provider.manifest.entry).unwrap()).unwrap();
    assert!(
        host.call_service(
            &settings,
            "example.services",
            "example.services",
            "forward",
            json!({"params":{"name":"Notch"}})
        )
        .is_err()
    );
    let stats = host.stop_all().await.unwrap();
    assert_eq!(stats.active, 0);
    assert_eq!(stats.created, stats.destroyed);
}

#[tokio::test]
async fn waiting_service_does_not_lock_settings_or_serialize_other_providers() {
    use hydrogen::launcher::http::Client;
    use std::sync::{Arc, mpsc};
    use std::time::Duration;
    let fixture = tempfile::tempdir().unwrap();
    let host = common::plugin_host(fixture.path());
    let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
    host.set_main_executor(Arc::new(move |task| {
        tx.send(task).unwrap();
        Ok(())
    }));
    let client = Client::new(&host, &Settings::default()).unwrap();
    let waiting = client.clone();
    let blocked = std::thread::spawn(move || {
        waiting.service("platform.windows.window", "geometry", json!({"handle":0}))
    });
    // Hold one real service at its executor boundary, without sleeps in plugins.
    let task = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let independent = client.clone();
    let (done, result) = mpsc::channel();
    let other = std::thread::spawn(move || {
        independent.set_settings(&Settings::default());
        done.send(independent.service("account.offline", "session", json!({"name":"Notch"})))
            .unwrap();
    });
    let response = result.recv_timeout(Duration::from_secs(1));
    // Always release/join before asserting, including on the old broken path.
    task();
    blocked.join().unwrap().unwrap();
    other.join().unwrap();
    host.stop_main().unwrap();
    host.stop_all().await.unwrap();
    assert_eq!(
        response
            .expect("unrelated service blocked on settings lock")
            .unwrap()["name"],
        "Notch"
    );
}

#[cfg(feature = "desktop")]
#[tokio::test]
async fn snapshot_waiting_for_a_provider_releases_global_store_lock() {
    use hydrogen::launcher::core::Launcher;
    use std::sync::{Arc, mpsc};
    use std::time::Duration;
    let fixture = tempfile::tempdir().unwrap();
    let core = Launcher::open(Some(fixture.path().join("data"))).unwrap();
    let mut plugins = manager::discover(&core.lease().root, &[]).unwrap();
    let plugin = plugins
        .iter_mut()
        .find(|p| p.manifest.id == "minecraft.instances")
        .unwrap();
    let path = plugin.root.join("plugin.json");
    let mut definition: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    definition["executionThread"] = json!("main");
    fs::write(path, serde_json::to_vec(&definition).unwrap()).unwrap();
    *plugin = manifest::load(&plugin.root).unwrap();
    core.plugins.set_plugins(plugins);
    let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
    core.plugins.set_main_executor(Arc::new(move |task| {
        tx.send(task).unwrap();
        Ok(())
    }));
    let reading = core.clone();
    let view = std::thread::spawn(move || reading.view());
    let task = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let store_available = core.store.try_lock().is_ok();
    task();
    view.join().unwrap().unwrap();
    core.plugins.stop_main().unwrap();
    core.plugins.stop_all().await.unwrap();
    assert!(
        store_available,
        "snapshot held the window's store lock during a provider call"
    );
}
