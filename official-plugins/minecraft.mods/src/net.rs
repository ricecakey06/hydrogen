use hydrogen_plugin_sdk::contracts::Result;
use sha1::{Digest, Sha1};
use std::path::Path;
use tokio::io::AsyncReadExt;
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
