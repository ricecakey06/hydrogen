mod common;
use hydrogen::launcher::{net, repository, storage::Settings, tasks::Tasks};
use serde_json::json;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

// Opt-in real repository check. Does not start Java or allow downloads.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires HYDROGEN_PREPARE_GAME_ROOT and a populated local repository"]
async fn cached_repository_preparation() {
    let root = PathBuf::from(std::env::var("HYDROGEN_PREPARE_GAME_ROOT").unwrap());
    let name = std::env::var("HYDROGEN_PREPARE_VERSION").unwrap_or_else(|_| "26.1.2".into());
    let fixture = tempfile::tempdir().unwrap();
    let host = common::plugin_host(fixture.path());
    let mut settings = Settings::default();
    settings.modules.insert("minecraft.download".into(), false);
    let client = net::client(&host, &settings).unwrap();
    let metadata = repository::load(&client, &root, &name).unwrap().metadata;
    let job = Tasks::default().start("launch", None).unwrap();
    let cancel = job.cancel.clone();
    let limit = std::env::var("HYDROGEN_PREPARE_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(90);
    let watchdog = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(limit)).await;
        cancel.cancel();
    });
    let started = Instant::now();
    let mut reported = Instant::now() - Duration::from_secs(5);
    let result = net::task_observed(
        &client,
        "minecraft.preinstall",
        "prepare",
        json!({"gameRoot":root,"settings":settings,"metadata":metadata,"architecture":"x64"}),
        Some(&job),
        |p| {
            if reported.elapsed() >= Duration::from_secs(2) {
                let files = p["files"].as_object();
                eprintln!(
                    "elapsed={:?} reported={} done={} plan={}",
                    started.elapsed(),
                    files.map_or(0, |v| v.len()),
                    files.map_or(0, |v| v.values().filter(|f| f["done"] == true).count()),
                    p["plan"]["files"].as_array().map_or(0, |v| v.len())
                );
                reported = Instant::now();
            }
            Ok(())
        },
    )
    .await;
    watchdog.abort();
    eprintln!(
        "prepare elapsed={:?} result={:?}",
        started.elapsed(),
        result.as_ref().map(|_| "complete")
    );
    host.stop_all().await.unwrap();
    assert!(result.is_ok(), "{result:?}");
}
