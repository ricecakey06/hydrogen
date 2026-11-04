use crate::contracts::{ApiError, Result};
pub use hydrogen_plugin_services::http::{Client, Response};
pub fn network_error(error: ApiError) -> ApiError {
    error
}
pub async fn bytes(client: &Client, url: &str, limit: usize) -> Result<Vec<u8>> {
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
