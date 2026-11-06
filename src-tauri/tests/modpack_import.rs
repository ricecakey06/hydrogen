mod common;
use hydrogen::{
    contracts::ApiError,
    launcher::{net, storage::Settings},
};
use serde_json::json;
use std::{path::PathBuf, time::Duration};

#[tokio::test]
#[ignore = "requires HYDROGEN_TEST_MODPACK pointing to a local CurseForge pack"]
async fn live_modpack_metadata_reaches_download_planning() {
    let source = PathBuf::from(std::env::var("HYDROGEN_TEST_MODPACK").unwrap());
    let fixture = tempfile::tempdir().unwrap();
    let data = fixture.path().join("data");
    let host = common::plugin_host(&data);
    let client = net::client(&host, &Settings::default()).unwrap();
    let started = std::time::Instant::now();
    let preview = client
        .service_async("minecraft.modpacks", "preview", json!({"source":source}))
        .await
        .unwrap();
    println!(
        "Local manifest read in {:?}: {} files",
        started.elapsed(),
        preview["files"]
    );
    let expected_files = preview["files"].as_u64().unwrap() as usize;
    let stage = fixture.path().join("stage");
    std::fs::create_dir(&stage).unwrap();
    let mut last_message = String::new();
    let mut last_report = std::time::Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(90),
        net::task_observed(
            &client,
            "minecraft.modpacks",
            "prepare",
            json!({"source":source,"stage":stage,"directory":"Modpack probe","optional":false,"downloadsEnabled":true,"preview":preview}),
            None,
            |progress| {
                let message = progress["message"].as_str().unwrap_or("");
                if last_message != message || last_report.elapsed() >= Duration::from_secs(2) {
                    let stats = client.service("network", "stats", json!({})).unwrap();
                    println!("{:?}: phase={} message={} network={}", started.elapsed(), progress["phase"], message, stats);
                    last_message = message.into();
                    last_report = std::time::Instant::now();
                }
                if progress["phase"] == "downloading" {
                    assert_eq!(progress["plan"]["files"].as_array().unwrap().len(), expected_files);
                    return Err(ApiError::new("PROBE_COMPLETE", "Download planning reached."));
                }
                Ok(())
            },
        ),
    )
    .await;
    let stats = client.service("network", "stats", json!({})).unwrap();
    println!("Finished after {:?}; network={}", started.elapsed(), stats);
    assert_eq!(stats["operations"], 0);
    assert_eq!(stats["streams"], 0);
    host.stop_all().await.unwrap();
    let error = result.expect("Modpack metadata timed out").unwrap_err();
    assert_eq!(error.code, "PROBE_COMPLETE", "{}", error.message);
}
