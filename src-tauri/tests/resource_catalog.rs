mod common;
use hydrogen::launcher::{net, storage::Settings};
use serde_json::json;
use std::time::{Duration, Instant};

#[tokio::test]
#[ignore = "requires live Modrinth and MCIM catalog access"]
async fn live_catalog_search_returns_distinct_keyword_results() {
    let fixture = tempfile::tempdir().unwrap();
    let host = common::plugin_host(fixture.path());
    let client = net::client(&host, &Settings::default()).unwrap();
    let mut previous = Vec::new();
    for query in ["sodium", "iris"] {
        let start = Instant::now();
        let result = tokio::time::timeout(
            Duration::from_secs(60),
            net::task(&client, "minecraft.resources", "search", json!({
                "query":query,"gameVersion":"","category":"","loader":"","offset":0,"source":"modrinth"
            }), None),
        ).await.expect("catalog search timed out").unwrap();
        let hits = result["hits"].as_array().unwrap();
        println!(
            "query={query} elapsed={:?} total={} titles={:?}",
            start.elapsed(),
            result["total_hits"],
            hits.iter()
                .take(3)
                .map(|hit| hit["title"].as_str().unwrap())
                .collect::<Vec<_>>()
        );
        assert!(!hits.is_empty());
        let ids: Vec<_> = hits
            .iter()
            .map(|hit| hit["project_id"].as_str().unwrap().to_owned())
            .collect();
        assert_ne!(ids, previous);
        previous = ids;
    }
    host.stop_all().await.unwrap();
}
