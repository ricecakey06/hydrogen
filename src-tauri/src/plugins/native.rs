//! Native ABI adapter. A worker owns each library, instance and shadow copy until all
//! queued calls finish. Nothing in this module implements a launcher's business service.
use super::{
    manifest::{self, MAX_MESSAGE, Plugin},
    package,
};
use crate::contracts::{ApiError, Result};
use hydrogen_plugin_sdk::{Entry, PluginApi};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::c_void,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
};

struct Library(*mut c_void);
impl Library {
    fn open(path: &Path) -> Result<Self> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn LoadLibraryExW(path: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
            }
            let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let handle = unsafe { LoadLibraryExW(path.as_ptr(), std::ptr::null_mut(), 0x1100) };
            if !handle.is_null() {
                return Ok(Self(handle));
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let path = std::ffi::CString::new(path.as_os_str().as_bytes())
                .map_err(|_| ApiError::new("INVALID_PATH", "无效原生入口。"))?;
            let handle = unsafe { dlopen(path.as_ptr(), 2) };
            if !handle.is_null() {
                return Ok(Self(handle));
            }
        }
        Err(ApiError::new(
            "NATIVE_LOAD",
            "无法加载原生插件，请检查平台、架构和依赖。",
        ))
    }
    fn symbol(&self, name: &std::ffi::CStr) -> *mut c_void {
        #[cfg(windows)]
        {
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
            }
            unsafe { GetProcAddress(self.0, name.as_ptr().cast()) }
        }
        #[cfg(unix)]
        unsafe {
            dlsym(self.0, name.as_ptr())
        }
    }
    fn entry(&self) -> Result<Entry> {
        let address = self.symbol(c"hydrogen_plugin_v1");
        if address.is_null() {
            return Err(ApiError::new(
                "NATIVE_ABI",
                "原生插件缺少 hydrogen_plugin_v1 入口。",
            ));
        }
        Ok(unsafe { std::mem::transmute::<*mut c_void, Entry>(address) })
    }
}
impl Drop for Library {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn FreeLibrary(module: *mut c_void) -> i32;
            }
            unsafe {
                FreeLibrary(self.0);
            }
        }
        #[cfg(unix)]
        unsafe {
            dlclose(self.0);
        }
    }
}
#[cfg(unix)]
#[cfg_attr(target_os = "linux", link(name = "dl"))]
unsafe extern "C" {
    fn dlopen(path: *const std::ffi::c_char, flags: i32) -> *mut c_void;
    fn dlsym(module: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
    fn dlclose(module: *mut c_void) -> i32;
}

struct Shadow(PathBuf);
impl Drop for Shadow {
    fn drop(&mut self) {
        let _ = super::manager::remove_tree(&self.0);
    }
}
#[derive(Default)]
struct Counters {
    created: AtomicUsize,
    destroyed: AtomicUsize,
    next: AtomicU64,
}

pub type ServiceRouter = Arc<dyn Fn(Value) -> Result<Value> + Send + Sync>;
struct Instance {
    message_limit: usize,
    services: Option<hydrogen_plugin_sdk::CallWithServices>,
    counters: Arc<Counters>,
    counted: bool,
    api: &'static PluginApi,
    state: *mut c_void,
    // Field order matters: unload the library before deleting its shadow directory.
    _library: Library,
    _shadow: Shadow,
}
impl Instance {
    fn open(plugin: &Plugin, counters: Arc<Counters>) -> Result<Self> {
        let checked = manifest::load(&plugin.root)?;
        if checked.digest != plugin.digest {
            return Err(changed());
        }
        let shadow = Shadow(
            std::env::temp_dir().join(format!("hydrogen-native-{}", uuid::Uuid::new_v4().simple())),
        );
        fs::create_dir(&shadow.0)?;
        for (name, source) in package::files(&plugin.root)?.0 {
            let destination = shadow.0.join(name);
            fs::create_dir_all(destination.parent().unwrap())?;
            fs::copy(source, destination)?;
        }
        if package::files(&shadow.0)?.1 != plugin.digest {
            return Err(changed());
        }
        let library = Library::open(&manifest::within(&shadow.0, &plugin.manifest.entry)?)?;
        let pointer = unsafe { library.entry()?() };
        if pointer.is_null() {
            return Err(ApiError::new("NATIVE_ABI", "原生插件返回了空接口。"));
        }
        // Native code is explicitly trusted. Check the ABI prefix before reading its function table.
        let version = unsafe { pointer.cast::<u32>().read() };
        let size = unsafe { pointer.cast::<u32>().add(1).read() };
        if version != 1 || size < std::mem::size_of::<PluginApi>() as u32 {
            return Err(ApiError::new("NATIVE_ABI", "原生插件 ABI 不兼容。"));
        }
        let api = unsafe { &*pointer };
        let state = unsafe { (api.create)() };
        if state.is_null() {
            return Err(ApiError::new("NATIVE_INIT", "无法创建原生插件实例。"));
        }
        let large = library.symbol(c"hydrogen_plugin_large_services_v1");
        let message_limit = if large.is_null() {
            MAX_MESSAGE
        } else {
            hydrogen_plugin_sdk::MAX_LARGE_MESSAGE
        };
        let extension = if large.is_null() {
            library.symbol(c"hydrogen_plugin_services_v1")
        } else {
            large
        };
        let services = (!extension.is_null()).then(|| unsafe {
            std::mem::transmute::<*mut c_void, hydrogen_plugin_sdk::CallWithServices>(extension)
        });
        let mut instance = Self {
            message_limit,
            services,
            api,
            state,
            _library: library,
            _shadow: shadow,
            counters,
            counted: false,
        };
        let description = instance.call("plugin.describe", Value::Null)?;
        if description["id"] != plugin.manifest.id || description["abiVersion"] != 1 {
            return Err(ApiError::new(
                "NATIVE_IDENTITY",
                "原生入口身份与插件清单不符。",
            ));
        }
        instance.counters.created.fetch_add(1, Ordering::Relaxed);
        instance.counted = true;
        Ok(instance)
    }
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.call_routed(method, params, None)
    }
    fn call_routed(
        &self,
        method: &str,
        params: Value,
        router: Option<ServiceRouter>,
    ) -> Result<Value> {
        struct Response {
            limit: usize,
            bytes: Option<Vec<u8>>,
            invalid: bool,
        }
        unsafe extern "C" fn receive(context: *mut c_void, bytes: *const u8, len: usize) {
            let response = unsafe { &mut *context.cast::<Response>() };
            if response.bytes.is_some() || bytes.is_null() || len > response.limit {
                response.invalid = true;
                return;
            }
            response.bytes = Some(unsafe { std::slice::from_raw_parts(bytes, len) }.to_vec());
        }
        let input = hydrogen_plugin_sdk::JsonBuffer(json!({"method":method,"params":params}));
        let mut request = serde_json::to_vec(&input.0)?;
        if request.len() > self.message_limit {
            request.fill(0);
            return Err(ApiError::new("MESSAGE_LIMIT", "原生插件请求超过限制。"));
        }
        let mut response = Response {
            limit: self.message_limit,
            bytes: None,
            invalid: false,
        };
        struct ServiceContext {
            router: ServiceRouter,
            limit: usize,
        }
        unsafe extern "C" fn service(
            context: *mut c_void,
            bytes: *const u8,
            len: usize,
            respond: hydrogen_plugin_sdk::Respond,
            reply: *mut c_void,
        ) -> i32 {
            let service = unsafe { &*context.cast::<ServiceContext>() };
            if bytes.is_null() || len > service.limit {
                return -1;
            }
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Value> {
                    let router = &service.router;
                    let mut request =
                        hydrogen_plugin_sdk::JsonBuffer(serde_json::from_slice(unsafe {
                            std::slice::from_raw_parts(bytes, len)
                        })?);
                    router(request.0.take())
                }));
            let result = match result {
                Ok(Ok(value)) => json!({"ok":true,"value":value}),
                Ok(Err(error)) => json!({"ok":false,"error":error}),
                Err(_) => {
                    json!({"ok":false,"error":{"code":"SERVICE_PANIC","message":"Service failed"}})
                }
            };
            let result = hydrogen_plugin_sdk::JsonBuffer(result);
            let Ok(mut bytes) = serde_json::to_vec(&result.0) else {
                return -1;
            };
            if bytes.len() > service.limit {
                bytes.fill(0);
                return -2;
            }
            unsafe {
                respond(reply, bytes.as_ptr(), bytes.len());
            }
            bytes.fill(0);
            0
        }
        let service_context = ServiceContext {
            router: router.unwrap_or_else(|| {
                Arc::new(|_| {
                    Err(ApiError::new(
                        "SERVICE_UNAVAILABLE",
                        "当前调用不提供跨插件服务。",
                    ))
                })
            }),
            limit: self.message_limit,
        };
        let status = unsafe {
            if let Some(extension) = self.services {
                extension(
                    self.state,
                    request.as_ptr(),
                    request.len(),
                    receive,
                    (&mut response as *mut Response).cast(),
                    service,
                    (&service_context as *const ServiceContext)
                        .cast_mut()
                        .cast(),
                )
            } else {
                (self.api.call)(
                    self.state,
                    request.as_ptr(),
                    request.len(),
                    receive,
                    (&mut response as *mut Response).cast(),
                )
            }
        };
        request.fill(0);
        if status != 0 || response.invalid || response.bytes.is_none() {
            return Err(ApiError::new("NATIVE_RESPONSE", "原生插件返回了无效响应。"));
        }
        let mut bytes = response.bytes.unwrap();
        let decoded = serde_json::from_slice(&bytes);
        bytes.fill(0);
        let mut buffer = hydrogen_plugin_sdk::JsonBuffer(decoded?);
        let value = &mut buffer.0;
        match value["ok"].as_bool() {
            Some(true) if value.get("value").is_some() => Ok(value["value"].take()),
            Some(false) => Err(ApiError::new(
                value["error"]["code"].as_str().unwrap_or("NATIVE_ERROR"),
                value["error"]["message"]
                    .as_str()
                    .unwrap_or("原生插件执行失败。"),
            )),
            _ => Err(ApiError::new("NATIVE_RESPONSE", "原生插件返回了无效响应。")),
        }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            (self.api.destroy)(self.state);
        }
        if self.counted {
            self.counters.destroyed.fetch_add(1, Ordering::Relaxed);
        }
    }
}
fn changed() -> ApiError {
    ApiError::new("PLUGIN_CHANGED", "插件文件已变化，请校验并重载。")
}

enum Request {
    Call(
        String,
        hydrogen_plugin_sdk::JsonBuffer,
        mpsc::SyncSender<Result<Value>>,
        Option<ServiceRouter>,
    ),
    Shutdown(mpsc::SyncSender<()>),
    Stop(mpsc::SyncSender<()>),
}
struct Worker {
    generation: u64,
    registrations: usize,
    digest: String,
    sender: mpsc::SyncSender<Request>,
}
impl Worker {
    fn new(plugin: Plugin, counters: Arc<Counters>) -> Result<Self> {
        let generation = counters.next.fetch_add(1, Ordering::Relaxed) + 1;
        let registrations =
            plugin.manifest.provides.len() + plugin.manifest.contributes.commands.len();
        let digest = plugin.digest.clone();
        let (sender, receiver) = mpsc::sync_channel(8);
        let (ready, response) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name(format!("plugin:{}", plugin.manifest.id))
            .spawn(move || {
                let instance = match Instance::open(&plugin, counters) {
                    Ok(instance) => instance,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                let _ = ready.send(Ok(()));
                let mut last_router: Option<ServiceRouter> = None;
                for request in receiver {
                    match request {
                        Request::Call(method, mut params, reply, router) => {
                            if router.is_some() {
                                last_router = router.clone();
                            }
                            let _ =
                                reply.send(instance.call_routed(&method, params.0.take(), router));
                        }
                        Request::Shutdown(reply) => {
                            let _ = instance.call_routed(
                                "plugin.stop",
                                Value::Null,
                                last_router.clone(),
                            );
                            let _ = reply.send(());
                        }
                        Request::Stop(reply) => {
                            let _ = instance.call_routed(
                                "plugin.stop",
                                Value::Null,
                                last_router.take(),
                            );
                            drop(instance);
                            let _ = reply.send(());
                            return;
                        }
                    }
                }
            })?;
        response
            .recv()
            .map_err(|_| ApiError::new("NATIVE_STOPPED", "原生插件初始化失败。"))??;
        Ok(Self {
            digest,
            sender,
            generation,
            registrations,
        })
    }
    fn stop(self) -> Result<()> {
        let (reply, response) = mpsc::sync_channel(1);
        self.sender
            .send(Request::Stop(reply))
            .map_err(|_| ApiError::new("NATIVE_STOPPED", "原生插件已停止。"))?;
        response
            .recv()
            .map_err(|_| ApiError::new("NATIVE_STOPPED", "原生插件停止失败。"))
    }
}

pub type MainDispatch = Arc<dyn Fn(Box<dyn FnOnce() + Send>) -> Result<()> + Send + Sync>;
#[derive(Clone)]
struct MainExecutor {
    thread: std::thread::ThreadId,
    dispatch: MainDispatch,
}
struct MainInstance {
    instance: Instance,
    digest: String,
    router: Option<ServiceRouter>,
    generation: u64,
    registrations: usize,
}
thread_local! {static MAIN_INSTANCES:std::cell::RefCell<BTreeMap<(usize,String),MainInstance>>=const{std::cell::RefCell::new(BTreeMap::new())};}
struct MainLifetime {
    key: usize,
    executor: Arc<Mutex<Option<MainExecutor>>>,
}
impl Drop for MainLifetime {
    fn drop(&mut self) {
        let key = self.key;
        let cleanup = move || {
            let removed = MAIN_INSTANCES.with(|entries| {
                let mut entries = entries.borrow_mut();
                let keys: Vec<_> = entries
                    .keys()
                    .filter(|(owner, _)| *owner == key)
                    .cloned()
                    .collect();
                keys.into_iter()
                    .filter_map(|key| entries.remove(&key))
                    .collect::<Vec<_>>()
            });
            for entry in removed {
                let _ =
                    entry
                        .instance
                        .call_routed("plugin.stop", Value::Null, entry.router.clone());
                drop(entry);
            }
        };
        if let Some(executor) = self.executor.lock().unwrap().clone() {
            if executor.thread == std::thread::current().id() {
                cleanup();
            } else {
                let _ = (executor.dispatch)(Box::new(cleanup));
            }
        }
    }
}
#[derive(Clone)]
pub struct NativeHost {
    _main_lifetime: Arc<MainLifetime>,
    main: Arc<Mutex<Option<MainExecutor>>>,
    main_active: Arc<Mutex<BTreeMap<String, (u64, usize)>>>,
    workers: Arc<Mutex<BTreeMap<String, Worker>>>,
    counters: Arc<Counters>,
}
impl Default for NativeHost {
    fn default() -> Self {
        let main = Arc::new(Mutex::new(None));
        let counters = Arc::new(Counters::default());
        Self {
            _main_lifetime: Arc::new(MainLifetime {
                key: Arc::as_ptr(&counters) as usize,
                executor: main.clone(),
            }),
            main,
            counters,
            main_active: Arc::default(),
            workers: Arc::default(),
        }
    }
}
impl NativeHost {
    pub fn set_main_executor(&self, dispatch: MainDispatch) {
        *self.main.lock().unwrap() = Some(MainExecutor {
            thread: std::thread::current().id(),
            dispatch,
        });
    }
    fn main_key(&self) -> usize {
        Arc::as_ptr(&self.counters) as usize
    }
    fn on_main<T: Send + 'static>(
        &self,
        f: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let main = self.main.lock().unwrap().clone().ok_or_else(|| {
            ApiError::new("MAIN_THREAD_UNAVAILABLE", "此插件需要桌面主线程执行器。")
        })?;
        if std::thread::current().id() == main.thread {
            return f();
        }
        let (tx, rx) = mpsc::sync_channel(1);
        (main.dispatch)(Box::new(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
                .unwrap_or_else(|_| Err(ApiError::new("NATIVE_PANIC", "主线程插件执行失败。")));
            let _ = tx.send(result);
        }))?;
        rx.recv()
            .map_err(|_| ApiError::new("MAIN_THREAD_UNAVAILABLE", "主线程执行器已停止。"))?
    }
    fn call_main(
        &self,
        plugin: Plugin,
        method: String,
        mut params: hydrogen_plugin_sdk::JsonBuffer,
        router: Option<ServiceRouter>,
    ) -> Result<Value> {
        let owner = self.main_key();
        let counters = self.counters.clone();
        let active = self.main_active.clone();
        self.on_main(move || {
            MAIN_INSTANCES.with(|entries| {
                // Remove during invocation so provider callbacks can enter other main-thread plugins.
                let key = (owner, plugin.manifest.id.clone());
                let existing = entries.borrow_mut().remove(&key);
                let mut item = if let Some(item) = existing {
                    item
                } else {
                    MainInstance {
                        instance: Instance::open(&plugin, counters.clone())?,
                        digest: plugin.digest.clone(),
                        router: None,
                        generation: counters.next.fetch_add(1, Ordering::Relaxed) + 1,
                        registrations: plugin.manifest.provides.len()
                            + plugin.manifest.contributes.commands.len(),
                    }
                };
                if item.digest != plugin.digest {
                    entries.borrow_mut().insert(key, item);
                    return Err(changed());
                }
                if router.is_some() {
                    item.router = router.clone();
                }
                let result = item.instance.call_routed(&method, params.0.take(), router);
                active
                    .lock()
                    .unwrap()
                    .insert(plugin.manifest.id, (item.generation, item.registrations));
                entries.borrow_mut().insert(key, item);
                result
            })
        })
    }
    pub fn stop_main(&self, ids: Option<&[String]>) -> Result<()> {
        let selected: Vec<_> = self
            .main_active
            .lock()
            .unwrap()
            .keys()
            .filter(|id| ids.is_none_or(|ids| ids.contains(id)))
            .cloned()
            .collect();
        if selected.is_empty() {
            return Ok(());
        }
        let owner = self.main_key();
        let active = self.main_active.clone();
        self.on_main(move || {
            for id in selected {
                let entry = MAIN_INSTANCES
                    .with(|entries| entries.borrow_mut().remove(&(owner, id.clone())));
                if let Some(entry) = entry {
                    let _ = entry.instance.call_routed(
                        "plugin.stop",
                        Value::Null,
                        entry.router.clone(),
                    );
                    drop(entry);
                }
                active.lock().unwrap().remove(&id);
            }
            Ok(())
        })
    }

    pub fn call(&self, plugin: Plugin, method: String, params: Value) -> Result<Value> {
        self.call_routed(plugin, method, params, None)
    }
    pub fn call_routed(
        &self,
        plugin: Plugin,
        method: String,
        params: Value,
        router: Option<ServiceRouter>,
    ) -> Result<Value> {
        let params = hydrogen_plugin_sdk::JsonBuffer(params);
        if manifest::load_runtime(&plugin.root)?.digest != plugin.digest {
            return Err(changed());
        }
        if plugin.manifest.execution_thread == "main" {
            return self.call_main(plugin, method, params, router);
        }
        let sender = {
            let mut workers = self.workers.lock().unwrap();
            if let Some(worker) = workers.get(&plugin.manifest.id) {
                if worker.digest != plugin.digest {
                    return Err(changed());
                }
            } else {
                workers.insert(
                    plugin.manifest.id.clone(),
                    Worker::new(plugin.clone(), self.counters.clone())?,
                );
            }
            workers[&plugin.manifest.id].sender.clone()
        };
        let (reply, response) = mpsc::sync_channel(1);
        sender
            .try_send(Request::Call(method, params, reply, router))
            .map_err(|_| ApiError::new("PLUGIN_BUSY", "插件队列已满或已停止。"))?;
        response
            .recv()
            .map_err(|_| ApiError::new("NATIVE_STOPPED", "原生插件已停止。"))?
    }
    pub fn reload(&self, plugin: Plugin, commit: impl FnOnce() -> Result<()>) -> Result<()> {
        if plugin.manifest.execution_thread == "main" {
            let owner = self.main_key();
            let checked = plugin.clone();
            let counters = self.counters.clone();
            // Validate candidate on its declared thread before committing installed files.
            self.on_main(move || {
                let candidate = Instance::open(&checked, counters)?;
                drop(candidate);
                Ok(())
            })?;
            commit()?;
            self.stop_main(Some(std::slice::from_ref(&plugin.manifest.id)))?;
            let _ = owner;
            self.call(plugin, "plugin.describe".into(), Value::Null)?;
            return Ok(());
        }
        let candidate = Worker::new(plugin.clone(), self.counters.clone())?;
        if let Err(error) = commit() {
            candidate.stop()?;
            return Err(error);
        }
        let previous = self
            .workers
            .lock()
            .unwrap()
            .insert(plugin.manifest.id, candidate);
        if let Some(previous) = previous {
            previous.stop()?;
        }
        Ok(())
    }
    /// Stateless calls requiring the caller's OS thread, such as a modal file dialog.
    pub fn call_inline(&self, plugin: Plugin, method: String, params: Value) -> Result<Value> {
        let mut params = hydrogen_plugin_sdk::JsonBuffer(params);
        let instance = Instance::open(&plugin, self.counters.clone())?;
        instance.call(&method, params.0.take())
    }
    pub fn shutdown(&self, id: &str) -> Result<()> {
        if self.main_active.lock().unwrap().contains_key(id) {
            return self.stop_main(Some(&[id.into()]));
        }
        let sender = self
            .workers
            .lock()
            .unwrap()
            .get(id)
            .map(|worker| worker.sender.clone());
        if let Some(sender) = sender {
            let (reply, response) = mpsc::sync_channel(1);
            sender
                .send(Request::Shutdown(reply))
                .map_err(|_| ApiError::new("NATIVE_STOPPED", "插件已停止。"))?;
            response
                .recv()
                .map_err(|_| ApiError::new("NATIVE_STOPPED", "插件停止失败。"))?;
        }
        Ok(())
    }
    pub fn generation(&self, id: &str) -> u64 {
        if let Some((generation, _)) = self.main_active.lock().unwrap().get(id) {
            return *generation;
        }
        self.workers
            .lock()
            .unwrap()
            .get(id)
            .map_or(0, |w| w.generation)
    }
    pub fn stats(&self) -> super::runtime::RuntimeStats {
        let workers = self.workers.lock().unwrap();
        let main = self.main_active.lock().unwrap();
        super::runtime::RuntimeStats {
            active: workers.len() + main.len(),
            created: self.counters.created.load(Ordering::Relaxed),
            destroyed: self.counters.destroyed.load(Ordering::Relaxed),
            registrations: workers.values().map(|w| w.registrations).sum::<usize>()
                + main.values().map(|(_, r)| r).sum::<usize>(),
            heap_bytes: 0,
        }
    }
    pub fn stop(&self, ids: Option<&[String]>) -> Result<()> {
        self.stop_main(ids)?;
        let removed: Vec<_> = {
            let mut workers = self.workers.lock().unwrap();
            let keys: Vec<_> = workers
                .keys()
                .filter(|id| ids.is_none_or(|ids| ids.contains(id)))
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|id| workers.remove(&id))
                .collect()
        };
        for worker in removed {
            worker.stop()?;
        }
        Ok(())
    }
}
