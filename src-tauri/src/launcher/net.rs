//! File cache and download service transport; no host transfer implementation.
pub use super::http::{Client, Response};
use super::tasks::Job;
use crate::contracts::{ApiError, Result};
use reqwest::Url;
use serde_json::{Value, json};
use std::path::Path;
pub fn client(
    host: &crate::plugins::host::PluginHost,
    settings: &super::storage::Settings,
) -> Result<Client> {
    Client::new(host, settings)
}
pub fn network_error(error: ApiError) -> ApiError {
    error
}
pub fn trusted(client: &Client, url: &Url) -> Result<bool> {
    Ok(client.service("minecraft.download", "trusted", json!({"url":url.as_str()}))? == true)
}
pub fn source_url(client: &Client, url: &str, mirror: bool) -> Result<String> {
    Ok(serde_json::from_value(client.service(
        "minecraft.download",
        "sourceUrl",
        json!({"url":url,"mirror":mirror}),
    )?)?)
}
pub fn valid_hash(hash: &str) -> bool {
    hash.len() == 40 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}
pub async fn task(
    client: &Client,
    provider: &str,
    method: &str,
    params: Value,
    job: Option<&Job>,
) -> Result<Value> {
    task_observed(client, provider, method, params, job, |_| Ok(())).await
}
pub async fn task_observed(
    client: &Client,
    provider: &str,
    method: &str,
    params: Value,
    job: Option<&Job>,
    mut observe: impl FnMut(&Value) -> Result<()>,
) -> Result<Value> {
    struct Pending<'a> {
        client: &'a Client,
        provider: &'a str,
        id: String,
    }
    impl Drop for Pending<'_> {
        fn drop(&mut self) {
            let _ = self
                .client
                .service(self.provider, "cancel", json!({"handle":self.id}));
        }
    }
    if let Some(job) = job {
        job.check()?;
    }
    let id = client.service_async(provider, method, params).await?;
    let pending = Pending {
        client,
        provider,
        id: id
            .as_str()
            .ok_or_else(|| ApiError::new("INVALID_RESPONSE", "无效任务句柄。"))?
            .into(),
    };
    let mut previous_plan = String::new();
    loop {
        if let Some(job) = job {
            job.check()?;
        }
        let result = client
            .service_async(provider, "poll", json!({"handle":pending.id}))
            .await?;
        if let Some(job) = job
            && let Some(id) = result["progress"]["plan"]["id"].as_str()
            && id != previous_plan
        {
            let files: Vec<(String, Option<u64>)> =
                serde_json::from_value(result["progress"]["plan"]["files"].clone())?;
            job.begin_download(files, result["progress"]["plan"]["countFiles"] == true);
            previous_plan = id.into();
        }
        if let Some(job) = job
            && let Some(files) = result["progress"]["files"].as_object()
        {
            for (name, file) in files {
                job.file_started(name);
                job.file_progress(
                    name,
                    file["bytes"].as_u64().unwrap_or(0),
                    file["done"] == true,
                );
            }
        }
        if let Some(job) = job
            && let Some(phase) = result["progress"]["phase"].as_str()
        {
            job.phase(phase, result["progress"]["message"].as_str().unwrap_or(""));
        }
        observe(&result["progress"])?;
        if result["ready"] == true {
            return Ok(result["result"].clone());
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
pub async fn hash(client: &Client, path: &Path) -> Result<String> {
    Ok(serde_json::from_value(
        task(
            client,
            "minecraft.files",
            "hash",
            json!({"path":path}),
            None,
        )
        .await?,
    )?)
}
#[allow(clippy::too_many_arguments)]
pub async fn ensure(
    client: &Client,
    root: &Path,
    relative: &str,
    url: &str,
    hash: &str,
    size: Option<u64>,
    mirror: bool,
    enabled: bool,
    job: &Job,
) -> Result<std::path::PathBuf> {
    ensure_game_file(
        client,
        root,
        relative,
        url,
        Some(hash),
        size,
        mirror,
        enabled,
        job,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
pub async fn ensure_game_file(
    client: &Client,
    root: &Path,
    relative: &str,
    url: &str,
    hash: Option<&str>,
    size: Option<u64>,
    mirror: bool,
    enabled: bool,
    job: &Job,
) -> Result<std::path::PathBuf> {
    Ok(serde_json::from_value(task(client,"minecraft.files","ensure",json!({"root":root,"relative":relative,"url":url,"hash":hash,"size":size,"mirror":mirror,"enabled":enabled}),Some(job)).await?)?)
}
#[cfg(test)]
async fn ensure_once(
    client: &Client,
    root: &Path,
    relative: &str,
    url: &str,
    hash: &str,
    size: Option<u64>,
    job: &Job,
) -> Result<std::path::PathBuf> {
    Ok(serde_json::from_value(
        task(
            client,
            "minecraft.download",
            "once",
            json!({"root":root,"relative":relative,"url":url,"hash":hash,"size":size}),
            Some(job),
        )
        .await?,
    )?)
}
pub async fn bytes(client: &Client, url: &str, limit: usize) -> Result<Vec<u8>> {
    let url = Url::parse(url).map_err(|_| ApiError::new("INVALID_URL", "下载地址无效。"))?;
    if !trusted(client, &url)? {
        return Err(ApiError::new(
            "UNTRUSTED_URL",
            "下载地址不属于支持的内容服务。",
        ));
    }
    response_bytes(client.get(url).send().await.map_err(network_error)?, limit).await
}
pub async fn response_bytes(mut response: Response, limit: usize) -> Result<Vec<u8>> {
    if !response.status().is_success() {
        let mut error = ApiError::new(
            "HTTP_ERROR",
            format!("内容服务返回 HTTP {}。", response.status().as_u16()),
        );
        error.retryable =
            response.status().is_server_error() || [408, 429].contains(&response.status().as_u16());
        return Err(error);
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(ApiError::new("RESPONSE_LIMIT", "服务响应超过大小限制。"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if bytes.len() + chunk.len() > limit {
            return Err(ApiError::new("RESPONSE_LIMIT", "服务响应超过大小限制。"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
pub async fn json(client: &Client, url: &str) -> Result<serde_json::Value> {
    Ok(serde_json::from_slice(
        &bytes(client, url, 64 * 1024 * 1024).await?,
    )?)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::tasks::Tasks;
    use sha1::{Digest, Sha1};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::{path::Path, time::Duration};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn test_client(_: &Path) -> (tempfile::TempDir, Client) {
        let plugins = tempfile::tempdir().unwrap();
        crate::plugins::manager::initialize(plugins.path()).unwrap();
        let host =
            crate::plugins::host::PluginHost::new(tokio::runtime::Handle::current(), Vec::new());
        host.set_plugins(crate::plugins::manager::discover(plugins.path(), &[]).unwrap());
        let client = client(&host, &super::super::storage::Settings::default()).unwrap();
        (plugins, client)
    }
    // Loopback is used only by this private transfer fixture; public ensure still enforces trusted HTTPS.
    async fn server(
        mode: &'static str,
    ) -> (
        String,
        Arc<Vec<u8>>,
        Arc<AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        let data = Arc::new(
            (0..8 * 1024 * 1024 + 13)
                .map(|i| (i % 251) as u8)
                .collect::<Vec<_>>(),
        );
        let ranges = Arc::new(AtomicUsize::new(0));
        let contents = data.clone();
        let count = ranges.clone();
        let handle = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let data = contents.clone();
                let count = count.clone();
                tokio::spawn(async move {
                    let mut input = Vec::new();
                    let mut buffer = [0; 1024];
                    while !input.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        let Ok(read) = stream.read(&mut buffer).await else {
                            return;
                        };
                        if read == 0 || input.len() > 8192 {
                            return;
                        }
                        input.extend_from_slice(&buffer[..read]);
                    }
                    let text = String::from_utf8_lossy(&input).to_ascii_lowercase();
                    let range = text
                        .lines()
                        .find_map(|line| line.strip_prefix("range: bytes="))
                        .map(|value| {
                            let (start, end) = value.split_once('-').unwrap();
                            (
                                start.parse::<usize>().unwrap(),
                                end.trim().parse::<usize>().unwrap(),
                            )
                        });
                    let partial = range.is_some()
                        && mode != "ignore"
                        && !(mode == "fallback" && range != Some((0, 0)));
                    let (header, body) = if partial {
                        count.fetch_add(1, Ordering::Relaxed);
                        let (start, end) = range.unwrap();
                        (
                            format!(
                                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                                end - start + 1,
                                start,
                                if mode == "wrong" { end + 1 } else { end },
                                data.len()
                            ),
                            &data[start..=end],
                        )
                    } else {
                        (
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                data.len()
                            ),
                            &data[..],
                        )
                    };
                    if stream.write_all(header.as_bytes()).await.is_ok() {
                        if mode == "slow" {
                            for chunk in body.chunks(64 * 1024) {
                                if stream.write_all(chunk).await.is_err() {
                                    break;
                                }
                                tokio::time::sleep(Duration::from_millis(5)).await;
                            }
                        } else {
                            let _ = stream.write_all(body).await;
                        }
                    }
                });
            }
        });
        (url, data, ranges, handle)
    }

    #[tokio::test]
    async fn segmented_download_and_both_full_download_fallbacks_verify_identical_content() {
        for mode in ["ranges", "ignore", "fallback"] {
            let (url, data, ranges, server) = server(mode).await;
            let root = tempfile::tempdir().unwrap();
            let tasks = Tasks::default();
            let job = tasks.start("test", None).unwrap();
            job.begin_download([("file.jar".into(), Some(data.len() as u64))], true);
            let (_plugins, client) = test_client(root.path());
            let expected = format!("{:x}", Sha1::digest(&*data));
            let path = ensure_once(
                &client,
                root.path(),
                "file.jar",
                &url,
                &expected,
                Some(data.len() as u64),
                &job,
            )
            .await
            .unwrap();
            assert_eq!(std::fs::read(path).unwrap(), *data, "{mode}");
            if mode == "ranges" {
                assert_eq!(ranges.load(Ordering::Relaxed), 17);
            }
            assert!(!std::fs::read_dir(root.path()).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "part")
            }));
            server.abort();
        }
    }

    #[tokio::test]
    async fn invalid_ranges_and_hashes_preserve_existing_files_and_remove_temporary_data() {
        for mode in ["wrong", "ignore"] {
            let (url, data, _, server) = server(mode).await;
            let root = tempfile::tempdir().unwrap();
            let destination = root.path().join("file.jar");
            std::fs::write(&destination, b"original").unwrap();
            let tasks = Tasks::default();
            let job = tasks.start("test", None).unwrap();
            let result = ensure_once(
                &test_client(root.path()).1,
                root.path(),
                "file.jar",
                &url,
                &"0".repeat(40),
                Some(data.len() as u64),
                &job,
            )
            .await;
            assert_eq!(
                result.unwrap_err().code,
                if mode == "wrong" {
                    "INVALID_RANGE"
                } else {
                    "HASH_MISMATCH"
                }
            );
            assert_eq!(std::fs::read(&destination).unwrap(), b"original");
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
            server.abort();
        }
    }
    #[tokio::test]
    async fn cancelled_stream_cleans_partial_data_and_preserves_the_existing_file() {
        let (url, _, _, server) = server("slow").await;
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("file.jar");
        std::fs::write(&destination, b"original").unwrap();
        let tasks = Tasks::default();
        let job = tasks.start("test", None).unwrap();
        let (_plugins, client) = test_client(root.path());
        let hash = "0".repeat(40);
        let transfer = ensure_once(&client, root.path(), "file.jar", &url, &hash, None, &job);
        let cancel = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            job.cancel.cancel();
        };
        let (result, _) = tokio::join!(transfer, cancel);
        assert_eq!(result.unwrap_err().code, "CANCELLED");
        assert_eq!(std::fs::read(destination).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        server.abort();
    }
}
