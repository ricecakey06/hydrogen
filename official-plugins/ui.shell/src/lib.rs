use serde_json::{Value, json};
#[derive(Default)]
struct State;
fn dispatch(_: &mut State, r: &mut Value) -> hydrogen_plugin_sdk::Result {
    match r["method"].as_str() {
        Some("plugin.describe") => Ok(json!({"id":"ui.shell","abiVersion":1})),
        _ => Err(("UNKNOWN_METHOD".into(), "Unknown frontend method".into())),
    }
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);
