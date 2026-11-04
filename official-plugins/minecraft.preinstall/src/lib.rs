mod prepare;
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
            "plugin.describe" => Ok(json!({"id":"minecraft.preinstall","abiVersion":1})),
            "plugin.stop" => {
                self.jobs.clear();
                Ok(Value::Null)
            }
            "poll" => self.jobs.poll(text(p, "handle")?),
            "cancel" => {
                self.jobs.cancel(text(p, "handle")?);
                Ok(Value::Null)
            }
            "prepare" => {
                let settings: Settings = serde_json::from_value(p["settings"].clone())?;
                let root = PathBuf::from(text(p, "gameRoot")?);
                let metadata = p["metadata"].clone();
                let architecture = text(p, "architecture")?.to_owned();
                let client = Client::new(&PluginHost, &settings)?;
                let job = Arc::new(Job::default());
                self.jobs.begin(job.clone(), async move {
                    Ok(serde_json::to_value(
                        prepare::prepare(
                            &PluginHost,
                            &client,
                            &root,
                            &settings,
                            metadata,
                            &architecture,
                            job,
                        )
                        .await?,
                    )?)
                })
            }
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知预安装方法。")),
        }
    }
}
fn dispatch(state: &mut State, request: &mut Value) -> hydrogen_plugin_sdk::Result {
    state.call(request).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);

#[cfg(test)]
mod scheduler_tests {
    use super::*;
    use futures_util::{StreamExt, stream::FuturesUnordered};
    use std::time::Duration;

    #[test]
    fn timed_out_poll_preserves_deferred_child_wakeups() {
        let mut jobs = Jobs::default();
        let id = jobs
            .begin(Arc::new(Job::default()), async {
                let mut children = FuturesUnordered::new();
                children.push(async {
                    // Let the slice timer become ready, then exhaust the cooperative
                    // budget. The child waker must survive the timeout return.
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    for _ in 0..256 {
                        tokio::task::consume_budget().await;
                    }
                    42
                });
                Ok(json!(children.next().await.unwrap()))
            })
            .unwrap();
        for _ in 0..20 {
            let result = jobs.poll(id.as_str().unwrap()).unwrap();
            if result["ready"] == true {
                assert_eq!(result["result"], 42);
                return;
            }
        }
        panic!("child wakeup was lost when the poll timed out");
    }
}
