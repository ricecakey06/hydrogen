use hydrogen_plugin_services::{
    contracts::{ApiError, Result},
    storage, text,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io::Read, path::Path};

// Leave room in the native response for the list of installed packs.
const PACK_LIMIT: usize = 160 * 1024;
const OPTIONS_LIMIT: usize = 4 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Bundle {
    locale: String,
    name: String,
    minecraft_language: String,
    messages: BTreeMap<String, String>,
}
fn valid_locale(locale: &str) -> bool {
    !locale.is_empty()
        && locale.len() <= 35
        && locale
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
}
fn bundle(root: &Path, locale: &str) -> Result<Bundle> {
    if !valid_locale(locale) {
        return Err(ApiError::new("INVALID_LOCALE", "语言标识无效。"));
    }
    let mut pack: Bundle = match locale {
        "zh-CN" => serde_json::from_str(include_str!("../assets/locales/zh-CN.json"))?,
        "en-US" => serde_json::from_str(include_str!("../assets/locales/en-US.json"))?,
        _ => {
            let path = storage::within(root, &format!("language-packs/{locale}.json"))?;
            if !path.is_file() {
                return Err(ApiError::new("UNKNOWN_LOCALE", "此语言包尚未安装。"));
            }
            if path.metadata()?.len() > PACK_LIMIT as u64 {
                return Err(ApiError::new("LANGUAGE_PACK_LIMIT", "语言包超过大小限制。"));
            }
            storage::read_json(&path)?
        }
    };
    if pack.locale != locale
        || pack.name.trim().is_empty()
        || pack.name.len() > 100
        || !regex::Regex::new(r"^[a-z]{2,3}_[a-z]{2,3}$")
            .unwrap()
            .is_match(&pack.minecraft_language)
    {
        return Err(ApiError::new("INVALID_LANGUAGE_PACK", "语言包格式无效。"));
    }
    if locale != "zh-CN" && locale != "en-US" {
        let base: Bundle = serde_json::from_str(include_str!("../assets/locales/en-US.json"))?;
        for (key, value) in base.messages {
            pack.messages.entry(key).or_insert(value);
        }
        if serde_json::to_vec(&pack)?.len() > PACK_LIMIT {
            return Err(ApiError::new("LANGUAGE_PACK_LIMIT", "语言包超过大小限制。"));
        }
    }
    Ok(pack)
}
fn languages(root: &Path) -> Result<Value> {
    let built_in = ["zh-CN", "en-US"];
    let mut languages = built_in
        .iter()
        .map(|locale| {
            let pack = bundle(root, locale)?;
            Ok(json!({"locale":pack.locale,"name":pack.name}))
        })
        .collect::<Result<Vec<_>>>()?;
    let directory = storage::within(root, "language-packs")?;
    if directory.is_dir() {
        for entry in fs::read_dir(directory)?.take(100) {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_file()
                && path.extension().is_some_and(|e| e == "json")
                && let Some(locale) = path.file_stem().and_then(|s| s.to_str())
                && !built_in.contains(&locale)
                && let Ok(pack) = bundle(root, locale)
            {
                languages.push(json!({"locale":pack.locale,"name":pack.name}));
            }
        }
    }
    Ok(json!(languages))
}
fn game_language(pack: &Bundle, version: &str) -> String {
    let legacy = if let Some(c) = regex::Regex::new(r"^1\.(\d+)(?:\D|$)")
        .unwrap()
        .captures(version)
    {
        c[1].parse::<u32>().is_ok_and(|minor| minor < 11)
    } else if let Some(c) = regex::Regex::new(r"^(\d{2})w(\d{2})")
        .unwrap()
        .captures(version)
    {
        (
            c[1].parse::<u32>().unwrap_or(0),
            c[2].parse::<u32>().unwrap_or(0),
        ) < (16, 32)
    } else {
        false
    };
    if legacy {
        let (language, country) = pack.minecraft_language.split_once('_').unwrap();
        format!("{language}_{}", country.to_ascii_uppercase())
    } else {
        pack.minecraft_language.clone()
    }
}
fn apply(root: &Path, directory: &Path, locale: &str, version: &str) -> Result<Value> {
    if !directory.is_absolute() {
        return Err(ApiError::new("INVALID_PATH", "游戏目录必须是完整路径。"));
    }
    let pack = bundle(root, locale).or_else(|_| bundle(root, "zh-CN"))?;
    let language = game_language(&pack, version);
    let options = storage::within(directory, "options.txt")?;
    let marker = storage::within(directory, ".hydrogen/default-language.json")?;
    let mut bytes = Vec::new();
    match fs::File::open(&options) {
        Ok(file) => {
            file.take(OPTIONS_LIMIT as u64 + 1)
                .read_to_end(&mut bytes)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if bytes.len() > OPTIONS_LIMIT {
        return Err(ApiError::new("OPTIONS_LIMIT", "游戏设置文件超过大小限制。"));
    }
    let current = bytes
        .split(|b| *b == b'\n')
        .filter_map(|line| line.strip_prefix(b"lang:"))
        .next_back()
        .map(|s| String::from_utf8_lossy(s).trim().to_owned());
    let previous = storage::read_json::<Value>(&marker)
        .ok()
        .and_then(|v| v["language"].as_str().map(String::from));
    if let Some(current) = &current
        && !current.is_empty()
        && previous.as_ref() != Some(current)
    {
        return Ok(json!({"changed":false,"reason":"player","language":current}));
    }
    if current.as_deref() == Some(&language) {
        return Ok(json!({"changed":false,"reason":"unchanged","language":language}));
    }
    let newline: &[u8] = if bytes.windows(2).any(|b| b == b"\r\n") {
        b"\r\n"
    } else {
        b"\n"
    };
    let mut updated = Vec::new();
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        if !line.starts_with(b"lang:") {
            updated.extend_from_slice(line);
        }
    }
    if !updated.is_empty() && !updated.ends_with(b"\n") {
        updated.extend_from_slice(newline);
    }
    updated.extend_from_slice(format!("lang:{language}").as_bytes());
    updated.extend_from_slice(newline);
    storage::atomic_write(&options, &updated)?;
    storage::write_json(&marker, &json!({"language":language}))?;
    Ok(json!({"changed":true,"reason":"default","language":language}))
}
#[derive(Default)]
struct State;
fn dispatch(_: &mut State, r: &mut Value) -> hydrogen_plugin_sdk::Result {
    let call = || -> Result<Value> {
        let p = &r["params"];
        if r["method"] == "plugin.describe" {
            return Ok(json!({"id":"i18n","abiVersion":1}));
        }
        let root = Path::new(text(p, "root")?);
        match r["method"].as_str().unwrap_or("") {
            "bundle" => {
                Ok(json!({"bundle":bundle(root,text(p,"locale")?)?,"languages":languages(root)?}))
            }
            "languages" => languages(root),
            "applyGameLanguage" => apply(
                root,
                Path::new(text(p, "directory")?),
                text(p, "locale")?,
                text(p, "version")?,
            ),
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知多语言服务方法。")),
        }
    };
    call().map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_language_follows_launcher_and_preserves_player_choices_and_other_options() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let game = root.join("game");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("options.txt"), b"music:0.5\r\ncustom:\xff\r\n").unwrap();
        apply(root, &game, "zh-CN", "1.20.1").unwrap();
        assert_eq!(
            fs::read(game.join("options.txt")).unwrap(),
            b"music:0.5\r\ncustom:\xff\r\nlang:zh_cn\r\n"
        );
        apply(root, &game, "en-US", "1.20.1").unwrap();
        assert!(
            fs::read(game.join("options.txt"))
                .unwrap()
                .ends_with(b"lang:en_us\r\n")
        );
        fs::write(game.join("options.txt"), b"lang:ja_jp\nmusic:0.7\n").unwrap();
        assert_eq!(
            apply(root, &game, "zh-CN", "1.20.1").unwrap()["reason"],
            "player"
        );
        assert_eq!(
            fs::read(game.join("options.txt")).unwrap(),
            b"lang:ja_jp\nmusic:0.7\n"
        );
        let old = root.join("old");
        apply(root, &old, "zh-CN", "1.7.10").unwrap();
        assert_eq!(fs::read(old.join("options.txt")).unwrap(), b"lang:zh_CN\n");
        assert!(bundle(root, "../en-US").is_err());
        let chinese = bundle(root, "zh-CN").unwrap();
        let english = bundle(root, "en-US").unwrap();
        assert_eq!(
            chinese.messages.keys().collect::<Vec<_>>(),
            english.messages.keys().collect::<Vec<_>>()
        );
    }
}
