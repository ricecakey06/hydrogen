use crate::contracts::{ApiError, Result};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Default)]
pub struct Job {
    pub cancel: tokio_util::sync::CancellationToken,
    pub progress: Mutex<Value>,
}
impl Job {
    pub fn progress(&self, completed: u64, total: Option<u64>, message: &str) {
        let mut p = self.progress.lock().unwrap();
        if !p.is_object() {
            *p = json!({});
        }
        p["completed"] = json!(completed);
        p["total"] = json!(total);
        p["message"] = json!(message);
    }
    pub fn phase(&self, phase: &str, message: impl Into<String>) {
        *self.progress.lock().unwrap() = json!({"phase":phase,"message":message.into()});
    }
    pub fn begin_download(
        &self,
        files: impl IntoIterator<Item = (String, Option<u64>)>,
        count_files: bool,
    ) {
        let mut progress = self.progress.lock().unwrap();
        if !progress.is_object() {
            *progress = json!({});
        }
        progress["plan"] = json!({"id":uuid::Uuid::new_v4().to_string(),"files":files.into_iter().collect::<Vec<_>>(),"countFiles":count_files});
        progress["files"] = json!({});
    }
    pub fn file_started(&self, name: &str) {
        self.file_progress(name, 0, false);
    }
    pub fn file_progress(&self, name: &str, bytes: u64, done: bool) {
        let mut progress = self.progress.lock().unwrap();
        if !progress.is_object() {
            *progress = json!({});
        }
        progress["files"][name] = json!({"bytes":bytes,"done":done});
    }
    pub fn apply(&self, progress: &Value) {
        {
            let mut target = self.progress.lock().unwrap();
            if !target.is_object() {
                *target = json!({});
            }
            if progress["plan"].is_object() && target["plan"]["id"] != progress["plan"]["id"] {
                target["plan"] = progress["plan"].clone();
                target["files"] = json!({});
            }
            for key in ["phase", "message"] {
                if progress[key].is_string() {
                    target[key] = progress[key].clone();
                }
            }
        }
        if let Some(files) = progress["files"].as_object() {
            for (name, file) in files {
                self.file_progress(
                    name,
                    file["bytes"].as_u64().unwrap_or(0),
                    file["done"] == true,
                );
            }
        }
    }
    pub fn check(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            Err(ApiError::new("CANCELLED", "任务已取消。"))
        } else {
            Ok(())
        }
    }
}
type Pending = Pin<Box<dyn Future<Output = Result<Value>>>>;
pub struct Jobs {
    limit: usize,
    runtime: tokio::runtime::Runtime,
    pending: BTreeMap<String, (Pending, Arc<Job>)>,
}
impl Default for Jobs {
    fn default() -> Self {
        Self {
            limit: 1,
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
            pending: BTreeMap::new(),
        }
    }
}
impl Jobs {
    pub fn with_limit(limit: usize) -> Self {
        Self {
            limit,
            ..Self::default()
        }
    }
    pub fn begin(
        &mut self,
        job: Arc<Job>,
        future: impl Future<Output = Result<Value>> + 'static,
    ) -> Result<Value> {
        if self.pending.len() >= self.limit {
            return Err(ApiError::new("PLUGIN_BUSY", "插件任务数量已达到上限。"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.pending.insert(id.clone(), (Box::pin(future), job));
        Ok(json!(id))
    }
    pub fn poll(&mut self, id: &str) -> Result<Value> {
        let (future, job) = self
            .pending
            .get_mut(id)
            .ok_or_else(|| ApiError::new("UNKNOWN_TASK", "任务已结束。"))?;
        let result = self.runtime.block_on(async {
            let result = tokio::time::timeout(Duration::from_millis(1), future).await;
            // Flush deferred cooperative/yield wakeups before this temporary
            // block_on scheduler context exits. Child futures survive it.
            tokio::task::yield_now().await;
            result
        });
        let progress = job.progress.lock().unwrap().clone();
        match result {
            Err(_) => Ok(json!({"ready":false,"progress":progress})),
            Ok(result) => {
                self.pending.remove(id);
                Ok(json!({"ready":true,"result":result?,"progress":progress}))
            }
        }
    }
    pub fn cancel(&mut self, id: &str) {
        self.pending.remove(id);
    }
    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

pub async fn service_task(provider: &str, method: &str, params: Value, job: &Job) -> Result<Value> {
    struct Handle {
        provider: String,
        id: String,
    }
    impl Drop for Handle {
        fn drop(&mut self) {
            let _ = crate::invoke(
                &self.provider,
                &self.provider,
                "cancel",
                json!({"handle":self.id}),
            );
        }
    }
    let result = crate::invoke(provider, provider, method, params)?;
    if result["ready"] == true {
        return Ok(result["result"].clone());
    }
    let handle = Handle {
        provider: provider.into(),
        id: result
            .as_str()
            .or_else(|| result["handle"].as_str())
            .ok_or_else(|| ApiError::new("INVALID_RESPONSE", "无效任务句柄。"))?
            .into(),
    };
    loop {
        job.check()?;
        let result = crate::invoke(provider, provider, "poll", json!({"handle":handle.id}))?;
        job.apply(&result["progress"]);
        if result["ready"] == true {
            return Ok(result["result"].clone());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
