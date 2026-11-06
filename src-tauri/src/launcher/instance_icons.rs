//! Local instance artwork is a view resource, never persisted in instance records.
use super::storage;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Cursor, Read},
    path::{Path, PathBuf},
    time::SystemTime,
};

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];
const NAMES: [&str; 3] = ["icon", "pack", "logo"];

struct Cached {
    stamp: (u64, Option<SystemTime>),
    source: Option<String>,
}

#[derive(Default)]
pub struct Icons {
    cache: BTreeMap<PathBuf, Cached>,
}

impl Icons {
    pub fn load(&mut self, game_root: &Path, directory: &str) -> Option<String> {
        if !crate::plugins::manifest::safe_relative(directory) || directory.contains(['/', '\\']) {
            return None;
        }
        let root = storage::within(game_root, &format!("versions/{directory}")).ok()?;
        let mut candidates = EXTENSIONS
            .map(|ext| format!(".hydrogen/instance-icon.{ext}"))
            .to_vec();
        for name in NAMES {
            for ext in EXTENSIONS {
                candidates.push(format!(".hydrogen/instance-icon/{name}.{ext}"));
            }
        }
        candidates.push("PCL/Logo.png".into());
        for name in NAMES {
            for ext in EXTENSIONS {
                candidates.push(format!("{name}.{ext}"));
            }
        }
        candidates.into_iter().find_map(|relative| {
            let path = storage::within(&root, &relative).ok()?;
            self.read(&path)
        })
    }

    fn read(&mut self, path: &Path) -> Option<String> {
        let metadata = fs::metadata(path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        let stamp = (metadata.len(), metadata.modified().ok());
        if let Some(cached) = self.cache.get(path).filter(|cached| cached.stamp == stamp) {
            return cached.source.clone();
        }
        let source = (|| {
            if metadata.len() > MAX_BYTES {
                return None;
            }
            let mut bytes = Vec::new();
            File::open(path)
                .ok()?
                .take(MAX_BYTES + 1)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() as u64 > MAX_BYTES {
                return None;
            }
            crate::plugins::themes::validate_image(&bytes, path, true).ok()?;
            let image = image::load_from_memory(&bytes).ok()?;
            let image = image.thumbnail(image.width().min(128), image.height().min(128));
            let mut png = Cursor::new(Vec::new());
            image.write_to(&mut png, image::ImageFormat::Png).ok()?;
            Some(format!(
                "data:image/png;base64,{}",
                STANDARD.encode(png.into_inner())
            ))
        })();
        // Keep snapshots cheap without holding artwork from every repository ever visited.
        if self.cache.len() >= 128 {
            self.cache.pop_first();
        }
        self.cache.insert(
            path.to_owned(),
            Cached {
                stamp,
                source: source.clone(),
            },
        );
        source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(path: &Path, size: u32) -> Vec<u8> {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::DynamicImage::new_rgba8(size, size)
            .save(path)
            .unwrap();
        fs::read(path).unwrap()
    }

    #[test]
    fn custom_artwork_priority_fallback_and_updates() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let pack = root.join("versions/Pack");
        let mut icons = Icons::default();
        assert!(icons.load(root, "Pack").is_none());
        let pcl = png(&pack.join("PCL/Logo.png"), 8);
        png(&pack.join("icon.png"), 16);
        assert_eq!(
            icons.load(root, "Pack").unwrap(),
            format!("data:image/png;base64,{}", STANDARD.encode(pcl))
        );
        let imported = pack.join(".hydrogen/instance-icon/icon.png");
        png(&imported, 256);
        let first = icons.load(root, "Pack").unwrap();
        let data = STANDARD.decode(first.split_once(',').unwrap().1).unwrap();
        assert_eq!(image::load_from_memory(&data).unwrap().width(), 128);
        fs::write(&imported, b"corrupt").unwrap();
        assert_ne!(icons.load(root, "Pack").unwrap(), first);
        png(&imported, 32);
        let updated = icons.load(root, "Pack").unwrap();
        assert_ne!(first, updated);
        assert_eq!(icons.load(root, "Pack").unwrap(), updated);
        fs::remove_file(imported).unwrap();
        assert_ne!(icons.load(root, "Pack").unwrap(), updated);
        assert!(icons.load(root, "Other instance").is_none());
    }

    #[test]
    fn invalid_images_and_paths_do_not_break_the_view() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let pack = root.join("versions/Pack");
        let mut icons = Icons::default();
        png(&pack.join("icon.png"), 1601);
        fs::write(
            pack.join("pack.jpg"),
            fs::read(pack.join("icon.png")).unwrap(),
        )
        .unwrap();
        fs::write(pack.join("logo.png"), vec![0; MAX_BYTES as usize + 1]).unwrap();
        assert!(icons.load(root, "Pack").is_none());
        png(&root.join("icon.png"), 8);
        for directory in ["../", "../Other", "Pack/..", "Pack\\..", "C:\\", ""] {
            assert!(icons.load(root, directory).is_none());
        }
    }

    #[test]
    fn linked_artwork_is_skipped() {
        let temp = tempfile::tempdir().unwrap();
        let pack = temp.path().join("versions/Pack");
        let outside = temp.path().join("outside.png");
        png(&outside, 8);
        fs::create_dir_all(&pack).unwrap();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(&outside, pack.join("icon.png"));
        #[cfg(not(windows))]
        let linked = std::os::unix::fs::symlink(&outside, pack.join("icon.png"));
        if linked.is_err() {
            // Windows may require Developer Mode to create a symlink.
            return;
        }
        assert!(Icons::default().load(temp.path(), "Pack").is_none());
    }
}
