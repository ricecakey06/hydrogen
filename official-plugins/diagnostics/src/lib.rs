use hydrogen_plugin_sdk::{
    Result,
    serde_json::{Value, json},
};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
};

struct Log {
    file: File,
    bytes: usize,
    secrets: Vec<String>,
}
impl Drop for Log {
    fn drop(&mut self) {
        for secret in &mut self.secrets {
            unsafe {
                secret.as_bytes_mut().fill(0);
            }
        }
    }
}
#[derive(Default)]
struct Diagnostics {
    logs: BTreeMap<String, Log>,
}

fn dispatch(state: &mut Diagnostics, request: &mut Value) -> Result {
    let params = &request["params"];
    match request["method"].as_str().unwrap_or("") {
        "plugin.describe" => Ok(json!({"id":"diagnostics","abiVersion":1})),
        "create" => {
            if state.logs.len() >= 32 {
                return Err(("LOG_BUDGET".into(), "同时打开的游戏日志超过限制。".into()));
            }
            let path = Path::new(
                params["path"]
                    .as_str()
                    .ok_or(("INVALID_PATH".into(), "缺少日志路径。".into()))?,
            );
            if !path.is_absolute() || path.ancestors().any(linked) {
                return Err((
                    "INVALID_PATH".into(),
                    "日志路径必须为不含链接的绝对路径。".into(),
                ));
            }
            let secrets: Vec<String> =
                hydrogen_plugin_sdk::serde_json::from_value(params["secrets"].clone())
                    .map_err(|_| ("INVALID_PARAMS".into(), "脱敏参数无效。".into()))?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(io_error)?;
            }
            let file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path)
                .map_err(io_error)?;
            let handle = uuid::Uuid::new_v4().simple().to_string();
            state.logs.insert(
                handle.clone(),
                Log {
                    file,
                    bytes: 0,
                    secrets,
                },
            );
            Ok(json!({"handle":handle}))
        }
        "append" => {
            let handle = params["handle"].as_str().unwrap_or("");
            let log = state
                .logs
                .get_mut(handle)
                .ok_or(("LOG_CLOSED".into(), "日志会话已关闭。".into()))?;
            let lines = params["lines"]
                .as_array()
                .ok_or(("INVALID_PARAMS".into(), "日志内容无效。".into()))?;
            for line in lines {
                if log.bytes >= 32 * 1024 * 1024 {
                    break;
                }
                let mut line = line
                    .as_str()
                    .ok_or(("INVALID_PARAMS".into(), "日志行必须为文本。".into()))?
                    .to_owned();
                for secret in &log.secrets {
                    if secret.len() > 6 {
                        line = line.replace(secret, "[redacted]");
                    }
                }
                line.push('\n');
                log.file.write_all(line.as_bytes()).map_err(io_error)?;
                log.bytes += line.len();
            }
            Ok(Value::Null)
        }
        "close" => {
            state.logs.remove(params["handle"].as_str().unwrap_or(""));
            Ok(Value::Null)
        }
        _ => Err(("UNKNOWN_METHOD".into(), "Unknown diagnostics method".into())),
    }
}
fn io_error(_: std::io::Error) -> (String, String) {
    ("LOG_IO".into(), "日志文件操作失败。".into())
}
fn linked(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}
hydrogen_plugin_sdk::export_plugin!(Diagnostics, dispatch);
