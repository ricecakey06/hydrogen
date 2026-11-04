mod transfer;
use hydrogen_plugin_services::{
    PluginHost, Settings,
    contracts::{ApiError, Result},
    http::Client,
    jobs::{Job, Jobs},
    text,
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
            "plugin.describe" => Ok(json!({"id":"minecraft.download","abiVersion":1})),
            "plugin.stop" => {
                self.jobs.clear();
                Ok(Value::Null)
            }
            "poll" => self.jobs.poll(text(p, "handle")?),
            "cancel" => {
                self.jobs.cancel(text(p, "handle")?);
                Ok(Value::Null)
            }
            "sourceUrl" => Ok(json!(transfer::source_url(
                text(p, "url")?,
                p["mirror"] == true
            )?)),
            "trusted" => Ok(json!(
                reqwest::Url::parse(text(p, "url")?).is_ok_and(|u| transfer::trusted(&u))
            )),
            "transfer" | "once" => {
                let root = PathBuf::from(text(p, "root")?);
                let relative = text(p, "relative")?.to_owned();
                let url = text(p, "url")?.to_owned();
                let hash = p["hash"].as_str().map(String::from);
                let size = p["size"].as_u64();
                let mirror = p["mirror"] == true;
                let client = Client::new(&PluginHost, &Settings::default())?;
                let job = Arc::new(Job::default());
                let progress = job.clone();
                self.jobs.begin(progress, async move {
                    let path = if method == "once" {
                        transfer::ensure_once(
                            &client,
                            &root,
                            &relative,
                            &url,
                            hash.as_deref().unwrap_or(""),
                            size,
                            &job,
                        )
                        .await?
                    } else {
                        transfer::ensure_game_file(
                            &client,
                            &root,
                            &relative,
                            &url,
                            hash.as_deref(),
                            size,
                            mirror,
                            &job,
                        )
                        .await?
                    };
                    Ok(json!(path))
                })
            }
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知下载方法。")),
        }
    }
}
fn dispatch(state: &mut State, request: &mut Value) -> hydrogen_plugin_sdk::Result {
    state.call(request).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);
