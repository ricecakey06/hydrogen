mod common;
use hydrogen::{
    launcher::{mods, storage::Settings, tasks::Tasks},
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
