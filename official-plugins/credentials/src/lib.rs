mod platform;
use base64::{Engine, engine::general_purpose::STANDARD};
use hydrogen_plugin_sdk::{
    Result,
    serde_json::{Value, json},
};

fn dispatch(_: &mut (), request: &mut Value) -> Result {
    let method = request["method"].as_str().unwrap_or("");
    if method == "plugin.describe" {
        return Ok(json!({"id":"credentials","abiVersion":1}));
    }
    if !matches!(method, "protect" | "unprotect") {
        return Err(("UNKNOWN_METHOD".into(), "Unknown credential method".into()));
    }
    let mut bytes = STANDARD
        .decode(
            request["params"]["data"]
                .as_str()
                .ok_or(("INVALID_PARAMS".into(), "缺少凭据内容。".into()))?,
        )
        .map_err(|_| ("INVALID_PARAMS".into(), "凭据编码无效。".into()))?;
    let result = platform::credentials(&mut bytes, method == "protect");
    bytes.fill(0);
    let mut output = result.map_err(|error| (error.code, error.message))?;
    let encoded = STANDARD.encode(&output);
    output.fill(0);
    Ok(json!({"data":encoded}))
}
hydrogen_plugin_sdk::export_plugin!((), dispatch);
