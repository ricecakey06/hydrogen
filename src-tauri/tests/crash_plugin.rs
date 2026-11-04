mod common;

use hydrogen::{launcher::storage::Settings, plugins::manager};
use serde_json::json;
use std::{fs, io::Read, time::UNIX_EPOCH};

const TASK: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn crash_plugin_uses_current_logs_and_exports_an_immutable_redacted_snapshot() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("launcher");
    let game = fixture.path().join("game");
    fs::create_dir_all(game.join("crash-reports")).unwrap();
    fs::create_dir_all(root.join("logs")).unwrap();
    let old = game.join("crash-reports/crash-old-client.txt");
    fs::write(&old, "UnsupportedClassVersionError: stale report").unwrap();
    fs::File::options()
        .write(true)
        .open(old)
        .unwrap()
        .set_times(
            fs::FileTimes::new().set_modified(UNIX_EPOCH + std::time::Duration::from_secs(10)),
        )
        .unwrap();
    // The old log is dated at 10 seconds; avoid filesystem/wall-clock precision races.
    let started = 20_000_u64;
    fs::write(
        root.join(format!("logs/game-{TASK}.log")),
        "OutOfMemoryError: Java heap space\n--accessToken console-secret-value",
    )
    .unwrap();
    let crash = game.join("crash-reports/crash-current-client.txt");
    fs::write(&crash, format!("MixinApplyError: test mod\nrefresh_token=private-refresh-value\nAuthorization: Bearer private-bearer-value\n{}", game.display())).unwrap();
    let host = common::plugin_host(&root);
    let settings = Settings::default();
    let report = host.call_service(&settings, "minecraft.crash", "minecraft.crash", "analyze", json!({
        "root":root,"directory":game,"taskId":TASK,"exitCode":1,"startedAt":started,
        "instanceName":"Test instance","gameVersion":"1.21.1","javaVersion":"21","javaArchitecture":"x64","maxMemoryMib":4096
    })).unwrap();
    let causes = report["causes"].as_array().unwrap();
    assert!(causes.iter().any(|c| c["id"] == "heap-memory"));
    assert!(causes.iter().any(|c| c["id"] == "mixin"));
    assert!(!causes.iter().any(|c| c["id"] == "java-version"));
    assert_eq!(
        std::path::Path::new(report["logPath"].as_str().unwrap()),
        crash
    );
    fs::write(&crash, "replaced by next launch").unwrap();
    let exported = host
        .call_service(
            &settings,
            "minecraft.crash",
            "minecraft.crash",
            "export",
            json!({"root":root,"taskId":TASK,"destination":fixture.path()}),
        )
        .unwrap();
    let path = std::path::Path::new(exported["path"].as_str().unwrap());
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    assert_eq!(zip.len(), 4); // Summary, metadata, current crash report and console.
    let mut all = String::new();
    for index in 0..zip.len() {
        let mut text = String::new();
        zip.by_index(index)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        all.push_str(&text);
    }
    assert!(all.contains("MixinApplyError"));
    assert!(all.contains("OutOfMemoryError"));
    assert!(!all.contains("stale report"));
    assert!(!all.contains("replaced by next launch"));
    for secret in [
        "console-secret-value",
        "private-refresh-value",
        "private-bearer-value",
    ] {
        assert!(!all.contains(secret));
    }
    assert!(!all.contains(&game.to_string_lossy().to_string()));
    assert!(!all.contains(&root.to_string_lossy().to_string()));
    let metadata: serde_json::Value = {
        let mut text = String::new();
        zip.by_name("report.json")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        serde_json::from_str(&text).unwrap()
    };
    assert_eq!(metadata["logPath"], "[local log]");
    let second = host
        .call_service(
            &settings,
            "minecraft.crash",
            "minecraft.crash",
            "export",
            json!({"root":root,"taskId":TASK,"destination":fixture.path()}),
        )
        .unwrap();
    assert_ne!(exported["path"], second["path"]);
    host.stop_all().await.unwrap();
}

#[tokio::test]
async fn missing_logs_have_unknown_cause_and_plugin_is_optional_for_launch() {
    let fixture = tempfile::tempdir().unwrap();
    let host = common::plugin_host(fixture.path());
    let settings = Settings::default();
    let report = host.call_service(&settings, "minecraft.crash", "minecraft.crash", "analyze", json!({
        "root":fixture.path(),"directory":fixture.path(),"taskId":TASK,"exitCode":-1073741819,"startedAt":0
    })).unwrap();
    assert_eq!(report["causes"][0]["id"], "unknown");
    assert!(report["files"].as_array().unwrap().is_empty());
    assert!(!report["notes"].as_array().unwrap().is_empty());
    let invalid = host
        .call_service(
            &settings,
            "minecraft.crash",
            "minecraft.crash",
            "export",
            json!({"root":fixture.path(),"taskId":"../outside","destination":fixture.path()}),
        )
        .unwrap_err();
    assert_eq!(invalid.code, "INVALID_TASK");
    let mut disabled = settings.clone();
    disabled.modules.insert("minecraft.crash".into(), false);
    let plugins = manager::discover(fixture.path(), &[]).unwrap();
    assert!(manager::order(&plugins, &disabled, "minecraft.launch").is_ok());
    assert!(
        host.call_service(
            &disabled,
            "minecraft.crash",
            "minecraft.crash",
            "analyze",
            json!({})
        )
        .is_err()
    );
    host.stop_all().await.unwrap();
}

#[tokio::test]
async fn export_rejects_snapshot_path_traversal() {
    let fixture = tempfile::tempdir().unwrap();
    let host = common::plugin_host(fixture.path());
    let settings = Settings::default();
    let mut report = host.call_service(&settings, "minecraft.crash", "minecraft.crash", "analyze", json!({
        "root":fixture.path(),"directory":fixture.path(),"taskId":TASK,"exitCode":1,"startedAt":0
    })).unwrap();
    report["files"] = json!(["../../settings.json"]);
    fs::write(
        fixture
            .path()
            .join(format!("crash-reports/{TASK}/report.json")),
        serde_json::to_vec(&report).unwrap(),
    )
    .unwrap();
    let error = host
        .call_service(
            &settings,
            "minecraft.crash",
            "minecraft.crash",
            "export",
            json!({"root":fixture.path(),"taskId":TASK,"destination":fixture.path()}),
        )
        .unwrap_err();
    assert_eq!(error.code, "INVALID_REPORT");
    assert!(!fs::read_dir(fixture.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|e| e == "zip")
    }));
    host.stop_all().await.unwrap();
}
