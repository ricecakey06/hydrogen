use hydrogen_plugin_services::{
    contracts::{ApiError, Result},
    jobs::Job,
    storage,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
use zip::ZipArchive;

pub const MAX_FILES: usize = 50_000;
pub const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_TOTAL: u64 = 16 * 1024 * 1024 * 1024;
pub const MAX_MANIFEST: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Format {
    Modrinth,
    Curseforge,
}
impl Format {
    pub fn manifest(self) -> &'static str {
        match self {
            Self::Modrinth => "modrinth.index.json",
            Self::Curseforge => "manifest.json",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Components {
    pub kind: String,
    pub loader: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexFile {
    pub path: String,
    pub hashes: BTreeMap<String, String>,
    pub downloads: Vec<String>,
    pub file_size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
}
impl IndexFile {
    fn client(&self) -> Option<&str> {
        self.env.as_ref()?.get("client").map(String::as_str)
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Index {
    pub format_version: u32,
    pub game: String,
    pub name: String,
    pub version_id: String,
    pub dependencies: BTreeMap<String, String>,
    pub files: Vec<IndexFile>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct CurseFile {
    #[serde(rename = "projectID")]
    pub project: u32,
    #[serde(rename = "fileID")]
    pub file: u32,
    #[serde(default = "required")]
    pub required: bool,
}
fn required() -> bool {
    true
}

#[derive(Deserialize, Serialize)]
pub struct Loader {
    pub id: String,
    #[serde(default)]
    pub primary: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Minecraft {
    pub version: String,
    #[serde(default)]
    pub mod_loaders: Vec<Loader>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub manifest_type: String,
    pub manifest_version: u32,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub author: String,
    pub minecraft: Minecraft,
    pub files: Vec<CurseFile>,
    pub overrides: String,
}

pub enum Pack {
    Modrinth(Index),
    Curseforge(Manifest),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview<'a> {
    format: Format,
    name: &'a str,
    version: &'a str,
    game_version: &'a str,
    components: Option<Components>,
    files: usize,
    optional_files: usize,
    needs_key: bool,
}

pub fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new("INVALID_MODPACK", message)
}

pub fn zip_error(error: zip::result::ZipError) -> ApiError {
    invalid(format!("整合包 ZIP 无法读取或写入（{error}）。"))
}

fn limit() -> ApiError {
    invalid("整合包超出文件数或大小限制。")
}

pub fn add_size(total: &mut u64, size: u64) -> Result<()> {
    if size > MAX_FILE || total.saturating_add(size) > MAX_TOTAL {
        return Err(limit());
    }
    *total += size;
    Ok(())
}

pub fn portable_path(path: &str) -> bool {
    storage::safe_relative(path)
        && path.split('/').all(|part| {
            let stem = part
                .split('.')
                .next()
                .unwrap_or("")
                .trim_end()
                .to_uppercase();
            !matches!(
                stem.as_str(),
                "CONIN$" | "CONOUT$" | "CON" | "PRN" | "AUX" | "NUL"
            ) && !((stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem[3..].chars().count() == 1
                && stem[3..].contains(|c: char| c.is_ascii_digit() || "¹²³".contains(c)))
        })
}

pub fn content_path(path: &str, directory: &str) -> bool {
    if !portable_path(path) {
        return false;
    }
    let root = path.split('/').next().unwrap_or_default().to_lowercase();
    !matches!(
        root.as_str(),
        "assets" | "libraries" | "versions" | ".hydrogen" | ".fabric" | ".quilt"
    ) && !root.starts_with("natives-")
        && !root.starts_with(".hydrogen-pack-")
        && root != format!("{}.json", directory.to_lowercase())
        && root != format!("{}.jar", directory.to_lowercase())
}

fn version_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._+-".contains(&c))
        && portable_path(value)
}

fn loader(kind: &str, version: &str) -> Result<Components> {
    if !matches!(kind, "fabric" | "forge" | "neoforge" | "quilt") {
        return Err(invalid(format!("暂不支持整合包加载器 {kind}。")));
    }
    if !version_token(version) {
        return Err(invalid("整合包加载器版本无效。"));
    }
    Ok(Components {
        kind: kind.into(),
        loader: version.into(),
    })
}

pub fn valid_hash(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Pack {
    pub fn open(path: &Path) -> Result<(ZipArchive<File>, Self)> {
        let mut zip = ZipArchive::new(File::open(path)?).map_err(zip_error)?;
        if zip.len() > MAX_FILES + 1 {
            return Err(limit());
        }
        let manifests: Vec<_> = zip
            .file_names()
            .filter(|name| {
                *name == Format::Modrinth.manifest() || *name == Format::Curseforge.manifest()
            })
            .map(str::to_owned)
            .collect();
        let [manifest] = manifests.as_slice() else {
            return Err(invalid(
                "请选择含有效 Modrinth 或 CurseForge 清单的整合包。",
            ));
        };
        let mut data = Vec::new();
        zip.by_name(manifest)
            .map_err(zip_error)?
            .take(MAX_MANIFEST + 1)
            .read_to_end(&mut data)?;
        if data.len() as u64 > MAX_MANIFEST {
            return Err(limit());
        }
        let data = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&data);
        let pack = if manifest == Format::Modrinth.manifest() {
            let index: Index = serde_json::from_slice(data)?;
            if index.game != "minecraft" || index.format_version != 1 {
                return Err(invalid("不支持此 Modrinth 整合包格式版本。"));
            }
            let mut seen = HashSet::new();
            let mut total = 0;
            if index.files.len() > MAX_FILES {
                return Err(limit());
            }
            for file in &index.files {
                if !portable_path(&file.path)
                    || !seen.insert(file.path.to_lowercase())
                    || !file.hashes.get("sha1").is_some_and(|h| valid_hash(h, 40))
                    || !file
                        .hashes
                        .get("sha512")
                        .is_some_and(|h| valid_hash(h, 128))
                    || !matches!(
                        file.client(),
                        None | Some("required" | "optional" | "unsupported")
                    )
                    || file.downloads.is_empty()
                {
                    return Err(invalid(format!("整合包文件条目无效（{}）。", file.path)));
                }
                add_size(&mut total, file.file_size)?;
            }
            Self::Modrinth(index)
        } else {
            let manifest: Manifest = serde_json::from_slice(data)?;
            if manifest.manifest_type != "minecraftModpack" || manifest.manifest_version != 1 {
                return Err(invalid("不支持此 CurseForge 整合包格式版本。"));
            }
            if !manifest.overrides.is_empty() && !portable_path(&manifest.overrides) {
                return Err(invalid("整合包覆盖目录无效。"));
            }
            let mut seen = HashSet::new();
            if manifest.files.len() > MAX_FILES {
                return Err(limit());
            }
            if manifest
                .files
                .iter()
                .any(|f| f.project == 0 || f.file == 0 || !seen.insert(f.file))
            {
                return Err(invalid("CurseForge 清单包含无效或重复的文件编号。"));
            }
            Self::Curseforge(manifest)
        };
        pack.preview()?;
        Ok((zip, pack))
    }

    pub fn preview(&self) -> Result<Preview<'_>> {
        let preview = match self {
            Self::Modrinth(index) => {
                let game_version = index
                    .dependencies
                    .get("minecraft")
                    .ok_or_else(|| invalid("整合包缺少 Minecraft 版本。"))?;
                let mut components = None;
                for (kind, version) in index
                    .dependencies
                    .iter()
                    .filter(|(kind, _)| *kind != "minecraft")
                {
                    if components.is_some() {
                        return Err(invalid("整合包指定了多个加载器。"));
                    }
                    components = Some(loader(
                        kind.strip_suffix("-loader").unwrap_or(kind),
                        version,
                    )?);
                }
                Preview {
                    format: Format::Modrinth,
                    name: &index.name,
                    version: &index.version_id,
                    game_version,
                    components,
                    files: index
                        .files
                        .iter()
                        .filter(|f| f.client() == Some("required") || f.client().is_none())
                        .count(),
                    optional_files: index
                        .files
                        .iter()
                        .filter(|f| f.client() == Some("optional"))
                        .count(),
                    needs_key: false,
                }
            }
            Self::Curseforge(manifest) => {
                let components = manifest
                    .minecraft
                    .mod_loaders
                    .iter()
                    .find(|l| l.primary)
                    .or_else(|| manifest.minecraft.mod_loaders.first())
                    .map(|item| {
                        let (kind, version) = item
                            .id
                            .split_once('-')
                            .ok_or_else(|| invalid("CurseForge 加载器条目无效。"))?;
                        loader(kind, version)
                    })
                    .transpose()?;
                Preview {
                    format: Format::Curseforge,
                    name: &manifest.name,
                    version: &manifest.version,
                    game_version: &manifest.minecraft.version,
                    components,
                    files: manifest.files.iter().filter(|f| f.required).count(),
                    optional_files: manifest.files.iter().filter(|f| !f.required).count(),
                    needs_key: !manifest.files.is_empty(),
                }
            }
        };
        if !version_token(preview.game_version) || preview.name.trim().is_empty() {
            return Err(invalid("整合包名称或 Minecraft 版本无效。"));
        }
        Ok(preview)
    }

    pub fn downloads(&self, optional: bool) -> impl Iterator<Item = &IndexFile> {
        let files = match self {
            Self::Modrinth(index) => &index.files[..],
            Self::Curseforge(_) => &[],
        };
        files.iter().filter(move |f| match f.client() {
            Some("unsupported") => false,
            Some("optional") => optional,
            _ => true,
        })
    }

    pub fn icons(&self, zip: &mut ZipArchive<File>) -> Result<Vec<Override>> {
        let mut icons = BTreeMap::new();
        for index in 0..zip.len() {
            let entry = zip.by_index_raw(index).map_err(zip_error)?;
            let name = entry.name().to_ascii_lowercase();
            let Some((stem, ext)) = name.rsplit_once('.') else {
                continue;
            };
            if !["icon", "pack", "logo"].contains(&stem)
                || !["png", "jpg", "jpeg", "webp"].contains(&ext)
                || entry.is_dir()
                || entry.is_symlink()
                || entry.size() > 2 * 1024 * 1024
            {
                continue;
            }
            icons.entry(name.clone()).or_insert(Override {
                path: format!(".hydrogen/instance-icon/{name}"),
                index,
                size: entry.size(),
            });
        }
        Ok(icons.into_values().collect())
    }

    pub fn overrides(
        &self,
        zip: &mut ZipArchive<File>,
        directory: &str,
    ) -> Result<BTreeMap<String, Override>> {
        let layers = match self {
            Self::Modrinth(_) => vec!["overrides", "client-overrides"],
            Self::Curseforge(manifest) if !manifest.overrides.is_empty() => {
                vec![manifest.overrides.as_str()]
            }
            Self::Curseforge(_) => vec![],
        };
        let mut result: BTreeMap<String, Override> = BTreeMap::new();
        let mut total = 0;
        for layer in layers {
            let prefix = format!("{layer}/");
            let mut seen = HashSet::new();
            for index in 0..zip.len() {
                let entry = zip.by_index_raw(index).map_err(zip_error)?;
                let Some(path) = entry.name().strip_prefix(&prefix) else {
                    continue;
                };
                if entry.is_dir() {
                    continue;
                }
                let key = path.to_lowercase();
                if !content_path(path, directory)
                    || entry.is_symlink()
                    || !seen.insert(key.clone())
                    || result
                        .get(&key)
                        .is_some_and(|existing| existing.path != path)
                {
                    return Err(invalid(format!("覆盖文件路径无效或冲突（{path}）。")));
                }
                add_size(&mut total, entry.size())?;
                result.insert(
                    key,
                    Override {
                        path: path.into(),
                        index,
                        size: entry.size(),
                    },
                );
            }
        }
        Ok(result)
    }
}

pub struct Override {
    pub path: String,
    index: usize,
    pub size: u64,
}
impl Override {
    pub async fn extract(&self, zip: &mut ZipArchive<File>, stage: &Path, job: &Job) -> Result<()> {
        let path = storage::within(stage, &self.path)?;
        fs::create_dir_all(path.parent().unwrap())?;
        let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut input = zip.by_index(self.index).map_err(zip_error)?;
        if copy(&mut input, &mut output, self.size, job).await? != self.size {
            return Err(invalid("整合包覆盖文件的大小与清单不符。"));
        }
        Ok(())
    }
}

pub async fn copy(
    input: &mut impl Read,
    output: &mut impl Write,
    limit: u64,
    job: &Job,
) -> Result<u64> {
    let mut buffer = vec![0; 128 * 1024];
    let mut total = 0;
    loop {
        job.check()?;
        let count = input.read(&mut buffer)?;
        if count == 0 {
            return Ok(total);
        }
        total += count as u64;
        if total > limit {
            return Err(invalid("整合包文件超过声明的大小。"));
        }
        output.write_all(&buffer[..count])?;
        tokio::task::consume_budget().await;
    }
}
