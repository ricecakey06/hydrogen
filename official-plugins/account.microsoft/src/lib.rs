mod account;
mod credentials;
mod environment;
mod net;
use hydrogen_plugin_services::{
    PluginHost, Settings,
    contracts::{ApiError, Result},
    jobs::{Job, Jobs},
};
pub use hydrogen_plugin_services::{contracts, text};
mod storage {
    pub use hydrogen_plugin_services::{Settings, storage::*};
}
mod tasks {
    pub use hydrogen_plugin_services::jobs::Job;
}
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
struct State {
    accounts: BTreeMap<PathBuf, Arc<account::Accounts>>,
    jobs: Jobs,
}
impl Default for State {
    fn default() -> Self {
        Self {
            accounts: BTreeMap::new(),
            jobs: Jobs::default(),
        }
    }
}
impl State {
    fn call(&mut self, request: &mut Value) -> Result<Value> {
        let method = request["method"].as_str().unwrap_or("").to_owned();
        let p = &mut request["params"];
        let settings: Settings = serde_json::from_value(p["settings"].clone()).unwrap_or_default();
        let root = PathBuf::from(p["root"].as_str().unwrap_or(""));
        if method == "reset" || method == "plugin.stop" {
            self.jobs.clear();
            self.accounts.clear();
            return Ok(Value::Null);
        }
        if method == "forget" {
            self.accounts.remove(&root);
            return Ok(Value::Null);
        }
        let account = self
            .accounts
            .entry(root.clone())
            .or_insert_with(|| Arc::new(account::Accounts::new(PluginHost)))
            .clone();
        match method.as_str() {
            "plugin.describe" => Ok(json!({"id":"account.microsoft","abiVersion":1})),
            "view" => Ok(serde_json::to_value(account.view(
                &root,
                &settings,
                p["signingIn"] == true,
            ))?),
            "signOut" => {
                account.sign_out(&root, &settings)?;
                Ok(Value::Null)
            }
            "failure" => {
                account.failure(p["message"].as_str().unwrap_or(""));
                Ok(Value::Null)
            }
            "callback" => Ok(json!(account::callback(
                text(p, "request")?,
                text(p, "state")?,
                p["port"]
                    .as_u64()
                    .ok_or_else(|| ApiError::new("INVALID_PARAMS", "缺少端口。"))?
                    as u16
            )?)),
            "poll" => self.jobs.poll(text(p, "handle")?),
            "cancel" => {
                self.jobs.cancel(text(p, "handle")?);
                Ok(Value::Null)
            }
            "signIn" | "session" => {
                let client = net::Client::new(&PluginHost, &settings)?;
                let job = Arc::new(Job::default());
                let progress = job.clone();
                let restore = p["restore"] == true;
                let session = method == "session";
                self.jobs.begin(progress, async move {
                    if session {
                        Ok(serde_json::to_value(
                            account.session(&client, &root, &settings).await?,
                        )?)
                    } else {
                        account
                            .sign_in(&client, &root, &settings, &job, restore)
                            .await?;
                        Ok(Value::Null)
                    }
                })
            }
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知账号方法。")),
        }
    }
}
fn dispatch(state: &mut State, request: &mut Value) -> hydrogen_plugin_sdk::Result {
    state.call(request).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);
