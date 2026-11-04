mod common;
use hydrogen::{
    launcher::{http::Client, storage::Settings},
    plugins::{manager, manifest},
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[tokio::test]
async fn network_plugin_owns_streams_cancels_pending_requests_and_has_no_missing_entry_fallback() {
    let root = tempfile::tempdir().unwrap();
    let host = common::plugin_host(root.path());
    let settings = Settings::default();
    let client = Client::new(&host, &settings).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = stream.read(&mut request).await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\npayload")
            .await
            .unwrap();
    });
    let mut response = client.get(&url).send().await.unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(response.chunk().await.unwrap().unwrap(), b"payload");
    assert!(response.chunk().await.unwrap().is_none());
    drop(response);
    server.await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = stream.read(&mut request).await;
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    });
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            client.get(url).send()
        )
        .await
        .is_err()
    );
    let stats = host
        .call_service(&settings, "network", "network", "stats", json!({}))
        .unwrap();
    assert_eq!(stats["operations"], 0);
    assert_eq!(stats["streams"], 0);
    server.abort();
    assert!(
        client
            .get("file:///C:/Windows/win.ini")
            .send()
            .await
            .is_err()
    );
    let plugin = manager::discover(root.path(), &[])
        .unwrap()
        .into_iter()
        .find(|p| p.manifest.id == "network")
        .unwrap();
    std::fs::remove_file(manifest::within(&plugin.root, &plugin.manifest.entry).unwrap()).unwrap();
    assert!(
        client
            .get("https://piston-meta.mojang.com/")
            .send()
            .await
            .is_err()
    );
    let stats = host.stop_all().await.unwrap();
    assert_eq!(stats.created, stats.destroyed);
}
