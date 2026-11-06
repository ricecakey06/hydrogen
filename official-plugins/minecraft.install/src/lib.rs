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
    let manifest = invoke(
        "minecraft.instances",
        "minecraft.instances",
        "manifestFile",
        json!({"root":game_root,"name":directory}),
    )?;
    // Pack overrides may contain top-level JSON files. Only the manifest
    // published by the installer can be reused for a pack instance.
    let existing = if instance["isolated"] == true {
        manifest == format!("versions/{directory}/{directory}.json")
    } else {
        !manifest.is_null()
    };
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
        let kind = instance["components"]["kind"].as_str().unwrap_or("fabric");
        if kind.contains(':') {
            let patch = invoke(
                "host.providers",
                "loaderProvider",
                "installPlan",
                json!({"id":kind,"params":{"gameVersion":version,"loaderVersion":instance["components"]["loader"]}}),
            )?;
            apply_loader_patch(&mut metadata, &patch)?;
            prepared=service_task("minecraft.preinstall","prepare",json!({"gameRoot":game_root,"settings":settings,"metadata":metadata,"architecture":if cfg!(target_arch="aarch64"){"arm64"}else{"x64"}}),&job).await?;
        } else {
            let service = if matches!(kind, "forge" | "neoforge") {
                "minecraft.forge"
            } else {
                "minecraft.fabric"
            };
            job.phase("preparing", "正在准备模组加载器…");
            prepared=service_task(service,"prepare",json!({"root":root,"gameRoot":game_root,"game":game,"settings":settings,"version":version,"components":instance["components"],"prepared":prepared,"installing":true}),&job).await?;
        }
    }
    job.check()?;
    job.phase("committing", "正在提交安装状态…");
    if !existing {
        invoke(
            "minecraft.instances",
            "minecraft.instances",
            "publish",
            json!({"root":game_root,"name":directory,"gameVersion":version,"base":base,"complete":prepared["metadata"],"loader":instance["components"]["loader"],"loaderKind":instance["components"]["kind"]}),
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
fn apply_loader_patch(metadata: &mut Value, patch: &Value) -> Result<()> {
    let object = patch
        .as_object()
        .ok_or_else(|| ApiError::new("INVALID_LOADER_PLAN", "加载器计划必须为对象。"))?;
    if object
        .keys()
        .any(|key| !["mainClass", "libraries", "arguments"].contains(&key.as_str()))
    {
        return Err(ApiError::new(
            "INVALID_LOADER_PLAN",
            "加载器计划修改了未开放的字段。",
        ));
    }
    if let Some(main) = object.get("mainClass") {
        if !main.as_str().is_some_and(|s| {
            !s.is_empty()
                && s.len() <= 200
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._$".contains(&b))
        }) {
            return Err(ApiError::new("INVALID_LOADER_PLAN", "启动类无效。"));
        }
        metadata["mainClass"] = main.clone();
    }
    if let Some(libraries) = object.get("libraries") {
        let libraries = libraries
            .as_array()
            .filter(|v| v.len() <= 256)
            .ok_or_else(|| ApiError::new("INVALID_LOADER_PLAN", "加载器库列表无效。"))?;
        for library in libraries {
            let artifact = &library["downloads"]["artifact"];
            if !library["name"].is_string()
                || !artifact["sha1"]
                    .as_str()
                    .is_some_and(hydrogen_plugin_services::storage::valid_hash)
                || !artifact["size"]
                    .as_u64()
                    .is_some_and(|n| n > 0 && n <= 1024 * 1024 * 1024)
                || !artifact["url"].is_string()
            {
                return Err(ApiError::new(
                    "INVALID_LOADER_PLAN",
                    "加载器库必须提供坐标、下载地址、大小和 SHA-1。",
                ));
            }
        }
        if metadata["libraries"].is_null() {
            metadata["libraries"] = json!([]);
        }
        metadata["libraries"]
            .as_array_mut()
            .ok_or_else(|| ApiError::new("INVALID_METADATA", "原版库列表无效。"))?
            .extend(libraries.iter().cloned());
    }
    if let Some(arguments) = object.get("arguments") {
        let arguments = arguments
            .as_object()
            .filter(|v| v.keys().all(|k| ["jvm", "game"].contains(&k.as_str())))
            .ok_or_else(|| ApiError::new("INVALID_LOADER_PLAN", "启动参数无效。"))?;
        for (key, values) in arguments {
            let values = values
                .as_array()
                .filter(|v| {
                    v.len() <= 128
                        && v.iter().all(|v| {
                            v.as_str()
                                .is_some_and(|s| s.len() <= 4096 && !s.contains('\0'))
                        })
                })
                .ok_or_else(|| {
                    ApiError::new("INVALID_LOADER_PLAN", "启动参数必须为有界字符串数组。")
                })?;
            if metadata["arguments"].is_null() {
                metadata["arguments"] = json!({});
            }
            if metadata["arguments"][key].is_null() {
                metadata["arguments"][key] = json!([]);
            }
            metadata["arguments"][key]
                .as_array_mut()
                .ok_or_else(|| ApiError::new("INVALID_METADATA", "原版启动参数无效。"))?
                .extend(values.iter().cloned());
        }
    }
    Ok(())
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
                let pack_stage = p["packStage"].clone();
                let job = Arc::new(Job::default());
                self.jobs.begin(job.clone(),async move{let versions=service_task("minecraft.content","versions",json!({"root":state["root"],"settings":state["settings"]}),&job).await?;if !versions.as_array().is_some_and(|v|v.iter().any(|v|v["id"]==version)){return Err(ApiError::new("UNKNOWN_VERSION","官方目录中没有此游戏版本。"));}invoke("minecraft.instances","minecraft.instances","records.create",json!({"state":state,"version":version,"name":name,"components":components,"packStage":pack_stage}))})
            }
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知安装方法。")),
        }
    }
}
fn dispatch(s: &mut State, r: &mut Value) -> hydrogen_plugin_sdk::Result {
    s.call(r).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);
