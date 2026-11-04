pub use super::records::{Components, Instance, Overrides, id_valid, validate_name};
pub use hydrogen_plugin_services::storage::*;
use hydrogen_plugin_services::{contracts::Result, http::Client, invoke};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub game_directory: Option<String>,
    #[serde(default)]
    pub isolate_game_directory: Option<bool>,
    pub java: Option<String>,
    pub max_memory_mib: u32,
    pub window_mode: String,
    pub window_width: u32,
    pub window_height: u32,
    pub after_launch: String,
    pub jvm_arguments: String,
    pub download_source: String,
    // Retained for existing configuration files; availability follows the plugin state.
    pub offline_enabled: bool,
    pub offline_player_name: String,
    pub account_mode: String,
    pub microsoft_client_id: String,
    pub modules: BTreeMap<String, bool>,
}
impl Settings {
    pub fn defaults() -> Result<Self> {
        Ok(serde_json::from_value(invoke(
            "storage",
            "storage",
            "defaults",
            serde_json::Value::Null,
        )?)?)
    }
    pub fn validate(&self, _client: &Client) -> Result<()> {
        invoke(
            "storage",
            "storage",
            "validate",
            serde_json::json!({"settings":self}),
        )?;
        Ok(())
    }
    pub fn effective(&self, _client: &Client, overrides: &Overrides) -> Result<Self> {
        Ok(serde_json::from_value(invoke(
            "storage",
            "storage",
            "effective",
            serde_json::json!({"settings":self,"overrides":overrides}),
        )?)?)
    }
}
