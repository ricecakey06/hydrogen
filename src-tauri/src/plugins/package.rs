use super::manifest::{MAX_SOURCE, safe_relative};
use crate::contracts::{ApiError, Result};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn files(root: &Path) -> Result<(Vec<(String, PathBuf)>, String)> {
    fn walk(
        root: &Path,
        dir: &Path,
        out: &mut Vec<(String, PathBuf)>,
        total: &mut u64,
        source: &mut u64,
        depth: usize,
    ) -> Result<()> {
        if depth > 32 {
            return Err(ApiError::new("PACKAGE_LIMIT", "目录层级超过 32。"));
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let checked = super::manifest::within(root, &name)?;
            let meta = fs::metadata(&checked)?;
            if meta.is_dir() {
                walk(root, &checked, out, total, source, depth + 1)?;
            } else {
                *total += meta.len();
                if name.ends_with(".js") {
                    *source += meta.len();
                }
                if *total > 256 * 1024 * 1024 || *source > MAX_SOURCE || out.len() >= 4096 {
                    return Err(ApiError::new(
                        "PACKAGE_LIMIT",
                        "包超过 256 MiB、4096 文件或 2 MiB JS 源码上限。",
                    ));
                }
                out.push((name, checked));
            }
        }
        Ok(())
    }
    let mut entries = Vec::new();
    walk(root, root, &mut entries, &mut 0, &mut 0, 0)?;
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut names = HashSet::new();
    let mut hash = Sha256::new();
    for (name, path) in &entries {
        if !safe_relative(name) || !names.insert(name.to_lowercase()) {
            return Err(ApiError::new(
                "INVALID_PATH",
                "包包含非法路径或大小写冲突的文件名。",
            ));
        }
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        // A bounded streaming digest avoids holding whole package content in memory.
        let mut input = fs::File::open(path)?;
        hash.update(fs::metadata(path)?.len().to_le_bytes());
        std::io::copy(&mut input, &mut hash)?;
    }
    Ok((entries, format!("{:x}", hash.finalize())))
}

pub fn pack(root: &Path, output: &Path) -> Result<()> {
    let plugin = super::manifest::load(root)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = dunce::canonicalize(parent)?;
    if parent.starts_with(&plugin.root) {
        return Err(ApiError::new(
            "INVALID_OUTPUT",
            "压缩包输出必须位于插件目录外。",
        ));
    }
    let (entries, _) = files(&plugin.root)?;
    let mut output_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    let result = (|| -> Result<()> {
        let mut archive = zip::ZipWriter::new(&mut output_file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, path) in entries {
            archive
                .start_file(name, options)
                .map_err(|_| ApiError::new("PACKAGE_WRITE", "无法写入插件包。"))?;
            std::io::copy(&mut fs::File::open(path)?, &mut archive)?;
        }
        archive
            .finish()
            .map_err(|_| ApiError::new("PACKAGE_WRITE", "无法完成插件包。"))?;
        output_file.flush()?;
        if output_file.metadata()?.len() > 256 * 1024 * 1024 {
            return Err(ApiError::new("PACKAGE_LIMIT", "压缩包超过 256 MiB。"));
        }
        Ok(())
    })();
    if result.is_err() {
        drop(output_file);
        let _ = fs::remove_file(output);
    }
    result
}
