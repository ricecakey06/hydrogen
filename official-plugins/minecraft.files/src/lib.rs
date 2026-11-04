use hydrogen_plugin_services::{
    contracts::{ApiError, Result},
    jobs::{Job, Jobs, service_task},
    storage, text,
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
struct State {
    jobs: Jobs,
}
impl Default for State {
    fn default() -> Self {
        Self {
            jobs: Jobs::with_limit(64),
        }
    }
}
impl State {
    fn call(&mut self, request: &mut Value) -> Result<Value> {
        let method = request["method"].as_str().unwrap_or("").to_owned();
        let p = &request["params"];
        match method.as_str() {
            "plugin.describe" => Ok(json!({"id":"minecraft.files","abiVersion":1})),
            "plugin.stop" => {
                self.jobs.clear();
                Ok(Value::Null)
            }
            "poll" => self.jobs.poll(text(p, "handle")?),
            "cancel" => {
                self.jobs.cancel(text(p, "handle")?);
                Ok(Value::Null)
            }
            "hash" => {
                let path = PathBuf::from(text(p, "path")?);
                if !path.is_absolute() || path.ancestors().any(storage::linked) {
                    return Err(ApiError::new("INVALID_PATH", "文件路径无效。"));
                }
                self.jobs.begin(Arc::new(Job::default()), async move {
                    Ok(json!(storage::hash(&path).await?))
                })
            }
            "ensure" => {
                let root = PathBuf::from(text(p, "root")?);
                let relative = text(p, "relative")?.to_owned();
                let hash = p["hash"].as_str().map(String::from);
                let size = p["size"].as_u64();
                let enabled = p["enabled"] == true;
                let params = p.clone();
                if hash.as_ref().is_some_and(|h| !storage::valid_hash(h)) {
                    return Err(ApiError::new("INVALID_HASH", "文件的 SHA-1 声明无效。"));
                }
                let destination = storage::within(&root, &relative)?;
                let job = Arc::new(Job::default());
                let progress = job.clone();
                self.jobs.begin(progress, async move {
                    job.file_started(&relative);
                    if destination.is_file()
                        && size.is_none_or(|n| destination.metadata().is_ok_and(|m| m.len() == n))
                        && (hash.is_none()
                            || storage::hash(&destination)
                                .await?
                                .eq_ignore_ascii_case(hash.as_ref().unwrap()))
                    {
                        job.file_progress(&relative, destination.metadata()?.len(), true);
                        return Ok(json!(destination));
                    }
                    if !enabled {
                        return Err(ApiError::new(
                            "DOWNLOADS_DISABLED",
                            "游戏下载已停用，缺少已校验的本地缓存。",
                        ));
                    }
                    service_task("minecraft.download", "transfer", params, &job).await
                })
            }
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知缓存方法。")),
        }
    }
}
fn dispatch(state: &mut State, request: &mut Value) -> hydrogen_plugin_sdk::Result {
    state.call(request).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);
