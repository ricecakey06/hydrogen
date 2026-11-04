use base64::{Engine, engine::general_purpose::STANDARD};
use hydrogen_plugin_services::{
    contracts::{ApiError, Result},
    invoke,
};
use serde_json::{Value, json};
pub struct Log {
    handle: String,
}
impl Log {
    pub fn create(path: &std::path::Path, secrets: Vec<String>) -> Result<Self> {
        let v = invoke(
            "diagnostics",
            "diagnostics",
            "create",
            json!({"path":path,"secrets":secrets}),
        )?;
        Ok(Self {
            handle: v["handle"].as_str().ok_or_else(invalid)?.into(),
        })
    }
    pub fn line(&mut self, line: &str) {
        let _ = invoke(
            "diagnostics",
            "diagnostics",
            "append",
            json!({"handle":self.handle,"lines":[line]}),
        );
    }
}
impl Drop for Log {
    fn drop(&mut self) {
        let _ = invoke(
            "diagnostics",
            "diagnostics",
            "close",
            json!({"handle":self.handle}),
        );
    }
}
pub struct Child {
    handle: String,
    pub pid: u32,
    pub stamp: Option<u64>,
}
impl Child {
    pub fn spawn(path: &str, args: Vec<String>, directory: &std::path::Path) -> Result<Self> {
        let v = invoke(
            "process",
            "process",
            "spawn",
            json!({"executable":path,"args":args,"directory":directory}),
        )?;
        Ok(Self {
            handle: v["handle"].as_str().ok_or_else(invalid)?.into(),
            pid: v["pid"].as_u64().ok_or_else(invalid)? as u32,
            stamp: v["stamp"].as_u64(),
        })
    }
    pub fn poll(&self) -> Result<(Value, Vec<u8>, Vec<u8>)> {
        let v = invoke("process", "process", "poll", json!({"handle":self.handle}))?;
        let stdout = STANDARD
            .decode(v["stdout"].as_str().ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        let stderr = STANDARD
            .decode(v["stderr"].as_str().ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        Ok((v, stdout, stderr))
    }
    pub fn kill(&self) -> Result<()> {
        invoke("process", "process", "kill", json!({"handle":self.handle}))?;
        Ok(())
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        let _ = invoke(
            "process",
            "process",
            "detach",
            json!({"handle":self.handle}),
        );
    }
}
fn invalid() -> ApiError {
    ApiError::new("INVALID_RESPONSE", "插件返回了无效进程或日志响应。")
}
