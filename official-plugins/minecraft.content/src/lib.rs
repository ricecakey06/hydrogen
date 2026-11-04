mod content;
use hydrogen_plugin_services::{
    PluginHost, Settings,
    contracts::{ApiError, Result},
    http::Client,
    invoke,
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
            jobs: Jobs::with_limit(16),
        }
    }
}
fn os_version() -> Result<String> {
    Ok(serde_json::from_value(invoke(
        "platform.windows.environment",
        "platform.windows.environment",
        "osVersion",
        Value::Null,
    )?)?)
}
impl State {
    fn call(&mut self, request: &mut Value) -> Result<Value> {
        let method = request["method"].as_str().unwrap_or("").to_owned();
        let p = &request["params"];
        match method.as_str() {
            "plugin.describe" => Ok(json!({"id":"minecraft.content","abiVersion":1})),
            "plugin.stop" => {
                self.jobs.clear();
                Ok(Value::Null)
            }
            "poll" => self.jobs.poll(text(p, "handle")?),
            "cancel" => {
                self.jobs.cancel(text(p, "handle")?);
                Ok(Value::Null)
            }
            "supported" => Ok(json!(content::supported(text(p, "id")?))),
            "allows" => Ok(json!(content::allows(
                &p["owner"],
                text(p, "architecture")?,
                p["customResolution"] == true
            )?)),
            "versions" | "resolve" => {
                let settings: Settings = serde_json::from_value(p["settings"].clone())?;
                let root = PathBuf::from(text(p, "root")?);
                let game = PathBuf::from(p["game"].as_str().unwrap_or(""));
                let id = p["id"].as_str().unwrap_or("").to_owned();
                let client = Client::new(&PluginHost, &settings)?;
                let job = Arc::new(Job::default());
                let progress = job.clone();
                self.jobs.begin(progress, async move {
                    if method == "versions" {
                        Ok(serde_json::to_value(
                            content::versions(&client, &root, &settings).await?,
                        )?)
                    } else {
                        content::resolve(&client, &root, &game, &settings, &id, &job).await
                    }
                })
            }
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知内容服务方法。")),
        }
    }
}
fn dispatch(state: &mut State, request: &mut Value) -> hydrogen_plugin_sdk::Result {
    state.call(request).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);
