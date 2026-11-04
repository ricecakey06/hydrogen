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
                "maven.fabricmc.net",
                "maven.minecraftforge.net",
                "files.minecraftforge.net",
                "maven.neoforged.net",
                "maven.quiltmc.org",
                "repo.maven.apache.org",
                "repo1.maven.org",
                "libraries.optifine.net",
                "api.modrinth.com",
                "cdn.modrinth.com",
                "cdn.jsdelivr.net",
                "raw.githubusercontent.com",
                "bmclapi2.bangbang93.com",
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
#[allow(clippy::too_many_arguments)]
pub async fn ensure_game_file(
    client: &Client,
    root: &Path,
    relative: &str,
    url: &str,
    sha1: Option<&str>,
    size: Option<u64>,
    mirror: bool,
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
    let source = source_url(url, mirror)?;
    let original = object_endpoint(url)?;
    let mapped = object_endpoint(&source)?;
    let sources = if mapped != original {
        vec![mapped, original]
    } else {
        vec![original]
    };
    let mut last_error = ApiError::new("DOWNLOAD_FAILED", "下载失败。");
    for (source_index, source) in sources.iter().enumerate() {
        for attempt in 0..3 {
            let result = ensure_once(client, root, relative, source, sha1, size, job).await;
            match result {
                Ok(path) => {
                    job.file_progress(relative, path.metadata()?.len(), true);
                    return Ok(path);
                }
                Err(error) => {
                    let retry = error.retryable && error.code != "CANCELLED" && attempt < 2;
                    last_error = error;
                    if !retry {
                        break;
                    }
                    tokio::select! { _=job.cancel.cancelled()=>return Err(ApiError::new("CANCELLED","下载已取消。")), _=tokio::time::sleep(Duration::from_millis(300*(attempt+1)))=>{} }
                }
            }
        }
        let fallback = last_error.code == "NETWORK"
            || matches!(
                last_error.code.as_str(),
                "DOWNLOAD_HTTP_404" | "DOWNLOAD_HTTP_410" | "DOWNLOAD_HTTP_429"
            );
        if source_index + 1 >= sources.len() || !fallback || job.cancel.is_cancelled() {
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
    let seconds = size.map_or(60, |n| (n / 131072 + 10).clamp(60, 600));
    tokio::time::timeout(Duration::from_secs(seconds), transfer.download(&temporary))
        .await
        .map_err(|_| verification_error("NETWORK", "下载请求超时，请检查网络后重试。"))??;
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
        let response = tokio::select! { _=self.job.cancel.cancelled()=>return Err(ApiError::new("CANCELLED","下载已取消。")), response=request.send()=>response.map_err(network_error)? };
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
    async fn download(&self, path: &Path) -> Result<()> {
        if self.size.is_some_and(|size| size >= 8 * 1024 * 1024) && self.segments(path).await? {
            return Ok(());
        }
        self.job.file_progress(self.relative, 0, false);
        let (mut response, _connection) = self.send(self.url, None).await?;
        let mut output = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .await?;
        let mut completed = 0_u64;
        while let Some(chunk) = tokio::select! { _=self.job.cancel.cancelled()=>return Err(ApiError::new("CANCELLED","下载已取消。")), chunk=response.chunk()=>chunk.map_err(network_error)? }
        {
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
        let ranges = (0..16).map(|index| {
            let url = &url;
            let completed = &completed;
            async move {
                let start = size * index / 16;
                let end = size * (index + 1) / 16 - 1;
                let (mut response, _connection) = self.send(url, Some((start, end))).await?;
                if matches!(response.status().as_u16(), 200 | 416) { return Err(ApiError::new("RANGE_UNSUPPORTED", "服务不支持分段下载。")); }
                validate_range(&response, start, end, size)?;
                let mut output = tokio::fs::OpenOptions::new().write(true).open(path).await?;
                output.seek(std::io::SeekFrom::Start(start)).await?;
                let mut position = start;
                while let Some(chunk) = tokio::select! { _=self.job.cancel.cancelled()=>return Err(ApiError::new("CANCELLED","下载已取消。")), chunk=response.chunk()=>chunk.map_err(network_error)? } {
                    position += chunk.len() as u64;
                    if position > end + 1 { return Err(verification_error("DOWNLOAD_SIZE", "分段响应超出请求范围。")); }
                    output.write_all(&chunk).await?;
                    let mut completed = completed.lock().unwrap();
                    *completed += chunk.len() as u64;
                    self.job.file_progress(self.relative, *completed, false);
                }
                if position != end + 1 { return Err(verification_error("DOWNLOAD_SIZE", "分段响应不完整。")); }
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
