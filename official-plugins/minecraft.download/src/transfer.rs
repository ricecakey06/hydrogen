use hydrogen_plugin_services::contracts::{ApiError, Result};
pub use hydrogen_plugin_services::http::{Client, Response};
use hydrogen_plugin_services::{jobs::Job, storage};
use reqwest::Url;
use sha1::{Digest, Sha1};
use std::{
    path::Path,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime},
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub fn trusted(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none_or(|p| p == 443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|h| {
            [
                "piston-meta.mojang.com",
                "piston-data.mojang.com",
                "launchermeta.mojang.com",
                "launcher.mojang.com",
                "libraries.minecraft.net",
                "resources.download.minecraft.net",
                "textures.minecraft.net",
                "meta.fabricmc.net",
                "meta.quiltmc.org",
                "maven.fabricmc.net",
                "maven.minecraftforge.net",
                "files.minecraftforge.net",
                "maven.neoforged.net",
                "maven.quiltmc.org",
                "repo.maven.apache.org",
                "repo1.maven.org",
                "libraries.optifine.net",
                "api.modrinth.com",
                "mod.mcimirror.top",
                "cdn.modrinth.com",
                "cdn.jsdelivr.net",
                "raw.githubusercontent.com",
                "github.com",
                "gitlab.com",
                "edge.forgecdn.net",
                "mediafilez.forgecdn.net",
                "media.forgecdn.net",
                "mediafiles.forgecdn.net",
                "bmclapi2.bangbang93.com",
                "mirror.nju.edu.cn",
                "login.microsoftonline.com",
                "user.auth.xboxlive.com",
                "xsts.auth.xboxlive.com",
                "api.minecraftservices.com",
            ]
            .contains(&h)
                || h.ends_with(".bmclapi.com")
                || h.ends_with(".bmclapi.cn")
        })
}
pub async fn hash(path: &Path) -> Result<String> {
    let mut input = tokio::fs::File::open(path).await?;
    let mut hash = Sha1::new();
    let mut buffer = vec![0; 256 * 1024];
    loop {
        let read = input.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn valid_hash(hash: &str) -> bool {
    hash.len() == 40 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}
pub fn source_url(url: &str, mirror: bool) -> Result<String> {
    let mut url = Url::parse(url).map_err(|_| ApiError::new("INVALID_URL", "下载地址无效。"))?;
    if !trusted(&url) {
        return Err(ApiError::new("UNTRUSTED_URL", "文件来自不受支持的服务。"));
    }
    if mirror && !mirror_cooling_down() {
        let prefix = match url.host_str().unwrap_or("") {
            "resources.download.minecraft.net" => Some("/assets"),
            "libraries.minecraft.net" => Some("/maven"),
            "maven.minecraftforge.net" => Some("/maven"),
            "files.minecraftforge.net" if url.path().starts_with("/maven/") => Some(""),
            "files.minecraftforge.net" => Some("/maven"),
            "piston-meta.mojang.com"
            | "piston-data.mojang.com"
            | "launchermeta.mojang.com"
            | "launcher.mojang.com" => Some(""),
            _ => None,
        };
        if let Some(prefix) = prefix {
            url.set_path(&format!("{prefix}{}", url.path()));
            let _ = url.set_host(Some("bmclapi2.bangbang93.com"));
        }
    }
    Ok(url.to_string())
}
fn connections() -> &'static tokio::sync::Semaphore {
    static CONNECTIONS: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    CONNECTIONS.get_or_init(|| tokio::sync::Semaphore::new(32))
}
fn mirror_retry_after() -> &'static Mutex<Option<Instant>> {
    static RETRY_AFTER: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    RETRY_AFTER.get_or_init(Mutex::default)
}
fn mirror_cooling_down() -> bool {
    mirror_retry_after()
        .lock()
        .unwrap()
        .is_some_and(|until| Instant::now() < until)
}
fn is_mod_mirror(url: &str) -> bool {
    url.starts_with("https://mod.mcimirror.top/")
}
fn mod_mirror_retry_after() -> &'static Mutex<Option<Instant>> {
    static RETRY_AFTER: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    RETRY_AFTER.get_or_init(Mutex::default)
}
fn mod_mirror_cooling_down() -> bool {
    mod_mirror_retry_after()
        .lock()
        .unwrap()
        .is_some_and(|until| Instant::now() < until)
}
fn observe_mirror(response: &Response) {
    if response.status().as_u16() != 429
        || response.url().host_str() != Some("bmclapi2.bangbang93.com")
    {
        return;
    }
    let delay = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|header| header.to_str().ok())
        .and_then(|value| {
            value
                .parse::<u64>()
                .ok()
                .map(Duration::from_secs)
                .or_else(|| {
                    httpdate::parse_http_date(value)
                        .ok()
                        .map(|date| date.duration_since(SystemTime::now()).unwrap_or_default())
                })
        })
        .unwrap_or(Duration::from_secs(60));
    let now = Instant::now();
    let until = now
        .checked_add(delay)
        .unwrap_or(now + Duration::from_secs(60));
    let mut after = mirror_retry_after().lock().unwrap();
    if after.is_none_or(|previous| until > previous) {
        *after = Some(until);
    }
}
fn object_endpoint(url: &str) -> Result<String> {
    let mut url = Url::parse(url).map_err(|_| ApiError::new("INVALID_URL", "下载地址无效。"))?;
    if !trusted(&url) {
        return Err(ApiError::new("UNTRUSTED_URL", "文件来自不受支持的服务。"));
    }
    let parts: Vec<_> = url
        .path()
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if url.host_str() == Some("piston-data.mojang.com")
        && parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "objects"
        && valid_hash(parts[2])
    {
        let _ = url.set_host(Some("launcher.mojang.com"));
    }
    Ok(url.to_string())
}
fn verification_error(code: &str, message: &str) -> ApiError {
    let mut error = ApiError::new(code, message);
    error.retryable = true;
    error
}
fn sources(url: &str, mirror: bool, mod_mirror: bool) -> Result<Vec<String>> {
    let mut parsed = Url::parse(url).map_err(|_| ApiError::new("INVALID_URL", "下载地址无效。"))?;
    if !trusted(&parsed) {
        return Err(ApiError::new("UNTRUSTED_URL", "文件来自不受支持的服务。"));
    }
    if parsed.host_str() == Some("mod.mcimirror.top") {
        let host = if parsed.path().starts_with("/data/") {
            "cdn.modrinth.com"
        } else if parsed.path().starts_with("/files/") {
            "edge.forgecdn.net"
        } else {
            return Err(ApiError::new("UNTRUSTED_URL", "文件来自不受支持的服务。"));
        };
        let _ = parsed.set_host(Some(host));
    }
    if matches!(
        parsed.host_str(),
        Some(
            "cdn.modrinth.com"
                | "edge.forgecdn.net"
                | "mediafilez.forgecdn.net"
                | "media.forgecdn.net"
                | "mediafiles.forgecdn.net"
        )
    ) {
        let curseforge = parsed.host_str() != Some("cdn.modrinth.com");
        let original = parsed.to_string();
        let mut urls = Vec::new();
        if mod_mirror {
            let _ = parsed.set_host(Some("mod.mcimirror.top"));
            urls.push(parsed.to_string());
        }
        if curseforge && parsed.path().starts_with("/files/") {
            // Some older CurseForge manifests use edge URLs that now return 404.
            let _ = parsed.set_host(Some("mediafilez.forgecdn.net"));
            urls.push(parsed.to_string());
        }
        urls.push(original);
        urls.dedup();
        return Ok(urls);
    }
    let original = object_endpoint(url)?;
    let mapped = object_endpoint(&source_url(url, true)?)?;
    let mut mirrors = Vec::new();
    if mapped != original {
        mirrors.push(mapped);
    }
    // BMCLAPI can redirect Maven downloads to nodes outside the trusted host list.
    // This fixed university mirror provides the same files without dynamic redirects.
    let parsed = Url::parse(url).map_err(|_| ApiError::new("INVALID_URL", "下载地址无效。"))?;
    let path = match parsed.host_str().unwrap_or("") {
        "maven.minecraftforge.net" | "libraries.minecraft.net" => Some(parsed.path()),
        "files.minecraftforge.net" => Some(
            parsed
                .path()
                .strip_prefix("/maven")
                .unwrap_or(parsed.path()),
        ),
        "bmclapi2.bangbang93.com" => parsed.path().strip_prefix("/maven"),
        _ => None,
    };
    if let Some(path) = path.filter(|path| path.starts_with('/')) {
        let mut university = parsed.clone();
        university
            .set_host(Some("mirror.nju.edu.cn"))
            .map_err(|_| ApiError::new("INVALID_URL", "镜像地址无效。"))?;
        university.set_path(&format!("/bmclapi{path}"));
        mirrors.push(university.to_string());
    }
    let mut urls = if mirror {
        mirrors.push(original);
        mirrors
    } else {
        let mut urls = vec![original];
        urls.extend(mirrors);
        urls
    };
    urls.dedup();
    Ok(urls)
}
fn source_failure(error: &ApiError) -> bool {
    error.code != "CANCELLED"
        && (error.retryable
            || matches!(
                error.code.as_str(),
                "NETWORK"
                    | "DOWNLOAD_HTTP_301"
                    | "DOWNLOAD_HTTP_302"
                    | "DOWNLOAD_HTTP_303"
                    | "DOWNLOAD_HTTP_307"
                    | "DOWNLOAD_HTTP_308"
                    | "DOWNLOAD_HTTP_401"
                    | "DOWNLOAD_HTTP_403"
                    | "DOWNLOAD_HTTP_404"
                    | "DOWNLOAD_HTTP_410"
                    | "DOWNLOAD_HTTP_429"
            ))
}
#[allow(clippy::too_many_arguments)]
pub async fn ensure_game_file(
    client: &Client,
    root: &Path,
    relative: &str,
    url: &str,
    sha1: Option<&str>,
    size: Option<u64>,
    mirror: bool,
    mod_mirror: bool,
    job: &Job,
) -> Result<std::path::PathBuf> {
    if sha1.is_some_and(|hash| !valid_hash(hash)) {
        return Err(ApiError::new("INVALID_HASH", "文件的 SHA-1 声明无效。"));
    }
    let sha1 = sha1.unwrap_or("");
    use std::sync::{Arc, Weak};
    type Locks =
        Mutex<std::collections::BTreeMap<std::path::PathBuf, Weak<tokio::sync::Mutex<()>>>>;
    static LOCKS: OnceLock<Locks> = OnceLock::new();
    let destination = storage::within(root, relative)?;
    let lock = {
        let mut locks = LOCKS.get_or_init(Mutex::default).lock().unwrap();
        locks.retain(|_, v| v.strong_count() > 0);
        if let Some(lock) = locks.get(&destination).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(destination.clone(), Arc::downgrade(&lock));
            lock
        }
    };
    let _guard = tokio::select! { _=job.cancel.cancelled()=>return Err(ApiError::new("CANCELLED","下载已取消。")), guard=lock.lock()=>guard };
    job.check()?;
    job.file_started(relative);
    let sources = sources(url, mirror, mod_mirror)?;
    let mut last_error = ApiError::new("DOWNLOAD_FAILED", "下载失败。");
    for (source_index, source) in sources.iter().enumerate() {
        let mcim = is_mod_mirror(source);
        if mcim && mod_mirror_cooling_down() {
            continue;
        }
        // Prefer the next available mod source before spending time on repeated failures.
        let attempts = if mcim
            || (source.starts_with("https://edge.forgecdn.net/")
                || source.starts_with("https://mediafilez.forgecdn.net/"))
                && source_index + 1 < sources.len()
        {
            1
        } else {
            3
        };
        for attempt in 0..attempts {
            let result = ensure_once(client, root, relative, source, sha1, size, job).await;
            match result {
                Ok(path) => {
                    job.file_progress(relative, path.metadata()?.len(), true);
                    return Ok(path);
                }
                Err(error) => {
                    if mcim
                        && (error.code == "NETWORK"
                            || error.code == "DOWNLOAD_HTTP_429"
                            || error.code.starts_with("DOWNLOAD_HTTP_5"))
                    {
                        // Avoid making every remaining file wait for the same unavailable service.
                        *mod_mirror_retry_after().lock().unwrap() =
                            Some(Instant::now() + Duration::from_secs(30));
                    }
                    let retry =
                        error.retryable && error.code != "CANCELLED" && attempt + 1 < attempts;
                    last_error = error;
                    if !retry {
                        break;
                    }
                    tokio::select! { _=job.cancel.cancelled()=>return Err(ApiError::new("CANCELLED","下载已取消。")), _=tokio::time::sleep(Duration::from_millis(300*(attempt+1)))=>{} }
                }
            }
        }
        if source_index + 1 >= sources.len()
            || !(source_failure(&last_error)
                || mcim
                    && (last_error.code.starts_with("DOWNLOAD_HTTP_")
                        || last_error.code == "UNTRUSTED_URL"))
            || job.cancel.is_cancelled()
        {
            break;
        }
    }
    last_error.message = format!("{}（{}）", last_error.message, relative);
    Err(last_error)
}
#[allow(clippy::too_many_arguments)]
pub async fn ensure_once(
    client: &Client,
    root: &Path,
    relative: &str,
    url: &str,
    sha1: &str,
    size: Option<u64>,
    job: &Job,
) -> Result<std::path::PathBuf> {
    job.check()?;
    let destination = storage::within(root, relative)?;
    let parent = destination.parent().unwrap();
    tokio::fs::create_dir_all(parent).await?;
    let temporary = parent.join(format!(".{}.part", uuid::Uuid::new_v4().simple()));
    struct Temporary(std::path::PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Temporary(temporary.clone());
    job.file_progress(relative, 0, false);
    let transfer = Transfer {
        client,
        url,
        size,
        relative,
        job,
    };
    // Connection and idle timeouts live in the transfer. Healthy large downloads
    // must not lose their accumulated data to a fixed whole-file deadline.
    transfer.download(&temporary).await?;
    if size.is_some_and(|n| temporary.metadata().is_ok_and(|m| m.len() != n))
        || (!sha1.is_empty() && !hash(&temporary).await?.eq_ignore_ascii_case(sha1))
    {
        return Err(verification_error(
            "HASH_MISMATCH",
            "下载文件校验失败，未替换已有文件。",
        ));
    }
    job.check()?;
    let checked = storage::within(root, relative)?;
    tokio::fs::rename(&temporary, &checked).await?;
    Ok(checked)
}
struct Transfer<'a> {
    client: &'a Client,
    url: &'a str,
    size: Option<u64>,
    relative: &'a str,
    job: &'a Job,
}
impl Transfer<'_> {
    async fn send(
        &self,
        url: &str,
        range: Option<(u64, u64)>,
    ) -> Result<(Response, tokio::sync::SemaphorePermit<'static>)> {
        let permit = tokio::select! { _=self.job.cancel.cancelled()=>return Err(ApiError::new("CANCELLED","下载已取消。")), permit=connections().acquire()=>permit.map_err(|_|ApiError::new("NETWORK_STOPPED","下载服务已停止。"))? };
        let mirror =
            Url::parse(url).is_ok_and(|url| url.host_str() == Some("bmclapi2.bangbang93.com"));
        if mirror && mirror_cooling_down() {
            return Err(ApiError::new("DOWNLOAD_HTTP_429", "镜像服务正在限流。"));
        }
        let mut request = self.client.get(url);
        if let Some((start, end)) = range {
            request = request
                .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
                .header(reqwest::header::ACCEPT_ENCODING, "identity");
        }
        let sending = async {
            let seconds = if is_mod_mirror(self.url) { 8 } else { 20 };
            tokio::time::timeout(Duration::from_secs(seconds), request.send())
                .await
                .map_err(|_| verification_error("NETWORK", "下载源响应超时，正在尝试备用地址。"))?
        };
        let response = tokio::select! { _=self.job.cancel.cancelled()=>return Err(ApiError::new("CANCELLED","下载已取消。")), response=sending=>response.map_err(network_error)? };
        observe_mirror(&response);
        if !response.status().is_success()
            && !(range.is_some() && response.status().as_u16() == 416)
        {
            let status = response.status();
            let mut error = ApiError::new(
                &format!("DOWNLOAD_HTTP_{}", status.as_u16()),
                format!("下载失败：HTTP {}。", status.as_u16()),
            );
            error.retryable = status.is_server_error() || status.as_u16() == 408;
            return Err(error);
        }
        Ok((response, permit))
    }
    async fn chunk(&self, response: &mut Response) -> Result<Option<Vec<u8>>> {
        let reading = async {
            let seconds = if is_mod_mirror(self.url) { 10 } else { 20 };
            tokio::time::timeout(Duration::from_secs(seconds), response.chunk())
                .await
                .map_err(|_| verification_error("NETWORK", "下载源传输停滞，正在尝试备用地址。"))?
        };
        tokio::select! {
            _ = self.job.cancel.cancelled() => Err(ApiError::new("CANCELLED", "下载已取消。")),
            chunk = reading => chunk.map_err(network_error),
        }
    }
    async fn download(&self, path: &Path) -> Result<()> {
        if self.size.is_some_and(|size| size >= 8 * 1024 * 1024) {
            match self.segments(path).await {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(error) if is_mod_mirror(self.url) && error.code != "INVALID_RANGE" => {
                    return Err(error);
                }
                Err(error) if source_failure(&error) => {
                    self.job.check()?;
                    match tokio::fs::remove_file(path).await {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                Err(error) => return Err(error),
            }
        }
        self.job.file_progress(self.relative, 0, false);
        let (mut response, _connection) = self.send(self.url, None).await?;
        let mut output = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .await?;
        let mut completed = 0_u64;
        while let Some(chunk) = self.chunk(&mut response).await? {
            completed += chunk.len() as u64;
            if completed > self.size.unwrap_or(2 * 1024 * 1024 * 1024) {
                return Err(verification_error(
                    "DOWNLOAD_SIZE",
                    "文件大小超过声明或下载限制。",
                ));
            }
            output.write_all(&chunk).await?;
            self.job.file_progress(self.relative, completed, false);
        }
        output.sync_all().await?;
        Ok(())
    }
    async fn segments(&self, path: &Path) -> Result<bool> {
        let size = self.size.unwrap();
        // Spend connections on waiting modpack files before splitting small files.
        let count = if Url::parse(self.url).is_ok_and(|url| {
            matches!(
                url.host_str(),
                Some(
                    "mod.mcimirror.top"
                        | "cdn.modrinth.com"
                        | "edge.forgecdn.net"
                        | "mediafilez.forgecdn.net"
                        | "media.forgecdn.net"
                        | "mediafiles.forgecdn.net"
                )
            )
        }) {
            (size / (8 * 1024 * 1024)).clamp(1, 4)
        } else {
            16
        };
        if count == 1 {
            return Ok(false);
        }
        let url = {
            let (probe, _connection) = self.send(self.url, Some((0, 0))).await?;
            if matches!(probe.status().as_u16(), 200 | 416) {
                return Ok(false);
            }
            validate_range(&probe, 0, 0, size)?;
            probe.url().to_string()
        };
        let output = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .await?;
        output.set_len(size).await?;
        drop(output);
        let completed = Mutex::new(0_u64);
        let ranges = (0..count).map(|index| {
            let url = &url;
            let completed = &completed;
            async move {
                let start = size * index / count;
                let end = size * (index + 1) / count - 1;
                let (mut response, _connection) = self.send(url, Some((start, end))).await?;
                if matches!(response.status().as_u16(), 200 | 416) {
                    return Err(ApiError::new("RANGE_UNSUPPORTED", "服务不支持分段下载。"));
                }
                validate_range(&response, start, end, size)?;
                let mut output = tokio::fs::OpenOptions::new().write(true).open(path).await?;
                output.seek(std::io::SeekFrom::Start(start)).await?;
                let mut position = start;
                while let Some(chunk) = self.chunk(&mut response).await? {
                    position += chunk.len() as u64;
                    if position > end + 1 {
                        return Err(verification_error(
                            "DOWNLOAD_SIZE",
                            "分段响应超出请求范围。",
                        ));
                    }
                    output.write_all(&chunk).await?;
                    let mut completed = completed.lock().unwrap();
                    *completed += chunk.len() as u64;
                    self.job.file_progress(self.relative, *completed, false);
                }
                if position != end + 1 {
                    return Err(verification_error("DOWNLOAD_SIZE", "分段响应不完整。"));
                }
                output.sync_all().await?;
                Ok(())
            }
        });
        match futures_util::future::try_join_all(ranges).await {
            Ok(_) => Ok(true),
            Err(error) if error.code == "RANGE_UNSUPPORTED" => {
                tokio::fs::remove_file(path).await?;
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }
}
fn validate_range(response: &Response, start: u64, end: u64, size: u64) -> Result<()> {
    let expected = format!("bytes {start}-{end}/{size}");
    if response.status().as_u16() != 206
        || !response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case(&expected))
        || response
            .content_length()
            .is_some_and(|length| length != end - start + 1)
        || response
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .is_some_and(|value| !value.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        return Err(verification_error(
            "INVALID_RANGE",
            "分段响应与请求范围不匹配。",
        ));
    }
    Ok(())
}

fn network_error(error: ApiError) -> ApiError {
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcim_failures_fall_back_once_and_cancellation_releases_streams() {
        use serde_json::{Value, json};
        use std::{collections::BTreeMap, ffi::c_void};
        struct Http {
            requests: Vec<String>,
            streams: BTreeMap<String, (String, u8)>,
            status: u16,
            stall: bool,
            slow: bool,
            ready_at: Option<tokio::time::Instant>,
        }
        unsafe extern "C" fn call(
            context: *mut c_void,
            bytes: *const u8,
            length: usize,
            respond: hydrogen_plugin_sdk::Respond,
            reply: *mut c_void,
        ) -> i32 {
            let http = unsafe { &mut *context.cast::<Http>() };
            let request: Value =
                serde_json::from_slice(unsafe { std::slice::from_raw_parts(bytes, length) })
                    .unwrap();
            let params = &request["params"];
            let value = match request["method"].as_str().unwrap() {
                "request" => {
                    let url = params["url"].as_str().unwrap().to_owned();
                    http.requests.push(url.clone());
                    let id = http.requests.len().to_string();
                    http.streams.insert(id.clone(), (url, 0));
                    json!(id)
                }
                "poll" => {
                    let (url, state) = http
                        .streams
                        .get(params["handle"].as_str().unwrap())
                        .unwrap();
                    if *state == 0 {
                        json!({"ready":true,"status":if is_mod_mirror(url) {http.status} else if url.starts_with("https://edge.forgecdn.net/") {404} else {200},"url":url,"length":if http.slow {15} else {3},"headers":{}})
                    } else if http.slow {
                        if *state > 15 {
                            json!({"ready":true,"chunk":null})
                        } else if http
                            .ready_at
                            .is_some_and(|time| tokio::time::Instant::now() < time)
                        {
                            json!({"ready":false})
                        } else {
                            json!({"ready":true,"chunk":"YQ=="})
                        }
                    } else if http.stall && is_mod_mirror(url) {
                        json!({"ready":false})
                    } else {
                        json!({"ready":true,"chunk":if *state > 1 {Value::Null} else {json!(if is_mod_mirror(url) {"YmFk"} else {"YWJj"})}})
                    }
                }
                "chunk" => {
                    http.streams
                        .get_mut(params["handle"].as_str().unwrap())
                        .unwrap()
                        .1 += 1;
                    if http.slow {
                        http.ready_at = Some(tokio::time::Instant::now() + Duration::from_secs(5));
                    }
                    Value::Null
                }
                "close" => {
                    http.streams.remove(params["handle"].as_str().unwrap());
                    Value::Null
                }
                _ => unreachable!(),
            };
            let bytes = serde_json::to_vec(&json!({"ok":true,"value":value})).unwrap();
            unsafe { respond(reply, bytes.as_ptr(), bytes.len()) };
            0
        }
        let root = std::env::temp_dir().join(format!("hydrogen-download-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
                *mod_mirror_retry_after().lock().unwrap() = None;
            }
        }
        let _cleanup = Cleanup(root.clone());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap();
        let client = Client::new(&Default::default(), &Default::default()).unwrap();
        let url = "https://cdn.modrinth.com/data/example.jar";
        let sha1 = "a9993e364706816aba3e25717850c26c9cd0d89d";
        let mut http = Http {
            requests: vec![],
            streams: BTreeMap::new(),
            status: 503,
            stall: false,
            slow: false,
            ready_at: None,
        };
        unsafe {
            hydrogen_plugin_sdk::with_services(call, (&mut http as *mut Http).cast(), || {
                runtime.block_on(async {
                    for name in ["first.jar", "second.jar"] {
                        let path = ensure_game_file(
                            &client,
                            &root,
                            name,
                            url,
                            Some(sha1),
                            Some(3),
                            false,
                            true,
                            &Job::default(),
                        )
                        .await
                        .unwrap();
                        assert_eq!(std::fs::read(path).unwrap(), b"abc");
                    }
                })
            });
        }
        // The failed mirror is not retried, including for the next file during its cooldown.
        assert_eq!(
            http.requests
                .iter()
                .filter(|url| is_mod_mirror(url))
                .count(),
            1
        );
        assert_eq!(http.requests.len(), 3);
        assert!(http.streams.is_empty());
        *mod_mirror_retry_after().lock().unwrap() = None;
        http.requests.clear();
        http.status = 200;
        unsafe {
            hydrogen_plugin_sdk::with_services(call, (&mut http as *mut Http).cast(), || {
                runtime.block_on(async {
                    let path = ensure_game_file(
                        &client,
                        &root,
                        "hash.jar",
                        url,
                        Some(sha1),
                        Some(3),
                        false,
                        true,
                        &Job::default(),
                    )
                    .await
                    .unwrap();
                    assert_eq!(std::fs::read(path).unwrap(), b"abc");
                })
            });
        }
        // A hash mismatch is also one mirror attempt, and must never publish the bad bytes.
        assert_eq!(http.requests.len(), 2);
        assert!(http.streams.is_empty());
        http.stall = true;
        let job = Job::default();
        unsafe {
            hydrogen_plugin_sdk::with_services(call, (&mut http as *mut Http).cast(), || {
                runtime.block_on(async {
                    let (result, ()) = tokio::join!(
                        ensure_game_file(
                            &client,
                            &root,
                            "cancel.jar",
                            url,
                            Some(sha1),
                            Some(3),
                            false,
                            true,
                            &job
                        ),
                        async {
                            tokio::time::sleep(Duration::from_millis(30)).await;
                            job.cancel.cancel();
                        }
                    );
                    assert_eq!(result.unwrap_err().code, "CANCELLED");
                })
            });
        }
        assert!(http.streams.is_empty());
        assert!(!root.join("cancel.jar").exists());
        http.requests.clear();
        unsafe {
            hydrogen_plugin_sdk::with_services(call, (&mut http as *mut Http).cast(), || {
                runtime.block_on(async {
                    let path = ensure_game_file(
                        &client,
                        &root,
                        "stalled.jar",
                        url,
                        Some(sha1),
                        Some(3),
                        false,
                        true,
                        &Job::default(),
                    )
                    .await
                    .unwrap();
                    assert_eq!(std::fs::read(path).unwrap(), b"abc");
                })
            });
        }
        assert_eq!(http.requests.len(), 2);
        assert!(http.streams.is_empty());
        http.requests.clear();
        unsafe {
            hydrogen_plugin_sdk::with_services(call, (&mut http as *mut Http).cast(), || {
                runtime.block_on(async {
                    let path = ensure_game_file(
                        &client,
                        &root,
                        "legacy-cdn.jar",
                        "https://edge.forgecdn.net/files/123/4/example.jar",
                        Some(sha1),
                        Some(3),
                        false,
                        false,
                        &Job::default(),
                    )
                    .await
                    .unwrap();
                    assert_eq!(std::fs::read(path).unwrap(), b"abc");
                })
            });
        }
        assert_eq!(http.requests.len(), 1);
        assert!(http.requests.iter().all(|url| !is_mod_mirror(url)));
        assert!(http.requests[0].starts_with("https://mediafilez.forgecdn.net/"));
        assert!(http.streams.is_empty());
        http.slow = true;
        unsafe {
            hydrogen_plugin_sdk::with_services(call, (&mut http as *mut Http).cast(), || {
                runtime.block_on(async {
                    let started = tokio::time::Instant::now();
                    let expected = format!("{:x}", Sha1::digest([b'a'; 15]));
                    let path = ensure_once(
                        &client,
                        &root,
                        "healthy.jar",
                        url,
                        &expected,
                        Some(15),
                        &Job::default(),
                    )
                    .await
                    .unwrap();
                    assert!(started.elapsed() >= Duration::from_secs(75));
                    assert_eq!(std::fs::read(path).unwrap(), [b'a'; 15]);
                })
            });
        }
        assert!(http.streams.is_empty());
        assert!(std::fs::read_dir(root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part")
        }));
    }

    #[test]
    fn mod_sources_prefer_mcim_and_disable_all_mod_mirrors() {
        for url in [
            "https://cdn.modrinth.com/data/Project1/versions/Version1/a%2Bb.jar?token=value",
            "https://edge.forgecdn.net/files/123/456/a%20b.jar",
            "https://mediafilez.forgecdn.net/files/123/456/a%20b.jar",
            "https://media.forgecdn.net/files/123/456/a%20b.jar",
            "https://mediafiles.forgecdn.net/files/123/456/a%20b.jar",
        ] {
            let mut mirror = Url::parse(url).unwrap();
            mirror.set_host(Some("mod.mcimirror.top")).unwrap();
            let mut origins = vec![url.to_owned()];
            if !url.starts_with("https://cdn.modrinth.com/") {
                let mut modern = Url::parse(url).unwrap();
                modern.set_host(Some("mediafilez.forgecdn.net")).unwrap();
                origins.insert(0, modern.to_string());
                origins.dedup();
            }
            let mut mirrored = vec![mirror.to_string()];
            mirrored.extend(origins.clone());
            assert_eq!(sources(url, false, true).unwrap(), mirrored);
            assert_eq!(sources(url, true, false).unwrap(), origins);
            if url.contains("cdn.modrinth.com") || url.contains("edge.forgecdn.net") {
                assert_eq!(sources(mirror.as_str(), false, false).unwrap(), origins);
            }
        }
        let mediafilez = "https://mediafilez.forgecdn.net/files/123/456/mod.jar";
        assert_eq!(sources(mediafilez, true, false).unwrap(), [mediafilez]);
        assert!(sources("https://mod.mcimirror.top/unknown", false, false).is_err());
    }
    #[test]
    fn forge_sources_keep_official_urls_and_recover_from_mirror_failures() {
        let path = "/net/minecraftforge/forge/1.20.1-47.4.0/forge-1.20.1-47.4.0-installer.jar";
        let official = format!("https://maven.minecraftforge.net{path}");
        let bmcl = format!("https://bmclapi2.bangbang93.com/maven{path}");
        let university = format!("https://mirror.nju.edu.cn/bmclapi{path}");
        assert_eq!(
            sources(&official, false, true).unwrap(),
            [official.clone(), bmcl.clone(), university.clone()]
        );
        assert_eq!(
            sources(&official, true, true).unwrap(),
            [bmcl.clone(), university.clone(), official]
        );
        assert_eq!(sources(&bmcl, true, true).unwrap(), [university, bmcl]);
        assert!(sources("https://untrusted.invalid/file", true, true).is_err());
        for code in [
            "DOWNLOAD_HTTP_302",
            "DOWNLOAD_HTTP_403",
            "DOWNLOAD_HTTP_429",
        ] {
            assert!(source_failure(&ApiError::new(code, "fixture")));
        }
        for code in [
            "NETWORK",
            "DOWNLOAD_HTTP_503",
            "INVALID_RANGE",
            "HASH_MISMATCH",
        ] {
            assert!(source_failure(&verification_error(code, "fixture")));
        }
        for code in ["CANCELLED", "IO_ERROR", "INVALID_HASH", "UNTRUSTED_URL"] {
            assert!(!source_failure(&ApiError::new(code, "fixture")));
        }
    }
}
