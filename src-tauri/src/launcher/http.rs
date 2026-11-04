//! HTTP transport DTOs. Connections and response streams are owned by the installed network plugin.
use super::storage::Settings;
use crate::{
    contracts::{ApiError, Result},
    plugins::host::PluginHost,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use serde_json::{Value, json};
#[derive(Clone)]
pub struct Client {
    host: PluginHost,
    settings: std::sync::Arc<std::sync::Mutex<Settings>>,
}
impl Client {
    pub fn new(host: &PluginHost, settings: &Settings) -> Result<Self> {
        Ok(Self {
            host: host.clone(),
            settings: std::sync::Arc::new(std::sync::Mutex::new(settings.clone())),
        })
    }
    pub fn set_settings(&self, settings: &Settings) {
        *self.settings.lock().unwrap() = settings.clone();
    }
    pub fn service(&self, provider: &str, method: &str, params: Value) -> Result<Value> {
        // Do not retain the settings guard while a native provider is running.
        let settings = self.settings.lock().unwrap().clone();
        self.host
            .call_service(&settings, provider, provider, method, params)
    }
    pub async fn service_async(
        &self,
        provider: &str,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        let client = self.clone();
        let provider = provider.to_owned();
        let method = method.to_owned();
        tokio::task::spawn_blocking(move || client.service(&provider, &method, params))
            .await
            .map_err(|_| ApiError::new("SERVICE_STOPPED", "插件服务执行线程已停止。"))?
    }
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let settings = self.settings.lock().unwrap().clone();
        self.host
            .call_service(&settings, "network", "network", method, params)
            .map_err(|mut e| {
                if e.code == "NETWORK" {
                    e.retryable = true;
                }
                e
            })
    }
    pub fn get(&self, url: impl ToString) -> RequestBuilder {
        self.request("GET", url.to_string())
    }
    pub fn post(&self, url: impl ToString) -> RequestBuilder {
        self.request("POST", url.to_string())
    }
    fn request(&self, method: &str, url: String) -> RequestBuilder {
        RequestBuilder {
            client: self.clone(),
            params: json!({"method":method,"url":url,"headers":{}}),
            invalid: false,
        }
    }
}
pub struct RequestBuilder {
    client: Client,
    params: Value,
    invalid: bool,
}
impl RequestBuilder {
    pub fn header(mut self, name: reqwest::header::HeaderName, value: impl AsRef<str>) -> Self {
        self.params["headers"][name.as_str()] = value.as_ref().into();
        self
    }
    pub fn bearer_auth(self, token: &str) -> Self {
        self.header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"))
    }
    pub fn json(mut self, value: &Value) -> Self {
        self.params["json"] = value.clone();
        self
    }
    pub fn form<T: Serialize + ?Sized>(mut self, fields: &T) -> Self {
        match serde_json::to_value(fields) {
            Ok(value) => self.params["form"] = value,
            Err(_) => self.invalid = true,
        };
        self
    }
    pub async fn send(mut self) -> Result<Response> {
        if self.invalid {
            return Err(ApiError::new("INVALID_REQUEST", "无效请求参数。"));
        }
        let id = self.client.call("request", self.params.take())?;
        let handle = Handle {
            client: self.client.clone(),
            id: id.as_str().ok_or_else(invalid)?.into(),
        };
        let result = handle.poll().await?;
        let status =
            reqwest::StatusCode::from_u16(result["status"].as_u64().ok_or_else(invalid)? as u16)
                .map_err(|_| invalid())?;
        let url = reqwest::Url::parse(result["url"].as_str().ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        let mut headers = reqwest::header::HeaderMap::new();
        for (key, value) in result["headers"].as_object().ok_or_else(invalid)? {
            headers.insert(
                reqwest::header::HeaderName::from_bytes(key.as_bytes()).map_err(|_| invalid())?,
                reqwest::header::HeaderValue::from_str(value.as_str().ok_or_else(invalid)?)
                    .map_err(|_| invalid())?,
            );
        }
        Ok(Response {
            handle,
            status,
            url,
            headers,
            length: result["length"].as_u64(),
        })
    }
}
impl Drop for RequestBuilder {
    fn drop(&mut self) {
        hydrogen_plugin_sdk::wipe_json(&mut self.params);
    }
}
struct Handle {
    client: Client,
    id: String,
}
impl Handle {
    async fn poll(&self) -> Result<Value> {
        loop {
            let result = self.client.call("poll", json!({"handle":self.id}))?;
            if result["ready"] == true {
                return Ok(result);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        let _ = self.client.call("close", json!({"handle":self.id}));
    }
}
pub struct Response {
    handle: Handle,
    status: reqwest::StatusCode,
    url: reqwest::Url,
    headers: reqwest::header::HeaderMap,
    length: Option<u64>,
}
impl Response {
    pub fn status(&self) -> reqwest::StatusCode {
        self.status
    }
    pub fn url(&self) -> &reqwest::Url {
        &self.url
    }
    pub fn headers(&self) -> &reqwest::header::HeaderMap {
        &self.headers
    }
    pub fn content_length(&self) -> Option<u64> {
        self.length
    }
    pub async fn chunk(&mut self) -> Result<Option<Vec<u8>>> {
        self.handle
            .client
            .call("chunk", json!({"handle":self.handle.id}))?;
        let result = self.handle.poll().await?;
        if result["chunk"].is_null() {
            return Ok(None);
        }
        Ok(Some(
            STANDARD
                .decode(result["chunk"].as_str().ok_or_else(invalid)?)
                .map_err(|_| invalid())?,
        ))
    }
}
fn invalid() -> ApiError {
    ApiError::new("NETWORK_RESPONSE", "网络插件响应无效。")
}
