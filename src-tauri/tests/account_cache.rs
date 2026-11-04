#![cfg(all(feature = "desktop", windows))]
mod common;
use base64::{Engine, engine::general_purpose::STANDARD};
use hydrogen::launcher::{core::Launcher, credentials};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

fn skin_fixture(alex: bool) -> String {
    let bytes: &[u8] = if alex {
        include_bytes!("fixtures/skin-alex.png")
    } else {
        include_bytes!("fixtures/skin-steve.png")
    };
    format!("data:image/png;base64,{}", STANDARD.encode(bytes))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn cached_profile(core: &Launcher, root: &Path, id: &str, name: &str, expires: u64, skin: &str) {
    let settings = core.store.lock().unwrap().settings.clone();
    let directory = root.join("accounts").join(id);
    fs::create_dir_all(&directory).unwrap();
    let mut plain = serde_json::to_vec(&json!({
        "schemaVersion":1,"clientId":settings.microsoft_client_id,"refreshToken":"test-refresh-token",
        "session":{"name":name,"uuid":id.replace('-',""),"access_token":"test-access-token",
            "xuid":"","client_id":settings.microsoft_client_id,"expires":expires,"offline":false},
        "skin":skin
    })).unwrap();
    let encrypted = credentials::transform(&core.plugins, &settings, &mut plain, true).unwrap();
    assert!(plain.iter().all(|b| *b == 0));
    assert!(!String::from_utf8_lossy(&encrypted).contains("test-access-token"));
    fs::write(directory.join("credentials.bin"), encrypted).unwrap();
}

fn registry(root: &Path, profiles: &[(&str, &str)]) {
    let records: Vec<Value> = profiles
        .iter()
        .map(|(id, name)| json!({"id":id,"kind":"microsoft","name":name,"uuid":id.replace('-',"")}))
        .collect();
    fs::write(
        root.join("profiles.json"),
        serde_json::to_vec(
            &json!({"schemaVersion":1,"profiles":records,"microsoft":profiles[0].0,"offline":null}),
        )
        .unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn startup_reuses_encrypted_session_name_and_skin_without_a_login_task() {
    let test = tempfile::tempdir().unwrap();
    let root = test.path().join("data");
    let core = Launcher::open(Some(root.clone())).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let skin = skin_fixture(false);
    cached_profile(&core, &root, &id, "CachedSteve", now() + 7200, &skin);
    registry(&root, &[(&id, "CachedSteve")]);
    drop(core);
    let reopened = Launcher::open(Some(root.clone())).unwrap();
    reopened.restore();
    let view = reopened.view().unwrap();
    assert_eq!(view.account.status, "signedIn");
    assert_eq!(view.account.player_name.as_deref(), Some("CachedSteve"));
    assert_eq!(view.account.skin.as_deref(), Some(skin.as_str()));
    assert!(view.tasks.iter().all(|t| t.kind != "login"));
    let session = reopened
        .accounts
        .session(&reopened.client, &root, &view.settings)
        .await
        .unwrap();
    assert_eq!(session.name, "CachedSteve");
    assert_eq!(session.access_token, "test-access-token");
    let public = serde_json::to_string(&view.account).unwrap();
    assert!(!public.contains("test-access-token"));
    assert!(!public.contains("test-refresh-token"));
    // Even an explicitly requested restore is identified as background work immediately.
    let task_id = reopened.login(true).unwrap();
    let task = reopened
        .tasks
        .list()
        .into_iter()
        .find(|t| t.id == task_id)
        .unwrap();
    assert_ne!(task.phase, "queued");
    assert_ne!(task.phase, "authorizing");
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while reopened
            .tasks
            .list()
            .iter()
            .any(|t| t.id == task_id && t.active())
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(reopened.view().unwrap().account.status, "signedIn");
}

#[tokio::test]
async fn expired_cache_keeps_identity_visible_during_restore_and_after_network_failure() {
    let test = tempfile::tempdir().unwrap();
    let root = test.path().join("data");
    let core = Launcher::open(Some(root.clone())).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let skin = skin_fixture(false);
    cached_profile(&core, &root, &id, "CachedAlex", now() - 60, &skin);
    registry(&root, &[(&id, "CachedAlex")]);
    // Simulate the startup task without submitting a test token to Microsoft's servers.
    let job = core.tasks.start("login", None).unwrap();
    job.phase("restoring", "正在恢复账号…");
    let view = core.view().unwrap();
    assert_eq!(view.account.status, "cached");
    assert_eq!(view.account.player_name.as_deref(), Some("CachedAlex"));
    assert_eq!(view.account.skin.as_deref(), Some(skin.as_str()));
    assert_ne!(view.account.status, "signedIn"); // Expired tokens cannot enable game launch.
    core.accounts
        .failure(&root, &view.settings, "测试网络暂时不可用");
    job.finish(Ok(()));
    let after = core.view().unwrap();
    assert_eq!(after.account.player_name, view.account.player_name);
    assert_eq!(after.account.skin, view.account.skin);
    assert_eq!(after.account.detail.as_deref(), Some("测试网络暂时不可用"));
    assert_eq!(after.account.status, "cached");
    assert_eq!(
        core.accounts.view(&root, &view.settings, true).status,
        "signingIn"
    );
}

#[tokio::test]
async fn profile_list_exposes_each_cached_skin_while_an_offline_profile_is_selected() {
    let test = tempfile::tempdir().unwrap();
    let root = test.path().join("data");
    let core = Launcher::open(Some(root.clone())).unwrap();
    let a = uuid::Uuid::new_v4().to_string();
    let b = uuid::Uuid::new_v4().to_string();
    let c = uuid::Uuid::new_v4().to_string();
    let d = uuid::Uuid::new_v4().to_string();
    let steve_skin = skin_fixture(false);
    let alex_skin = skin_fixture(true);
    cached_profile(&core, &root, &a, "Steve", now() + 7200, &steve_skin);
    cached_profile(&core, &root, &b, "Alex", now() - 60, &alex_skin);
    cached_profile(&core, &root, &c, "InvalidSkin", now() + 7200, "invalid");
    registry(
        &root,
        &[
            (&a, "Steve"),
            (&b, "Alex"),
            (&c, "InvalidSkin"),
            (&d, "NoCache"),
        ],
    );
    let offline = core
        .profile_change("profiles.addOffline", json!({"name":"OfflinePlayer"}))
        .unwrap();
    core.select_profile(offline["id"].as_str().unwrap())
        .unwrap();

    let view = core.view().unwrap();
    assert_eq!(view.account.mode, "offline");
    assert!(view.account.skin.is_none());
    let profiles = &view.account.profiles;
    assert_eq!(
        profiles.iter().find(|p| p.id == a).unwrap().skin.as_deref(),
        Some(steve_skin.as_str())
    );
    assert_eq!(
        profiles.iter().find(|p| p.id == b).unwrap().skin.as_deref(),
        Some(alex_skin.as_str())
    );
    assert!(
        profiles
            .iter()
            .filter(|p| p.id != a && p.id != b)
            .all(|p| p.skin.is_none())
    );
    assert!(view.tasks.is_empty());
    let listed = core
        .accounts
        .profiles(&root, &view.settings, "profiles.list", json!({}))
        .unwrap();
    assert_eq!(listed["profiles"], serde_json::to_value(profiles).unwrap());
    let public = serde_json::to_string(&view.account).unwrap();
    assert!(!public.contains("test-access-token"));
    assert!(!public.contains("test-refresh-token"));
    assert!(
        !fs::read_to_string(root.join("profiles.json"))
            .unwrap()
            .contains("data:image")
    );

    let mut settings = view.settings;
    settings.modules.insert("account.microsoft".into(), false);
    core.save_settings(settings).unwrap();
    let unavailable = core.view().unwrap();
    assert_eq!(
        unavailable.account.selected_profile_id,
        view.account.selected_profile_id
    );
    assert!(
        unavailable
            .account
            .profiles
            .iter()
            .all(|p| p.skin.is_none())
    );
}

#[tokio::test]
async fn switching_cached_profiles_uses_their_own_identity_and_logout_clears_cache() {
    let test = tempfile::tempdir().unwrap();
    let root = test.path().join("data");
    let core = Launcher::open(Some(root.clone())).unwrap();
    let a = uuid::Uuid::new_v4().to_string();
    let b = uuid::Uuid::new_v4().to_string();
    cached_profile(
        &core,
        &root,
        &a,
        "Steve",
        now() + 7200,
        &skin_fixture(false),
    );
    cached_profile(&core, &root, &b, "Alex", now() + 7200, &skin_fixture(true));
    registry(&root, &[(&a, "Steve"), (&b, "Alex")]);
    core.select_profile(&a).unwrap();
    assert_eq!(
        core.view().unwrap().account.player_name.as_deref(),
        Some("Steve")
    );
    core.select_profile(&b).unwrap();
    let view = core.view().unwrap();
    assert_eq!(view.account.player_name.as_deref(), Some("Alex"));
    assert_eq!(
        view.account.skin.as_deref(),
        Some(skin_fixture(true).as_str())
    );
    assert!(view.tasks.is_empty());
    core.sign_out().unwrap();
    let view = core.view().unwrap();
    assert_eq!(view.account.status, "signedOut");
    assert!(view.account.player_name.is_none());
    assert!(view.account.skin.is_none());
    assert!(
        !root
            .join("accounts")
            .join(&b)
            .join("credentials.bin")
            .exists()
    );
    assert!(
        root.join("accounts")
            .join(&a)
            .join("credentials.bin")
            .exists()
    );
}

#[tokio::test]
async fn old_refresh_only_credentials_reuse_profile_name_without_browser_login_status() {
    let test = tempfile::tempdir().unwrap();
    let root = test.path().join("data");
    let core = Launcher::open(Some(root.clone())).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let settings = core.store.lock().unwrap().settings.clone();
    let mut plain = serde_json::to_vec(&json!({"schemaVersion":1,"clientId":settings.microsoft_client_id,"refreshToken":"legacy-test-token"})).unwrap();
    let encrypted = credentials::transform(&core.plugins, &settings, &mut plain, true).unwrap();
    fs::create_dir_all(root.join("accounts").join(&id)).unwrap();
    fs::write(
        root.join("accounts").join(&id).join("credentials.bin"),
        encrypted,
    )
    .unwrap();
    registry(&root, &[(&id, "LegacyPlayer")]);
    let job = core.tasks.start("login", None).unwrap();
    job.phase("restoring", "正在恢复账号…");
    let view = core.view().unwrap();
    assert_eq!(view.account.status, "cached");
    assert_eq!(view.account.player_name.as_deref(), Some("LegacyPlayer"));
    assert!(view.account.skin.is_none());
    job.finish(Ok(()));
}

#[tokio::test]
async fn missing_or_invalid_avatar_requires_repair_even_with_a_valid_game_session() {
    for skin in ["", "data:image/png;base64,bm90LWEtcG5n"] {
        let test = tempfile::tempdir().unwrap();
        let root = test.path().join("data");
        let core = Launcher::open(Some(root.clone())).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        cached_profile(&core, &root, &id, "Steve", now() + 7200, skin);
        registry(&root, &[(&id, "Steve")]);
        let view = core.view().unwrap();
        assert_eq!(view.account.status, "signedIn");
        assert_eq!(view.account.player_name.as_deref(), Some("Steve"));
        assert!(view.account.skin.is_none());
        assert!(view.account.needs_restore());
    }
}
