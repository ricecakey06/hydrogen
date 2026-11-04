use hydrogen_plugin_services::{
    contracts::{ApiError, Result},
    invoke,
    jobs::{Job, Jobs, service_task},
    text,
};
use serde_json::{Value, json};
use std::sync::Arc;
struct State {
    jobs: Jobs,
}
impl Default for State {
    fn default() -> Self {
        Self {
            jobs: Jobs::with_limit(16),
        }
    }
}
async fn install(mut state: Value, mut instance: Value, job: Arc<Job>) -> Result<Value> {
    let root = state["root"].clone();
    let game_root = state["game_root"].clone();
    let settings = state["settings"].clone();
    let id = text(&instance, "id")?.to_owned();
    let version = text(&instance, "gameVersion")?.to_owned();
    let directory = text(&instance, "directoryName")?.to_owned();
    job.phase("resolving", "正在解析游戏文件…");
    let existing = !invoke(
        "minecraft.instances",
        "minecraft.instances",
        "manifestFile",
        json!({"root":game_root,"name":directory}),
    )?
    .is_null();
    let mut metadata = if existing {
        invoke(
            "minecraft.instances",
            "minecraft.instances",
            "load",
            json!({"root":game_root,"name":directory}),
        )?["metadata"]
            .clone()
    } else {
        service_task(
            "minecraft.content",
            "resolve",
            json!({"root":root,"game":game_root,"settings":settings,"id":version}),
            &job,
        )
        .await?
    };
    metadata["id"] = json!(directory);
    let game = invoke(
        "minecraft.instances",
        "minecraft.instances",
        "records.gameDir",
        json!({"state":state,"id":id}),
    )?;
    metadata["_hydrogenRunDirectory"] = game.clone();
    let base = metadata.clone();
    job.phase("downloading", "正在下载并校验游戏文件…");
    let mut prepared=service_task("minecraft.preinstall","prepare",json!({"gameRoot":game_root,"settings":settings,"metadata":metadata,"architecture":if cfg!(target_arch="aarch64"){"arm64"}else{"x64"}}),&job).await?;
    if !existing && instance["components"].is_object() {
        job.phase("preparing", "正在准备 Fabric…");
        prepared=service_task("minecraft.fabric","prepare",json!({"root":root,"gameRoot":game_root,"game":game,"settings":settings,"version":version,"components":instance["components"],"prepared":prepared,"installing":true}),&job).await?;
    }
    job.check()?;
    job.phase("committing", "正在提交安装状态…");
    if !existing {
        invoke(
            "minecraft.instances",
            "minecraft.instances",
            "publish",
            json!({"root":game_root,"name":directory,"gameVersion":version,"base":base,"complete":prepared["metadata"],"loader":instance["components"]["loader"]}),
        )?;
    }
    instance = invoke(
        "minecraft.instances",
        "minecraft.instances",
        "records.get",
        json!({"state":state,"id":id}),
    )?;
    instance["isInstalled"] = json!(true);
    invoke(
        "minecraft.instances",
        "minecraft.instances",
        "records.write",
        json!({"state":state,"instance":instance}),
    )?;
    if state["selected"].is_null() {
        state["selected"] = invoke(
            "minecraft.instances",
            "minecraft.instances",
            "records.select",
            json!({"state":state,"id":id}),
        )?;
        state = invoke(
            "storage",
            "storage",
            "save",
            json!({"state":state,"settings":settings}),
        )?;
    }
    Ok(state)
}
impl State {
    fn call(&mut self, r: &mut Value) -> Result<Value> {
        let p = &r["params"];
        match r["method"].as_str().unwrap_or("") {
            "plugin.describe" => Ok(json!({"id":"minecraft.install","abiVersion":1})),
            "plugin.stop" => {
                self.jobs.clear();
                Ok(Value::Null)
            }
            "poll" => self.jobs.poll(text(p, "handle")?),
            "cancel" => {
                self.jobs.cancel(text(p, "handle")?);
                Ok(Value::Null)
            }
            "install" => {
                let state = p["state"].clone();
                let instance = p["instance"].clone();
                let job = Arc::new(Job::default());
                self.jobs.begin(
                    job.clone(),
                    async move { install(state, instance, job).await },
                )
            }
            "create" => {
                let state = p["state"].clone();
                let version = text(p, "version")?.to_owned();
                let name = text(p, "name")?.to_owned();
                let components = p["components"].clone();
                let job = Arc::new(Job::default());
                self.jobs.begin(job.clone(),async move{let versions=service_task("minecraft.content","versions",json!({"root":state["root"],"settings":state["settings"]}),&job).await?;if !versions.as_array().is_some_and(|v|v.iter().any(|v|v["id"]==version)){return Err(ApiError::new("UNKNOWN_VERSION","官方目录中没有此游戏版本。"));}invoke("minecraft.instances","minecraft.instances","records.create",json!({"state":state,"version":version,"name":name,"components":components}))})
            }
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知安装方法。")),
        }
    }
}
fn dispatch(s: &mut State, r: &mut Value) -> hydrogen_plugin_sdk::Result {
    s.call(r).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);
