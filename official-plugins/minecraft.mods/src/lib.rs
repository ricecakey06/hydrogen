mod mods;
mod net;
mod storage;
use base64::{Engine, engine::general_purpose::STANDARD};
use hydrogen_plugin_sdk::{
    contracts::{ApiError, Result},
    serde_json::{Value, json},
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

struct Import {
    stage: PathBuf,
    game: PathBuf,
    seen: BTreeSet<String>,
    plans: Vec<(PathBuf, PathBuf)>,
    originals: Vec<mods::ModRef>,
    preserve: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Update {
    sha1: String,
    old: Option<mods::ModRef>,
    enabled: bool,
}
impl Drop for Import {
    fn drop(&mut self) {
        if !self.preserve {
            let _ = fs::remove_dir_all(&self.stage);
        }
    }
}
impl Import {
    fn new(game: &Path) -> Result<Self> {
        let stage = storage::within(
            game,
            &format!(".hydrogen/import/{}", uuid::Uuid::new_v4().simple()),
        )?;
        fs::create_dir_all(&stage)?;
        Ok(Self {
            stage,
            game: game.into(),
            seen: BTreeSet::new(),
            plans: Vec::new(),
            originals: Vec::new(),
            preserve: false,
        })
    }
    async fn add(&mut self, path: &Path, update: Option<Update>) -> Result<()> {
        let limit = if update.is_some() { 1127 } else { 1000 };
        if self.seen.len() >= limit
            || !path.is_absolute()
            || path.ancestors().any(storage::linked)
            || !path.is_file()
        {
            return Err(error("所选文件无效或超过 1000 个文件。"));
        }
        let source_name = path
            .file_name()
            .ok_or_else(|| error("文件名无效。"))?
            .to_string_lossy();
        let old = update.as_ref().and_then(|update| update.old.as_ref());
        let enabled = if let Some(old) = old {
            mods::filename(&old.file_name)?;
            !old.file_name.to_ascii_lowercase().ends_with(".disabled")
        } else {
            update.as_ref().is_none_or(|update| update.enabled)
        };
        let name = if enabled {
            source_name.to_string()
        } else {
            format!("{source_name}.disabled")
        };
        mods::filename(&name)?;
        if !self.seen.insert(name.to_lowercase()) {
            return Err(error("选择中包含同名模组。"));
        }
        let destination = storage::within(&self.game, &format!("mods/{name}"))?;
        let hash = net::hash(path).await?;
        if let Some(update) = &update {
            if !source_name.to_ascii_lowercase().ends_with(".jar")
                || !hash.eq_ignore_ascii_case(&update.sha1)
            {
                return Err(error("已下载的模组校验失败，请重新更新。"));
            }
            let opposite = if enabled {
                format!("{name}.disabled")
            } else {
                source_name.to_string()
            };
            if storage::within(&self.game, &format!("mods/{opposite}"))?.exists() {
                return Err(error(
                    "目标模组已有另一种启用状态的同名文件，请先处理重复模组。",
                ));
            }
        }
        let same_name = old.is_some_and(|old| old.file_name.to_lowercase() == name.to_lowercase());
        if destination.exists() && !same_name {
            if old.is_some() {
                return Err(error("更新目标文件已存在，请先处理重复模组。"));
            }
            if net::hash(&destination).await? == hash {
                return Ok(());
            }
            return Err(error("同名模组已存在，导入不会覆盖原文件。"));
        }
        let temp = storage::within(&self.stage, &name)?;
        tokio::fs::copy(path, &temp).await?;
        if net::hash(&temp).await? != hash {
            return Err(error("导入期间源文件发生变化。"));
        }
        if let Some(old) = update.and_then(|update| update.old) {
            self.originals.push(old);
        }
        self.plans.push((temp, destination));
        Ok(())
    }
    async fn commit(mut self) -> Result<usize> {
        fs::create_dir_all(storage::within(&self.game, "mods")?)?;
        // Validate every path before moving any file so a late validation failure is atomic.
        let mut moves = Vec::new();
        if !self.originals.is_empty() {
            let backup = storage::within(&self.stage, "originals")?;
            fs::create_dir(&backup)?;
            let mut originals = BTreeSet::new();
            for old in &self.originals {
                let source = mods::checked(&self.game, old).await?;
                if !originals.insert(old.file_name.to_lowercase()) {
                    return Err(error("所选模组文件无效或重复。"));
                }
                let destination = storage::within(&backup, &old.file_name)?;
                moves.push((source, destination));
            }
        }
        for (source, destination) in &self.plans {
            storage::within(
                &self.game,
                &format!(
                    "mods/{}",
                    destination.file_name().unwrap().to_string_lossy()
                ),
            )?;
            storage::within(&self.stage, &source.file_name().unwrap().to_string_lossy())?;
        }
        let count = self.plans.len();
        moves.append(&mut self.plans);
        let mut done = Vec::new();
        for (source, destination) in &moves {
            if let Err(failure) = storage::move_new(source, destination) {
                for &(source, destination) in done.iter().rev() {
                    if storage::move_new(destination, source).is_err() {
                        self.preserve = true;
                    }
                }
                if self.preserve && !self.originals.is_empty() {
                    return Err(ApiError::new(
                        "MOD_UPDATE_ROLLBACK",
                        format!(
                            "无法完整恢复模组文件，备份保留在 {}。",
                            self.stage.display()
                        ),
                    ));
                }
                return Err(failure);
            }
            done.push((source, destination));
        }
        Ok(count)
    }
}
struct State {
    runtime: tokio::runtime::Runtime,
    blobs: BTreeMap<String, Vec<u8>>,
    imports: BTreeMap<String, Import>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            runtime: tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
            blobs: BTreeMap::new(),
            imports: BTreeMap::new(),
        }
    }
}
fn error(message: &str) -> ApiError {
    ApiError::new("MOD_OPERATION", message)
}
fn text<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    p[key].as_str().ok_or_else(|| error("缺少模组服务参数。"))
}
impl State {
    fn blob(&mut self, value: Value) -> Result<Value> {
        if self.blobs.len() >= 4 {
            return Err(error("请先关闭已有扫描结果。"));
        }
        let bytes = serde_json::to_vec(&value)?;
        if bytes.len() > 32 * 1024 * 1024 {
            return Err(error("模组列表超过大小限制。"));
        }
        let handle = uuid::Uuid::new_v4().to_string();
        let result = json!({"handle":handle,"length":bytes.len()});
        self.blobs.insert(handle, bytes);
        Ok(result)
    }
    fn call(&mut self, request: &Value) -> Result<Value> {
        let p = &request["params"];
        match request["method"].as_str().unwrap_or("") {
            "plugin.describe" => Ok(json!({"id":"minecraft.mods","abiVersion":1})),
            "inventory" => {
                let root = Path::new(text(p, "root")?);
                let game = Path::new(text(p, "game")?);
                let mut items = self.runtime.block_on(mods::inventory(root, game))?;
                let notice = mods::enrich(root, &mut items).err().map(|e| e.message);
                self.blob(json!({"items":items,"notice":notice}))
            }
            "readArchive" => {
                let path = Path::new(text(p, "path")?);
                if !path.is_absolute() || path.ancestors().any(storage::linked) {
                    return Err(error("模组路径无效。"));
                }
                self.blob(serde_json::to_value(mods::read_archive(
                    path,
                    text(p, "sha1")?,
                ))?)
            }
            "read" => {
                let bytes = self
                    .blobs
                    .get(text(p, "handle")?)
                    .ok_or_else(|| error("扫描结果已关闭。"))?;
                let offset = p["offset"].as_u64().ok_or_else(|| error("偏移无效。"))? as usize;
                if offset > bytes.len() {
                    return Err(error("偏移超出结果。"));
                }
                Ok(json!(STANDARD.encode(
                    &bytes[offset..(offset + 48 * 1024).min(bytes.len())]
                )))
            }
            "close" => {
                self.blobs.remove(text(p, "handle")?);
                Ok(Value::Null)
            }
            "enable" | "remove" => {
                let game = Path::new(text(p, "game")?);
                let files: Vec<mods::ModRef> = serde_json::from_value(p["files"].clone())?;
                if request["method"] == "enable" {
                    self.runtime.block_on(mods::set_enabled(
                        game,
                        &files,
                        p["enabled"]
                            .as_bool()
                            .ok_or_else(|| error("启停参数无效。"))?,
                    ))?;
                } else {
                    self.runtime.block_on(mods::remove(game, &files))?;
                }
                Ok(Value::Null)
            }
            "name" => {
                let file = serde_json::from_value(p["file"].clone())?;
                self.runtime.block_on(mods::name_mod(
                    Path::new(text(p, "root")?),
                    Path::new(text(p, "game")?),
                    &file,
                    text(p, "name")?,
                ))?;
                Ok(Value::Null)
            }
            "importBegin" => {
                if self.imports.len() >= 4 {
                    return Err(error("导入事务过多。"));
                }
                let import = Import::new(Path::new(text(p, "game")?))?;
                let handle = uuid::Uuid::new_v4().to_string();
                self.imports.insert(handle.clone(), import);
                Ok(json!(handle))
            }
            "importAdd" => {
                let handle = text(p, "handle")?;
                let update = serde_json::from_value(p["update"].clone())?;
                let result = self.runtime.block_on(
                    self.imports
                        .get_mut(handle)
                        .ok_or_else(|| error("导入事务已结束。"))?
                        .add(Path::new(text(p, "path")?), update),
                );
                if result.is_err() {
                    self.imports.remove(handle);
                }
                result?;
                Ok(Value::Null)
            }
            "importCommit" => {
                let import = self
                    .imports
                    .remove(text(p, "handle")?)
                    .ok_or_else(|| error("导入事务已结束。"))?;
                Ok(json!(self.runtime.block_on(import.commit())?))
            }
            "importAbort" => {
                self.imports.remove(text(p, "handle")?);
                Ok(Value::Null)
            }
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知模组服务方法。")),
        }
    }
}
fn dispatch(state: &mut State, request: &mut Value) -> hydrogen_plugin_sdk::Result {
    state.call(request).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);
