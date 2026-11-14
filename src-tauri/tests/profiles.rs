#![cfg(feature = "desktop")]
mod common;
use hydrogen::launcher::core::Launcher;
use serde_json::json;
use std::fs;

#[tokio::test]
async fn offline_profiles_persist_and_supply_the_selected_game_identity() {
    let test = tempfile::tempdir().unwrap();
    let root = test.path().join("data");
    let core = Launcher::open(Some(root.clone())).unwrap();
    let steve = core
        .profile_change("profiles.addOffline", json!({"name":"Steve"}))
        .unwrap();
    let alex = core
        .profile_change("profiles.addOffline", json!({"name":"Alex"}))
        .unwrap();
    core.select_profile(steve["id"].as_str().unwrap()).unwrap();
    let settings = core.view().unwrap().settings;
    let steve_session = core
        .accounts
        .session(&core.client, &root, &settings)
        .await
        .unwrap();
    assert_eq!(steve_session.name, "Steve");
    assert!(steve_session.offline);
    core.select_profile(alex["id"].as_str().unwrap()).unwrap();
    let settings = core.view().unwrap().settings;
    let alex_session = core
        .accounts
        .session(&core.client, &root, &settings)
        .await
        .unwrap();
    assert_eq!(alex_session.name, "Alex");
    assert_ne!(steve_session.uuid, alex_session.uuid);
    core.profile_change(
        "profiles.move",
        json!({"id":alex["id"],"target":"legacy-offline","after":false}),
    )
    .unwrap();
    drop(core);
    let reopened = Launcher::open(Some(root.clone())).unwrap();
    let view = reopened.view().unwrap();
    assert_eq!(
        view.account.selected_profile_id.as_deref(),
        alex["id"].as_str()
    );
    assert_eq!(view.account.player_name.as_deref(), Some("Alex"));
    assert_eq!(view.account.profiles.len(), 3); // Includes the migrated Player identity.
    assert_eq!(view.account.profiles[0].id, alex["id"]);
    let restored = reopened
        .accounts
        .session(&reopened.client, &root, &view.settings)
        .await
        .unwrap();
    assert_eq!(restored.uuid, alex_session.uuid);
}

#[tokio::test]
async fn invalid_or_duplicate_names_are_rejected_without_changing_profiles() {
    let test = tempfile::tempdir().unwrap();
    let core = Launcher::open(Some(test.path().join("data"))).unwrap();
    core.profile_change("profiles.addOffline", json!({"name":"  Steve  "}))
        .unwrap();
    let count = core.view().unwrap().account.profiles.len();
    for name in [
        "ab",
        "contains spaces",
        "../player",
        "玩家名称",
        "12345678901234567",
    ] {
        assert_eq!(
            core.profile_change("profiles.addOffline", json!({"name":name}))
                .unwrap_err()
                .code,
            "INVALID_PLAYER_NAME"
        );
    }
    assert_eq!(
        core.profile_change("profiles.addOffline", json!({"name":"steve"}))
            .unwrap_err()
            .code,
        "PROFILE_EXISTS"
    );
    assert_eq!(core.view().unwrap().account.profiles.len(), count);
}

#[tokio::test]
async fn deleting_current_and_last_profiles_does_not_recreate_them_or_touch_saves() {
    let test = tempfile::tempdir().unwrap();
    let root = test.path().join("data");
    let core = Launcher::open(Some(root.clone())).unwrap();
    fs::create_dir_all(root.join(".minecraft/saves/world")).unwrap();
    fs::write(
        root.join(".minecraft/saves/world/level.dat"),
        b"saved world",
    )
    .unwrap();
    let profile = core
        .profile_change("profiles.addOffline", json!({"name":"Alex"}))
        .unwrap();
    let id = profile["id"].as_str().unwrap();
    core.select_profile(id).unwrap();
    assert_eq!(
        core.delete_profile(id, "wrong name").unwrap_err().code,
        "CONFIRMATION_REQUIRED"
    );
    core.delete_profile(id, "Alex").unwrap();
    assert_eq!(
        core.view().unwrap().account.player_name.as_deref(),
        Some("Player")
    );
    core.delete_profile("legacy-offline", "Player").unwrap();
    assert!(core.view().unwrap().account.profiles.is_empty());
    assert_eq!(core.view().unwrap().account.status, "signedOut");
    drop(core);
    let reopened = Launcher::open(Some(root.clone())).unwrap();
    assert!(reopened.view().unwrap().account.profiles.is_empty());
    assert_eq!(
        fs::read(root.join(".minecraft/saves/world/level.dat")).unwrap(),
        b"saved world"
    );
}

#[tokio::test]
async fn profile_changes_are_blocked_during_login_and_launch_account_checks() {
    let test = tempfile::tempdir().unwrap();
    let core = Launcher::open(Some(test.path().join("data"))).unwrap();
    for kind in ["login", "launch"] {
        let job = core.tasks.start(kind, None).unwrap();
        assert_eq!(
            core.profile_change("profiles.addOffline", json!({"name":"Alex"}))
                .unwrap_err()
                .code,
            "ACCOUNT_BUSY"
        );
        assert_eq!(
            core.select_profile("legacy-offline").unwrap_err().code,
            "ACCOUNT_BUSY"
        );
        assert_eq!(
            core.delete_profile("legacy-offline", "Player")
                .unwrap_err()
                .code,
            "ACCOUNT_BUSY"
        );
        assert_eq!(
            core.add_microsoft_profile().unwrap_err().code,
            "ACCOUNT_BUSY"
        );
        job.finish(Ok(()));
    }
    let game = core.tasks.start("launch", Some("same-instance")).unwrap();
    game.phase("running", "running");
    core.select_profile("legacy-offline").unwrap();
    let next = core.tasks.start("launch", Some("same-instance")).unwrap();
    assert!(core.tasks.start("install", Some("same-instance")).is_err());
    assert!(core.tasks.start("launch", Some("same-instance")).is_err());
    next.phase("running", "running");
    game.phase("exited", "exited");
    assert!(core.tasks.start("install", Some("same-instance")).is_err());
}

#[tokio::test]
async fn legacy_credentials_are_preserved_and_cancelled_add_does_not_replace_them() {
    let test = tempfile::tempdir().unwrap();
    let root = test.path().join("data");
    let core = Launcher::open(Some(root.clone())).unwrap();
    fs::write(
        root.join("credentials.bin"),
        b"existing encrypted credential",
    )
    .unwrap();
    let view = core.view().unwrap();
    assert!(
        view.account
            .profiles
            .iter()
            .any(|p| p.id == "legacy-microsoft")
    );
    // Provider jobs are lazy. Cancel before polling, so no browser or network is opened.
    let begin = core
        .accounts
        .profiles(&root, &view.settings, "signIn", json!({"newProfile":true}))
        .unwrap();
    core.accounts
        .profiles(
            &root,
            &view.settings,
            "cancel",
            json!({"handle":begin["handle"]}),
        )
        .unwrap();
    let after = core.view().unwrap();
    assert_eq!(
        after.account.selected_profile_id.as_deref(),
        Some("legacy-microsoft")
    );
    assert_eq!(after.account.profiles.len(), 2);
    assert_eq!(
        fs::read(root.join("credentials.bin")).unwrap(),
        b"existing encrypted credential"
    );
    let public = serde_json::to_string(&after.account).unwrap();
    assert!(!public.contains("refreshToken"));
    assert!(!public.contains("access_token"));
}

#[tokio::test]
async fn deleting_one_microsoft_profile_keeps_other_credentials_and_game_files() {
    let test = tempfile::tempdir().unwrap();
    let root = test.path().join("data");
    let core = Launcher::open(Some(root.clone())).unwrap();
    let a = uuid::Uuid::new_v4().to_string();
    let b = uuid::Uuid::new_v4().to_string();
    for id in [&a, &b] {
        fs::create_dir_all(root.join("accounts").join(id)).unwrap();
        fs::write(
            root.join("accounts").join(id).join("credentials.bin"),
            id.as_bytes(),
        )
        .unwrap();
    }
    fs::write(
        root.join("profiles.json"),
        serde_json::to_vec(&json!({"schemaVersion":1,"profiles":[
        {"id":a,"kind":"microsoft","name":"Steve","uuid":"00000000000000000000000000000001"},
        {"id":b,"kind":"microsoft","name":"Alex","uuid":"00000000000000000000000000000002"}
    ],"microsoft":a,"offline":null}))
        .unwrap(),
    )
    .unwrap();
    core.profile_change("profiles.delete", json!({"id":a,"confirmation":"Steve"}))
        .unwrap();
    assert!(
        !root
            .join("accounts")
            .join(&a)
            .join("credentials.bin")
            .exists()
    );
    assert_eq!(
        fs::read(root.join("accounts").join(&b).join("credentials.bin")).unwrap(),
        b.as_bytes()
    );
    let view = core.view().unwrap();
    assert_eq!(view.account.profiles.len(), 1);
    assert_eq!(
        view.account.selected_profile_id.as_deref(),
        Some(b.as_str())
    );
    assert_eq!(view.account.status, "signedOut");
}

#[tokio::test]
async fn unavailable_provider_cannot_be_selected_and_corrupt_registry_is_preserved() {
    let test = tempfile::tempdir().unwrap();
    let root = test.path().join("data");
    let core = Launcher::open(Some(root.clone())).unwrap();
    let mut settings = core.view().unwrap().settings;
    settings.modules.insert("account.offline".into(), false);
    core.save_settings(settings).unwrap();
    assert_eq!(
        core.select_profile("legacy-offline").unwrap_err().code,
        "PLUGIN_DISABLED"
    );
    assert_eq!(core.view().unwrap().account.mode, "microsoft");
    let invalid = br#"{"schemaVersion":99}"#;
    fs::write(root.join("profiles.json"), invalid).unwrap();
    assert!(
        core.profile_change("profiles.addOffline", json!({"name":"Alex"}))
            .is_err()
    );
    assert_eq!(fs::read(root.join("profiles.json")).unwrap(), invalid);
}
