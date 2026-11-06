//! Native ABI v1. Only byte slices, integers and opaque instance pointers cross the boundary.
//! Calls are serialized per instance. Plugins must not retain request/response pointers or
//! create work which outlives `destroy`. Native plugins run with the process's full privileges.
use std::ffi::c_void;

pub const ABI_VERSION: u32 = 1;
pub const MAX_MESSAGE: usize = 256 * 1024;
/// Explicit optional native transport capability, independent of script/panel message limits.
pub const MAX_LARGE_MESSAGE: usize = 64 * 1024 * 1024;
thread_local! {static MESSAGE_LIMIT:std::cell::Cell<usize>=const{std::cell::Cell::new(MAX_MESSAGE)};}
pub fn message_limit() -> usize {
    MESSAGE_LIMIT.with(|v| v.get())
}
pub fn with_large_messages<T>(f: impl FnOnce() -> T) -> T {
    struct Restore(usize);
    impl Drop for Restore {
        fn drop(&mut self) {
            MESSAGE_LIMIT.with(|v| v.set(self.0));
        }
    }
    let _restore = Restore(MESSAGE_LIMIT.with(|v| v.replace(MAX_LARGE_MESSAGE)));
    f()
}

pub type Respond = unsafe extern "C" fn(*mut c_void, *const u8, usize);

#[repr(C)]
pub struct PluginApi {
    pub abi_version: u32,
    pub struct_size: u32,
    pub create: unsafe extern "C" fn() -> *mut c_void,
    pub call: unsafe extern "C" fn(*mut c_void, *const u8, usize, Respond, *mut c_void) -> i32,
    pub destroy: unsafe extern "C" fn(*mut c_void),
}

pub type Entry = unsafe extern "C" fn() -> *const PluginApi;
pub type Result = std::result::Result<serde_json::Value, (String, String)>;
pub use serde_json;

/// Basic error values for native implementations, independent of the launcher crate.
pub mod contracts {
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct ThemeChoice {
        pub id: String,
        pub mode: String,
        pub material: String,
    }
    impl Default for ThemeChoice {
        fn default() -> Self {
            Self {
                id: "builtin:default".into(),
                mode: "system".into(),
                material: "auto".into(),
            }
        }
    }
    impl ThemeChoice {
        pub fn is_valid(&self) -> bool {
            let id = |id: &str| {
                !id.is_empty()
                    && id.len() <= 100
                    && id.as_bytes()[0].is_ascii_lowercase()
                    && id.bytes().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'-')
                    })
                    && !id.contains("..")
                    && !id.ends_with('.')
            };
            ["light", "dark", "system"].contains(&self.mode.as_str())
                && ["auto", "opaque"].contains(&self.material.as_str())
                && (self.id == "builtin:default"
                    || self
                        .id
                        .split_once(':')
                        .is_some_and(|(p, t)| p != "builtin" && id(p) && id(t)))
        }
    }
    #[derive(Debug)]
    pub struct ApiError {
        pub code: String,
        pub message: String,
    }
    pub type Result<T> = std::result::Result<T, ApiError>;
    impl ApiError {
        pub fn new(code: &str, message: impl Into<String>) -> Self {
            Self {
                code: code.into(),
                message: message.into(),
            }
        }
    }
    impl From<std::io::Error> for ApiError {
        fn from(_: std::io::Error) -> Self {
            Self::new("IO_ERROR", "文件操作失败。")
        }
    }
    impl From<serde_json::Error> for ApiError {
        fn from(_: serde_json::Error) -> Self {
            Self::new("INVALID_JSON", "消息字段无效。")
        }
    }
}

/// Clear transient protocol buffers, including credential values, before releasing them.
pub fn wipe_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => unsafe {
            text.as_bytes_mut().fill(0);
        },
        serde_json::Value::Array(items) => items.iter_mut().for_each(wipe_json),
        serde_json::Value::Object(items) => {
            for (mut key, mut value) in std::mem::take(items) {
                unsafe {
                    key.as_bytes_mut().fill(0);
                }
                wipe_json(&mut value);
            }
        }
        _ => *value = serde_json::Value::Null,
    }
}

pub struct JsonBuffer(pub serde_json::Value);
impl Drop for JsonBuffer {
    fn drop(&mut self) {
        wipe_json(&mut self.0);
    }
}

/// Optional synchronous service-call extension. The v1 function table remains binary compatible.
pub type ServiceCall =
    unsafe extern "C" fn(*mut c_void, *const u8, usize, Respond, *mut c_void) -> i32;
pub type CallWithServices = unsafe extern "C" fn(
    *mut c_void,
    *const u8,
    usize,
    Respond,
    *mut c_void,
    ServiceCall,
    *mut c_void,
) -> i32;
thread_local! {
    static SERVICES: std::cell::Cell<Option<(ServiceCall,*mut c_void)>> = const { std::cell::Cell::new(None) };
}
/// The callback and context are valid only during this invocation, on this thread.
/// # Safety
/// The caller must keep the callback context alive for the entire invocation.
pub unsafe fn with_services<T>(
    call: ServiceCall,
    context: *mut c_void,
    invoke: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<(ServiceCall, *mut c_void)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SERVICES.with(|s| s.set(self.0));
        }
    }
    let previous = SERVICES.with(|s| s.replace(Some((call, context))));
    let _restore = Restore(previous);
    invoke()
}
pub fn call_service(
    provider: &str,
    service: &str,
    method: &str,
    params: serde_json::Value,
) -> contracts::Result<serde_json::Value> {
    let input = JsonBuffer(
        serde_json::json!({"provider":provider,"service":service,"method":method,"params":params}),
    );
    let (call, context) = SERVICES.with(|s| s.get()).ok_or_else(|| {
        contracts::ApiError::new("SERVICE_UNAVAILABLE", "No service context on this thread")
    })?;
    struct Reply {
        bytes: Vec<u8>,
        received: bool,
        invalid: bool,
    }
    unsafe extern "C" fn receive(context: *mut c_void, bytes: *const u8, len: usize) {
        let reply = unsafe { &mut *context.cast::<Reply>() };
        if reply.received || bytes.is_null() || len > message_limit() {
            reply.invalid = true;
            return;
        }
        reply.received = true;
        reply
            .bytes
            .extend_from_slice(unsafe { std::slice::from_raw_parts(bytes, len) });
    }
    let mut bytes = serde_json::to_vec(&input.0)?;
    if bytes.len() > message_limit() {
        bytes.fill(0);
        return Err(contracts::ApiError::new(
            "MESSAGE_LIMIT",
            "Service request too large",
        ));
    }
    let mut reply = Reply {
        bytes: Vec::new(),
        received: false,
        invalid: false,
    };
    let status = unsafe {
        call(
            context,
            bytes.as_ptr(),
            bytes.len(),
            receive,
            (&mut reply as *mut Reply).cast(),
        )
    };
    bytes.fill(0);
    let decoded = serde_json::from_slice(&reply.bytes);
    reply.bytes.fill(0);
    if status != 0 || !reply.received || reply.invalid {
        return Err(contracts::ApiError::new(
            "SERVICE_RESPONSE",
            "Invalid service response",
        ));
    }
    let mut result = JsonBuffer(decoded?);
    if result.0["ok"] == true {
        Ok(result.0["value"].take())
    } else {
        Err(contracts::ApiError::new(
            result.0["error"]["code"]
                .as_str()
                .unwrap_or("SERVICE_ERROR"),
            result.0["error"]["message"]
                .as_str()
                .unwrap_or("Service failed"),
        ))
    }
}

/// Export a stateful, synchronous plugin. Dispatch receives `{method, params}` and returns
/// a JSON value or a public error. Panics never unwind through the C boundary.
#[macro_export]
macro_rules! export_plugin {
    ($state:ty, $dispatch:path) => {
        unsafe extern "C" fn create() -> *mut std::ffi::c_void {
            match std::panic::catch_unwind(|| Box::new(<$state>::default())) {
                Ok(state) => Box::into_raw(state).cast(),
                Err(_) => std::ptr::null_mut(),
            }
        }
        unsafe extern "C" fn call(
            state: *mut std::ffi::c_void,
            request: *const u8,
            len: usize,
            respond: $crate::Respond,
            context: *mut std::ffi::c_void,
        ) -> i32 {
            if state.is_null() || request.is_null() || len > $crate::message_limit() { return -1; }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let bytes = unsafe { std::slice::from_raw_parts(request, len) };
                let request = $crate::serde_json::from_slice(bytes)
                    .map_err(|_| ("INVALID_JSON".into(), "Invalid request".into()))?;
                let mut request = $crate::JsonBuffer(request);
                $dispatch(unsafe { &mut *state.cast::<$state>() }, &mut request.0)
            }));
            let response = match result {
                Ok(Ok(value)) => $crate::serde_json::json!({"ok":true,"value":value}),
                Ok(Err((code,message))) => $crate::serde_json::json!({"ok":false,"error":{"code":code,"message":message}}),
                Err(_) => $crate::serde_json::json!({"ok":false,"error":{"code":"NATIVE_PANIC","message":"Native plugin failed"}}),
            };
            let response = $crate::JsonBuffer(response);
            let mut bytes = $crate::serde_json::to_vec(&response.0).unwrap();
            if bytes.len() > $crate::message_limit() { bytes.fill(0); return -2; }
            unsafe { respond(context, bytes.as_ptr(), bytes.len()); }
            bytes.fill(0);
            0
        }
        unsafe extern "C" fn destroy(state: *mut std::ffi::c_void) {
            if !state.is_null() {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
                    drop(Box::from_raw(state.cast::<$state>()));
                }));
            }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn hydrogen_plugin_services_v1(
            state:*mut std::ffi::c_void, request:*const u8,len:usize,
            respond:$crate::Respond,context:*mut std::ffi::c_void,
            services:$crate::ServiceCall,service_context:*mut std::ffi::c_void,
        )->i32 {
            unsafe { $crate::with_services(services,service_context,||call(state,request,len,respond,context)) }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn hydrogen_plugin_large_services_v1(state:*mut std::ffi::c_void,request:*const u8,len:usize,respond:$crate::Respond,context:*mut std::ffi::c_void,services:$crate::ServiceCall,service_context:*mut std::ffi::c_void)->i32{
            $crate::with_large_messages(||unsafe{$crate::with_services(services,service_context,||call(state,request,len,respond,context))})
        }
        static API: $crate::PluginApi = $crate::PluginApi {
            abi_version: $crate::ABI_VERSION,
            struct_size: std::mem::size_of::<$crate::PluginApi>() as u32,
            create, call, destroy,
        };
        #[unsafe(no_mangle)]
        pub extern "C" fn hydrogen_plugin_v1() -> *const $crate::PluginApi { &API }
    };
}
