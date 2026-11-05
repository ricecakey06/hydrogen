use crate::{
    contracts::{ApiError, Result},
    launcher::net::{self, Client},
};
use semver::Version;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(tag = "status", content = "version", rename_all = "camelCase")]
pub(super) enum Update {
    NoRelease,
    UpToDate,
    Available(String),
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
}

impl Release {
    fn compare(&self, installed: &Version) -> Result<Update> {
        if !self.assets.iter().any(|asset| asset.name == "Launcher.exe") {
            return Ok(Update::NoRelease);
        }
        let version = Version::parse(self.tag_name.strip_prefix('v').unwrap_or(&self.tag_name))
            .map_err(|_| ApiError::new("UPDATE_VERSION", "发布版本号无效。"))?;
        // The legacy launcher also shipped an asset named Launcher.exe.
        if version < Version::new(0, 2, 7) {
            return Ok(Update::NoRelease);
        }
        Ok(if version.cmp_precedence(installed).is_gt() {
            Update::Available(version.to_string())
        } else {
            Update::UpToDate
        })
    }
}

pub(super) async fn check_update(client: &Client) -> Result<Update> {
    let response = client
        .get("https://api.github.com/repos/ricecakey06/hydrogen/releases/latest")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Update::NoRelease);
    }
    let release: Release =
        serde_json::from_slice(&net::response_bytes(response, 512 * 1024).await?)?;
    release.compare(&Version::parse(env!("CARGO_PKG_VERSION")).expect("valid Cargo version"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_tauri_releases_by_semver() {
        let installed = Version::new(0, 2, 9);
        let mut release = Release {
            tag_name: "v0.2.10".into(),
            assets: vec![Asset {
                name: "Launcher.exe".into(),
            }],
        };
        assert_eq!(
            release.compare(&installed).unwrap(),
            Update::Available("0.2.10".into())
        );
        release.tag_name = "v0.2.9".into();
        assert_eq!(release.compare(&installed).unwrap(), Update::UpToDate);
        release.tag_name = "v0.2.8".into();
        assert_eq!(release.compare(&installed).unwrap(), Update::UpToDate);
        release.tag_name = "v0.1.0".into();
        assert_eq!(release.compare(&installed).unwrap(), Update::NoRelease);
        release.tag_name = "v0.2.10".into();
        release.assets.clear();
        assert_eq!(release.compare(&installed).unwrap(), Update::NoRelease);
    }
}
