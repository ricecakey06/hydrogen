mod analysis;

use hydrogen_plugin_services::{
    contracts::{ApiError, Result},
    storage, text,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const LOG_LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    id: String,
    created_at: u64,
    exit_code: i64,
    instance_name: String,
    game_version: String,
    java_version: String,
    java_architecture: String,
    max_memory_mib: u64,
    log_path: String,
    causes: Vec<analysis::Cause>,
    files: Vec<String>,
    notes: Vec<String>,
}

fn id(p: &Value) -> Result<&str> {
    let id = text(p, "taskId")?;
    if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::new("INVALID_TASK", "崩溃报告任务编号无效。"));
    }
    Ok(id)
}

fn checked(path: &Path) -> Result<()> {
    if !path.is_absolute() || path.ancestors().any(storage::linked) {
        return Err(ApiError::new(
            "INVALID_PATH",
            "报告路径必须为不含链接的绝对路径。",
        ));
    }
    Ok(())
}

struct Redactor {
    paths: Vec<(Regex, &'static str)>,
    credentials: Regex,
    bearer: Regex,
}
impl Redactor {
    fn new(root: &Path, game: &Path) -> Self {
        let mut paths = vec![
            (root.to_string_lossy().into_owned(), "[launcher]"),
            (game.to_string_lossy().into_owned(), "[game]"),
        ];
        if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            paths.push((home.to_string_lossy().into_owned(), "[user]"));
        }
        paths.sort_by_key(|(path, _)| std::cmp::Reverse(path.len()));
        Self {
            paths: paths.into_iter().filter(|(path, _)| !path.is_empty()).map(|(path, label)| {
                (Regex::new(&format!("(?i){}", regex::escape(&path).replace(r"\\", r"[\\/]"))).unwrap(), label)
            }).collect(),
            credentials: Regex::new(r#"(?i)((?:--)?(?:access[_-]?token|refresh[_-]?token|client[_-]?token|id[_-]?token|authorization|xuid|clientsecret)["']?\s*(?:[:=]\s*|\s+)["']?)[^\s"',;\]}]+"#).unwrap(),
            bearer: Regex::new(r"(?i)(bearer\s+)[A-Za-z0-9._~+/=-]+").unwrap(),
        }
    }
    fn clean(&self, input: &str) -> String {
        let mut output = self
            .bearer
            .replace_all(input, "${1}[redacted]")
            .into_owned();
        output = self
            .credentials
            .replace_all(&output, "${1}[redacted]")
            .into_owned();
        for (pattern, label) in &self.paths {
            output = pattern.replace_all(&output, *label).into_owned();
        }
        output
    }
}

fn read_tail(path: &Path) -> Result<(String, bool)> {
    checked(path)?;
    let mut file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(ApiError::new("LOG_MISSING", "日志不是普通文件。"));
    }
    let length = file.metadata()?.len();
    let truncated = length > LOG_LIMIT;
    if truncated {
        file.seek(SeekFrom::End(-(LOG_LIMIT as i64)))?;
    }
    let mut bytes = Vec::new();
    file.take(LOG_LIMIT).read_to_end(&mut bytes)?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), truncated))
}

fn recent(directory: &Path, started: u64, jvm: bool) -> Vec<PathBuf> {
    if checked(directory).is_err() {
        return vec![];
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return vec![];
    };
    let mut paths: Vec<_> = entries
        .take(4096)
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !(if jvm {
                name.starts_with("hs_err_pid") && name.ends_with(".log")
            } else {
                name.starts_with("crash-") && name.ends_with(".txt")
            }) {
                return None;
            }
            let path = entry.path();
            checked(&path).ok()?;
            let metadata = entry.metadata().ok()?;
            let modified = metadata
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_millis() as u64;
            (metadata.is_file() && modified >= started).then_some((modified, path))
        })
        .collect();
    paths.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
    paths.into_iter().take(2).map(|(_, path)| path).collect()
}

fn analyze(p: &Value) -> Result<Value> {
    let task_id = id(p)?;
    let root = Path::new(text(p, "root")?);
    let game = Path::new(text(p, "directory")?);
    checked(game)?;
    let exit_code = p["exitCode"]
        .as_i64()
        .ok_or_else(|| ApiError::new("INVALID_PARAMS", "缺少退出码。"))?;
    if exit_code == 0 {
        return Ok(Value::Null);
    }
    let started = p["startedAt"]
        .as_u64()
        .ok_or_else(|| ApiError::new("INVALID_PARAMS", "缺少本次启动时间。"))?;
    let console = storage::within(root, &format!("logs/game-{task_id}.log"))?;
    let report_dir = storage::within(root, &format!("crash-reports/{task_id}"))?;
    // Each task owns an immutable snapshot; later launches cannot change its exported logs.
    fs::create_dir(&report_dir).or_else(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            fs::create_dir_all(report_dir.parent().unwrap())?;
            fs::create_dir(&report_dir)
        } else {
            Err(error)
        }
    })?;
    let redactor = Redactor::new(root, game);
    let mut sources = recent(&game.join("crash-reports"), started, false);
    sources.extend(recent(game, started, true));
    let latest = game.join("logs/latest.log");
    if checked(&latest).is_ok()
        && fs::metadata(&latest)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .is_some_and(|t| t.as_millis() >= started as u128)
    {
        sources.push(latest);
    }
    sources.push(console.clone());
    let mut report = Report {
        id: task_id.into(),
        created_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        exit_code,
        instance_name: p["instanceName"].as_str().unwrap_or("").into(),
        game_version: p["gameVersion"].as_str().unwrap_or("").into(),
        java_version: p["javaVersion"].as_str().unwrap_or("").into(),
        java_architecture: p["javaArchitecture"].as_str().unwrap_or("").into(),
        max_memory_mib: p["maxMemoryMib"].as_u64().unwrap_or(0),
        log_path: console.to_string_lossy().into_owned(),
        causes: vec![],
        files: vec![],
        notes: vec![],
    };
    let mut logs = Vec::new();
    for (index, path) in sources.iter().enumerate() {
        match read_tail(path) {
            Ok((content, truncated)) => {
                if report.files.is_empty() {
                    report.log_path = path.to_string_lossy().into_owned();
                }
                let content = redactor.clean(&content);
                let name = format!("log-{index}.txt");
                fs::write(report_dir.join(&name), &content)?;
                report.files.push(name.clone());
                report.notes.push(format!(
                    "{name}：{}",
                    redactor.clean(&path.file_name().unwrap_or_default().to_string_lossy())
                ));
                if truncated {
                    report.notes.push(format!("{name} 仅包含末尾 2 MiB。"));
                }
                logs.push(content);
            }
            Err(_) => report
                .notes
                .push(format!("第 {} 份日志不可读取，已跳过。", index + 1)),
        }
    }
    report.causes = analysis::analyze(&logs);
    storage::write_json(&report_dir.join("report.json"), &report)?;
    Ok(serde_json::to_value(report)?)
}

fn export(p: &Value) -> Result<Value> {
    let task_id = id(p)?;
    let root = Path::new(text(p, "root")?);
    let destination = Path::new(text(p, "destination")?);
    checked(destination)?;
    if !destination.is_dir() {
        return Err(ApiError::new("DIRECTORY_MISSING", "导出目录不存在。"));
    }
    let directory = storage::within(root, &format!("crash-reports/{task_id}"))?;
    let mut report: Report = storage::read_json(&storage::within(&directory, "report.json")?)?;
    if report.id != task_id || report.files.len() > 6 {
        return Err(ApiError::new("INVALID_REPORT", "崩溃报告内容无效。"));
    }
    // The UI needs the original path; shared archives only need a local-path placeholder.
    report.log_path = "[local log]".into();
    let path = destination.join(format!(
        "Hydrogen-crash-{task_id}-{}.zip",
        uuid::Uuid::new_v4().simple()
    ));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let result = (|| -> Result<()> {
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let zip_error =
            |_| ApiError::new("REPORT_EXPORT", "无法生成崩溃报告压缩包。请检查磁盘空间。");
        let mut summary = format!(
            "Hydrogen 游戏崩溃报告\n实例：{}\nMinecraft：{}\nJava：{} {}\n最大内存：{} MiB\n退出码：{}\n\n以下结论是日志规则匹配得到的可能原因。\n",
            report.instance_name,
            report.game_version,
            report.java_version,
            report.java_architecture,
            report.max_memory_mib,
            report.exit_code
        );
        for cause in &report.causes {
            summary.push_str(&format!("\n{}\n{}\n", cause.title, cause.summary));
            for suggestion in &cause.suggestions {
                summary.push_str(&format!("- {suggestion}\n"));
            }
        }
        for note in &report.notes {
            summary.push_str(&format!("\n{note}\n"));
        }
        summary.push_str(
            "\n日志中的常见令牌及本机路径已脱敏，分享前仍建议检查模组自行输出的私人内容。\n",
        );
        let redactor = Redactor::new(root, root);
        archive
            .start_file("summary.txt", options)
            .map_err(zip_error)?;
        archive.write_all(redactor.clean(&summary).as_bytes())?;
        archive
            .start_file("report.json", options)
            .map_err(zip_error)?;
        archive.write_all(
            redactor
                .clean(&serde_json::to_string_pretty(&report)?)
                .as_bytes(),
        )?;
        let log_name = Regex::new(r"^log-[0-5]\.txt$").unwrap();
        for name in &report.files {
            if !log_name.is_match(name) {
                return Err(ApiError::new("INVALID_REPORT", "报告包含无效日志路径。"));
            }
            let (content, _) = read_tail(&storage::within(&directory, name)?)?;
            archive.start_file(name, options).map_err(zip_error)?;
            archive.write_all(content.as_bytes())?;
        }
        archive.finish().map_err(zip_error)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&path);
    }
    result?;
    Ok(json!({"path":path}))
}

fn dispatch(_: &mut (), request: &mut Value) -> hydrogen_plugin_sdk::Result {
    let result = match request["method"].as_str().unwrap_or("") {
        "plugin.describe" => Ok(json!({"id":"minecraft.crash","abiVersion":1})),
        "analyze" => analyze(&request["params"]),
        "export" => export(&request["params"]),
        _ => Err(ApiError::new("UNKNOWN_METHOD", "未知崩溃分析方法。")),
    };
    result.map_err(|error| (error.code, error.message))
}
hydrogen_plugin_sdk::export_plugin!((), dispatch);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redacts_command_json_bearer_and_local_paths() {
        let redactor = Redactor::new(Path::new(r"C:\Launcher"), Path::new(r"C:\Games\Minecraft"));
        let input = r#"--accessToken secret-value {"refresh_token":"another-secret"} Authorization: Bearer bearer-secret C:\Games\Minecraft\mods C:/Launcher/logs"#;
        let output = redactor.clean(input);
        for secret in [
            "secret-value",
            "another-secret",
            "bearer-secret",
            "C:\\Games",
            "C:/Launcher",
        ] {
            assert!(!output.contains(secret), "{output}");
        }
    }
    #[test]
    fn normal_exit_does_not_create_a_report() {
        let root = tempfile::tempdir().unwrap();
        let result = analyze(&json!({"root":root.path(),"directory":root.path(),"taskId":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","exitCode":0})).unwrap();
        assert!(result.is_null());
        assert!(!root.path().join("crash-reports").exists());
    }
    #[test]
    fn reading_large_logs_is_bounded_and_keeps_the_last_exception() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("large.log");
        let mut file = File::create(&path).unwrap();
        file.set_len(LOG_LIMIT * 2).unwrap();
        file.seek(SeekFrom::End(0)).unwrap();
        file.write_all(b"OutOfMemoryError: Java heap space")
            .unwrap();
        drop(file);
        let (content, truncated) = read_tail(&path).unwrap();
        assert!(truncated);
        assert!(content.len() <= LOG_LIMIT as usize);
        assert!(content.ends_with("OutOfMemoryError: Java heap space"));
    }
}
