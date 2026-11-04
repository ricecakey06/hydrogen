//! Process service transport; the plugin owns process and pipe handles.
use super::storage::Settings;
use crate::{
    contracts::{ApiError, Result},
    plugins::host::PluginHost,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;
use std::path::Path;
pub fn stamp(host: &PluginHost, settings: &Settings, pid: u32) -> Result<Option<u64>> {
    Ok(serde_json::from_value(host.call_service(
        settings,
        "process",
        "process",
        "stamp",
        json!({"pid":pid}),
    )?)?)
}
pub struct Child {
    host: PluginHost,
    settings: Settings,
    handle: String,
    pub pid: u32,
    pub stamp: Option<u64>,
}
pub struct Output {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub eof: bool,
}
impl Child {
    pub async fn spawn(
        host: &PluginHost,
        settings: &Settings,
        executable: &Path,
        args: Vec<String>,
        directory: &Path,
    ) -> Result<Self> {
        let host = host.clone();
        let settings = settings.clone();
        let executable = executable.to_owned();
        let directory = directory.to_owned();
        tokio::task::spawn_blocking(move || {
            let response = host.call_service(
                &settings,
                "process",
                "process",
                "spawn",
                json!({"executable":executable,"args":args,"directory":directory}),
            )?;
            Ok(Self {
                handle: response["handle"].as_str().ok_or_else(invalid)?.into(),
                pid: response["pid"].as_u64().ok_or_else(invalid)? as u32,
                stamp: response["stamp"].as_u64(),
                host,
                settings,
            })
        })
        .await
        .map_err(|_| invalid())?
    }
    pub async fn poll(&self) -> Result<Output> {
        let host = self.host.clone();
        let settings = self.settings.clone();
        let handle = self.handle.clone();
        tokio::task::spawn_blocking(move || {
            let response = host.call_service(
                &settings,
                "process",
                "process",
                "poll",
                json!({"handle":handle}),
            )?;
            Ok(Output {
                exit_code: response["exitCode"].as_i64().map(|v| v as i32),
                stdout: STANDARD
                    .decode(response["stdout"].as_str().ok_or_else(invalid)?)
                    .map_err(|_| invalid())?,
                stderr: STANDARD
                    .decode(response["stderr"].as_str().ok_or_else(invalid)?)
                    .map_err(|_| invalid())?,
                eof: response["stdoutEof"] == true && response["stderrEof"] == true,
            })
        })
        .await
        .map_err(|_| invalid())?
    }
    pub async fn kill(&mut self) -> Result<()> {
        let host = self.host.clone();
        let settings = self.settings.clone();
        let handle = self.handle.clone();
        tokio::task::spawn_blocking(move || {
            host.call_service(
                &settings,
                "process",
                "process",
                "kill",
                json!({"handle":handle}),
            )
        })
        .await
        .map_err(|_| invalid())??;
        Ok(())
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.host.call_service(
            &self.settings,
            "process",
            "process",
            "detach",
            json!({"handle":self.handle}),
        );
    }
}
fn invalid() -> ApiError {
    ApiError::new("PROCESS_RESPONSE", "进程服务返回了无效响应。")
}
