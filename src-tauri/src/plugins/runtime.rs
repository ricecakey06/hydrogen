use super::manifest::{self, MAX_MESSAGE, MAX_SOURCE, Plugin};
use crate::contracts::{ApiError, InstanceInfo, Result};
use rquickjs::{
    Context, Ctx, Function, Module, Promise, Runtime, Value,
    loader::{ImportAttributes, Loader, Resolver},
    module::Declared,
};
use serde::Serialize;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

const HEAP_LIMIT: usize = 8 * 1024 * 1024;
const TOTAL_HEAP_LIMIT: usize = 64 * 1024 * 1024;

struct PackageResolver {
    root: PathBuf,
    resolutions: usize,
}
impl Resolver for PackageResolver {
    fn resolve<'js>(
        &mut self,
        _: &Ctx<'js>,
        base: &str,
        name: &str,
        _: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        self.resolutions += 1;
        if self.resolutions > 64 || !name.starts_with("./") {
            return Err(rquickjs::Error::new_resolving_message(
                base,
                name,
                "Only package-relative ./ JavaScript imports are available; module limit is 64.",
            ));
        }
        let parent = base.rsplit_once('/').map_or("", |(p, _)| p);
        let path = if parent.is_empty() {
            name[2..].to_owned()
        } else {
            format!("{parent}/{}", &name[2..])
        };
        if !path.ends_with(".js")
            || path.split('/').count() > 32
            || manifest::within(&self.root, &path).is_err()
        {
            return Err(rquickjs::Error::new_resolving_message(
                base,
                name,
                "Import is outside the plugin package or too deep.",
            ));
        }
        Ok(path)
    }
}
struct PackageLoader {
    root: PathBuf,
    bytes: u64,
}
impl Loader for PackageLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        let path =
            manifest::within(&self.root, name).map_err(|_| rquickjs::Error::new_loading(name))?;
        let bytes = fs::read(path).map_err(|_| rquickjs::Error::new_loading(name))?;
        self.bytes += bytes.len() as u64;
        if self.bytes > MAX_SOURCE {
            return Err(rquickjs::Error::new_loading_message(
                name,
                "Source limit exceeded",
            ));
        }
        Module::declare(ctx.clone(), name, bytes)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    pub plugin_id: String,
    pub generation: u64,
    pub result: serde_json::Value,
    pub notifications: Vec<String>,
    pub heap_bytes: usize,
    pub registrations: usize,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStats {
    pub active: usize,
    pub created: usize,
    pub destroyed: usize,
    pub registrations: usize,
    pub heap_bytes: usize,
}

#[derive(Default)]
struct Counters {
    created: AtomicUsize,
    destroyed: AtomicUsize,
}

struct Active {
    // Context must be released before its owning Runtime.
    context: Context,
    runtime: Runtime,
    plugin: Plugin,
    generation: u64,
    deadline: Arc<Mutex<Instant>>,
    alive: Arc<AtomicBool>,
    responses: mpsc::Receiver<(u32, Result<String>)>,
    tasks: Rc<RefCell<Vec<tokio::task::JoinHandle<()>>>>,
    notices: Arc<Mutex<Vec<String>>>,
    counters: Arc<Counters>,
}

impl Active {
    fn new(
        plugin: Plugin,
        generation: u64,
        handle: tokio::runtime::Handle,
        instances: Arc<Mutex<Vec<InstanceInfo>>>,
        counters: Arc<Counters>,
    ) -> Result<Self> {
        let runtime =
            Runtime::new().map_err(|_| ApiError::new("ENGINE_INIT", "无法创建脚本运行时。"))?;
        runtime.set_memory_limit(HEAP_LIMIT);
        runtime.set_max_stack_size(256 * 1024);
        runtime.set_loader(
            PackageResolver {
                root: plugin.root.clone(),
                resolutions: 0,
            },
            PackageLoader {
                root: plugin.root.clone(),
                bytes: 0,
            },
        );
        let deadline = Arc::new(Mutex::new(Instant::now() + Duration::from_millis(100)));
        let interrupt = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            Instant::now() >= *interrupt.lock().unwrap()
        })));
        let context = Context::full(&runtime)
            .map_err(|_| ApiError::new("ENGINE_INIT", "无法创建脚本上下文。"))?;
        let alive = Arc::new(AtomicBool::new(true));
        let (sender, responses) = mpsc::channel();
        let tasks = Rc::new(RefCell::new(Vec::new()));
        let notices = Arc::new(Mutex::new(Vec::<String>::new()));
        let permissions = plugin.manifest.permissions.clone();
        let tasks_copy = tasks.clone();
        let alive_copy = alive.clone();
        let notices_copy = notices.clone();
        let pending = Arc::new(AtomicUsize::new(0));
        context
            .with(|ctx| -> rquickjs::Result<()> {
                let host = Function::new(
                    ctx.clone(),
                    move |id: u32, method: String, params: String| -> bool {
                        if !alive_copy.load(Ordering::Acquire)
                            || params.len() > MAX_MESSAGE
                            || pending.load(Ordering::Relaxed) >= 8
                        {
                            return false;
                        }
                        let permission = match method.as_str() {
                            "instances.list" => "instances.read",
                            "ui.notify" => "ui.contribute",
                            _ => "denied",
                        };
                        let response = if !permissions.iter().any(|p| p == permission) {
                            Err(ApiError::new("PERMISSION_DENIED", "插件未声明此能力。"))
                        } else if method == "instances.list" {
                            serde_json::to_string(&*instances.lock().unwrap())
                                .map_err(ApiError::from)
                        } else {
                            let value: serde_json::Value =
                                serde_json::from_str(&params).unwrap_or_default();
                            match value
                                .get("message")
                                .and_then(|v| v.as_str())
                                .filter(|s| s.len() <= 4096)
                            {
                                Some(message) if notices_copy.lock().unwrap().len() < 32 => {
                                    notices_copy.lock().unwrap().push(message.into());
                                    Ok("null".into())
                                }
                                _ => Err(ApiError::new("INVALID_MESSAGE", "通知无效或超过配额。")),
                            }
                        };
                        let sender = sender.clone();
                        let alive = alive_copy.clone();
                        let pending = pending.clone();
                        pending.fetch_add(1, Ordering::Relaxed);
                        tasks_copy
                            .borrow_mut()
                            .retain(|t: &tokio::task::JoinHandle<()>| !t.is_finished());
                        tasks_copy.borrow_mut().push(handle.spawn(async move {
                            // The P0 adapter deliberately crosses a real asynchronous boundary on the shared executor.
                            tokio::task::yield_now().await;
                            if alive.load(Ordering::Acquire) {
                                let _ = sender.send((id, response));
                            }
                            pending.fetch_sub(1, Ordering::Relaxed);
                        }));
                        true
                    },
                )?;
                ctx.globals().set("__hostRequest", host)?;
                ctx.globals().set(
                    "__declared",
                    serde_json::to_string(
                        &plugin
                            .manifest
                            .contributes
                            .commands
                            .iter()
                            .map(|c| &c.id)
                            .collect::<Vec<_>>(),
                    )
                    .unwrap(),
                )?;
                ctx.eval::<(), _>(include_str!("bootstrap.js"))
            })
            .map_err(|_| ApiError::new("ENGINE_INIT", "无法初始化插件 SDK。"))?;
        counters.created.fetch_add(1, Ordering::Relaxed);
        let active = Self {
            context,
            runtime,
            plugin,
            generation,
            deadline,
            alive,
            responses,
            tasks,
            notices,
            counters,
        };
        let entry = serde_json::to_string(&format!("./{}", active.plugin.manifest.entry)).unwrap();
        active.execute(
            &format!("import {{ activate }} from {entry}; await activate(globalThis.__context);"),
            true,
        )?;
        Ok(active)
    }

    fn renew_deadline(&self) {
        *self.deadline.lock().unwrap() = Instant::now() + Duration::from_millis(100);
    }

    fn execute(&self, source: &str, module: bool) -> Result<serde_json::Value> {
        self.renew_deadline();
        self.context.with(|ctx| {
            let operation = || -> rquickjs::Result<serde_json::Value> {
                let promise: Promise = if module {
                    Module::evaluate(ctx.clone(), "__activation.js", source)?
                } else {
                    ctx.eval(source)?
                };
                let operation_deadline = Instant::now() + Duration::from_secs(3);
                loop {
                    if let Some(value) = promise.result::<Value>() {
                        let value = value?;
                        if module {
                            return Ok(serde_json::Value::Null);
                        }
                        let json: String = rquickjs::FromJs::from_js(&ctx, value)?;
                        if json.len() > MAX_MESSAGE {
                            return Err(rquickjs::Error::new_from_js_message(
                                "result",
                                "JSON",
                                "Message exceeds 256 KiB",
                            ));
                        }
                        return serde_json::from_str(&json)
                            .map_err(|_| rquickjs::Error::new_from_js("result", "JSON"));
                    }
                    if Instant::now() > operation_deadline {
                        return Err(rquickjs::Error::WouldBlock);
                    }
                    while ctx.execute_pending_job() {
                        if Instant::now() >= *self.deadline.lock().unwrap() {
                            return Err(rquickjs::Error::WouldBlock);
                        }
                    }
                    if let Ok((id, result)) = self.responses.recv_timeout(Duration::from_millis(1))
                    {
                        self.renew_deadline();
                        let settle: Function = ctx.globals().get("__settle")?;
                        let (success, json) = match result {
                            Ok(json) => (true, json),
                            Err(error) => (false, serde_json::to_string(&error).unwrap()),
                        };
                        settle.call::<_, ()>((id, success, json))?;
                    }
                }
            };
            operation().map_err(|error| {
                // Do not put arbitrary exception values (which can contain user data) in diagnostics.
                let _ = ctx.catch();
                ApiError::new(
                    if matches!(error, rquickjs::Error::WouldBlock)
                        || Instant::now() >= *self.deadline.lock().unwrap()
                    {
                        "SCRIPT_TIMEOUT"
                    } else {
                        "SCRIPT_ERROR"
                    },
                    format!(
                        "插件 {} 执行失败：{}。请检查语法、权限或资源配额。",
                        self.plugin.manifest.id, error
                    ),
                )
            })
        })
    }

    fn call(&self, command: &str) -> Result<RunReport> {
        if !self
            .plugin
            .manifest
            .contributes
            .commands
            .iter()
            .any(|c| c.id == command)
        {
            return Err(ApiError::new(
                "UNKNOWN_COMMAND",
                "命令未在此插件清单中声明。",
            ));
        }
        self.notices.lock().unwrap().clear();
        let command = serde_json::to_string(command).unwrap();
        let result = self.execute(&format!("Promise.resolve().then(() => __invoke({command})).then(value => JSON.stringify(value ?? null))"), false)?;
        self.runtime.run_gc();
        Ok(RunReport {
            plugin_id: self.plugin.manifest.id.clone(),
            generation: self.generation,
            result,
            notifications: self.notices.lock().unwrap().clone(),
            heap_bytes: self.heap(),
            registrations: self.plugin.manifest.contributes.commands.len(),
        })
    }
    fn heap(&self) -> usize {
        self.runtime.memory_usage().memory_used_size.max(0) as usize
    }
}
impl Drop for Active {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        for task in self.tasks.borrow_mut().drain(..) {
            task.abort();
        }
        self.counters.destroyed.fetch_add(1, Ordering::Relaxed);
    }
}

enum Request {
    Run {
        plugin: Box<Plugin>,
        command: Option<String>,
        reload: bool,
        commit: Option<Box<dyn FnOnce() -> Result<()> + Send>>,
        reply: oneshot::Sender<Result<RunReport>>,
    },
    Stop {
        ids: Option<Vec<String>>,
        reply: oneshot::Sender<RuntimeStats>,
    },
}

#[derive(Clone)]
pub struct ScriptHost {
    sender: mpsc::SyncSender<Request>,
    instances: Arc<Mutex<Vec<InstanceInfo>>>,
}
impl ScriptHost {
    pub fn new(handle: tokio::runtime::Handle, instances: Vec<InstanceInfo>) -> Self {
        let instances = Arc::new(Mutex::new(instances));
        let current_instances = instances.clone();
        let (sender, receiver) = mpsc::sync_channel::<Request>(32);
        std::thread::Builder::new()
            .name("hydrogen-scripts".into())
            .spawn(move || {
                let mut active = BTreeMap::<String, Active>::new();
                let counters = Arc::new(Counters::default());
                let mut generation = 0;
                for request in receiver {
                    match request {
                        Request::Run {
                            plugin,
                            command,
                            reload,
                            commit,
                            reply,
                        } => {
                            let id = plugin.manifest.id.clone();
                            let needs_activation = reload || !active.contains_key(&id);
                            if !needs_activation && active[&id].plugin.digest != plugin.digest {
                                let _ = reply.send(Err(ApiError::new(
                                    "PLUGIN_CHANGED",
                                    "开发目录已更改，请先校验并重载。",
                                )));
                                continue;
                            }
                            if needs_activation {
                                generation += 1;
                                let heap: usize = active.values().map(Active::heap).sum();
                                if heap + HEAP_LIMIT > TOTAL_HEAP_LIMIT {
                                    let _ = reply.send(Err(ApiError::new(
                                        "SCRIPT_BUDGET",
                                        "脚本总内存预算不足。",
                                    )));
                                    continue;
                                }
                                match Active::new(
                                    *plugin,
                                    generation,
                                    handle.clone(),
                                    current_instances.clone(),
                                    counters.clone(),
                                ) {
                                    Ok(candidate) => {
                                        if let Some(commit) = commit
                                            && let Err(error) = commit()
                                        {
                                            let _ = reply.send(Err(error));
                                            continue;
                                        }
                                        active.insert(id.clone(), candidate);
                                    }
                                    Err(error) => {
                                        let _ = reply.send(Err(error));
                                        continue;
                                    }
                                }
                            }
                            let other_heap: usize = active
                                .iter()
                                .filter(|(key, _)| **key != id)
                                .map(|(_, v)| v.heap())
                                .sum();
                            active[&id]
                                .runtime
                                .set_memory_limit(HEAP_LIMIT.min(TOTAL_HEAP_LIMIT - other_heap));
                            let result = if let Some(command) = command {
                                active[&id].call(&command)
                            } else {
                                Ok(RunReport {
                                    plugin_id: id.clone(),
                                    generation: active[&id].generation,
                                    result: serde_json::Value::Null,
                                    notifications: Vec::new(),
                                    heap_bytes: active[&id].heap(),
                                    registrations: active[&id]
                                        .plugin
                                        .manifest
                                        .contributes
                                        .commands
                                        .len(),
                                })
                            };
                            if result.is_err() {
                                active.remove(&id);
                            }
                            let _ = reply.send(result);
                        }
                        Request::Stop { ids, reply } => {
                            active
                                .retain(|id, _| ids.as_ref().is_some_and(|ids| !ids.contains(id)));
                            let _ = reply.send(RuntimeStats {
                                active: active.len(),
                                created: counters.created.load(Ordering::Relaxed),
                                destroyed: counters.destroyed.load(Ordering::Relaxed),
                                registrations: active
                                    .values()
                                    .map(|v| v.plugin.manifest.contributes.commands.len())
                                    .sum(),
                                heap_bytes: active.values().map(Active::heap).sum(),
                            });
                        }
                    }
                }
            })
            .expect("script scheduler thread");
        Self { sender, instances }
    }
    pub fn set_instances(&self, instances: Vec<InstanceInfo>) {
        *self.instances.lock().unwrap() = instances;
    }
    pub async fn run(&self, plugin: Plugin, command: String, reload: bool) -> Result<RunReport> {
        self.dispatch(plugin, Some(command), reload, None).await
    }
    pub async fn activate(&self, plugin: Plugin) -> Result<RunReport> {
        self.dispatch(plugin, None, false, None).await
    }
    pub async fn reload(&self, plugin: Plugin) -> Result<RunReport> {
        self.dispatch(plugin, None, true, None).await
    }
    pub async fn reload_with_commit(
        &self,
        plugin: Plugin,
        commit: impl FnOnce() -> Result<()> + Send + 'static,
    ) -> Result<RunReport> {
        self.dispatch(plugin, None, true, Some(Box::new(commit)))
            .await
    }
    async fn dispatch(
        &self,
        plugin: Plugin,
        command: Option<String>,
        reload: bool,
        commit: Option<Box<dyn FnOnce() -> Result<()> + Send>>,
    ) -> Result<RunReport> {
        if plugin.manifest.runtime != "javascript" {
            return Err(ApiError::new(
                "NATIVE_PLUGIN",
                "原生入口不能由脚本引擎执行。",
            ));
        }
        let (reply, response) = oneshot::channel();
        self.sender
            .try_send(Request::Run {
                plugin: Box::new(plugin),
                command,
                reload,
                commit,
                reply,
            })
            .map_err(|_| ApiError::new("SCRIPT_BUSY", "脚本调度器繁忙或已经停止。"))?;
        response
            .await
            .map_err(|_| ApiError::new("SCRIPT_STOPPED", "脚本调度器已经停止。"))?
    }
    pub async fn stop_all(&self) -> Result<RuntimeStats> {
        self.stop(None).await
    }
    pub async fn stop_plugins(&self, ids: Vec<String>) -> Result<RuntimeStats> {
        self.stop(Some(ids)).await
    }
    async fn stop(&self, ids: Option<Vec<String>>) -> Result<RuntimeStats> {
        let (reply, response) = oneshot::channel();
        self.sender
            .try_send(Request::Stop { ids, reply })
            .map_err(|_| ApiError::new("SCRIPT_BUSY", "脚本调度器繁忙。"))?;
        response
            .await
            .map_err(|_| ApiError::new("SCRIPT_STOPPED", "脚本调度器已经停止。"))
    }
}
