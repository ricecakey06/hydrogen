//! Launch task and window-event transport. Game startup implementation is installed code.
use super::{
    accounts::Session, core::Launcher, java::Java, minecraft::Prepared, net, storage::Settings,
};
use crate::contracts::{ApiError, Result};
use serde_json::json;
use std::{path::Path, sync::Arc};
pub fn arguments(
    client: &net::Client,
    game: &Prepared,
    java: &Java,
    session: &Session,
    settings: &Settings,
    directory: &Path,
    natives: &Path,
) -> Result<Vec<String>> {
    Ok(serde_json::from_value(client.service("minecraft.launch","arguments",json!({"game":game,"java":java,"session":session,"settings":settings,"directory":directory,"natives":natives}))?)?)
}
pub fn extract(client: &net::Client, game: &Prepared, directory: &Path) -> Result<()> {
    client.service(
        "minecraft.launch",
        "extract",
        json!({"game":game,"directory":directory}),
    )?;
    Ok(())
}
impl Launcher {
    pub fn launch(self: &Arc<Self>, id: String, window: tauri::WebviewWindow) -> Result<String> {
        let _identity = self.identity.lock().unwrap();
        if self
            .tasks
            .list()
            .iter()
            .any(|t| t.kind == "login" && t.active())
        {
            return Err(ApiError::new(
                "ACCOUNT_BUSY",
                "请等待账号登录完成后启动游戏。",
            ));
        }
        self.require("minecraft.launch")?;
        let instance_lock = self.instance_lock(&id)?;
        let root = self.lease();
        let instance = self.store.lock().unwrap().instance(&id)?;
        let job = self.tasks.start("launch", Some(&id))?;
        let task_id = job.state.lock().unwrap().id.clone();
        let this = self.clone();
        let task = task_id.clone();
        tauri::async_runtime::spawn(async move {
            let _lock = instance_lock;
            let _lease = &root;
            let mut last_action = String::new();
            let result=net::task_observed(&this.client,"minecraft.launch","launch",json!({"root":root.root,"gameRoot":root.game_root,"settings":root.settings,"instance":instance,"java":this.java.lock().unwrap().clone(),"taskId":task,"minimized":window.is_minimized().unwrap_or(false)}),Some(&job),|progress|{
    if let Some(path)=progress["logPath"].as_str(){job.state.lock().unwrap().log_path=Some(path.into());}
    if let Some(code)=progress["exitCode"].as_i64(){job.state.lock().unwrap().exit_code=Some(code as i32);}
    if progress["crashReport"].is_object(){job.state.lock().unwrap().crash_report=Some(progress["crashReport"].clone());}
    if let Some(error)=progress["crashError"].as_str(){job.state.lock().unwrap().crash_error=Some(error.into());}
    if let Some(action)=progress["windowAction"].as_str() && action!=last_action{match action{"minimize"=>{let _=window.minimize();},"restore"=>{let _=window.unminimize();let _=window.set_focus();},_=>return Err(ApiError::new("INVALID_RESPONSE","未知窗口操作。"))}last_action=action.into();}Ok(())
   }).await;
            if let Err(error) = result {
                if last_action == "minimize" {
                    let _ = window.unminimize();
                }
                job.finish(Err(error));
            }
        });
        Ok(task_id)
    }
}
