use crate::contracts::{ApiError, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Java {
    pub path: String,
    pub version: String,
    pub major: u32,
    pub architecture: String,
}
pub fn parse(path: &Path, output: &str) -> Option<Java> {
    let property = |name: &str| {
        output.lines().find_map(|line| {
            let (key, value) = line.trim().split_once('=')?;
            (key.trim() == name).then(|| value.trim().to_string())
        })
    };
    let version = property("java.version")?;
    let parts: Vec<_> = version.split(['.', '-', '+', '_']).collect();
    let mut major = parts.first()?.parse::<u32>().ok()?;
    if major == 1 {
        major = parts.get(1)?.parse().ok()?;
    }
    let architecture = match property("os.arch")?.as_str() {
        "amd64" | "x86_64" | "x64" => "x64",
        "aarch64" | "arm64" => "arm64",
        "x86" | "i386" | "i686" => "x86",
        _ => return None,
    };
    Some(Java {
        path: path.to_string_lossy().into(),
        version,
        major,
        architecture: architecture.into(),
    })
}
pub async fn inspect(path: &Path) -> Result<Java> {
    if !path.is_absolute() || !path.is_file() {
        return Err(ApiError::new("JAVA_MISSING", "Java 可执行文件不存在。"));
    }
    let path = if path
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("javaw.exe"))
    {
        path.with_file_name("java.exe")
    } else {
        path.to_path_buf()
    };
    let mut command = tokio::process::Command::new(&path);
    command
        .args(["-XshowSettings:properties", "-version"])
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = output(command).await?;
    if !output.status.success() || output.stderr.len() + output.stdout.len() > 1024 * 1024 {
        return Err(ApiError::new("INVALID_JAVA", "无法识别此 Java 运行时。"));
    }
    parse(
        &path,
        &format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
    .ok_or_else(|| ApiError::new("INVALID_JAVA", "无法识别 Java 版本或架构。"))
}
async fn output(mut command: tokio::process::Command) -> Result<std::process::Output> {
    use tokio::io::AsyncReadExt;
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn()?;
    async fn read(input: impl tokio::io::AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        input.take(512 * 1024 + 1).read_to_end(&mut bytes).await?;
        if bytes.len() > 512 * 1024 {
            return Err(std::io::Error::other("probe output limit"));
        }
        Ok(bytes)
    }
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        let (stdout, stderr, status) = tokio::try_join!(read(stdout), read(stderr), child.wait())?;
        Ok(std::process::Output {
            status,
            stdout,
            stderr,
        })
    })
    .await
    .map_err(|_| ApiError::new("JAVA_TIMEOUT", "Java 探测超时。"))?
}
pub async fn scan(saved: Vec<String>) -> Vec<Java> {
    let executable = if cfg!(windows) { "java.exe" } else { "java" };
    let mut paths: BTreeSet<PathBuf> = saved.into_iter().map(PathBuf::from).collect();
    for name in ["JAVA_HOME", "JDK_HOME"] {
        if let Some(home) = std::env::var_os(name) {
            paths.insert(PathBuf::from(home).join("bin").join(executable));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path).map(|p| p.join(executable)));
    }
    fn under(root: &Path, depth: u32, executable: &str, paths: &mut BTreeSet<PathBuf>) {
        if depth == 0 || paths.len() > 512 || linked(root) {
            return;
        }
        let own = root.join("bin").join(executable);
        if own.is_file() {
            paths.insert(own);
        }
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.take(100).flatten() {
                if entry.path().is_dir() {
                    under(&entry.path(), depth - 1, executable, paths);
                }
            }
        }
    }
    let mut roots = vec![
        PathBuf::from("/usr/lib/jvm"),
        PathBuf::from("/Library/Java/JavaVirtualMachines"),
    ];
    for name in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = std::env::var_os(name) {
            for vendor in [
                "Java",
                "Eclipse Adoptium",
                "Eclipse Foundation",
                "Microsoft",
                "Zulu",
                "BellSoft",
                "Amazon Corretto",
                "Semeru",
            ] {
                roots.push(PathBuf::from(&root).join(vendor));
            }
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        roots.push(PathBuf::from(&home).join(".jdks"));
        roots.push(PathBuf::from(home).join(".minecraft/runtime"));
    }
    if let Some(root) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(root).join(".minecraft/runtime"));
    }
    if let Some(root) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(root).join("Programs/Eclipse Adoptium"));
    }
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        for hive in ["HKLM", "HKCU"] {
            for architecture in ["32", "64"] {
                let mut command =
                    tokio::process::Command::new(PathBuf::from(&root).join("System32/reg.exe"));
                command.args([
                    "query",
                    &format!("{hive}\\SOFTWARE\\JavaSoft"),
                    "/s",
                    "/v",
                    "JavaHome",
                    &format!("/reg:{architecture}"),
                ]);
                if let Ok(result) = output(command).await {
                    for line in String::from_utf8_lossy(&result.stdout).lines() {
                        if let Some((_, home)) = line.split_once("REG_SZ") {
                            roots.push(PathBuf::from(home.trim()));
                        }
                    }
                }
            }
        }
    }
    for root in roots {
        under(&root, 6, executable, &mut paths);
    }
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
    let mut tasks = tokio::task::JoinSet::new();
    for path in paths.into_iter().take(512).filter(|p| p.is_file()) {
        let semaphore = semaphore.clone();
        tasks.spawn(async move {
            let _permit = semaphore.acquire().await.ok()?;
            inspect(&path).await.ok()
        });
    }
    let mut found = Vec::new();
    while let Some(Ok(value)) = tasks.join_next().await {
        if let Some(java) = value {
            found.push(java);
        }
    }
    found.sort_by(|a, b| b.major.cmp(&a.major).then(a.path.cmp(&b.path)));
    found
}

fn linked(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}
#[cfg(test)]
mod tests {
    #[test]
    fn parses_versions_and_rejects_unknown_architecture() {
        let java = super::parse(
            std::path::Path::new("C:/Java/bin/java.exe"),
            "java.version = 21.0.5\nos.arch = amd64",
        )
        .unwrap();
        assert_eq!(java.major, 21);
        assert_eq!(java.architecture, "x64");
        assert_eq!(
            super::parse(
                std::path::Path::new("java"),
                "java.version = 1.8.0_402\nos.arch = x86"
            )
            .unwrap()
            .major,
            8
        );
        assert!(
            super::parse(
                std::path::Path::new("java"),
                "java.version = 21\nos.arch = other"
            )
            .is_none()
        );
    }
}
