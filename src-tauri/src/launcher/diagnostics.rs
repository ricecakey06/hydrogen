//! Log session transport. Formatting, redaction, file ownership and limits live in diagnostics.
use super::storage::Settings;
use crate::{
    contracts::{ApiError, Result},
    plugins::host::PluginHost,
};
use serde_json::json;
use std::path::Path;
pub struct Log {
    host: PluginHost,
    settings: Settings,
    handle: String,
}
impl Log {
    pub fn create(
        host: &PluginHost,
        settings: &Settings,
        path: &Path,
        secrets: Vec<String>,
    ) -> Result<Self> {
        let response = host.call_service(
            settings,
            "diagnostics",
            "diagnostics",
            "create",
            json!({"path":path,"secrets":secrets}),
        )?;
        let handle = response["handle"]
            .as_str()
            .ok_or_else(|| ApiError::new("LOG_RESPONSE", "日志服务返回了无效句柄。"))?
            .into();
        Ok(Self {
            host: host.clone(),
            settings: settings.clone(),
            handle,
        })
    }
    pub fn line(&mut self, line: &str) {
        // A diagnostic write failure must not interrupt a running game.
        let _ = self.host.call_service(
            &self.settings,
            "diagnostics",
            "diagnostics",
            "append",
            json!({"handle":self.handle,"lines":[line]}),
        );
    }
}
impl Drop for Log {
    fn drop(&mut self) {
        let _ = self.host.call_service(
            &self.settings,
            "diagnostics",
            "diagnostics",
            "close",
            json!({"handle":self.handle}),
        );
    }
}
