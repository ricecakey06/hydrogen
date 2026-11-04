use super::{
    net,
    storage::{Components, Settings},
    tasks::Job,
};
use crate::contracts::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub id: String,
    pub release_time: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Prepared {
    pub metadata: Value,
    pub classpath: Vec<PathBuf>,
    pub natives: Vec<(PathBuf, Vec<String>)>,
    pub assets: PathBuf,
    pub libraries: PathBuf,
    pub asset_index: String,
    pub logging: Option<String>,
}
pub fn supported(client: &net::Client, id: &str) -> Result<bool> {
    Ok(client.service("minecraft.content", "supported", json!({"id":id}))? == true)
}
pub fn allows(
    client: &net::Client,
    owner: &Value,
    architecture: &str,
    custom_resolution: bool,
) -> Result<bool> {
    Ok(client.service(
        "minecraft.content",
        "allows",
        json!({"owner":owner,"architecture":architecture,"customResolution":custom_resolution}),
    )? == true)
}
pub async fn versions(
    client: &net::Client,
    root: &Path,
    settings: &Settings,
) -> Result<Vec<Version>> {
    Ok(serde_json::from_value(
        net::task(
            client,
            "minecraft.content",
            "versions",
            json!({"root":root,"settings":settings}),
            None,
        )
        .await?,
    )?)
}
pub async fn resolve(
    client: &net::Client,
    data_root: &Path,
    game_root: &Path,
    settings: &Settings,
    id: &str,
    job: &Job,
) -> Result<Value> {
    net::task(
        client,
        "minecraft.content",
        "resolve",
        json!({"root":data_root,"game":game_root,"settings":settings,"id":id}),
        Some(job),
    )
    .await
}
pub async fn prepare(
    _plugins: &crate::plugins::host::PluginHost,
    client: &net::Client,
    game_root: &Path,
    settings: &Settings,
    metadata: Value,
    architecture: &str,
    job: Arc<Job>,
) -> Result<Prepared> {
    Ok(serde_json::from_value(net::task(client,"minecraft.preinstall","prepare",json!({"gameRoot":game_root,"settings":settings,"metadata":metadata,"architecture":architecture}),Some(&job)).await?)?)
}
pub async fn fabric_options(client: &net::Client, version: &str) -> Result<Value> {
    net::task(
        client,
        "minecraft.fabric",
        "options",
        json!({"version":version}),
        None,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
pub async fn fabric(
    client: &net::Client,
    data_root: &Path,
    game_root: &Path,
    game: &Path,
    settings: &Settings,
    version: &str,
    components: &Components,
    prepared: &mut Prepared,
    job: &Job,
    installing: bool,
) -> Result<()> {
    *prepared=serde_json::from_value(net::task(client,"minecraft.fabric","prepare",json!({"root":data_root,"gameRoot":game_root,"game":game,"settings":settings,"version":version,"components":components,"prepared":prepared,"installing":installing}),Some(job)).await?)?;
    Ok(())
}
