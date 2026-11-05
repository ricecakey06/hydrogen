mod prepare;
mod services;
use hydrogen_plugin_services::{
    PluginHost,
    contracts::{ApiError, Result},
    invoke,
    jobs::{Job, Jobs, service_task},
    net::Client,
    storage, text,
};
use prepare::{Java, OutputLines, Prepared, Session, Settings};
use serde_json::{Value, json};
use services::{Child, Log};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
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
async fn launch(p: Value, job: Arc<Job>) -> Result<Value> {
    let root = PathBuf::from(text(&p, "root")?);
    let game_root = p["gameRoot"].clone();
    let instance = &p["instance"];
    let id = text(instance, "id")?;
    if instance["isInstalled"] != true {
        return Err(ApiError::new("NOT_INSTALLED", "请先完成实例安装。"));
    }
    let raw = invoke(
        "storage",
        "storage",
        "effective",
        json!({"settings":p["settings"],"overrides":instance["overrides"]}),
    )?;
    let settings: Settings = serde_json::from_value(raw.clone())?;
    let network_settings = serde_json::from_value(raw.clone())?;
    let client = Client::new(&PluginHost, &network_settings)?;
    job.phase("checking", "正在检查账号与 Java…");
    let session: Session = serde_json::from_value(
        service_task(
            "account.manager",
            "session",
            json!({"root":root,"settings":raw}),
            &job,
        )
        .await?,
    )?;
    let name = text(instance, "directoryName")?;
    let mut metadata = invoke(
        "minecraft.instances",
        "minecraft.instances",
        "load",
        json!({"root":game_root,"name":name}),
    )?["metadata"]
        .clone();
    let game: PathBuf = serde_json::from_value(invoke(
        "minecraft.instances",
        "minecraft.instances",
        "runDirectory",
        json!({"root":game_root,"name":name,"isolate":(instance["isolated"] == true).then_some(true).or(settings.isolate_game_directory).or_else(||instance["components"].is_object().then_some(true))}),
    )?)?;
    metadata["_hydrogenRunDirectory"] = json!(game);
    let required = metadata["javaVersion"]["majorVersion"]
        .as_u64()
        .unwrap_or(0) as u32;
    let java: Java = if let Some(path) = &settings.java {
        serde_json::from_value(invoke(
            "java.runtime",
            "java.runtime",
            "inspect",
            json!({"path":path}),
        )?)?
    } else {
        let mut runtimes: Vec<Java> = serde_json::from_value(p["java"].clone())?;
        if !runtimes.iter().any(|j| j.major == required) {
            runtimes = serde_json::from_value(invoke(
                "java.runtime",
                "java.runtime",
                "scan",
                json!({"saved":[]}),
            )?)?;
        }
        let selected = runtimes
            .iter()
            .filter(|j| j.major == required)
            .max_by_key(|j| {
                j.architecture
                    == if cfg!(target_arch = "aarch64") {
                        "arm64"
                    } else {
                        "x64"
                    }
            })
            .ok_or_else(|| {
                ApiError::new(
                    "JAVA_MISSING",
                    format!("此版本需要 Java {required}，请扫描或手动选择。"),
                )
            })?;
        serde_json::from_value(invoke(
            "java.runtime",
            "java.runtime",
            "inspect",
            json!({"path":selected.path}),
        )?)?
    };
    if java.major != required || java.architecture == "x86" && settings.max_memory_mib > 1400 {
        return Err(ApiError::new(
            "JAVA_INCOMPATIBLE",
            format!("需要 Java {required}，32 位 Java 的内存不能超过 1400 MB。"),
        ));
    }
    job.phase("preparing", "正在准备并校验游戏文件…");
    let prepared:Prepared=serde_json::from_value(service_task("minecraft.preinstall","prepare",json!({"gameRoot":game_root,"settings":raw,"metadata":metadata,"architecture":java.architecture}),&job).await?)?;
    std::fs::create_dir_all(&game)?;
    let natives: PathBuf = serde_json::from_value(invoke(
        "minecraft.instances",
        "minecraft.instances",
        "nativeDirectory",
        json!({"root":game_root,"name":name,"architecture":java.architecture}),
    )?)?;
    prepare::extract(&prepared, &natives)?;
    let args = prepare::arguments(
        &client, &prepared, &java, &session, &settings, &game, &natives,
    )?;
    job.check()?;
    let log_path = storage::within(&root, &format!("logs/game-{}.log", text(&p, "taskId")?))?;
    let mut log = Log::create(
        &log_path,
        if session.offline {
            vec![]
        } else {
            vec![session.access_token.clone(), session.xuid.clone()]
        },
    )?;
    log.line(&format!(
        "Minecraft {} · Java {} {} · heap {} MiB",
        text(instance, "gameVersion")?,
        java.version,
        java.architecture,
        settings.max_memory_mib
    ));
    if PluginHost.available(&network_settings, "i18n")
        && let Err(error) = invoke(
            "i18n",
            "i18n",
            "applyGameLanguage",
            json!({"root":root,"directory":game,"locale":settings.locale,"version":instance["gameVersion"]}),
        )
    {
        log.line(&format!("Game language: {}", error.message));
    }
    job.phase("starting", "正在启动游戏…");
    job.check()?;
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let child = Child::spawn(&java.path, args, &game)?;
    let marker = storage::within(&root, &format!("instances/{id}/process.json"))?;
    let saved = child
        .stamp
        .ok_or_else(|| ApiError::new("PROCESS_IDENTITY", "无法确认游戏进程身份。"))
        .and_then(|stamp| storage::write_json(&marker, &json!({"pid":child.pid,"stamp":stamp})));
    if let Err(error) = saved {
        let _ = child.kill();
        return Err(error);
    }
    job.phase("running", "游戏正在运行");
    let minimize = settings.after_launch == "minimize" && p["minimized"] != true;
    {
        let mut progress = job.progress.lock().unwrap();
        progress["logPath"] = json!(log_path);
        if minimize {
            progress["windowAction"] = json!("minimize");
        }
    }
    let mut stdout = OutputLines::default();
    let mut stderr = OutputLines::default();
    let mut exited = None;
    let code = loop {
        let (v, out, err) = child.poll()?;
        stdout.feed(&out, &mut log);
        stderr.feed(&err, &mut log);
        if let Some(code) = v["exitCode"].as_i64() {
            let deadline = exited.get_or_insert_with(std::time::Instant::now);
            if v["stdoutEof"] == true && v["stderrEof"] == true
                || deadline.elapsed() >= std::time::Duration::from_secs(3)
            {
                break code;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    stdout.finish(&mut log);
    stderr.finish(&mut log);
    log.line(&format!("Game exited: {code}"));
    drop(log);
    let _ = std::fs::remove_file(marker);
    let mut crash_report = None;
    let mut crash_error = None;
    if code != 0 {
        job.phase("analyzing", "游戏异常退出，正在分析崩溃原因…");
        if minimize {
            job.progress.lock().unwrap()["windowAction"] = json!("restore");
        }
        let report = invoke(
            "minecraft.crash",
            "minecraft.crash",
            "analyze",
            json!({"root":root,"directory":game,"taskId":text(&p,"taskId")?,"startedAt":started_at,
                "exitCode":code,"instanceName":instance["name"],"gameVersion":instance["gameVersion"],
                "javaVersion":java.version,"javaArchitecture":java.architecture,"maxMemoryMib":settings.max_memory_mib}),
        );
        match report {
            Ok(report) => crash_report = Some(report),
            Err(error) => crash_error = Some(error.message),
        }
    }
    finish_exit(&job, code, &log_path, minimize, crash_report, crash_error);
    Ok(json!({"exitCode":code}))
}

fn finish_exit(
    job: &Job,
    code: i64,
    log_path: &Path,
    minimize: bool,
    crash_report: Option<Value>,
    crash_error: Option<String>,
) {
    // phase() replaces progress, so attach the report after setting the terminal phase.
    job.phase(
        if code == 0 { "exited" } else { "failed" },
        if code == 0 {
            "游戏已退出".into()
        } else {
            format!("游戏异常退出（退出码 {code}）")
        },
    );
    {
        let mut progress = job.progress.lock().unwrap();
        progress["exitCode"] = json!(code);
        progress["logPath"] = json!(log_path);
        progress["crashReport"] = json!(crash_report);
        progress["crashError"] = json!(crash_error);
        if minimize {
            progress["windowAction"] = json!("restore");
        }
    }
}
impl State {
    fn call(&mut self, r: &mut Value) -> Result<Value> {
        let p = &r["params"];
        match r["method"].as_str().unwrap_or("") {
            "plugin.describe" => Ok(json!({"id":"minecraft.launch","abiVersion":1})),
            "plugin.stop" => {
                self.jobs.clear();
                Ok(Value::Null)
            }
            "poll" => self.jobs.poll(text(p, "handle")?),
            "cancel" => {
                self.jobs.cancel(text(p, "handle")?);
                Ok(Value::Null)
            }
            "launch" => {
                let p = p.clone();
                let job = Arc::new(Job::default());
                self.jobs
                    .begin(job.clone(), async move { launch(p, job).await })
            }
            "arguments" => {
                let client =
                    Client::new(&PluginHost, &serde_json::from_value(p["settings"].clone())?)?;
                Ok(json!(prepare::arguments(
                    &client,
                    &serde_json::from_value(p["game"].clone())?,
                    &serde_json::from_value(p["java"].clone())?,
                    &serde_json::from_value(p["session"].clone())?,
                    &serde_json::from_value(p["settings"].clone())?,
                    Path::new(text(p, "directory")?),
                    Path::new(text(p, "natives")?)
                )?))
            }
            "extract" => {
                prepare::extract(
                    &serde_json::from_value(p["game"].clone())?,
                    Path::new(text(p, "directory")?),
                )?;
                Ok(Value::Null)
            }
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知启动方法。")),
        }
    }
}
fn dispatch(s: &mut State, r: &mut Value) -> hydrogen_plugin_sdk::Result {
    s.call(r).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);

#[cfg(test)]
mod exit_tests {
    use super::*;
    #[test]
    fn terminal_progress_keeps_report_and_window_restore() {
        let job = Job::default();
        job.phase("analyzing", "正在分析");
        finish_exit(
            &job,
            1,
            Path::new("game.log"),
            true,
            Some(json!({"id":"report"})),
            None,
        );
        let progress = job.progress.lock().unwrap();
        assert_eq!(progress["phase"], "failed");
        assert_eq!(progress["crashReport"]["id"], "report");
        assert_eq!(progress["exitCode"], 1);
        assert_eq!(progress["logPath"], "game.log");
        assert_eq!(progress["windowAction"], "restore");
    }
    #[test]
    fn optional_analysis_failure_preserves_the_game_exit_status() {
        let job = Job::default();
        finish_exit(
            &job,
            1,
            Path::new("game.log"),
            false,
            None,
            Some("插件未启用".into()),
        );
        assert_eq!(job.progress.lock().unwrap()["crashError"], "插件未启用");
        finish_exit(&job, 0, Path::new("game.log"), false, None, None);
        let progress = job.progress.lock().unwrap();
        assert_eq!(progress["phase"], "exited");
        assert!(progress["crashReport"].is_null());
        assert!(progress["crashError"].is_null());
    }
}
