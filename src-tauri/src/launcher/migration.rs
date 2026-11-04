//! Legacy import confirmation/task transport. Planning and copying belong to minecraft.instances.
use super::{core::Launcher, net, tasks::Job};
use crate::contracts::{ApiError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc};
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Plan(Value);
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub instances: usize,
    pub bytes: u64,
    pub settings: bool,
    pub source: String,
}
pub fn preview(client: &net::Client, root: &Path, chosen: &Path) -> Result<(Plan, Preview)> {
    Ok(serde_json::from_value(client.service(
        "minecraft.instances",
        "migration.preview",
        json!({"root":root,"source":chosen}),
    )?)?)
}
pub fn execute(core: &Launcher, plan: Plan, job: &Job) -> Result<()> {
    let state = core.store.lock().unwrap().state();
    if plan.0["root"] != state["root"] {
        return Err(ApiError::new(
            "LEGACY_IMPORT",
            "数据目录已切换，请重新预览。",
        ));
    }
    struct Pending<'a> {
        client: &'a net::Client,
        id: String,
    }
    impl Drop for Pending<'_> {
        fn drop(&mut self) {
            let _ = self
                .client
                .service("minecraft.instances", "cancel", json!({"handle":self.id}));
        }
    }
    job.check()?;
    let value = core.client.service(
        "minecraft.instances",
        "migration.execute",
        json!({"state":state,"plan":plan.0}),
    )?;
    let pending = Pending {
        client: &core.client,
        id: value
            .as_str()
            .ok_or_else(|| ApiError::new("INVALID_RESPONSE", "无效导入任务。"))?
            .into(),
    };
    loop {
        job.check()?;
        let value =
            core.client
                .service("minecraft.instances", "poll", json!({"handle":pending.id}))?;
        let p = &value["progress"];
        if let Some(phase) = p["phase"].as_str() {
            job.phase(phase, p["message"].as_str().unwrap_or(""));
        }
        if let Some(completed) = p["completed"].as_u64() {
            job.progress(
                completed,
                p["total"].as_u64(),
                p["message"].as_str().unwrap_or(""),
            );
        }
        if value["ready"] == true {
            let result = &value["result"];
            let mut store = core.store.lock().unwrap();
            store.settings = serde_json::from_value(result["settings"].clone())?;
            store.selected = serde_json::from_value(result["selected"].clone())?;
            core.client.set_settings(&store.settings);
            store.rebind_plugins(core.plugins.clone())?;
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
impl Launcher {
    pub fn import_legacy(self: &Arc<Self>, plan: Plan) -> Result<String> {
        let lease = self.lease();
        if plan.0["root"] != json!(lease.root) {
            return Err(ApiError::new(
                "LEGACY_IMPORT",
                "数据目录已切换，请重新预览。",
            ));
        }
        self.store.lock().unwrap().writable()?;
        let job = self.tasks.start("legacyImport", None)?;
        let id = job.state.lock().unwrap().id.clone();
        let core = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _lease = lease;
            job.finish(execute(&core, plan, &job));
        });
        Ok(id)
    }
}
