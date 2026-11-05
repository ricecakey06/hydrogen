mod common;
use hydrogen::{
    launcher::{mods, net, storage::Settings, tasks::Tasks},
    plugins::{manager, manifest},
};
use serde_json::json;
use std::{fs, path::Path};
fn staged(game: &Path) -> usize {
    fs::read_dir(game.join(".hydrogen/import"))
        .map(|items| items.count())
        .unwrap_or(0)
}
#[tokio::test]
async fn mod_plugin_import_transactions_abort_rollback_and_reject_missing_payload() {
    let root = tempfile::tempdir().unwrap();
    let host = common::plugin_host(root.path());
    let settings = Settings::default();
    let game = root.path().join("game");
    fs::create_dir(&game).unwrap();
    let first = root.path().join("first.jar");
    let second = root.path().join("second.jar");
    fs::write(&first, b"first").unwrap();
    fs::write(&second, b"second").unwrap();
    let id = mods::service(&host, &settings, "importBegin", json!({"game":game}))
        .await
        .unwrap();
    for path in [&first, &second] {
        mods::service(
            &host,
            &settings,
            "importAdd",
            json!({"handle":id,"path":path}),
        )
        .await
        .unwrap();
    }
    assert_eq!(staged(&game), 1);
    fs::create_dir_all(game.join("mods")).unwrap();
    fs::write(game.join("mods/second.jar"), b"concurrent writer").unwrap();
    assert!(
        mods::service(&host, &settings, "importCommit", json!({"handle":id}))
            .await
            .is_err()
    );
    assert!(!game.join("mods/first.jar").exists());
    assert_eq!(
        fs::read(game.join("mods/second.jar")).unwrap(),
        b"concurrent writer"
    );
    assert_eq!(staged(&game), 0);
    let id = mods::service(&host, &settings, "importBegin", json!({"game":game}))
        .await
        .unwrap();
    mods::service(
        &host,
        &settings,
        "importAdd",
        json!({"handle":id,"path":first}),
    )
    .await
    .unwrap();
    mods::service(&host, &settings, "importAbort", json!({"handle":id}))
        .await
        .unwrap();
    assert_eq!(staged(&game), 0);
    let tasks = Tasks::default();
    let job = tasks.start("import", None).unwrap();
    job.cancel.cancel();
    assert!(
        mods::import(&host, &settings, &game, std::slice::from_ref(&first), &job)
            .await
            .is_err()
    );
    assert_eq!(staged(&game), 0);
    let job = Tasks::default().start("import", None).unwrap();
    mods::import(&host, &settings, &game, std::slice::from_ref(&first), &job)
        .await
        .unwrap();
    assert_eq!(fs::read(game.join("mods/first.jar")).unwrap(), b"first");
    assert_eq!(staged(&game), 0);
    let plugin = manager::discover(root.path(), &[])
        .unwrap()
        .into_iter()
        .find(|p| p.manifest.id == "minecraft.mods")
        .unwrap();
    fs::remove_file(manifest::within(&plugin.root, &plugin.manifest.entry).unwrap()).unwrap();
    assert!(
        mods::inventory(&host, &settings, root.path(), &game)
            .await
            .is_err()
    );
    manager::initialize(root.path()).unwrap();
    assert!(!plugin.root.join(&plugin.manifest.entry).exists());
    let stats = host.stop_all().await.unwrap();
    assert_eq!(stats.created, stats.destroyed);
}

#[tokio::test]
async fn mod_update_rolls_back_a_failed_batch_and_preserves_disabled_files() {
    let root = tempfile::tempdir().unwrap();
    let host = common::plugin_host(root.path());
    let settings = Settings::default();
    let client = net::client(&host, &settings).unwrap();
    let game = root.path().join("game");
    let directory = game.join("mods");
    fs::create_dir_all(&directory).unwrap();
    let old_disabled = directory.join("disabled.jar.disabled");
    let old_enabled = directory.join("old.jar");
    let new_disabled = root.path().join("disabled.jar");
    let new_enabled = root.path().join("new.jar");
    let dependency = root.path().join("library.jar");
    for (path, bytes) in [
        (&old_disabled, "old disabled"),
        (&old_enabled, "old enabled"),
        (&new_disabled, "new disabled"),
        (&new_enabled, "new enabled"),
        (&dependency, "dependency"),
    ] {
        fs::write(path, bytes).unwrap();
    }
    let collision = directory.join("new.jar");
    for fail in [true, false] {
        let handle = mods::service(&host, &settings, "importBegin", json!({"game":game}))
            .await
            .unwrap();
        for (path, old) in [
            (&dependency, None),
            (&new_disabled, Some(&old_disabled)),
            (&new_enabled, Some(&old_enabled)),
        ] {
            let original = match old {
                Some(old) => json!({
                    "fileName": old.file_name().unwrap().to_str().unwrap(),
                    "sha1": net::hash(&client, old).await.unwrap(),
                }),
                None => serde_json::Value::Null,
            };
            mods::service(&host, &settings, "importAdd", json!({
                "handle": handle, "path": path,
                "update": {"old": original, "sha1": net::hash(&client, path).await.unwrap(), "enabled": true}
            })).await.unwrap();
        }
        assert_eq!(fs::read_to_string(&old_disabled).unwrap(), "old disabled");
        assert_eq!(fs::read_to_string(&old_enabled).unwrap(), "old enabled");
        if fail {
            fs::write(&collision, "concurrent writer").unwrap();
        }
        let result =
            mods::service(&host, &settings, "importCommit", json!({"handle":handle})).await;
        if fail {
            assert!(result.is_err());
            assert_eq!(fs::read_to_string(&old_disabled).unwrap(), "old disabled");
            assert_eq!(fs::read_to_string(&old_enabled).unwrap(), "old enabled");
            assert_eq!(fs::read_to_string(&collision).unwrap(), "concurrent writer");
            assert!(!directory.join("library.jar").exists());
            fs::remove_file(&collision).unwrap();
        } else {
            assert_eq!(result.unwrap(), json!(3));
            assert_eq!(fs::read_to_string(&old_disabled).unwrap(), "new disabled");
            assert_eq!(fs::read_to_string(&collision).unwrap(), "new enabled");
            assert_eq!(
                fs::read_to_string(directory.join("library.jar")).unwrap(),
                "dependency"
            );
            assert!(!old_enabled.exists());
            assert!(!directory.join("disabled.jar").exists());
        }
        assert_eq!(staged(&game), 0);
    }
}
