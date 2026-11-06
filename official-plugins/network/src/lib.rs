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
    finished: bool,
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
fn mirror_request(
    client: &reqwest::Client,
    original: &reqwest::RequestBuilder,
    url: reqwest::Url,
) -> Option<reqwest::RequestBuilder> {
    let mut request = original.try_clone()?.build().ok()?;
    *request.url_mut() = url;
    // The mirror is public; keep the CurseForge key for the origin only.
    request.headers_mut().remove("x-api-key");
    Some(reqwest::RequestBuilder::from_parts(client.clone(), request))
}
async fn send_request(
    original: reqwest::RequestBuilder,
    mirror: Option<reqwest::RequestBuilder>,
) -> Result<Stream> {
    if let Some(mirror) = mirror {
        let cached = async {
            let mut response = mirror.send().await.map_err(failure)?;
            if !response.status().is_success() {
                return Err(error("镜像请求失败。"));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(failure)? {
                if bytes.len() + chunk.len() > 32 * 1024 * 1024 {
                    return Err(error("镜像响应超过限制。"));
                }
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice::<Value>(&bytes)?;
            Ok(Stream {
                response,
                pending: bytes.into(),
                finished: true,
            })
        };
        if let Ok(Ok(stream)) =
            tokio::time::timeout(std::time::Duration::from_secs(30), cached).await
        {
            return Ok(stream);
        }
    }
    Ok(Stream {
        response: original.send().await.map_err(failure)?,
        pending: VecDeque::new(),
        finished: false,
    })
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
                    "GET" => self.client.get(url.clone()),
                    "POST" => self.client.post(url.clone()),
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
                let mirror = if p["modMirror"] != false {
                    policy::mod_api_mirror(&url)
                        .and_then(|url| mirror_request(&self.client, &builder, url))
                } else {
                    None
                };
                let id = uuid::Uuid::new_v4().to_string();
                let task = self.runtime.as_ref().unwrap().spawn(async move {
                    send_request(builder, mirror).await.map(Outcome::Headers)
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
                        if stream.finished {
                            return Ok(Outcome::Chunk(stream, None));
                        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn server(status: u16, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 4096];
            let length = stream.read(&mut bytes).unwrap();
            write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            String::from_utf8_lossy(&bytes[..length]).into_owned()
        });
        (url, handle)
    }

    #[tokio::test]
    async fn mirror_success_buffers_json_and_failures_replay_the_origin_request() {
        let client = reqwest::Client::new();
        for (status, body) in [(404, "{}"), (429, "{}"), (503, "{}"), (200, "invalid json")] {
            let (origin, origin_server) = server(200, "{\"origin\":true}");
            let (mirror, mirror_server) = server(status, body);
            let original = client
                .post(&origin)
                .header("x-api-key", "origin-key")
                .json(&json!({"hashes":["abc"]}));
            let mirrored =
                mirror_request(&client, &original, reqwest::Url::parse(&mirror).unwrap());
            let stream = send_request(original, mirrored).await.unwrap();
            assert_eq!(stream.response.url().as_str(), origin);
            let original_request = origin_server.join().unwrap();
            assert!(original_request.starts_with("POST / HTTP/1.1"));
            assert!(original_request.contains("x-api-key: origin-key"));
            assert!(!mirror_server.join().unwrap().contains("x-api-key"));
        }
        let (mirror, mirror_server) = server(200, "{\"cached\":true}");
        let stream = send_request(client.get("http://127.0.0.1:1/"), Some(client.get(&mirror)))
            .await
            .unwrap();
        assert!(stream.finished);
        assert_eq!(
            stream.pending.into_iter().collect::<Vec<_>>(),
            b"{\"cached\":true}"
        );
        mirror_server.join().unwrap();
        let (origin, origin_server) = server(200, "{}");
        let stream = send_request(client.get(&origin), Some(client.get("http://127.0.0.1:1/")))
            .await
            .unwrap();
        assert_eq!(stream.response.url().as_str(), origin);
        origin_server.join().unwrap();
    }

    #[test]
    fn mirror_urls_preserve_encoded_paths_and_queries_and_reject_other_hosts() {
        for (host, prefix) in [
            ("api.modrinth.com", "/modrinth"),
            ("api.curseforge.com", "/curseforge"),
        ] {
            let url = reqwest::Url::parse(&format!("https://{host}/v2/search?q=a%2Bb&offset=20"))
                .unwrap();
            assert_eq!(
                policy::mod_api_mirror(&url).unwrap().as_str(),
                format!("https://mod.mcimirror.top{prefix}/v2/search?q=a%2Bb&offset=20")
            );
        }
        for url in [
            "https://api.modrinth.com.evil.invalid/v2/search",
            "http://api.modrinth.com/v2/search",
            "https://api.modrinth.com:8443/v2/search",
            "https://user@api.modrinth.com/v2/search",
        ] {
            assert!(policy::mod_api_mirror(&reqwest::Url::parse(url).unwrap()).is_none());
        }
    }
}
