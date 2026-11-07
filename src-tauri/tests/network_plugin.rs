mod common;
use hydrogen::{
    launcher::{http::Client, storage::Settings},
    plugins::{manager, manifest},
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn fast_streams_are_batched_and_slow_streams_still_return_partial_data() {
    let root = tempfile::tempdir().unwrap();
    let host = common::plugin_host(root.path());
    let client = Client::new(&host, &Settings::default()).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let payload = vec![42; 8 * 1024 * 1024];
    let body = payload.clone();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        stream.read(&mut request).await.unwrap();
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        for bytes in body.chunks(16 * 1024) {
            stream
                .write_all(format!("{:x}\r\n", bytes.len()).as_bytes())
                .await
                .unwrap();
            stream.write_all(bytes).await.unwrap();
            stream.write_all(b"\r\n").await.unwrap();
        }
        stream.write_all(b"0\r\n\r\n").await.unwrap();
    });
    let started = std::time::Instant::now();
    let mut response = client.get(url).send().await.unwrap();
    let mut received = Vec::new();
    let mut calls = 0;
    while let Some(bytes) = response.chunk().await.unwrap() {
        calls += 1;
        assert!(bytes.len() <= 128 * 1024);
        received.extend(bytes);
    }
    drop(response);
    server.await.unwrap();
    println!(
        "8 MiB streamed in {:?}, using {calls} chunk calls",
        started.elapsed()
    );
    assert_eq!(received, payload);
    assert!(
        calls <= 256,
        "fast reads returned tiny network frames ({calls} calls)"
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let (release, wait) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        stream.read(&mut request).await.unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nprefix")
            .await
            .unwrap();
        wait.await.unwrap();
        stream.write_all(b"tail").await.unwrap();
    });
    let mut response = client.get(url).send().await.unwrap();
    let first = tokio::time::timeout(std::time::Duration::from_secs(2), response.chunk()).await;
    release.send(()).unwrap();
    assert_eq!(first.unwrap().unwrap().unwrap(), b"prefix");
    assert_eq!(response.chunk().await.unwrap().unwrap(), b"tail");
    assert!(response.chunk().await.unwrap().is_none());
    drop(response);
    server.await.unwrap();
    assert_eq!(
        client.service("network", "stats", json!({})).unwrap(),
        json!({"operations":0,"streams":0})
    );
    host.stop_all().await.unwrap();
}
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
