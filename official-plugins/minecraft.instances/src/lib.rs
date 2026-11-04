mod migration;
mod migration_model;
mod records;
mod repository;
use hydrogen_plugin_services::{
    contracts::{ApiError, Result},
    text,
};
use serde_json::{Value, json};
use std::path::Path;
#[derive(Default)]
struct State {
    jobs: hydrogen_plugin_services::jobs::Jobs,
}
impl State {
    fn call(&mut self, r: &mut Value) -> Result<Value> {
        let method = r["method"].as_str().unwrap_or("");
        let p = &r["params"];
        if method == "plugin.describe" {
            return Ok(json!({"id":"minecraft.instances","abiVersion":1}));
        }
        match method {
            "plugin.stop" => {
                self.jobs.clear();
                return Ok(Value::Null);
            }
            "poll" => return self.jobs.poll(text(p, "handle")?),
            "cancel" => {
                self.jobs.cancel(text(p, "handle")?);
                return Ok(Value::Null);
            }
            "migration.preview" => {
                let client = hydrogen_plugin_services::http::Client::new(
                    &hydrogen_plugin_services::PluginHost,
                    &hydrogen_plugin_services::Settings::default(),
                )?;
                return Ok(serde_json::to_value(migration::preview(
                    &client,
                    Path::new(text(p, "root")?),
                    Path::new(text(p, "source")?),
                )?)?);
            }
            "migration.execute" => {
                let mut store: records::Store = serde_json::from_value(p["state"].clone())?;
                let plan = serde_json::from_value(p["plan"].clone())?;
                let job = std::sync::Arc::new(hydrogen_plugin_services::jobs::Job::default());
                return self.jobs.begin(job.clone(), async move {
                    migration::execute(&mut store, plan, &job).await?;
                    Ok(serde_json::to_value(store)?)
                });
            }
            _ => {}
        }
        if method.starts_with("records.") {
            let mut store: records::Store = serde_json::from_value(p["state"].clone())?;
            return match method {
                "records.delete" => {
                    store.delete(text(p, "id")?, text(p, "confirmation")?)?;
                    Ok(serde_json::to_value(store)?)
                }
                "records.list" => Ok(serde_json::to_value(store.instances_with_issues()?)?),
                "records.get" => Ok(serde_json::to_value(store.instance(text(p, "id")?)?)?),
                "records.write" => {
                    store.write_instance(&serde_json::from_value(p["instance"].clone())?)?;
                    Ok(Value::Null)
                }
                "records.create" => Ok(serde_json::to_value(store.create(
                    text(p, "version")?,
                    text(p, "name")?,
                    serde_json::from_value(p["components"].clone())?,
                )?)?),
                "records.gameDir" => Ok(serde_json::to_value(store.game_dir(text(p, "id")?)?)?),
                "records.versionDir" => {
                    Ok(serde_json::to_value(store.version_dir(text(p, "id")?)?)?)
                }
                "records.select" => {
                    store.select(text(p, "id")?)?;
                    Ok(serde_json::to_value(store.selected)?)
                }
                "records.rename" => {
                    store.rename(text(p, "id")?, text(p, "name")?)?;
                    Ok(Value::Null)
                }
                _ => Err(ApiError::new("UNKNOWN_METHOD", "未知实例记录方法。")),
            };
        }
        let root = Path::new(text(p, "root")?);
        match method {
            "instanceId" => Ok(json!(repository::instance_id(root, text(p, "name")?))),
            "load" => Ok(serde_json::to_value(repository::load(
                root,
                text(p, "name")?,
            )?)?),
            "scan" => Ok(serde_json::to_value(repository::scan(root)?)?),
            "manifestFile" => Ok(serde_json::to_value(repository::manifest_file(
                root,
                text(p, "name")?,
            )?)?),
            "runDirectory" => Ok(serde_json::to_value(repository::run_directory(
                root,
                text(p, "name")?,
                p["isolate"].as_bool(),
            )?)?),
            "nativeDirectory" => Ok(serde_json::to_value(repository::native_directory(
                root,
                text(p, "name")?,
                text(p, "architecture")?,
            )?)?),
            "publish" => Ok(serde_json::to_value(repository::publish(
                root,
                text(p, "name")?,
                text(p, "gameVersion")?,
                &p["base"],
                &p["complete"],
                p["loader"].as_str(),
            )?)?),
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知实例方法。")),
        }
    }
}
fn dispatch(s: &mut State, r: &mut Value) -> hydrogen_plugin_sdk::Result {
    s.call(r).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);
