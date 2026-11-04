mod fabric;
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
            jobs: Jobs::with_limit(16),
        }
    }
}
impl State {
    fn call(&mut self, request: &mut Value) -> Result<Value> {
        let method = request["method"].as_str().unwrap_or("").to_owned();
        let p = &request["params"];
        match method.as_str() {
            "plugin.describe" => Ok(json!({"id":"minecraft.fabric","abiVersion":1})),
            "plugin.stop" => {
                self.jobs.clear();
                Ok(Value::Null)
            }
            "poll" => self.jobs.poll(text(p, "handle")?),
            "cancel" => {
                self.jobs.cancel(text(p, "handle")?);
                Ok(Value::Null)
            }
            "options" => {
                let version = text(p, "version")?.to_owned();
                let client = Client::new(&PluginHost, &Settings::default())?;
                self.jobs.begin(Arc::new(Job::default()), async move {
                    fabric::fabric_options(&client, &version).await
                })
            }
            "prepare" => {
                let settings: Settings = serde_json::from_value(p["settings"].clone())?;
                let root = PathBuf::from(text(p, "root")?);
                let game_root = PathBuf::from(text(p, "gameRoot")?);
                let game = PathBuf::from(text(p, "game")?);
                let version = text(p, "version")?.to_owned();
                let components = serde_json::from_value(p["components"].clone())?;
                let mut prepared = serde_json::from_value(p["prepared"].clone())?;
                let installing = p["installing"] == true;
                let client = Client::new(&PluginHost, &settings)?;
                let job = Arc::new(Job::default());
                let progress = job.clone();
                self.jobs.begin(progress, async move {
                    fabric::fabric(
                        &client,
                        &root,
                        &game_root,
                        &game,
                        &settings,
                        &version,
                        &components,
                        &mut prepared,
                        &job,
                        installing,
                    )
                    .await?;
                    Ok(serde_json::to_value(prepared)?)
                })
            }
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知 Fabric 方法。")),
        }
    }
}
fn dispatch(state: &mut State, request: &mut Value) -> hydrogen_plugin_sdk::Result {
    state.call(request).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);
