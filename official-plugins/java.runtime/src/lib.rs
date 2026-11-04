mod contracts {
    pub type Result<T> = std::result::Result<T, ApiError>;
    #[derive(Debug)]
    pub struct ApiError {
        pub code: &'static str,
        pub message: String,
    }
    impl ApiError {
        pub fn new(code: &'static str, message: impl Into<String>) -> Self {
            Self {
                code,
                message: message.into(),
            }
        }
    }
    impl From<std::io::Error> for ApiError {
        fn from(_: std::io::Error) -> Self {
            Self::new("IO_ERROR", "Java 文件操作失败。")
        }
    }
}
mod java;
use hydrogen_plugin_sdk::{
    Result,
    serde_json::{Value, json},
};
use std::path::Path;
struct Provider {
    runtime: tokio::runtime::Runtime,
}
impl Default for Provider {
    fn default() -> Self {
        Self {
            runtime: tokio::runtime::Runtime::new().expect("Java executor"),
        }
    }
}
fn dispatch(provider: &mut Provider, request: &mut Value) -> Result {
    let params = &request["params"];
    let path = || {
        params["path"]
            .as_str()
            .ok_or(("INVALID_PARAMS".into(), "缺少 Java 路径。".into()))
    };
    match request["method"].as_str().unwrap_or("") {
        "plugin.describe" => Ok(json!({"id":"java.runtime","abiVersion":1})),
        "parse" => Ok(json!(java::parse(
            Path::new(path()?),
            params["output"].as_str().unwrap_or("")
        ))),
        "inspect" => provider
            .runtime
            .block_on(java::inspect(Path::new(path()?)))
            .map(|v| json!(v))
            .map_err(|e| (e.code.into(), e.message)),
        "scan" => {
            let saved: Vec<String> =
                hydrogen_plugin_sdk::serde_json::from_value(params["saved"].clone())
                    .map_err(|_| ("INVALID_PARAMS".into(), "Java 路径列表无效。".into()))?;
            Ok(json!(provider.runtime.block_on(java::scan(saved))))
        }
        _ => Err(("UNKNOWN_METHOD".into(), "Unknown Java method".into())),
    }
}
hydrogen_plugin_sdk::export_plugin!(Provider, dispatch);
