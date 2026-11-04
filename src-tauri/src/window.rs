//! Desktop handle/event transport to an installed native UI-thread plugin.
use crate::{
    contracts::{ApiError, Result},
    launcher::core::Launcher,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::WebviewWindow;
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowGeometry {
    pub outer_width: i32,
    pub outer_height: i32,
    pub client_width: i32,
    pub client_height: i32,
    pub client_left: i32,
    pub client_top: i32,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    pub material: String,
    pub reason: String,
    pub rounded: bool,
}
fn handle(window: &WebviewWindow) -> Result<isize> {
    #[cfg(windows)]
    {
        window
            .hwnd()
            .map(|h| h.0 as isize)
            .map_err(|_| ApiError::new("INVALID_WINDOW", "无法取得窗口句柄。"))
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Err(ApiError::new(
            "UNSUPPORTED_PLATFORM",
            "窗口插件不支持当前平台。",
        ))
    }
}
fn call(
    window: &WebviewWindow,
    core: &Launcher,
    method: &str,
    mut params: serde_json::Value,
) -> Result<serde_json::Value> {
    params["handle"] = serde_json::json!(handle(window)?);
    let settings = core.store.lock().unwrap().settings.clone();
    core.plugins.call_service(
        &settings,
        "platform.windows.window",
        "platform.windows.window",
        method,
        params,
    )
}
pub fn geometry(window: &WebviewWindow, core: &Launcher) -> Result<Option<WindowGeometry>> {
    Ok(serde_json::from_value(call(
        window,
        core,
        "geometry",
        serde_json::json!({}),
    )?)?)
}
pub fn apply(window: &WebviewWindow, core: &Launcher) -> Result<Appearance> {
    Ok(serde_json::from_value(call(
        window,
        core,
        "apply",
        serde_json::json!({}),
    )?)?)
}
pub fn attach_resize(window: &WebviewWindow, core: Arc<Launcher>) {
    let controlled = window.clone();
    let initial = window.inner_size().ok();
    window.on_window_event(move|event|{
 let (kind,width,height)=match event{tauri::WindowEvent::Resized(size)=>("resized",size.width,size.height),tauri::WindowEvent::ScaleFactorChanged{..}=>("scale",0,0),tauri::WindowEvent::Moved(_)|tauri::WindowEvent::Focused(_)=>("sync",0,0),tauri::WindowEvent::Destroyed=>("destroyed",0,0),_=>return};
 let result=call(&controlled,&core,"event",serde_json::json!({"event":kind,"width":width,"height":height,"dpi":controlled.scale_factor().unwrap_or(1.0),"initialWidth":initial.map(|s|s.width),"initialHeight":initial.map(|s|s.height)}));
 if let Ok(value)=result && let Some(size)=value["size"].as_array() && size.len()==2 && let (Some(w),Some(h))=(size[0].as_u64(),size[1].as_u64()){let _=controlled.set_size(tauri::PhysicalSize::new(w as u32,h as u32));}
});
}
