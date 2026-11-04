mod glass;
mod window;
use hydrogen_plugin_sdk::contracts::{ApiError, Result};
use serde_json::{Value, json};
use std::collections::BTreeMap;
struct Handle(isize);
struct Size {
    width: u32,
    height: u32,
}
struct Window {
    handle: isize,
}
impl Window {
    fn hwnd(&self) -> std::result::Result<Handle, String> {
        if self.handle == 0 {
            return Err("Invalid window".into());
        }
        Ok(Handle(self.handle))
    }
    fn outer_size(&self) -> std::result::Result<Size, String> {
        let g = window::geometry(self).ok_or("Window unavailable")?;
        Ok(Size {
            width: g.outer_width as u32,
            height: g.outer_height as u32,
        })
    }
}
#[derive(Default)]
struct State {
    glass: glass::State,
    owner: Option<isize>,
    previous: BTreeMap<isize, (u32, u32)>,
}
impl State {
    fn call(&mut self, r: &mut Value) -> Result<Value> {
        let p = &r["params"];
        let method = r["method"].as_str().unwrap_or("");
        if method == "plugin.describe" {
            return Ok(json!({"id":"platform.windows.window","abiVersion":1}));
        }
        if method == "plugin.stop" {
            self.glass.disable();
            self.previous.clear();
            self.owner = None;
            return Ok(Value::Null);
        }
        let window = Window {
            handle: p["handle"]
                .as_i64()
                .ok_or_else(|| ApiError::new("INVALID_WINDOW", "缺少窗口句柄。"))?
                as isize,
        };
        match method {
            "apply" => {
                self.owner = Some(window.handle);
                Ok(serde_json::to_value(window::apply(
                    &window,
                    &mut self.glass,
                ))?)
            }
            "geometry" => Ok(serde_json::to_value(window::geometry(&window))?),
            "event" => {
                let kind = p["event"].as_str().unwrap_or("");
                if kind == "destroyed" {
                    self.glass.disable();
                    self.previous.remove(&window.handle);
                    self.owner = None;
                    return Ok(Value::Null);
                }
                if self.owner != Some(window.handle) {
                    window::apply(&window, &mut self.glass);
                    self.owner = Some(window.handle);
                }
                let mut result = Value::Null;
                if kind == "resized" {
                    let size = (
                        p["width"].as_u64().unwrap_or(0) as u32,
                        p["height"].as_u64().unwrap_or(0) as u32,
                    );
                    if size.0 > 0 && size.1 > 0 {
                        let previous = self.previous.entry(window.handle).or_insert((
                            p["initialWidth"].as_u64().unwrap_or(1000) as u32,
                            p["initialHeight"].as_u64().unwrap_or(714) as u32,
                        ));
                        let target = window::proportional_size(
                            *previous,
                            size,
                            p["dpi"].as_f64().unwrap_or(1.0),
                        );
                        *previous = target;
                        if target.0.abs_diff(size.0) > 1 || target.1.abs_diff(size.1) > 1 {
                            result = json!({"size":target});
                        }
                        window::clip(&window);
                    }
                } else if kind == "scale" {
                    window::clip(&window);
                }
                self.glass.sync();
                Ok(result)
            }
            _ => Err(ApiError::new("UNKNOWN_METHOD", "未知窗口方法。")),
        }
    }
}
fn dispatch(s: &mut State, r: &mut Value) -> hydrogen_plugin_sdk::Result {
    s.call(r).map_err(|e| (e.code, e.message))
}
hydrogen_plugin_sdk::export_plugin!(State, dispatch);
