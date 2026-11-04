mod settings;
mod store;
use hydrogen_plugin_services::{
    contracts::{ApiError, Result},
    text,
};
use serde_json::{Value, json};
#[derive(Default)]
struct State;
impl State {
    fn call(&mut self, r: &mut Value) -> Result<Value> {
        let p = &r["params"];
        match r["method"].as_str().unwrap_or("") {
            "plugin.describe" => Ok(json!({"id":"storage","abiVersion":1})),
            "open" => Ok(serde_json::to_value(store::Store::open(
                std::path::PathBuf::from(text(p, "root")?),
                std::path::PathBuf::from(text(p, "gameRoot")?),
            )?)?),
            "save" => {
                let mut state: store::Store = serde_json::from_value(p["state"].clone())?;
                state.save(serde_json::from_value(p["settings"].clone())?)?;
                Ok(serde_json::to_value(state)?)
            }
            "writable" => {
                let state: store::Store = serde_json::from_value(p["state"].clone())?;
                state.writable()?;
                Ok(Value::Null)
            }
            "defaults" => Ok(serde_json::to_value(settings::Settings::default())?),
            "memory" => Ok(serde_json::to_value(settings::memory_advice())?),
            "jvmArguments" => Ok(json!(settings::jvm_arguments(text(p, "text")?)?)),
            "validate" => {
                let mut settings: settings::Settings =
                    serde_json::from_value(p["settings"].clone())?;
                settings.sync_account_mode();
                settings.validate()?;
                Ok(serde_json::to_value(settings)?)
            }
            "effective" => {
                let settings: settings::Settings = serde_json::from_value(p["settings"].clone())?;
                Ok(serde_json::to_value(settings.effective(
                    &serde_json::from_value(p["overrides"].clone())?,
                )?)?)
            }
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知配置方法。")),
        }
    }
}
fn dispatch(s: &mut State, r: &mut Value) -> hydrogen_plugin_sdk::Result {
    s.call(r).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);
