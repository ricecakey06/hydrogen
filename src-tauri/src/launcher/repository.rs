use super::net::Client;
use crate::contracts::Result;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Version {
    pub name: String,
    pub manifest_file: String,
    pub metadata: Value,
    pub game_version: String,
    pub loader: String,
    pub loader_version: Option<String>,
}
pub fn load(client: &Client, root: &Path, name: &str) -> Result<Version> {
    Ok(serde_json::from_value(client.service(
        "minecraft.instances",
        "load",
        json!({"root":root,"name":name}),
    )?)?)
}
pub fn scan(client: &Client, root: &Path) -> Result<(Vec<Version>, Vec<String>)> {
    Ok(serde_json::from_value(client.service(
        "minecraft.instances",
        "scan",
        json!({"root":root,}),
    )?)?)
}
pub fn manifest_file(client: &Client, root: &Path, name: &str) -> Result<Option<String>> {
    Ok(serde_json::from_value(client.service(
        "minecraft.instances",
        "manifestFile",
        json!({"root":root,"name":name}),
    )?)?)
}
pub fn run_directory(
    client: &Client,
    root: &Path,
    name: &str,
    isolate: Option<bool>,
) -> Result<PathBuf> {
    Ok(serde_json::from_value(client.service(
        "minecraft.instances",
        "runDirectory",
        json!({"root":root,"name":name,"isolate":isolate}),
    )?)?)
}
pub fn native_directory(
    client: &Client,
    root: &Path,
    name: &str,
    architecture: &str,
) -> Result<PathBuf> {
    Ok(serde_json::from_value(client.service(
        "minecraft.instances",
        "nativeDirectory",
        json!({"root":root,"name":name,"architecture":architecture}),
    )?)?)
}
pub fn publish(
    client: &Client,
    root: &Path,
    name: &str,
    game_version: &str,
    base: &Value,
    complete: &Value,
    loader: Option<&str>,
) -> Result<()> {
    Ok(serde_json::from_value(client.service("minecraft.instances","publish",json!({"root":root,"name":name,"gameVersion":game_version,"base":base,"complete":complete,"loader":loader}))?)?)
}
