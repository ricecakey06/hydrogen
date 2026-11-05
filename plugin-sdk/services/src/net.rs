pub use crate::http::{Client, Response};
use crate::{
    contracts::{ApiError, Result},
    jobs::{Job, service_task},
};
use reqwest::Url;
use serde_json::json;
use std::path::Path;
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
    crate::storage::valid_hash(hash)
}
pub async fn hash(_client: &Client, path: &Path) -> Result<String> {
    Ok(serde_json::from_value(
        service_task(
            "minecraft.files",
            "hash",
            json!({"path":path}),
            &Job::default(),
        )
        .await?,
    )?)
}
#[allow(clippy::too_many_arguments)]
pub async fn ensure(
    _client: &Client,
    root: &Path,
    relative: &str,
    url: &str,
    hash: &str,
    size: Option<u64>,
    mirror: bool,
    enabled: bool,
    job: &Job,
) -> Result<std::path::PathBuf> {
    Ok(serde_json::from_value(service_task("minecraft.files","ensure",json!({"root":root,"relative":relative,"url":url,"hash":hash,"size":size,"mirror":mirror,"enabled":enabled}),job).await?)?)
}
pub async fn bytes(client: &Client, url: &str, limit: usize) -> Result<Vec<u8>> {
    let url = Url::parse(url).map_err(|_| ApiError::new("INVALID_URL", "下载地址无效。"))?;
    if !trusted(client, &url)? {
        return Err(ApiError::new(
            "UNTRUSTED_URL",
            "下载地址不属于支持的内容服务。",
        ));
    }
    for attempt in 0..3 {
        let result = async {
            response_bytes(client.get(&url).send().await.map_err(network_error)?, limit).await
        }
        .await;
        match result {
            Err(error) if error.retryable && attempt < 2 => {
                tokio::time::sleep(std::time::Duration::from_millis(300 * (attempt + 1))).await;
            }
            result => return result,
        }
    }
    unreachable!()
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

#[allow(clippy::too_many_arguments)]
pub async fn ensure_game_file(
    _client: &Client,
    root: &Path,
    relative: &str,
    url: &str,
    hash: Option<&str>,
    size: Option<u64>,
    mirror: bool,
    enabled: bool,
    job: &Job,
) -> Result<std::path::PathBuf> {
    Ok(serde_json::from_value(service_task("minecraft.files","ensure",json!({"root":root,"relative":relative,"url":url,"hash":hash,"size":size,"mirror":mirror,"enabled":enabled}),job).await?)?)
}
