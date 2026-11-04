use hydrogen_plugin_sdk::{
    Result,
    serde_json::{Value, json},
};
#[derive(Default)]
struct Counter(u32);
fn dispatch(counter: &mut Counter, request: &mut Value) -> Result {
    match request["method"].as_str().unwrap_or("") {
        "plugin.describe" => Ok(json!({"id":"example.counter","abiVersion":1})),
        "echo" => Ok(request["params"].take()),
        "increment" => {
            counter.0 += if cfg!(feature = "step-two") { 2 } else { 1 };
            Ok(json!(counter.0))
        }
        _ => Err(("UNKNOWN_METHOD".into(), "Unknown counter method".into())),
    }
}
hydrogen_plugin_sdk::export_plugin!(Counter, dispatch);
