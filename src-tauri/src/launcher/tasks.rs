use crate::contracts::{ApiError, Result};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub instance_id: Option<String>,
    pub kind: String,
    pub phase: String,
    pub message: String,
    pub completed: u64,
    pub total: Option<u64>,
    pub sequence: u64,
    pub started_at: u64,
    pub log_path: Option<String>,
    pub exit_code: Option<i32>,
    pub crash_report: Option<serde_json::Value>,
    pub crash_error: Option<String>,
    pub instance_name: Option<String>,
    pub game_version: Option<String>,
    pub download: Option<DownloadProgress>,
}
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub completed: u64,
    pub total: Option<u64>,
    pub completed_files: u64,
    pub total_files: u64,
    pub active_files: Vec<String>,
}
#[derive(Default)]
struct FileProgress {
    size: Option<u64>,
    bytes: u64,
    done: bool,
}
#[derive(Default)]
struct Downloads {
    files: BTreeMap<String, FileProgress>,
    active: BTreeSet<String>,
    progress: DownloadProgress,
}
impl Task {
    pub fn active(&self) -> bool {
        !["completed", "failed", "cancelled", "exited"].contains(&self.phase.as_str())
    }
    fn retryable_installation(&self) -> bool {
        matches!(self.phase.as_str(), "failed" | "cancelled")
            && matches!(self.kind.as_str(), "install" | "modpackImport")
    }
}
pub struct Job {
    pub state: Mutex<Task>,
    pub cancel: CancellationToken,
    modpack_source: Option<PathBuf>,
    updated: Mutex<Instant>,
    downloads: Mutex<Downloads>,
}
impl Job {
    pub fn phase(&self, phase: &str, message: impl Into<String>) {
        let mut state = self.state.lock().unwrap();
        state.phase = phase.into();
        state.message = message.into();
        state.sequence += 1;
    }
    pub fn begin_commit(&self, message: &str) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        self.check()?;
        state.phase = "committing".into();
        state.message = message.into();
        state.sequence += 1;
        Ok(())
    }
    pub fn progress(&self, completed: u64, total: Option<u64>, message: &str) {
        let mut updated = self.updated.lock().unwrap();
        if updated.elapsed().as_millis() < 100 {
            return;
        }
        *updated = Instant::now();
        let mut state = self.state.lock().unwrap();
        state.completed = completed;
        state.total = total;
        state.message = message.into();
        state.sequence += 1;
    }
    pub fn begin_download(
        &self,
        files: impl IntoIterator<Item = (String, Option<u64>)>,
        count_files: bool,
    ) {
        let mut downloads = self.downloads.lock().unwrap();
        *downloads = Downloads::default();
        downloads.files = files
            .into_iter()
            .map(|(name, size)| {
                (
                    name,
                    FileProgress {
                        size,
                        ..FileProgress::default()
                    },
                )
            })
            .collect();
        downloads.progress.total_files = if count_files {
            downloads.files.len() as u64
        } else {
            0
        };
        downloads.progress.total = downloads
            .files
            .values()
            .try_fold(0_u64, |sum, file| sum.checked_add(file.size?));
        self.report_download(&downloads, true);
    }
    pub fn file_started(&self, name: &str) {
        let mut downloads = self.downloads.lock().unwrap();
        if downloads.files.contains_key(name) {
            downloads.active.insert(name.into());
            self.report_download(&downloads, downloads.progress.total.is_none());
        }
    }
    pub fn file_progress(&self, name: &str, bytes: u64, done: bool) {
        let mut downloads = self.downloads.lock().unwrap();
        let Some(file) = downloads.files.get_mut(name) else {
            return;
        };
        let bytes = bytes.min(file.size.unwrap_or(u64::MAX));
        let previous = file.bytes;
        file.bytes = bytes;
        let completed = done && !file.done;
        file.done |= done;
        if done && file.size.is_none() {
            file.size = Some(bytes);
            downloads.progress.total = downloads
                .files
                .values()
                .try_fold(0_u64, |sum, file| sum.checked_add(file.size?));
        }
        downloads.progress.completed = downloads
            .progress
            .completed
            .saturating_sub(previous)
            .saturating_add(bytes);
        if done {
            downloads.active.remove(name);
        }
        if completed && downloads.progress.total_files > 0 {
            downloads.progress.completed_files += 1;
        }
        self.report_download(
            &downloads,
            done && (downloads.progress.total_files == 0
                || downloads.progress.completed_files == downloads.progress.total_files),
        );
    }
    fn report_download(&self, downloads: &Downloads, force: bool) {
        let mut updated = self.updated.lock().unwrap();
        if !force && updated.elapsed().as_millis() < 100 {
            return;
        }
        let mut state = self.state.lock().unwrap();
        if !state.active() || self.cancel.is_cancelled() {
            return;
        }
        *updated = Instant::now();
        let mut progress = downloads.progress.clone();
        progress.active_files = downloads
            .active
            .iter()
            .map(|name| {
                name.strip_prefix(".minecraft/")
                    .unwrap_or(name)
                    .replace('/', "\\")
            })
            .collect();
        state.completed = progress.completed_files;
        state.total = Some(progress.total_files);
        state.download = Some(progress);
        state.sequence += 1;
    }
    pub fn check(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            return Err(ApiError::new(
                "CANCELLED",
                "操作已取消，可重新开始以复用已校验缓存。",
            ));
        }
        Ok(())
    }
    pub fn finish(&self, result: Result<()>) {
        match result {
            Ok(()) => {
                let mut state = self.state.lock().unwrap();
                if let Some(total) = state.total {
                    state.completed = total;
                }
                if let Some(progress) = &mut state.download {
                    progress.completed = progress.total.unwrap_or(progress.completed);
                    progress.completed_files = progress.total_files;
                    progress.active_files.clear();
                }
                drop(state);
                self.phase("completed", "已完成");
            }
            Err(error) => self.phase(
                if error.code == "CANCELLED" {
                    "cancelled"
                } else {
                    "failed"
                },
                error.message,
            ),
        }
    }
}
#[derive(Default)]
pub struct Tasks(Mutex<BTreeMap<String, Arc<Job>>>);
pub enum InstallationRetry {
    Instance(String),
    Modpack(PathBuf),
}
impl Tasks {
    pub fn start(&self, kind: &str, instance: Option<&str>) -> Result<Arc<Job>> {
        self.start_job(kind, instance, None, None)
    }
    pub fn start_modpack_import(&self, source: PathBuf, retry: Option<&str>) -> Result<Arc<Job>> {
        self.start_job("modpackImport", None, Some(source), retry)
    }
    fn start_job(
        &self,
        kind: &str,
        instance: Option<&str>,
        modpack_source: Option<PathBuf>,
        retry: Option<&str>,
    ) -> Result<Arc<Job>> {
        let mut tasks = self.0.lock().unwrap();
        if tasks.values().any(|j| {
            let state = j.state.lock().unwrap();
            state.active()
                && (instance.is_some() && state.instance_id.as_deref() == instance
                    || instance.is_none() && state.kind == kind)
        }) {
            return Err(ApiError::new(
                "TASK_BUSY",
                "此实例或操作正在运行，请先等待完成。",
            ));
        }
        if let Some(id) = retry {
            let previous = tasks.get(id).ok_or_else(unknown_task)?;
            let state = previous.state.lock().unwrap();
            if !state.retryable_installation()
                || state.instance_id.is_some()
                || previous.modpack_source != modpack_source
            {
                return Err(not_retryable());
            }
            drop(state);
            tasks.remove(id);
        }
        if kind == "install" {
            tasks.retain(|_, job| {
                let state = job.state.lock().unwrap();
                !matches!(state.kind.as_str(), "install" | "modpackImport")
                    || state.instance_id.as_deref() != instance
            });
        }
        if tasks.len() >= 100 {
            let completed: Vec<_> = tasks
                .iter()
                .filter(|(_, j)| !j.state.lock().unwrap().active())
                .map(|(id, _)| id.clone())
                .collect();
            for id in completed {
                tasks.remove(&id);
            }
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let job = Arc::new(Job {
            state: Mutex::new(Task {
                id: id.clone(),
                instance_id: instance.map(String::from),
                kind: kind.into(),
                phase: "queued".into(),
                message: "等待开始".into(),
                completed: 0,
                total: None,
                sequence: 1,
                started_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                log_path: None,
                exit_code: None,
                crash_report: None,
                crash_error: None,
                instance_name: None,
                game_version: None,
                download: None,
            }),
            cancel: CancellationToken::new(),
            modpack_source,
            updated: Mutex::new(Instant::now()),
            downloads: Mutex::new(Downloads::default()),
        });
        tasks.insert(id, job.clone());
        Ok(job)
    }
    pub fn installation_retry(&self, id: &str) -> Result<InstallationRetry> {
        let tasks = self.0.lock().unwrap();
        let job = tasks.get(id).ok_or_else(unknown_task)?;
        let state = job.state.lock().unwrap();
        if state.retryable_installation() {
            if let Some(id) = &state.instance_id {
                return Ok(InstallationRetry::Instance(id.clone()));
            }
            if let Some(source) = &job.modpack_source {
                return Ok(InstallationRetry::Modpack(source.clone()));
            }
        }
        Err(not_retryable())
    }
    pub fn list(&self) -> Vec<Task> {
        let mut tasks: Vec<_> = self
            .0
            .lock()
            .unwrap()
            .values()
            .map(|j| j.state.lock().unwrap().clone())
            .collect();
        tasks.sort_by_key(|task| std::cmp::Reverse(task.started_at));
        tasks
    }
    pub fn busy(&self, instance: Option<&str>) -> bool {
        self.list()
            .iter()
            .any(|j| j.active() && instance.is_none_or(|id| j.instance_id.as_deref() == Some(id)))
    }
    pub fn forget_downloads(&self, instance: Option<&str>) {
        self.0.lock().unwrap().retain(|_, job| {
            let state = job.state.lock().unwrap();
            !matches!(state.kind.as_str(), "install" | "modpackImport")
                || state.active()
                || instance.is_some_and(|id| state.instance_id.as_deref() != Some(id))
        });
    }
    pub fn cancel(&self, id: &str) -> Result<()> {
        let tasks = self.0.lock().unwrap();
        let job = tasks.get(id).ok_or_else(unknown_task)?;
        let state = job.state.lock().unwrap();
        if ["starting", "running", "committing"].contains(&state.phase.as_str()) {
            return Err(ApiError::new(
                "TASK_NOT_CANCELLABLE",
                "提交阶段需等待完成；取消准备不会终止已运行的游戏。",
            ));
        }
        job.cancel.cancel();
        Ok(())
    }
    pub fn stop_preparation(&self) {
        for job in self.0.lock().unwrap().values() {
            if job.state.lock().unwrap().phase != "running" {
                job.cancel.cancel();
            }
        }
    }
}

fn unknown_task() -> ApiError {
    ApiError::new("UNKNOWN_TASK", "任务不存在。")
}

fn not_retryable() -> ApiError {
    ApiError::new("TASK_NOT_RETRYABLE", "此安装任务无法重试。")
}
