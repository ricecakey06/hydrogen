use super::manifest::{self, MAX_MESSAGE, MAX_SOURCE, Plugin};
use crate::contracts::{ApiError, InstanceInfo, Result};
use rquickjs::{
    Context, Ctx, Function, Module, Persistent, Promise, Runtime, Value,
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

/// Bound by PluginHost; the script never supplies its own identity or generation.
pub type Bridge =
    Arc<dyn Fn(u64, &str, serde_json::Value) -> Result<serde_json::Value> + Send + Sync>;
type Invocation = (String, String, serde_json::Value);
type Commit = Box<dyn FnOnce() -> Result<()> + Send>;
type Reply = mpsc::Sender<Result<RunReport>>;

#[derive(Default)]
struct Counters {
    created: AtomicUsize,
    destroyed: AtomicUsize,
}
struct Operation {
    promise: Persistent<Promise<'static>>,
    module: bool,
    started: Instant,
    hook: bool,
}
struct Active {
    // Persistent JS values and Context must drop before Runtime.
    operation: Option<Operation>,
    suspended: Vec<(Operation, Option<Bridge>)>,
    context: Context,
    runtime: Runtime,
    plugin: Plugin,
    generation: u64,
    deadline: Arc<Mutex<Instant>>,
    alive: Arc<AtomicBool>,
    committed: Arc<AtomicBool>,
    hook: Arc<AtomicBool>,
    router: Rc<RefCell<Option<Bridge>>>,
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
        router: Option<Bridge>,
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
        let committed = Arc::new(AtomicBool::new(false));
        let hook = Arc::new(AtomicBool::new(false));
        let hook_copy = hook.clone();
        let (sender, responses) = mpsc::channel();
        let tasks = Rc::new(RefCell::new(Vec::new()));
        let notices = Arc::new(Mutex::new(Vec::<String>::new()));
        let router = Rc::new(RefCell::new(router));
        let (tasks_copy, alive_copy, notices_copy, router_copy, committed_copy) = (
            tasks.clone(),
            alive.clone(),
            notices.clone(),
            router.clone(),
            committed.clone(),
        );
        let permissions = plugin.manifest.permissions.clone();
        let pending = Arc::new(AtomicUsize::new(0));
        context.with(|ctx| -> rquickjs::Result<()> {
            let host = Function::new(ctx.clone(), move |id: u32, method: String, params: String| -> bool {
                if !alive_copy.load(Ordering::Acquire) || params.len() > MAX_MESSAGE
                    || pending.load(Ordering::Relaxed) >= 8 { return false; }
                let sender = sender.clone();
                let alive = alive_copy.clone();
                let pending = pending.clone();
                let router = router_copy.borrow().clone();
                let permissions = permissions.clone();
                let instances = instances.clone();
                let notices = notices_copy.clone();
                let committed = committed_copy.load(Ordering::Acquire);
                let hook = hook_copy.load(Ordering::Acquire);
                pending.fetch_add(1, Ordering::Relaxed);
                tasks_copy.borrow_mut().retain(|t: &tokio::task::JoinHandle<()>| !t.is_finished());
                tasks_copy.borrow_mut().push(handle.spawn(async move {
                    let work = tokio::task::spawn_blocking(move || -> Result<String> {
                        let params: serde_json::Value = serde_json::from_str(&params)?;
                        if !committed && !super::capabilities::read_only(&method) {
                            return Err(ApiError::new("ACTIVATION_SIDE_EFFECT", "激活验证期间只允许读取；请在命令或提交后的事件中执行写入。"));
                        }
                        if hook && !super::capabilities::read_only(&method) {
                            return Err(ApiError::new("HOOK_SIDE_EFFECT", "Hook 中只允许读取数据。"));
                        }
                        let value = if method == "ui.notify" {
                            if !permissions.iter().any(|p| p == "ui.contribute") {
                                return Err(ApiError::new("PERMISSION_DENIED", "插件未声明此能力。"));
                            }
                            let message = params["message"].as_str().filter(|s| s.len() <= 4096)
                                .ok_or_else(|| ApiError::new("INVALID_MESSAGE", "通知无效。"))?;
                            let mut notices = notices.lock().unwrap();
                            if notices.len() >= 32 { return Err(ApiError::new("REQUEST_LIMIT", "通知超过配额。")); }
                            notices.push(message.into());
                            serde_json::Value::Null
                        } else if let Some(router) = router {
                            router(generation, &method, params)?
                        } else if method == "instances.list" && permissions.iter().any(|p| p == "instances.read") {
                            serde_json::to_value(&*instances.lock().unwrap())?
                        } else {
                            return Err(ApiError::new("PERMISSION_DENIED", "此能力未授权或宿主没有连接业务入口。"));
                        };
                        let json = serde_json::to_string(&value)?;
                        if json.len() > MAX_MESSAGE { return Err(ApiError::new("MESSAGE_LIMIT", "响应超过 256 KiB。")); }
                        Ok(json)
                    });
                    let response = match tokio::time::timeout(Duration::from_secs(25), work).await {
                        Ok(Ok(value)) => value,
                        Ok(Err(_)) => Err(ApiError::new("HOST_ERROR", "宿主操作异常结束。")),
                        Err(_) => Err(ApiError::new("REQUEST_TIMEOUT", "宿主请求超时。")),
                    };
                    if alive.load(Ordering::Acquire) { let _ = sender.send((id, response)); }
                    pending.fetch_sub(1, Ordering::Relaxed);
                }));
                true
            })?;
            ctx.globals().set("__hostRequest", host)?;
            ctx.globals().set("__manifest", serde_json::to_string(&plugin.manifest).unwrap())?;
            ctx.eval::<(), _>(include_str!("bootstrap.js"))
        }).map_err(|_| ApiError::new("ENGINE_INIT", "无法初始化插件 SDK。"))?;
        counters.created.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            operation: None,
            suspended: Vec::new(),
            context,
            runtime,
            plugin,
            generation,
            deadline,
            alive,
            committed,
            hook,
            router,
            responses,
            tasks,
            notices,
            counters,
        })
    }
    fn renew(&self) {
        *self.deadline.lock().unwrap() = Instant::now() + Duration::from_millis(100);
    }
    fn error(&self, ctx: &Ctx<'_>, error: rquickjs::Error) -> ApiError {
        let caught = ctx.catch();
        if let Some(object) = caught.as_object()
            && let Ok(code) = object.get::<_, String>("code")
            && !code.is_empty()
            && code.len() <= 80
            && code
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        {
            return ApiError::new(
                &code,
                object
                    .get::<_, String>("message")
                    .unwrap_or_default()
                    .chars()
                    .take(4096)
                    .collect::<String>(),
            );
        }
        ApiError::new(
            if matches!(error, rquickjs::Error::WouldBlock)
                || Instant::now() >= *self.deadline.lock().unwrap()
            {
                "SCRIPT_TIMEOUT"
            } else {
                "SCRIPT_ERROR"
            },
            format!("插件 {} 执行失败：{error}", self.plugin.manifest.id),
        )
    }
    fn begin(&mut self, source: &str, module: bool) -> Result<()> {
        self.renew();
        let promise = self.context.with(|ctx| {
            let result: rquickjs::Result<Promise> = if module {
                Module::evaluate(ctx.clone(), "__activation.js", source)
            } else {
                ctx.eval(source)
            };
            result
                .map(|p| Persistent::save(&ctx, p))
                .map_err(|e| self.error(&ctx, e))
        })?;
        self.operation = Some(Operation {
            promise,
            module,
            started: Instant::now(),
            hook: false,
        });
        Ok(())
    }
    fn activate(&mut self) -> Result<()> {
        let entry = serde_json::to_string(&format!("./{}", self.plugin.manifest.entry))?;
        self.begin(
            &format!("import {{ activate }} from {entry}; await activate(globalThis.__context);"),
            true,
        )
    }
    fn invoke(&mut self, call: &Invocation) -> Result<()> {
        if call.0 == "command"
            && !self
                .plugin
                .manifest
                .contributes
                .commands
                .iter()
                .any(|c| c.id == call.1)
        {
            return Err(ApiError::new(
                "UNKNOWN_COMMAND",
                "命令未在此插件清单中声明。",
            ));
        }
        if serde_json::to_vec(&call.2)?.len() > MAX_MESSAGE {
            return Err(ApiError::new("MESSAGE_LIMIT", "参数超过限制。"));
        }
        self.notices.lock().unwrap().clear();
        let kind = serde_json::to_string(&call.0)?;
        let id = serde_json::to_string(&call.1)?;
        let params = serde_json::to_string(&call.2)?;
        let invoke = if call.0 == "event" {
            format!("__emit({id},{params})")
        } else {
            format!("__invoke({kind},{id},{params})")
        };
        self.begin(&format!("Promise.resolve().then(() => {invoke}).then(value => JSON.stringify(value ?? null))"), false)?;
        self.operation.as_mut().unwrap().hook = call.0 == "hook";
        Ok(())
    }
    fn poll(&mut self) -> Option<Result<serde_json::Value>> {
        let operation = self.operation.as_ref()?;
        self.hook.store(operation.hook, Ordering::Release);
        if operation.started.elapsed() > Duration::from_secs(if operation.module { 3 } else { 30 })
        {
            self.operation = None;
            return Some(Err(ApiError::new(
                "SCRIPT_TIMEOUT",
                "脚本请求超过时间限制。",
            )));
        }
        self.renew();
        let result = self.context.with(|ctx| {
            let poll = || -> rquickjs::Result<Option<serde_json::Value>> {
                while let Ok((id, result)) = self.responses.try_recv() {
                    let settle: Function = ctx.globals().get("__settle")?;
                    let (ok, json) = match result {
                        Ok(v) => (true, v),
                        Err(e) => (false, serde_json::to_string(&e).unwrap()),
                    };
                    settle.call::<_, ()>((id, ok, json))?;
                }
                while ctx.execute_pending_job() {
                    if Instant::now() >= *self.deadline.lock().unwrap() {
                        return Err(rquickjs::Error::WouldBlock);
                    }
                }
                let promise = operation.promise.clone().restore(&ctx)?;
                let Some(value) = promise.result::<Value>() else {
                    return Ok(None);
                };
                let value = value?;
                if operation.module {
                    return Ok(Some(serde_json::Value::Null));
                }
                let json: String = rquickjs::FromJs::from_js(&ctx, value)?;
                if json.len() > MAX_MESSAGE {
                    return Err(rquickjs::Error::new_from_js("result", "bounded JSON"));
                }
                Ok(Some(serde_json::from_str(&json).map_err(|_| {
                    rquickjs::Error::new_from_js("result", "JSON")
                })?))
            };
            poll().map_err(|e| self.error(&ctx, e))
        });
        match result {
            Ok(None) => None,
            Ok(Some(value)) => {
                self.operation = None;
                self.runtime.run_gc();
                Some(Ok(value))
            }
            Err(error) => {
                self.operation = None;
                Some(Err(error))
            }
        }
    }
    fn report(&self, result: serde_json::Value) -> RunReport {
        RunReport {
            plugin_id: self.plugin.manifest.id.clone(),
            generation: self.generation,
            result,
            notifications: self.notices.lock().unwrap().clone(),
            heap_bytes: self.heap(),
            registrations: self.plugin.manifest.contributes.commands.len()
                + self.plugin.manifest.provides.len(),
        }
    }
    fn heap(&self) -> usize {
        self.runtime.memory_usage().memory_used_size.max(0) as usize
    }
}
impl Drop for Active {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        if self.committed.load(Ordering::Acquire)
            && let Some(router) = self.router.borrow().clone()
        {
            let _ = router(self.generation, "lifecycle.stop", serde_json::Value::Null);
        }
        self.operation = None;
        self.suspended.clear();
        // Cleanup is bounded even when a disposer throws or never resolves.
        if self
            .begin("Promise.resolve(__dispose()).then(() => 'null')", false)
            .is_ok()
        {
            let until = Instant::now() + Duration::from_millis(100);
            while Instant::now() < until && self.poll().is_none() {}
        }
        self.operation = None;
        for task in self.tasks.borrow_mut().drain(..) {
            task.abort();
        }
        self.counters.destroyed.fetch_add(1, Ordering::Relaxed);
    }
}
struct Pending {
    call: Option<Invocation>,
    commit: Option<Commit>,
    reply: Reply,
}
#[derive(Default)]
struct Slot {
    suspended: Vec<Option<Pending>>,
    active: Option<Active>,
    candidate: Option<Active>,
    pending: Option<Pending>,
    events: std::collections::VecDeque<Invocation>,
}
impl Slot {
    fn heap(&self) -> usize {
        self.active.as_ref().map_or(0, Active::heap)
            + self.candidate.as_ref().map_or(0, Active::heap)
    }
    fn fail(&mut self, error: ApiError) {
        if let Some(p) = self.pending.take() {
            let _ = p.reply.send(Err(error));
        }
    }
}
enum Request {
    Run {
        plugin: Box<Plugin>,
        call: Option<Invocation>,
        reload: bool,
        commit: Option<Commit>,
        router: Option<Bridge>,
        reply: Reply,
    },
    Stop {
        ids: Option<Vec<String>>,
        reply: mpsc::Sender<RuntimeStats>,
    },
    Event {
        event: String,
        params: serde_json::Value,
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
        let (sender, receiver) = mpsc::sync_channel::<Request>(64);
        std::thread::Builder::new()
            .name("hydrogen-scripts".into())
            .spawn(move || {
                let mut slots = BTreeMap::<String, Slot>::new();
                let counters = Arc::new(Counters::default());
                let mut generation = 0;
                loop {
                    match receiver.recv_timeout(Duration::from_millis(1)) {
                        Ok(Request::Run {
                            plugin,
                            call,
                            reload,
                            commit,
                            router,
                            reply,
                        }) => {
                            let id = plugin.manifest.id.clone();
                            let heap: usize = slots.values().map(Slot::heap).sum();
                            let slot = slots.entry(id).or_default();
                            if slot.pending.is_some()
                                || slot.active.as_ref().is_some_and(|a| a.operation.is_some())
                            {
                                if !reload
                                    && slot.candidate.is_none()
                                    && call.as_ref().is_some_and(|c| c.0 == "hook")
                                    && let Some(active) = slot.active.as_mut()
                                    && active.suspended.len() < 8
                                {
                                    if let Some(operation) = active.operation.take() {
                                        active
                                            .suspended
                                            .push((operation, active.router.borrow().clone()));
                                        slot.suspended.push(slot.pending.take());
                                    }
                                } else {
                                    let _ = reply.send(Err(ApiError::new(
                                        "SCRIPT_BUSY",
                                        "此插件已有在途调用，请稍后重试。",
                                    )));
                                    continue;
                                }
                            }
                            if !reload
                                && slot
                                    .active
                                    .as_ref()
                                    .is_some_and(|a| a.plugin.digest != plugin.digest)
                            {
                                let _ = reply.send(Err(ApiError::new(
                                    "PLUGIN_CHANGED",
                                    "开发目录已变化，请先校验重载。",
                                )));
                                continue;
                            }
                            slot.pending = Some(Pending {
                                call,
                                commit,
                                reply,
                            });
                            if reload || slot.active.is_none() {
                                if heap + HEAP_LIMIT > TOTAL_HEAP_LIMIT {
                                    slot.fail(ApiError::new(
                                        "SCRIPT_BUDGET",
                                        "脚本总内存预算不足。",
                                    ));
                                    continue;
                                }
                                generation += 1;
                                let candidate = Active::new(
                                    *plugin,
                                    generation,
                                    handle.clone(),
                                    current_instances.clone(),
                                    counters.clone(),
                                    router,
                                )
                                .and_then(|mut a| {
                                    a.activate()?;
                                    Ok(a)
                                });
                                match candidate {
                                    Ok(a) => slot.candidate = Some(a),
                                    Err(e) => slot.fail(e),
                                }
                            } else {
                                let active = slot.active.as_mut().unwrap();
                                *active.router.borrow_mut() = router;
                                if let Some(call) = &slot.pending.as_ref().unwrap().call {
                                    if let Err(e) = active.invoke(call) {
                                        slot.fail(e);
                                    }
                                } else {
                                    let p = slot.pending.take().unwrap();
                                    let _ =
                                        p.reply.send(Ok(active.report(serde_json::Value::Null)));
                                }
                            }
                        }
                        Ok(Request::Stop { ids, reply }) => {
                            slots.retain(|id, slot| {
                                if ids.as_ref().is_none_or(|ids| ids.contains(id)) {
                                    slot.fail(ApiError::new("SCRIPT_STOPPED", "插件已停止。"));
                                    for p in slot.suspended.drain(..).flatten() {
                                        let _ = p.reply.send(Err(ApiError::new(
                                            "SCRIPT_STOPPED",
                                            "插件已停止。",
                                        )));
                                    }
                                    false
                                } else {
                                    true
                                }
                            });
                            let _ = reply.send(RuntimeStats {
                                active: slots.values().filter(|s| s.active.is_some()).count(),
                                created: counters.created.load(Ordering::Relaxed),
                                destroyed: counters.destroyed.load(Ordering::Relaxed),
                                registrations: slots
                                    .values()
                                    .filter_map(|s| s.active.as_ref())
                                    .map(|a| a.report(serde_json::Value::Null).registrations)
                                    .sum(),
                                heap_bytes: slots.values().map(Slot::heap).sum(),
                            });
                        }
                        Ok(Request::Event { event, params }) => {
                            for slot in slots.values_mut() {
                                if slot.events.len() < 64
                                    && slot.active.as_ref().is_some_and(|a| {
                                        a.plugin.manifest.permissions.iter().any(|p| p == "events")
                                            && (event != "theme.changed"
                                                || a.plugin.manifest.permissions.iter().any(|p| {
                                                    p == "ui.contribute" || p == "theme.contribute"
                                                }))
                                    })
                                {
                                    slot.events.push_back((
                                        "event".into(),
                                        event.clone(),
                                        params.clone(),
                                    ));
                                }
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                    let mut total_heap: usize = slots.values().map(Slot::heap).sum();
                    for slot in slots.values_mut() {
                        let before = slot.heap();
                        if let Some(candidate) = slot.candidate.as_mut() {
                            candidate.runtime.set_memory_limit(HEAP_LIMIT.min(
                                TOTAL_HEAP_LIMIT.saturating_sub(total_heap - candidate.heap()),
                            ));
                            if let Some(result) = candidate.poll() {
                                let result = result.and_then(|_| {
                                    slot.pending
                                        .as_mut()
                                        .unwrap()
                                        .commit
                                        .take()
                                        .map_or(Ok(()), |commit| commit())
                                });
                                match result {
                                    Err(e) => {
                                        slot.candidate = None;
                                        slot.fail(e);
                                    }
                                    Ok(()) => {
                                        let active = slot.candidate.take().unwrap();
                                        active.committed.store(true, Ordering::Release);
                                        slot.events.clear();
                                        if active
                                            .context
                                            .with(|ctx| ctx.eval::<bool, _>("__hasReady()"))
                                            .unwrap_or(false)
                                        {
                                            slot.events.push_back((
                                                "event".into(),
                                                "__ready".into(),
                                                serde_json::Value::Null,
                                            ));
                                        }
                                        slot.active = Some(active);
                                        let active = slot.active.as_mut().unwrap();
                                        if let Some(call) = &slot.pending.as_ref().unwrap().call {
                                            if let Err(e) = active.invoke(call) {
                                                slot.fail(e);
                                            }
                                        } else {
                                            let p = slot.pending.take().unwrap();
                                            let _ = p
                                                .reply
                                                .send(Ok(active.report(serde_json::Value::Null)));
                                        }
                                    }
                                }
                            }
                        } else if let Some(active) = slot.active.as_mut() {
                            active.runtime.set_memory_limit(
                                HEAP_LIMIT.min(
                                    TOTAL_HEAP_LIMIT.saturating_sub(total_heap - active.heap()),
                                ),
                            );
                            if let Some(result) = active.poll() {
                                if let Some(p) = slot.pending.take() {
                                    let failed = result.is_err();
                                    let _ = p.reply.send(result.map(|value| active.report(value)));
                                    if let Some((operation, router)) = active.suspended.pop() {
                                        active.operation = Some(operation);
                                        *active.router.borrow_mut() = router;
                                        slot.pending = slot.suspended.pop().flatten();
                                    } else if failed {
                                        slot.active = None;
                                    }
                                }
                            }
                            if slot.pending.is_none()
                                && let Some(active) = slot.active.as_mut()
                                && active.operation.is_none()
                                && let Some(event) = slot.events.pop_front()
                            {
                                let _ = active.invoke(&event);
                            }
                        }
                        total_heap = total_heap - before + slot.heap();
                    }
                }
            })
            .expect("script scheduler thread");
        Self { sender, instances }
    }
    pub fn set_instances(&self, instances: Vec<InstanceInfo>) {
        let mut previous = self.instances.lock().unwrap();
        if serde_json::to_value(&*previous).ok() != serde_json::to_value(&instances).ok() {
            self.emit(
                "instance.changed",
                serde_json::json!({"instances": instances}),
            );
        }
        *previous = instances;
    }
    pub fn instances(&self) -> Vec<InstanceInfo> {
        self.instances.lock().unwrap().clone()
    }
    pub fn emit(&self, event: &str, params: serde_json::Value) {
        let _ = self.sender.try_send(Request::Event {
            event: event.into(),
            params,
        });
    }
    pub async fn run(&self, plugin: Plugin, command: String, reload: bool) -> Result<RunReport> {
        self.dispatch(
            plugin,
            Some(("command".into(), command, serde_json::Value::Null)),
            reload,
            None,
            None,
        )
        .await
    }
    pub async fn activate(&self, plugin: Plugin) -> Result<RunReport> {
        self.dispatch(plugin, None, false, None, None).await
    }
    pub async fn reload(&self, plugin: Plugin) -> Result<RunReport> {
        self.dispatch(plugin, None, true, None, None).await
    }
    pub async fn reload_with_commit(
        &self,
        plugin: Plugin,
        commit: impl FnOnce() -> Result<()> + Send + 'static,
    ) -> Result<RunReport> {
        self.dispatch(plugin, None, true, Some(Box::new(commit)), None)
            .await
    }
    pub async fn dispatch(
        &self,
        plugin: Plugin,
        call: Option<Invocation>,
        reload: bool,
        commit: Option<Commit>,
        router: Option<Bridge>,
    ) -> Result<RunReport> {
        let response = self.enqueue(plugin, call, reload, commit, router)?;
        tokio::task::spawn_blocking(move || {
            response
                .recv()
                .map_err(|_| ApiError::new("SCRIPT_STOPPED", "脚本调度器已停止。"))?
        })
        .await
        .map_err(|_| ApiError::new("SCRIPT_STOPPED", "脚本调度器已停止。"))?
    }
    pub fn call_blocking(
        &self,
        plugin: Plugin,
        call: Invocation,
        router: Option<Bridge>,
    ) -> Result<RunReport> {
        self.enqueue(plugin, Some(call), false, None, router)?
            .recv()
            .map_err(|_| ApiError::new("SCRIPT_STOPPED", "脚本调度器已停止。"))?
    }
    fn enqueue(
        &self,
        plugin: Plugin,
        call: Option<Invocation>,
        reload: bool,
        commit: Option<Commit>,
        router: Option<Bridge>,
    ) -> Result<mpsc::Receiver<Result<RunReport>>> {
        if plugin.manifest.runtime != "javascript" {
            return Err(ApiError::new(
                "NATIVE_PLUGIN",
                "原生入口不能由脚本引擎执行。",
            ));
        }
        let (reply, response) = mpsc::channel();
        self.sender
            .try_send(Request::Run {
                plugin: Box::new(plugin),
                call,
                reload,
                commit,
                router,
                reply,
            })
            .map_err(|_| ApiError::new("SCRIPT_BUSY", "脚本调度器繁忙或已经停止。"))?;
        Ok(response)
    }
    pub async fn stop_all(&self) -> Result<RuntimeStats> {
        self.stop(None).await
    }
    pub async fn stop_plugins(&self, ids: Vec<String>) -> Result<RuntimeStats> {
        self.stop(Some(ids)).await
    }
    async fn stop(&self, ids: Option<Vec<String>>) -> Result<RuntimeStats> {
        let (reply, response) = mpsc::channel();
        self.sender
            .try_send(Request::Stop { ids, reply })
            .map_err(|_| ApiError::new("SCRIPT_BUSY", "脚本调度器繁忙。"))?;
        tokio::task::spawn_blocking(move || {
            response
                .recv()
                .map_err(|_| ApiError::new("SCRIPT_STOPPED", "脚本调度器已停止。"))
        })
        .await
        .map_err(|_| ApiError::new("SCRIPT_STOPPED", "脚本调度器已停止。"))?
    }
}
