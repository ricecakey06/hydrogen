use super::{manager, manifest::Plugin};
use crate::{
    contracts::{ApiError, Result},
    launcher::storage,
};
use std::path::Path;
include!(concat!(env!("OUT_DIR"), "/bundled-plugins.rs"));

pub fn migrated(id: &str) -> bool {
    PACKAGES.iter().any(|(key, _)| *key == id)
}

pub fn preinstalled(root: &Path, id: &str, digest: &str) -> bool {
    storage::within(root, "plugin-seeds.json")
        .and_then(|path| storage::read_json::<serde_json::Value>(&path))
        .ok()
        .is_some_and(|seeds| seeds[id]["installed"].as_str() == Some(digest))
}

pub fn extract(root: &Path) -> Result<()> {
    seed_packages(root)
}

/// Embedded bytes are only an installation source. The runtime always loads the disk package.
fn seed_packages(root: &Path) -> Result<()> {
    deploy(root, PACKAGES)
}

/// Apply an application's distribution through the ordinary package installation transaction.
/// Changed distribution bytes replace an equal/older installed version, including modified or missing files.
/// An unchanged distribution does not resurrect a removed entry, and never downgrades a plugin.
pub fn deploy(root: &Path, packages: &[(&str, &[u8])]) -> Result<()> {
    use sha2::{Digest, Sha256};
    use std::{collections::BTreeMap, fs};
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Seed {
        archive: String,
        installed: Option<String>,
        #[serde(default)]
        version: Option<String>,
    }
    let record = storage::within(root, "plugin-seeds.json")?;
    let mut seeds: BTreeMap<String, Seed> = if record.exists() {
        storage::read_json(&record)?
    } else {
        BTreeMap::new()
    };
    for &(id, bytes) in packages {
        if !super::manifest::valid_id(id) {
            return Err(ApiError::new("BUNDLED_ID", "预装目录包含无效插件 ID。"));
        }
        let archive_digest = format!("{:x}", Sha256::digest(bytes));
        if seeds
            .get(id)
            .is_some_and(|seed| seed.archive == archive_digest)
        {
            continue;
        }
        let archive = storage::within(
            root,
            &format!("plugin-staging/{}.hplugin", uuid::Uuid::new_v4().simple()),
        )?;
        fs::create_dir_all(archive.parent().unwrap())?;
        fs::write(&archive, bytes)?;
        let prepared = manager::prepare(root, &archive);
        fs::remove_file(&archive)?;
        let candidate = prepared?;
        if candidate.plugin.manifest.id != id {
            return Err(ApiError::new("BUNDLED_ID", "预装包 ID 与发行目录不符。"));
        }
        let next = semver::Version::parse(&candidate.plugin.manifest.version)
            .map_err(|_| ApiError::new("BUNDLED_VERSION", "预装插件版本无效。"))?;
        let destination = storage::within(root, &format!("plugins/{id}"))?;
        let installed_version = if destination.exists() {
            super::manifest::within(&destination, "plugin.json")
                .ok()
                .and_then(|path| storage::read_json::<serde_json::Value>(&path).ok())
                .filter(|value| value["id"] == id)
                .and_then(|value| {
                    value["version"]
                        .as_str()
                        .and_then(|v| semver::Version::parse(v).ok())
                })
        } else {
            None
        };
        let known_version = installed_version.or_else(|| {
            seeds
                .get(id)
                .and_then(|seed| seed.version.as_deref())
                .and_then(|v| semver::Version::parse(v).ok())
        });
        let previous = manager::current_digest(root, id)?;
        let replace = known_version
            .as_ref()
            .is_none_or(|version| next >= *version);
        let installed = if replace {
            Some(manager::install(root, &candidate, previous.as_deref())?.digest)
        } else {
            previous.filter(|digest| {
                digest == &candidate.plugin.digest
                    || seeds
                        .get(id)
                        .is_some_and(|seed| seed.installed.as_ref() == Some(digest))
            })
        };
        seeds.insert(
            id.into(),
            Seed {
                archive: archive_digest,
                installed,
                version: Some(candidate.plugin.manifest.version.clone()),
            },
        );
        storage::write_json(&record, &seeds)?;
    }
    Ok(())
}

pub fn load_all(root: &Path) -> Result<Vec<Plugin>> {
    PACKAGES
        .iter()
        .map(|(id, _)| super::manifest::load(&storage::within(root, &format!("plugins/{id}"))?))
        .collect()
}
