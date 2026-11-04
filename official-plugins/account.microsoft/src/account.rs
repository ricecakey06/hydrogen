use super::{
    credentials, environment, net,
    storage::{self, Settings},
    tasks::Job,
    text,
};
use crate::contracts::{ApiError, Result};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use hydrogen_plugin_services::PluginHost;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn auth_error() -> ApiError {
    ApiError::new(
        "AUTHENTICATION_FAILED",
        "账号验证未完成，请检查应用授权、账号限制或重试登录。",
    )
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub name: String,
    pub uuid: String,
    pub access_token: String,
    pub xuid: String,
    pub client_id: String,
    pub expires: u64,
    pub offline: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub uuid: Option<String>,
    pub status: String,
    pub player_name: Option<String>,
    pub mode: String,
    pub offline_available: bool,
    pub detail: Option<String>,
    pub skin: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Credential {
    schema_version: u32,
    client_id: String,
    refresh_token: String,
    #[serde(default)]
    session: Option<Session>,
    #[serde(default)]
    skin: Option<String>,
}
fn valid_skin_png(bytes: &[u8]) -> bool {
    if bytes.len() < 24 || bytes[..8] != *b"\x89PNG\r\n\x1a\n" {
        return false;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    (64..=1024).contains(&width)
        && width.is_multiple_of(64)
        && (height == width || height == width / 2)
}
fn valid_cached_skin(skin: &str) -> bool {
    skin.strip_prefix("data:image/png;base64,")
        .filter(|data| data.len() <= 1_400_000)
        .and_then(|data| STANDARD.decode(data).ok())
        .is_some_and(|bytes| valid_skin_png(&bytes))
}
pub struct Accounts {
    gate: tokio::sync::Mutex<()>,
    cache_loaded: Mutex<bool>,
    session: Mutex<Option<Session>>,
    refresh: Mutex<Option<String>>,
    detail: Mutex<Option<String>>,
    skin: Mutex<Option<String>>,
    plugins: PluginHost,
}
impl Accounts {
    pub fn new(plugins: PluginHost) -> Self {
        Self {
            gate: tokio::sync::Mutex::new(()),
            cache_loaded: Mutex::new(false),
            session: Mutex::new(None),
            refresh: Mutex::new(None),
            detail: Mutex::new(None),
            skin: Mutex::new(None),
            plugins,
        }
    }
}
impl Accounts {
    fn load_cached(&self, root: &Path, settings: &Settings) -> Result<()> {
        let mut loaded = self.cache_loaded.lock().unwrap();
        if *loaded {
            return Ok(());
        }
        let path = storage::within(root, "credentials.bin")?;
        if !path.is_file() {
            *loaded = true;
            return Ok(());
        }
        if std::fs::metadata(&path)?.len() > 2 * 1024 * 1024 {
            return Err(auth_error());
        }
        let mut encrypted = std::fs::read(&path)?;
        let mut plaintext = credentials::transform(&self.plugins, settings, &mut encrypted, false)?;
        let saved = serde_json::from_slice::<Credential>(&plaintext);
        plaintext.fill(0);
        let saved = saved?;
        if saved.schema_version != 1 || saved.client_id != settings.microsoft_client_id {
            return Err(ApiError::new(
                "CREDENTIAL_CLIENT_ID",
                "保存的凭据属于其他应用，请重新登录。",
            ));
        }
        if saved.refresh_token.is_empty() {
            return Err(auth_error());
        }
        if let Some(session) = &saved.session {
            if session.client_id != saved.client_id
                || session.offline
                || uuid::Uuid::parse_str(&session.uuid).is_err()
                || session.name.is_empty()
                || session.access_token.is_empty()
            {
                return Err(auth_error());
            }
        }
        *loaded = true;
        *self.session.lock().unwrap() = saved.session;
        *self.refresh.lock().unwrap() = Some(saved.refresh_token);
        *self.skin.lock().unwrap() = saved.skin.filter(|skin| valid_cached_skin(skin));
        Ok(())
    }
    fn save_cached(&self, root: &Path, settings: &Settings) -> Result<()> {
        let refresh_token = self
            .refresh
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(auth_error)?;
        let session = self.session.lock().unwrap().clone();
        let skin = self.skin.lock().unwrap().clone();
        let saved = Credential {
            schema_version: 1,
            client_id: settings.microsoft_client_id.clone(),
            refresh_token,
            session,
            skin,
        };
        let mut plaintext = serde_json::to_vec(&saved)?;
        let encrypted = credentials::transform(&self.plugins, settings, &mut plaintext, true)?;
        storage::atomic_write(&storage::within(root, "credentials.bin")?, &encrypted)
    }
    pub fn view(&self, root: &Path, settings: &Settings, signing_in: bool) -> AccountView {
        if let Err(error) = self.load_cached(root, settings) {
            self.failure(&error.message);
        }
        let session = self.session.lock().unwrap();
        AccountView {
            uuid: session.as_ref().map(|s| s.uuid.clone()),
            status: if signing_in {
                "signingIn"
            } else if session.as_ref().is_some_and(|s| s.expires > now() + 120) {
                "signedIn"
            } else if session.is_some() || self.refresh.lock().unwrap().is_some() {
                "cached"
            } else {
                "signedOut"
            }
            .into(),
            player_name: session.as_ref().map(|s| s.name.clone()),
            mode: "microsoft".into(),
            offline_available: false,
            detail: self.detail.lock().unwrap().clone(),
            skin: self.skin.lock().unwrap().clone(),
        }
    }
    pub fn reset(&self) {
        self.session.lock().unwrap().take();
        self.refresh.lock().unwrap().take();
        self.detail.lock().unwrap().take();
        self.skin.lock().unwrap().take();
    }
    pub fn sign_out(&self, root: &Path, _settings: &Settings) -> Result<()> {
        let _guard = self
            .gate
            .try_lock()
            .map_err(|_| ApiError::new("ACCOUNT_BUSY", "账号正在验证，请稍后重试。"))?;

        let path = storage::within(root, "credentials.bin")?;
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        self.reset();
        *self.cache_loaded.lock().unwrap() = true;
        Ok(())
    }
    pub async fn sign_in(
        &self,
        client: &net::Client,
        root: &Path,
        settings: &Settings,
        job: &Job,
        restore: bool,
    ) -> Result<()> {
        let _guard = self.gate.lock().await;
        if !settings.enabled("account.microsoft") {
            return Err(ApiError::new(
                "PLUGIN_DISABLED",
                "Microsoft 账号模块已停用。",
            ));
        }

        if uuid::Uuid::parse_str(&settings.microsoft_client_id).is_err()
            || settings.microsoft_client_id == uuid::Uuid::nil().to_string()
        {
            return Err(ApiError::new(
                "CLIENT_ID_REQUIRED",
                "Microsoft 登录尚未配置，请由应用发布者配置 HYDROGEN_MICROSOFT_CLIENT_ID。",
            ));
        }
        self.detail.lock().unwrap().take();
        let tokens = if restore {
            self.load_cached(root, settings)?;
            let cached_session = self.session.lock().unwrap().clone();
            if let Some(session) = cached_session
                && session.expires > now() + 120
            {
                if self.skin.lock().unwrap().is_none() {
                    // Missing avatars must be repaired even while the game token is still valid.
                    let profile = get_authorized(
                        client,
                        "https://api.minecraftservices.com/minecraft/profile",
                        &session.access_token,
                    )
                    .await?;
                    if text(&profile, "id")? != session.uuid {
                        return Err(auth_error());
                    }
                    self.update_skin(client, &profile).await?;
                    self.save_cached(root, settings)?;
                }
                return Ok(());
            }
            let refresh = self.refresh.lock().unwrap().clone();
            let Some(refresh) = refresh else {
                return Ok(());
            };
            exchange(
                client,
                &[
                    ("client_id", settings.microsoft_client_id.as_str()),
                    ("grant_type", "refresh_token"),
                    ("refresh_token", refresh.as_str()),
                    ("scope", "XboxLive.signin offline_access"),
                ],
            )
            .await?
        } else {
            authorize(
                client,
                &settings.microsoft_client_id,
                job,
                &self.plugins,
                settings,
            )
            .await?
        };
        self.complete(client, root, settings, tokens).await
    }
    async fn complete(
        &self,
        client: &net::Client,
        root: &Path,
        settings: &Settings,
        tokens: Value,
    ) -> Result<()> {
        let refresh = text(&tokens, "refresh_token")?.to_string();
        let xbox = post(client, "https://user.auth.xboxlive.com/user/authenticate", json!({"Properties":{"AuthMethod":"RPS","SiteName":"user.auth.xboxlive.com","RpsTicket":format!("d={}",text(&tokens,"access_token")?)},"RelyingParty":"http://auth.xboxlive.com","TokenType":"JWT"})).await?;
        let xsts = post(client, "https://xsts.auth.xboxlive.com/xsts/authorize", json!({"Properties":{"SandboxId":"RETAIL","UserTokens":[text(&xbox,"Token")?]},"RelyingParty":"rp://api.minecraftservices.com/","TokenType":"JWT"})).await?;
        let claim = xsts["DisplayClaims"]["xui"]
            .as_array()
            .and_then(|a| a.first())
            .ok_or_else(auth_error)?;
        let minecraft = post(client, "https://api.minecraftservices.com/authentication/login_with_xbox", json!({"identityToken":format!("XBL3.0 x={};{}",text(claim,"uhs")?,text(&xsts,"Token")?)})).await?;
        let access = text(&minecraft, "access_token")?;
        let ownership = get_authorized(
            client,
            "https://api.minecraftservices.com/entitlements/mcstore",
            access,
        )
        .await?;
        if !ownership["items"].as_array().is_some_and(|items| {
            items
                .iter()
                .any(|item| item["name"] == "game_minecraft" || item["name"] == "product_minecraft")
        }) {
            return Err(ApiError::new(
                "NO_GAME_OWNERSHIP",
                "此微软账号尚未验证 Minecraft Java 版游戏资格。",
            ));
        }
        let profile = get_authorized(
            client,
            "https://api.minecraftservices.com/minecraft/profile",
            access,
        )
        .await?;
        let uuid = text(&profile, "id")?;
        if uuid::Uuid::parse_str(uuid).is_err() {
            return Err(auth_error());
        }
        let expires = minecraft["expires_in"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or_else(auth_error)?;
        // Keep the previous identity visible until verification succeeds.
        *self.cache_loaded.lock().unwrap() = true;
        *self.refresh.lock().unwrap() = Some(refresh);
        if self
            .session
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|s| s.uuid != uuid)
        {
            self.skin.lock().unwrap().take();
        }
        *self.session.lock().unwrap() = Some(Session {
            name: text(&profile, "name")?.into(),
            uuid: uuid.into(),
            access_token: access.into(),
            xuid: claim["xid"].as_str().unwrap_or("").into(),
            client_id: settings.microsoft_client_id.clone(),
            expires: now() + expires.min(86400),
            offline: false,
        });
        let _ = self.update_skin(client, &profile).await;
        // Session tokens and identity/skin are stored together under Windows DPAPI protection.
        let persistent = self.save_cached(root, settings).is_ok();
        *self.detail.lock().unwrap() = (!persistent).then(|| "本次登录仅在当前会话有效。".into());
        Ok(())
    }
    async fn update_skin(&self, client: &net::Client, profile: &Value) -> Result<()> {
        let Some(url) = profile["skins"]
            .as_array()
            .and_then(|skins| skins.iter().find(|s| s["state"] == "ACTIVE"))
            .and_then(|s| s["url"].as_str())
        else {
            return Ok(());
        };
        let mut url = reqwest::Url::parse(url).map_err(|_| auth_error())?;
        if url.host_str() != Some("textures.minecraft.net")
            || !["http", "https"].contains(&url.scheme())
        {
            return Err(ApiError::new("INVALID_SKIN", "皮肤图片地址无效。"));
        }
        let _ = url.set_scheme("https");
        let bytes = net::bytes(client, url.as_str(), 1024 * 1024).await?;
        if !valid_skin_png(&bytes) {
            return Err(ApiError::new(
                "INVALID_SKIN",
                "皮肤图片格式无效，请稍后重试。",
            ));
        }
        *self.skin.lock().unwrap() =
            Some(format!("data:image/png;base64,{}", STANDARD.encode(bytes)));
        Ok(())
    }
    pub async fn session(
        &self,
        client: &net::Client,
        root: &Path,
        settings: &Settings,
    ) -> Result<Session> {
        let _guard = self.gate.lock().await;
        if !settings.enabled("account.microsoft") {
            return Err(ApiError::new(
                "PLUGIN_DISABLED",
                "Microsoft 账号模块已停用。",
            ));
        }

        self.load_cached(root, settings).map_err(|_| {
            ApiError::new("ACCOUNT_REQUIRED", "账号缓存不可用，请重新登录微软账号。")
        })?;
        if let Some(session) = self.session.lock().unwrap().clone()
            && session.expires > now() + 120
        {
            return Ok(session);
        }
        let refresh = self
            .refresh
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| ApiError::new("ACCOUNT_REQUIRED", "请先登录微软账号。"))?;
        let tokens = exchange(
            client,
            &[
                ("client_id", settings.microsoft_client_id.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh.as_str()),
                ("scope", "XboxLive.signin offline_access"),
            ],
        )
        .await?;
        self.complete(client, root, settings, tokens).await?;
        self.session.lock().unwrap().clone().ok_or_else(auth_error)
    }
    pub fn failure(&self, message: &str) {
        *self.detail.lock().unwrap() = Some(message.into());
    }
}
async fn post(client: &net::Client, url: &str, body: Value) -> Result<Value> {
    let response = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(net::network_error)?;
    if !response.status().is_success() {
        return Err(auth_error());
    }
    Ok(serde_json::from_slice(
        &net::response_bytes(response, 1024 * 1024).await?,
    )?)
}
async fn get_authorized(client: &net::Client, url: &str, token: &str) -> Result<Value> {
    let response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(net::network_error)?;
    if !response.status().is_success() {
        return Err(auth_error());
    }
    Ok(serde_json::from_slice(
        &net::response_bytes(response, 1024 * 1024).await?,
    )?)
}
async fn exchange(client: &net::Client, fields: &[(&str, &str)]) -> Result<Value> {
    let response = client
        .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
        .form(fields)
        .send()
        .await
        .map_err(net::network_error)?;
    if !response.status().is_success() {
        return Err(auth_error());
    }
    Ok(serde_json::from_slice(
        &net::response_bytes(response, 1024 * 1024).await?,
    )?)
}

pub fn callback(request: &str, expected: &str, port: u16) -> Result<Option<String>> {
    let mut lines = request.split("\r\n");
    let first = lines.next().unwrap_or("");
    let parts: Vec<_> = first.split(' ').collect();
    if parts.len() != 3
        || parts[0] != "GET"
        || !parts[1].starts_with("/?")
        || !parts[2].starts_with("HTTP/1.")
    {
        return Err(auth_error());
    }
    let hosts: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(k, _)| k.eq_ignore_ascii_case("host"))
        .collect();
    if hosts.len() != 1 || hosts[0].1.trim() != format!("localhost:{port}") {
        return Err(auth_error());
    }
    let url =
        reqwest::Url::parse(&format!("http://localhost{}", parts[1])).map_err(|_| auth_error())?;
    let mut fields = std::collections::BTreeMap::new();
    for (key, value) in url.query_pairs() {
        if fields
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Err(auth_error());
        }
    }
    let state = fields
        .get("state")
        .map(String::as_bytes)
        .unwrap_or_default();
    if state.len() != expected.len()
        || state
            .iter()
            .zip(expected.as_bytes())
            .fold(0_u8, |value, (a, b)| value | (a ^ b))
            != 0
    {
        return Err(auth_error());
    }
    if fields.contains_key("error") == fields.contains_key("code") {
        return Err(auth_error());
    }
    if fields.contains_key("error") {
        return Ok(None);
    }
    fields
        .remove("code")
        .filter(|c| !c.is_empty())
        .map(Some)
        .ok_or_else(auth_error)
}
async fn authorize(
    client: &net::Client,
    client_id: &str,
    job: &Job,
    plugins: &PluginHost,
    settings: &Settings,
) -> Result<Value> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    let verifier = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let state = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let redirect = format!("http://localhost:{port}/");
    let mut url =
        reqwest::Url::parse("https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize")
            .map_err(|_| auth_error())?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", client_id),
        ("response_type", "code"),
        ("redirect_uri", redirect.as_str()),
        ("response_mode", "query"),
        ("scope", "XboxLive.signin offline_access"),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("prompt", "select_account"),
    ]);
    environment::browser(plugins, settings, url.as_str())?;
    job.phase("authorizing", "请在系统浏览器中完成微软登录，可取消。");
    let deadline = tokio::time::sleep(Duration::from_secs(300));
    tokio::pin!(deadline);
    loop {
        let (mut stream, _) = tokio::select! { _ = job.cancel.cancelled() => return Err(ApiError::new("CANCELLED","登录已取消。")), _ = &mut deadline => return Err(ApiError::new("CANCELLED","登录等待超时。")), result=listener.accept()=>result? };
        let read = async {
            let mut request = vec![0; 8192];
            let mut length = 0;
            loop {
                if length == request.len() {
                    return Err(auth_error());
                }
                let count = stream.read(&mut request[length..]).await?;
                if count == 0 {
                    return Err(auth_error());
                }
                length += count;
                if request[..length].windows(4).any(|s| s == b"\r\n\r\n") {
                    break;
                }
            }
            callback(&String::from_utf8_lossy(&request[..length]), &state, port)
        };
        let result = match tokio::time::timeout(Duration::from_secs(5), read).await {
            Ok(result) => result,
            Err(_) => continue,
        };
        let body = "<!doctype html><meta charset=utf-8><title>Hydrogen</title><p>Return to Hydrogen. You may close this page.</p>";
        let response = format!(
            "HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\n\r\n{}",
            if result.is_ok() {
                "200 OK"
            } else {
                "400 Bad Request"
            },
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes()).await;
        match result {
            Ok(Some(code)) => {
                return exchange(
                    client,
                    &[
                        ("client_id", client_id),
                        ("grant_type", "authorization_code"),
                        ("code", code.as_str()),
                        ("redirect_uri", redirect.as_str()),
                        ("code_verifier", verifier.as_str()),
                        ("scope", "XboxLive.signin offline_access"),
                    ],
                )
                .await;
            }
            Ok(None) => return Err(ApiError::new("CANCELLED", "微软授权已取消。")),
            Err(_) => {}
        }
    }
}
