use crate::{
    contracts::{ApiError, Result},
    http::{Client, RequestBuilder},
    jobs::Job,
    net::response_bytes,
    storage,
};
use reqwest::{Url, header::HeaderName};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{Read, Seek},
    path::Path,
};

const API: &str = "https://api.curseforge.com/v1/";
const KEY_HEADER: HeaderName = HeaderName::from_static("x-api-key");

pub fn api_key() -> Option<String> {
    std::env::var("HYDROGEN_CURSEFORGE_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
        .or_else(|| {
            option_env!("HYDROGEN_CURSEFORGE_API_KEY")
                .filter(|key| !key.trim().is_empty())
                .map(str::to_owned)
        })
}

pub fn endpoint(path: &str) -> Url {
    Url::parse(&format!("{API}{path}")).expect("valid CurseForge API URL")
}

pub async fn fetch(request: RequestBuilder, key: &str) -> Result<Value> {
    let response = request.header(KEY_HEADER, key.trim()).send().await?;
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err(ApiError::new(
            "CURSEFORGE_KEY",
            "CurseForge 拒绝了请求，请检查 API Key。",
        ));
    }
    Ok(serde_json::from_slice(
        &response_bytes(response, 32 * 1024 * 1024).await?,
    )?)
}

pub fn missing_key() -> ApiError {
    ApiError::new("CURSEFORGE_KEY", "下载 CurseForge 模组需要 API Key。")
}

pub fn download_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.port().is_none_or(|p| p == 443)
            && url.username().is_empty()
            && url.password().is_none()
            && matches!(
                url.host_str(),
                Some(
                    "edge.forgecdn.net"
                        | "mediafilez.forgecdn.net"
                        | "media.forgecdn.net"
                        | "mediafiles.forgecdn.net"
                )
            )
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurseFile {
    pub id: u32,
    pub mod_id: u32,
    pub game_id: u32,
    pub file_name: String,
    pub file_length: u64,
    pub file_fingerprint: u32,
    pub download_url: Option<String>,
    #[serde(default)]
    hashes: Vec<CurseHash>,
}
pub async fn exact_matches(
    client: &Client,
    key: &str,
    fingerprints: &[u32],
) -> Result<Vec<CurseFile>> {
    #[derive(Deserialize)]
    struct Match {
        file: CurseFile,
    }
    let response = fetch(
        client
            .post(endpoint("fingerprints/432"))
            .json(&json!({"fingerprints":fingerprints})),
        key,
    )
    .await?;
    let matches: Vec<Match> = serde_json::from_value(response["data"]["exactMatches"].clone())?;
    Ok(matches
        .into_iter()
        .map(|entry| entry.file)
        .filter(|file| file.game_id == 432)
        .collect())
}
#[derive(Deserialize)]
struct CurseHash {
    algo: u32,
    value: String,
}
impl CurseFile {
    pub fn sha1(&self) -> Option<&str> {
        self.hashes
            .iter()
            .find(|h| h.algo == 1 && storage::valid_hash(&h.value))
            .map(|h| h.value.as_str())
    }
    pub fn url(&self) -> Result<&str> {
        self.download_url
            .as_deref()
            .filter(|url| download_url(url))
            .ok_or_else(|| {
                ApiError::new(
                    "CURSEFORGE_DOWNLOAD",
                    format!(
                        "CurseForge 未向第三方开放此文件的下载（{} / {}）。",
                        self.mod_id, self.id
                    ),
                )
            })
    }
}

pub async fn fingerprint(path: &Path, job: &Job) -> Result<u32> {
    fn meaningful(byte: u8) -> bool {
        !matches!(byte, 9 | 10 | 13 | 32)
    }
    let mut input = File::open(path)?;
    let mut buffer = vec![0; 128 * 1024];
    let mut normalized = 0_u32;
    loop {
        job.check()?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        normalized = normalized
            .checked_add(
                buffer[..count]
                    .iter()
                    .copied()
                    .filter(|b| meaningful(*b))
                    .count() as u32,
            )
            .ok_or_else(|| ApiError::new("FILE_LIMIT", "模组文件超过大小限制。"))?;
        tokio::task::yield_now().await;
    }
    // CurseForge uses MurmurHash2 with seed 1 over the normalized bytes.
    const M: u32 = 0x5bd1e995;
    let mut hash = 1 ^ normalized;
    let mut word = [0; 4];
    let mut offset = 0;
    input.rewind()?;
    loop {
        job.check()?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        for byte in buffer[..count].iter().copied().filter(|b| meaningful(*b)) {
            word[offset] = byte;
            offset += 1;
            if offset == 4 {
                let value = u32::from_le_bytes(word).wrapping_mul(M);
                hash = hash.wrapping_mul(M) ^ (value ^ (value >> 24)).wrapping_mul(M);
                word = [0; 4];
                offset = 0;
            }
        }
        tokio::task::yield_now().await;
    }
    if offset != 0 {
        hash = (hash ^ u32::from_le_bytes(word)).wrapping_mul(M);
    }
    hash = (hash ^ (hash >> 13)).wrapping_mul(M);
    Ok(hash ^ (hash >> 15))
}
