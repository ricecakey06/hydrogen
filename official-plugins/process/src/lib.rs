mod identity;
use base64::{Engine, engine::general_purpose::STANDARD};
use hydrogen_plugin_sdk::{
    Result,
    serde_json::{Value, json},
};
use std::{
    collections::BTreeMap,
    io::Read,
    path::Path,
    process::{Child, Command, Stdio},
};
struct OwnedChild {
    child: Child,
    stdout_eof: bool,
    stderr_eof: bool,
}
#[derive(Default)]
struct Processes {
    children: BTreeMap<String, OwnedChild>,
}
fn dispatch(state: &mut Processes, request: &mut Value) -> Result {
    let params = &request["params"];
    match request["method"].as_str().unwrap_or("") {
        "plugin.describe" => Ok(json!({"id":"process","abiVersion":1})),
        "stamp" => Ok(json!(identity::process_stamp(
            params["pid"]
                .as_u64()
                .ok_or(("INVALID_PARAMS".into(), "缺少进程 ID。".into()))? as u32
        ))),
        "spawn" => {
            if state.children.len() >= 32 {
                return Err(("PROCESS_BUDGET".into(), "启动器跟踪的进程超过限制。".into()));
            }
            let executable = Path::new(
                params["executable"]
                    .as_str()
                    .ok_or(("INVALID_PARAMS".into(), "缺少可执行文件。".into()))?,
            );
            let directory = Path::new(
                params["directory"]
                    .as_str()
                    .ok_or(("INVALID_PARAMS".into(), "缺少工作目录。".into()))?,
            );
            if !executable.is_absolute()
                || !executable.is_file()
                || !directory.is_absolute()
                || !directory.is_dir()
            {
                return Err((
                    "PROCESS_START_FAILED".into(),
                    "可执行文件或工作目录无效。".into(),
                ));
            }
            let args: Vec<String> =
                hydrogen_plugin_sdk::serde_json::from_value(params["args"].clone())
                    .map_err(|_| ("INVALID_PARAMS".into(), "进程参数必须是字符串数组。".into()))?;
            let mut command = Command::new(executable);
            command
                .args(args)
                .current_dir(directory)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(env) = params["environment"].as_object() {
                for (name, value) in env {
                    command.env(
                        name,
                        value
                            .as_str()
                            .ok_or(("INVALID_PARAMS".into(), "环境变量必须为文本。".into()))?,
                    );
                }
            }
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }
            let mut child = command.spawn().map_err(io_error)?;
            let pid = child.id();
            let stamp = identity::process_stamp(pid);
            let setup = setup_pipe(child.stdout.as_ref().unwrap())
                .and_then(|_| setup_pipe(child.stderr.as_ref().unwrap()));
            if let Err(error) = setup {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
            let handle = uuid::Uuid::new_v4().simple().to_string();
            state.children.insert(
                handle.clone(),
                OwnedChild {
                    child,
                    stdout_eof: false,
                    stderr_eof: false,
                },
            );
            Ok(json!({"handle":handle,"pid":pid,"stamp":stamp}))
        }
        "poll" => {
            let process = state
                .children
                .get_mut(params["handle"].as_str().unwrap_or(""))
                .ok_or(("PROCESS_CLOSED".into(), "进程会话不存在。".into()))?;
            let status = process.child.try_wait().map_err(io_error)?;
            let out = read_pipe(
                process.child.stdout.as_mut().unwrap(),
                &mut process.stdout_eof,
            )?;
            let err = read_pipe(
                process.child.stderr.as_mut().unwrap(),
                &mut process.stderr_eof,
            )?;
            Ok(
                json!({"exitCode":status.map(|status| status.code().unwrap_or(-1)),"stdout":STANDARD.encode(out),"stderr":STANDARD.encode(err),"stdoutEof":process.stdout_eof,"stderrEof":process.stderr_eof}),
            )
        }
        "kill" => {
            if let Some(mut process) = state
                .children
                .remove(params["handle"].as_str().unwrap_or(""))
            {
                let _ = process.child.kill();
                let _ = process.child.wait();
            }
            Ok(Value::Null)
        }
        "detach" => {
            state
                .children
                .remove(params["handle"].as_str().unwrap_or(""));
            Ok(Value::Null)
        }
        _ => Err(("UNKNOWN_METHOD".into(), "Unknown process method".into())),
    }
}
fn io_error(_: std::io::Error) -> (String, String) {
    ("PROCESS_IO".into(), "进程操作失败。".into())
}

#[cfg(windows)]
fn setup_pipe<T>(_: &T) -> std::result::Result<(), (String, String)> {
    Ok(())
}
#[cfg(unix)]
fn setup_pipe<T: std::os::fd::AsRawFd>(pipe: &T) -> std::result::Result<(), (String, String)> {
    unsafe extern "C" {
        fn fcntl(fd: i32, command: i32, ...) -> i32;
    }
    let flags = unsafe { fcntl(pipe.as_raw_fd(), 3) };
    let nonblock = if cfg!(target_os = "macos") { 4 } else { 0x800 };
    if flags < 0 || unsafe { fcntl(pipe.as_raw_fd(), 4, flags | nonblock) } < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(())
}
#[cfg(windows)]
fn read_pipe<T: Read + std::os::windows::io::AsRawHandle>(
    pipe: &mut T,
    eof: &mut bool,
) -> std::result::Result<Vec<u8>, (String, String)> {
    use windows::Win32::{Foundation::HANDLE, System::Pipes::PeekNamedPipe};
    let mut available = 0;
    if let Err(error) = unsafe {
        PeekNamedPipe(
            HANDLE(pipe.as_raw_handle()),
            None,
            0,
            None,
            Some(&mut available),
            None,
        )
    } {
        if matches!(error.code().0 as u32 & 0xffff, 109 | 233) {
            *eof = true;
            return Ok(Vec::new());
        }
        return Err(("PROCESS_IO".into(), "无法读取进程输出。".into()));
    }
    if available == 0 {
        return Ok(Vec::new());
    }
    let mut bytes = vec![0; available.min(8192) as usize];
    let read = pipe.read(&mut bytes).map_err(io_error)?;
    bytes.truncate(read);
    if read == 0 {
        *eof = true;
    }
    Ok(bytes)
}
#[cfg(unix)]
fn read_pipe<T: Read>(
    pipe: &mut T,
    eof: &mut bool,
) -> std::result::Result<Vec<u8>, (String, String)> {
    let mut bytes = vec![0; 8192];
    match pipe.read(&mut bytes) {
        Ok(0) => {
            *eof = true;
            Ok(Vec::new())
        }
        Ok(read) => {
            bytes.truncate(read);
            Ok(bytes)
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(Vec::new()),
        Err(error) => Err(io_error(error)),
    }
}
hydrogen_plugin_sdk::export_plugin!(Processes, dispatch);
