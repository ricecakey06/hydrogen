use super::Window as WebviewWindow;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowGeometry {
    pub outer_width: i32,
    pub outer_height: i32,
    pub client_width: i32,
    pub client_height: i32,
    pub client_left: i32,
    pub client_top: i32,
}

pub fn geometry(window: &WebviewWindow) -> Option<WindowGeometry> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::{POINT, RECT},
            Graphics::Gdi::ClientToScreen,
            UI::WindowsAndMessaging::{GetClientRect, GetWindowRect},
        };
        let handle = window.hwnd().ok()?;
        unsafe {
            let mut outer: RECT = std::mem::zeroed();
            let mut client: RECT = std::mem::zeroed();
            let mut origin = POINT { x: 0, y: 0 };
            if GetWindowRect(handle.0 as _, &mut outer) == 0
                || GetClientRect(handle.0 as _, &mut client) == 0
                || ClientToScreen(handle.0 as _, &mut origin) == 0
            {
                return None;
            }
            Some(WindowGeometry {
                outer_width: outer.right - outer.left,
                outer_height: outer.bottom - outer.top,
                client_width: client.right,
                client_height: client.bottom,
                client_left: origin.x - outer.left,
                client_top: origin.y - outer.top,
            })
        }
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        None
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    pub material: &'static str,
    pub reason: &'static str,
    pub rounded: bool,
}

pub fn apply(window: &WebviewWindow, glass: &mut super::glass::State) -> Appearance {
    #[cfg(windows)]
    {
        let rounded = clip(window);
        if let Some(reason) = solid_reason() {
            glass.disable();
            return Appearance {
                material: "solid",
                reason,
                rounded,
            };
        }
        let glass = window
            .hwnd()
            .map_err(|error| error.to_string())
            .and_then(|handle| {
                glass
                    .enable(windows::Win32::Foundation::HWND(handle.0 as _))
                    .map_err(|error| error.to_string())
            });
        if glass.is_ok() {
            return Appearance {
                material: "blur",
                reason: "系统圆角玻璃（HostBackdrop）",
                rounded: clip(window),
            };
        }
        eprintln!("Rounded backdrop: {}", glass.unwrap_err());
        Appearance {
            material: "solid",
            reason: "当前环境不支持系统背景模糊",
            rounded,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Appearance {
            material: "solid",
            reason: "当前环境不支持系统背景模糊",
            rounded: false,
        }
    }
}

#[cfg(windows)]
fn solid_reason() -> Option<&'static str> {
    use windows_sys::Win32::{
        System::Registry::*,
        UI::{Accessibility::*, WindowsAndMessaging::*},
    };
    unsafe {
        if GetSystemMetrics(SM_REMOTESESSION) != 0 {
            return Some("远程桌面使用实色背景");
        }
        let mut contrast: HIGHCONTRASTW = std::mem::zeroed();
        contrast.cbSize = std::mem::size_of::<HIGHCONTRASTW>() as u32;
        if SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            &mut contrast as *mut _ as _,
            0,
        ) != 0
            && contrast.dwFlags & HCF_HIGHCONTRASTON != 0
        {
            return Some("高对比度模式使用实色背景");
        }
        let key: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
            .encode_utf16()
            .collect();
        let value: Vec<u16> = "EnableTransparency\0".encode_utf16().collect();
        let mut enabled = 1_u32;
        let mut size = 4;
        if RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut enabled as *mut _ as _,
            &mut size,
        ) == 0
            && enabled == 0
        {
            return Some("系统已关闭透明效果");
        }
    }
    None
}

pub fn clip(window: &WebviewWindow) -> bool {
    #[cfg(windows)]
    if let (Ok(handle), Ok(size)) = (window.hwnd(), window.outer_size()) {
        use windows_sys::Win32::Graphics::Gdi::{
            CreateRectRgn, CreateRoundRectRgn, DeleteObject, GetWindowRgn, PtInRegion, SetWindowRgn,
        };
        let scale = (size.width as f64 / 1000.0).min(size.height as f64 / 714.0);
        let diameter = (28.0 * scale).round() as i32;
        unsafe {
            let region = CreateRoundRectRgn(
                0,
                0,
                size.width as i32 + 1,
                size.height as i32 + 1,
                diameter,
                diameter,
            );
            // SetWindowRgn takes ownership only on success; release failed allocations ourselves.
            if region.is_null() {
                return false;
            }
            if SetWindowRgn(handle.0 as _, region, 1) == 0 {
                DeleteObject(region);
                return false;
            }
            let probe = CreateRectRgn(0, 0, 0, 0);
            if probe.is_null() {
                return false;
            }
            let rounded = GetWindowRgn(handle.0 as _, probe) > 1
                && PtInRegion(probe, 0, 0) == 0
                && PtInRegion(probe, size.width as i32 / 2, size.height as i32 / 2) != 0;
            DeleteObject(probe);
            return rounded;
        }
    }
    false
}

pub fn proportional_size(previous: (u32, u32), proposed: (u32, u32), dpi: f64) -> (u32, u32) {
    let horizontal = (proposed.0 as f64 - previous.0 as f64).abs() / 1000.0;
    let vertical = (proposed.1 as f64 - previous.1 as f64).abs() / 714.0;
    let scale = if horizontal >= vertical {
        proposed.0 as f64 / 1000.0
    } else {
        proposed.1 as f64 / 714.0
    };
    let scale = scale.max(0.3 * dpi);
    (
        (1000.0 * scale).round() as u32,
        (714.0 * scale).round() as u32,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn resize_follows_dragged_axis_and_keeps_scaled_minimum() {
        use super::proportional_size;
        assert_eq!(
            proportional_size((1000, 714), (1200, 714), 1.0),
            (1200, 857)
        );
        assert_eq!(
            proportional_size((1000, 714), (1000, 857), 1.0),
            (1200, 857)
        );
        assert_eq!(proportional_size((1000, 714), (100, 100), 1.0), (300, 214));
        assert_eq!(proportional_size((2000, 1428), (100, 100), 2.0), (600, 428));
    }
}
