mod forge;
use hydrogen_plugin_services::{
    PluginHost, Settings,
    contracts::{ApiError, Result},
    http::Client,
    jobs::{Job, Jobs},
    text,
};
use serde_json::{Value, json};
use std::sync::Arc;

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
    fn call(&mut self, r: &Value) -> Result<Value> {
        let p = &r["params"];
        match r["method"].as_str().unwrap_or("") {
            "plugin.describe" => Ok(json!({"id":"minecraft.forge","abiVersion":1})),
            "plugin.stop" => {
                self.jobs.clear();
                Ok(Value::Null)
            }
            "poll" => self.jobs.poll(text(p, "handle")?),
            "cancel" => {
                self.jobs.cancel(text(p, "handle")?);
                Ok(Value::Null)
            }
            "options" | "prepare" => {
                let method = text(r, "method")?.to_owned();
                let p = p.clone();
                let settings: Settings = if p["settings"].is_object() {
                    serde_json::from_value(p["settings"].clone())?
                } else {
                    Settings::default()
                };
                let client = Client::new(&PluginHost, &settings)?;
                let job = Arc::new(Job::default());
                self.jobs.begin(job.clone(), async move {
                    if method == "options" {
                        forge::options(&client, &settings, text(&p, "version")?, text(&p, "kind")?)
                            .await
                    } else {
                        forge::prepare(&client, &settings, &p, &job).await
                    }
                })
            }
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知 Forge 安装方法。")),
        }
    }
}
fn dispatch(s: &mut State, r: &mut Value) -> hydrogen_plugin_sdk::Result {
    s.call(r).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);
