mod common;
use hydrogen::launcher::{credentials, diagnostics::Log, environment, storage::Settings};
use serde_json::{Value, json};
use std::fs;

#[cfg(windows)]
#[tokio::test]
async fn credentials_are_protected_by_the_real_plugin_and_missing_code_never_falls_back() {
    let root = tempfile::tempdir().unwrap();
    let host = common::plugin_host(root.path());
    let settings = Settings::default();
    let mut plain = b"a-fake-refresh-token-for-the-test".to_vec();
    let mut encrypted = credentials::transform(&host, &settings, &mut plain, true).unwrap();
    assert!(plain.iter().all(|b| *b == 0));
    assert_ne!(encrypted, b"a-fake-refresh-token-for-the-test");
    let mut restored = credentials::transform(&host, &settings, &mut encrypted, false).unwrap();
    assert_eq!(restored, b"a-fake-refresh-token-for-the-test");
    assert!(encrypted.iter().all(|b| *b == 0));
    restored.fill(0);
    fs::remove_file(root.path().join("plugins/credentials/bin/plugin.dll")).unwrap();
    let mut input = b"must-be-cleared-on-error".to_vec();
    assert!(credentials::transform(&host, &settings, &mut input, true).is_err());
    assert!(input.iter().all(|b| *b == 0));
    let stats = host.stop_all().await.unwrap();
    assert_eq!(stats.created, stats.destroyed);
}

#[tokio::test]
async fn diagnostic_plugin_owns_redaction_and_file_handles() {
    let root = tempfile::tempdir().unwrap();
    let host = common::plugin_host(root.path());
    let path = root.path().join("logs/test.log");
    {
        let mut log = Log::create(
            &host,
            &Settings::default(),
            &path,
            vec!["fake-secret-token-123".into()],
        )
        .unwrap();
        log.line("hello fake-secret-token-123 world");
    }
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "hello [redacted] world\n"
    );
    let stats = host.stop_all().await.unwrap();
    assert_eq!(stats.created, stats.destroyed);
    fs::rename(&path, root.path().join("closed.log")).unwrap();
}

#[tokio::test]
async fn environment_provider_rejects_unauthorized_browser_schemes_and_reports_disk_space() {
    let root = tempfile::tempdir().unwrap();
    let host = common::plugin_host(root.path());
    assert_eq!(
        environment::browser(&host, &Settings::default(), "file:///example.exe")
            .unwrap_err()
            .code,
        "INVALID_BROWSER_URL"
    );
    let space = environment::available_space(&host, &Settings::default(), root.path()).unwrap();
    #[cfg(windows)]
    assert!(space.is_some_and(|bytes| bytes > 0));
    #[cfg(not(windows))]
    assert!(space.is_none());
    host.stop_all().await.unwrap();
}

#[test]
#[ignore = "child process fixture, invoked only through the process plugin"]
fn process_child_fixture() {
    assert_eq!(std::env::var("HYDROGEN_PROCESS_FIXTURE").unwrap(), "1");
    println!("native child stdout");
    eprintln!("native child stderr");
    std::thread::sleep(std::time::Duration::from_millis(50));
}

#[tokio::test]
async fn process_provider_tracks_identity_and_drains_both_pipes_without_a_host_process_implementation()
 {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let root = tempfile::tempdir().unwrap();
    let host = common::plugin_host(root.path());
    let settings = Settings::default();
    let started = host.call_service(&settings, "process", "process", "spawn", json!({"executable":std::env::current_exe().unwrap(),"directory":root.path(),"args":["--ignored","--exact","process_child_fixture","--nocapture"],"environment":{"HYDROGEN_PROCESS_FIXTURE":"1"}})).unwrap();
    #[cfg(any(windows, target_os = "linux"))]
    assert!(started["stamp"].as_u64().is_some());
    let mut out = Vec::new();
    let mut err = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        assert!(std::time::Instant::now() < deadline);
        let value = host
            .call_service(
                &settings,
                "process",
                "process",
                "poll",
                json!({"handle":started["handle"]}),
            )
            .unwrap();
        out.extend(STANDARD.decode(value["stdout"].as_str().unwrap()).unwrap());
        err.extend(STANDARD.decode(value["stderr"].as_str().unwrap()).unwrap());
        if value["exitCode"] != Value::Null
            && value["stdoutEof"] == true
            && value["stderrEof"] == true
        {
            assert_eq!(value["exitCode"], 0);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(String::from_utf8_lossy(&out).contains("native child stdout"));
    assert!(String::from_utf8_lossy(&err).contains("native child stderr"));
    host.call_service(
        &settings,
        "process",
        "process",
        "detach",
        json!({"handle":started["handle"]}),
    )
    .unwrap();
    let stats = host.stop_all().await.unwrap();
    assert_eq!(stats.created, stats.destroyed);
}
