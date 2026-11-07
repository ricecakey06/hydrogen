mod common;
use hydrogen::{
    contracts::ApiError,
    launcher::{net, storage::Settings},
};
use serde_json::json;
use std::{path::PathBuf, time::Duration};

#[tokio::test]
#[ignore = "downloads HYDROGEN_TEST_MODPACK; set HYDROGEN_TEST_MOD_MIRROR=0 to compare origins"]
async fn live_modpack_files_install_and_release_network_handles() {
    let source = PathBuf::from(std::env::var("HYDROGEN_TEST_MODPACK").unwrap());
    let fixture = tempfile::tempdir().unwrap();
    println!("Benchmark scratch directory: {}", fixture.path().display());
    let host = common::plugin_host(&fixture.path().join("data"));
    let settings = Settings {
        use_mod_mirror: std::env::var("HYDROGEN_TEST_MOD_MIRROR").as_deref() != Ok("0"),
        ..Settings::default()
    };
    let client = net::client(&host, &settings).unwrap();
    let preview = client
        .service_async("minecraft.modpacks", "preview", json!({"source":source}))
        .await
        .unwrap();
    let stage = fixture.path().join("stage");
    std::fs::create_dir(&stage).unwrap();
    let started = std::time::Instant::now();
    let mut downloading = None;
    let mut last_report = started;
    let mut peak_requests = 0;
    let mut planned = Vec::<(String, Option<u64>)>::new();
    let cancel = std::env::var("HYDROGEN_TEST_MODPACK_CANCEL").as_deref() == Ok("1");
    let result = tokio::time::timeout(
        Duration::from_secs(600),
        net::task_observed(
            &client,
            "minecraft.modpacks",
            "prepare",
            json!({"source":source,"stage":stage,"directory":"Modpack benchmark","optional":false,"downloadsEnabled":true,"preview":preview}),
            None,
            |progress| {
                let stats = client.service("network", "stats", json!({}))?;
                peak_requests = peak_requests.max(stats["operations"].as_u64().unwrap() + stats["streams"].as_u64().unwrap());
                if progress["phase"] == "downloading" && downloading.is_none() {
                    downloading = Some(std::time::Instant::now());
                    planned = serde_json::from_value(progress["plan"]["files"].clone()).unwrap();
                    println!("Planning: {:?}; {} files, {} bytes; mirror={}", started.elapsed(), planned.len(), planned.iter().map(|(_, size)| size.unwrap_or(0)).sum::<u64>(), settings.use_mod_mirror);
                }
                if cancel && downloading.is_some() && peak_requests > 1 {
                    return Err(ApiError::new("PROBE_CANCELLED", "Cancel concurrent downloads."));
                }
                if last_report.elapsed() >= Duration::from_secs(5) {
                    let done = progress["files"].as_object().map_or(0, |files| files.values().filter(|file| file["done"] == true).count());
                    let bytes = progress["files"].as_object().map_or(0, |files| files.values().map(|file| file["bytes"].as_u64().unwrap_or(0)).sum::<u64>());
                    let active = progress["files"].as_object().map_or_else(Vec::new, |files| files.iter().filter(|(_, file)| file["done"] != true).take(3).map(|(path, _)| path.as_str()).collect());
                    println!("{:?}: phase={} done={} bytes={} active={:?} network={}", started.elapsed(), progress["phase"], done, bytes, active, stats);
                    last_report = std::time::Instant::now();
                }
                Ok(())
            },
        ),
    ).await;
    let stats = client.service("network", "stats", json!({})).unwrap();
    println!(
        "Total: {:?}; download+extract: {:?}; peak requests: {}; network={}",
        started.elapsed(),
        downloading.map(|time| time.elapsed()),
        peak_requests,
        stats
    );
    assert_eq!(stats["operations"], 0);
    assert_eq!(stats["streams"], 0);
    host.stop_all().await.unwrap();
    let result = result.expect("Modpack installation timed out");
    if cancel {
        assert_eq!(result.unwrap_err().code, "PROBE_CANCELLED");
        return;
    }
    result.unwrap();
    for (path, size) in planned {
        assert_eq!(
            std::fs::metadata(stage.join(path)).unwrap().len(),
            size.unwrap()
        );
    }
}
