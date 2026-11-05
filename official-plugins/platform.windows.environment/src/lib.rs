mod platform;
use hydrogen_plugin_sdk::{
    Result,
    contracts::{ApiError, Result as ServiceResult},
    serde_json::{Value, json},
};
use std::path::Path;

fn dispatch(_: &mut (), request: &mut Value) -> Result {
    let result: ServiceResult<Value> = (|| {
        let params = &request["params"];
        let text = |key: &str| {
            params[key]
                .as_str()
                .ok_or_else(|| ApiError::new("INVALID_PARAMS", "缺少环境服务参数。"))
        };
        match request["method"].as_str().unwrap_or("") {
            "plugin.describe" => Ok(json!({"id":"platform.windows.environment","abiVersion":1})),
            "osVersion" => Ok(json!(platform::os_version())),
            "choose" => Ok(json!(platform::choose(
                params["owner"].as_i64().unwrap_or(0) as isize,
                params["folder"].as_bool().unwrap_or(false),
                params["multiple"].as_bool().unwrap_or(false)
            )?)),
            "browser" => {
                platform::browser(text("url")?)?;
                Ok(Value::Null)
            }
            "openAboutLink" => {
                platform::open_about_link(text("link")?)?;
                Ok(Value::Null)
            }
            "openFolder" => {
                platform::open_folder(Path::new(text("path")?))?;
                Ok(Value::Null)
            }
            "openText" => {
                platform::open_text(Path::new(text("path")?))?;
                Ok(Value::Null)
            }
            "availableSpace" => {
                #[cfg(windows)]
                {
                    Ok(json!(platform::available_space(Path::new(text("path")?))))
                }
                #[cfg(not(windows))]
                {
                    Ok(Value::Null)
                }
            }
            _ => Err(ApiError::new(
                "UNKNOWN_METHOD",
                "Unknown environment method",
            )),
        }
    })();
    result.map_err(|error| (error.code, error.message))
}
hydrogen_plugin_sdk::export_plugin!((), dispatch);
