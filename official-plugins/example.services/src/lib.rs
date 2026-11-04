use hydrogen_plugin_sdk::{
    Result,
    serde_json::{Value, json},
};
fn dispatch(_: &mut (), request: &mut Value) -> Result {
    if request["method"] == "plugin.describe" {
        return Ok(json!({"id":"example.services","abiVersion":1}));
    }
    let p = &mut request["params"];
    let provider = p["provider"]
        .as_str()
        .unwrap_or("account.offline")
        .to_owned();
    let service = p["service"].as_str().unwrap_or(&provider).to_owned();
    let method = p["method"].as_str().unwrap_or("session").to_owned();
    hydrogen_plugin_sdk::call_service(&provider, &service, &method, p["params"].take())
        .map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!((), dispatch);
