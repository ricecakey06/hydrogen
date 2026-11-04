mod policy;
use base64::{Engine, engine::general_purpose::STANDARD};
use hydrogen_plugin_sdk::{
    contracts::{ApiError, Result},
    serde_json::{Value, json},
};
use std::collections::{BTreeMap, VecDeque};
struct Stream {
    response: reqwest::Response,
    pending: VecDeque<u8>,
}
enum Outcome {
    Headers(Stream),
    Chunk(Stream, Option<Vec<u8>>),
}
struct State {
    operations: BTreeMap<String, tokio::task::JoinHandle<Result<Outcome>>>,
    streams: BTreeMap<String, Stream>,
    client: reqwest::Client,
    runtime: Option<tokio::runtime::Runtime>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            operations: BTreeMap::new(),
            streams: BTreeMap::new(),
            client: policy::client().expect("HTTPS initialization"),
            runtime: Some(
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .unwrap(),
            ),
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        for (_, task) in std::mem::take(&mut self.operations) {
            task.abort();
        }
        self.streams.clear();
        // Runtime drop waits for owned workers; no plugin code survives library unload.
        drop(self.runtime.take());
    }
}
fn error(message: &str) -> ApiError {
    ApiError::new("NETWORK", message)
}
fn text<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    p[key]
        .as_str()
        .ok_or_else(|| ApiError::new("INVALID_REQUEST", "缺少网络参数。"))
}
fn failure(_: reqwest::Error) -> ApiError {
    error("网络请求失败，请检查连接或稍后重试。")
}
impl State {
    fn call(&mut self, request: &Value) -> Result<Value> {
        let p = &request["params"];
        match request["method"].as_str().unwrap_or("") {
            "plugin.describe" => Ok(json!({"id":"network","abiVersion":1})),
            "stats" => Ok(json!({"operations":self.operations.len(),"streams":self.streams.len()})),
            "request" => {
                if self.operations.len() + self.streams.len() >= 64 {
                    return Err(ApiError::new("NETWORK_BUSY", "网络请求过多。"));
                }
                let url =
                    reqwest::Url::parse(text(p, "url")?).map_err(|_| error("网络地址无效。"))?;
                // Explicit loopback HTTP supports local services; redirects still use HTTPS policy.
                let local = url.scheme() == "http"
                    && url
                        .host_str()
                        .and_then(|h| h.parse::<std::net::IpAddr>().ok())
                        .is_some_and(|ip| ip.is_loopback())
                    && url.username().is_empty()
                    && url.password().is_none();
                if !policy::trusted(&url) && !local {
                    return Err(ApiError::new(
                        "UNTRUSTED_URL",
                        "网络地址不属于受支持的服务。",
                    ));
                }
                let mut builder = match text(p, "method")? {
                    "GET" => self.client.get(url),
                    "POST" => self.client.post(url),
                    _ => return Err(error("不支持的 HTTP 方法。")),
                };
                if let Some(headers) = p["headers"].as_object() {
                    for (name, value) in headers {
                        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                            .map_err(|_| error("无效请求头。"))?;
                        let value = reqwest::header::HeaderValue::from_str(
                            value.as_str().ok_or_else(|| error("无效请求头。"))?,
                        )
                        .map_err(|_| error("无效请求头。"))?;
                        builder = builder.header(name, value);
                    }
                }
                if !p["json"].is_null() {
                    builder = builder.json(&p["json"]);
                }
                if !p["form"].is_null() {
                    let fields: Vec<(String, String)> = serde_json::from_value(p["form"].clone())?;
                    builder = builder.form(&fields);
                }
                let id = uuid::Uuid::new_v4().to_string();
                let task = self.runtime.as_ref().unwrap().spawn(async move {
                    Ok(Outcome::Headers(Stream {
                        response: builder.send().await.map_err(failure)?,
                        pending: VecDeque::new(),
                    }))
                });
                self.operations.insert(id.clone(), task);
                Ok(json!(id))
            }
            "poll" => {
                let id = text(p, "handle")?;
                let task = self
                    .operations
                    .get(id)
                    .ok_or_else(|| error("网络请求已关闭。"))?;
                if !task.is_finished() {
                    return Ok(json!({"ready":false}));
                }
                let task = self.operations.remove(id).unwrap();
                let outcome = self
                    .runtime
                    .as_ref()
                    .unwrap()
                    .block_on(task)
                    .map_err(|_| error("网络任务已停止。"))??;
                let (stream, value) = match outcome {
                    Outcome::Headers(stream) => {
                        let headers: BTreeMap<String, String> = stream
                            .response
                            .headers()
                            .iter()
                            .filter_map(|(k, v)| {
                                v.to_str().ok().map(|v| (k.as_str().into(), v.into()))
                            })
                            .collect();
                        let value = json!({"ready":true,"status":stream.response.status().as_u16(),"url":stream.response.url().as_str(),"length":stream.response.content_length(),"headers":headers});
                        (stream, value)
                    }
                    Outcome::Chunk(stream, chunk) => (
                        stream,
                        json!({"ready":true,"chunk":chunk.map(|b|STANDARD.encode(b))}),
                    ),
                };
                self.streams.insert(id.into(), stream);
                Ok(value)
            }
            "chunk" => {
                let id = text(p, "handle")?;
                let mut stream = self
                    .streams
                    .remove(id)
                    .ok_or_else(|| error("网络流不存在或正在读取。"))?;
                let task = self.runtime.as_ref().unwrap().spawn(async move {
                    if stream.pending.is_empty() {
                        let Some(bytes) = stream.response.chunk().await.map_err(failure)? else {
                            return Ok(Outcome::Chunk(stream, None));
                        };
                        if bytes.len() > 8 * 1024 * 1024 {
                            return Err(ApiError::new("RESPONSE_LIMIT", "响应块超过限制。"));
                        }
                        stream.pending.extend(bytes);
                    }
                    let chunk: Vec<_> = stream
                        .pending
                        .drain(..stream.pending.len().min(128 * 1024))
                        .collect();
                    Ok(Outcome::Chunk(stream, Some(chunk)))
                });
                self.operations.insert(id.into(), task);
                Ok(Value::Null)
            }
            "close" => {
                let id = text(p, "handle")?;
                if let Some(task) = self.operations.remove(id) {
                    task.abort();
                    let _ = self.runtime.as_ref().unwrap().block_on(task);
                }
                self.streams.remove(id);
                Ok(Value::Null)
            }
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知网络服务方法。")),
        }
    }
}
fn dispatch(state: &mut State, request: &mut Value) -> hydrogen_plugin_sdk::Result {
    state.call(request).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);
