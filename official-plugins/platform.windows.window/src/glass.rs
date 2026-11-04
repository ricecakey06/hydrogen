//! A compositor-owned, rounded desktop backdrop behind the single main WebView.
//! Window regions alone cannot clip DWM's whole-window Acrylic visual.
use windows::{
    System::{DispatcherQueue, DispatcherQueueController},
    UI::Composition::{
        CompositionRoundedRectangleGeometry, Compositor, Desktop::DesktopWindowTarget, SpriteVisual,
    },
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::Dwm::{DWMWA_USE_HOSTBACKDROPBRUSH, DwmSetWindowAttribute},
        System::{
            LibraryLoader::GetModuleHandleW,
            WinRT::{
                Composition::ICompositorDesktopInterop, CreateDispatcherQueueController,
                DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT, DispatcherQueueOptions,
            },
        },
        UI::WindowsAndMessaging::*,
    },
    core::{Interface, Result, w},
};
use windows_numerics::Vector2;

struct WindowClass {
    name: Vec<u16>,
    instance: windows::Win32::Foundation::HINSTANCE,
}
impl Drop for WindowClass {
    fn drop(&mut self) {
        unsafe {
            let _ = UnregisterClassW(
                windows::core::PCWSTR(self.name.as_ptr()),
                Some(self.instance),
            );
        }
    }
}
struct Surface {
    handle: HWND,
    _class: WindowClass,
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.handle);
        }
    }
}

struct Backdrop {
    owner: HWND,
    surface: Surface,
    target: DesktopWindowTarget,
    visual: SpriteVisual,
    geometry: CompositionRoundedRectangleGeometry,
    compositor: Compositor,
    _queue: Option<DispatcherQueueController>,
}

unsafe extern "system" fn surface_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCHITTEST {
        return LRESULT(HTTRANSPARENT as isize);
    }
    if message == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn dispatcher() -> Result<Option<DispatcherQueueController>> {
    if DispatcherQueue::GetForCurrentThread().is_ok() {
        return Ok(None);
    }
    let options = DispatcherQueueOptions {
        dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
        threadType: DQTYPE_THREAD_CURRENT,
        apartmentType: DQTAT_COM_NONE,
    };
    Ok(Some(unsafe { CreateDispatcherQueueController(options)? }))
}
impl Backdrop {
    fn new(owner: HWND) -> Result<Self> {
        let queue = dispatcher()?;
        let class_name: Vec<u16> = format!("HydrogenBackdrop-{}\0", uuid::Uuid::new_v4().simple())
            .encode_utf16()
            .collect();
        let surface = unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(surface_proc),
                hInstance: instance.into(),
                lpszClassName: windows::core::PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err(windows::core::Error::from_thread());
            }
            let registration = WindowClass {
                name: class_name,
                instance: instance.into(),
            };
            let handle = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_NOREDIRECTIONBITMAP,
                class.lpszClassName,
                w!("Hydrogen backdrop"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            Surface {
                handle,
                _class: registration,
            }
        };
        unsafe {
            let enabled = 1_i32;
            DwmSetWindowAttribute(
                surface.handle,
                DWMWA_USE_HOSTBACKDROPBRUSH,
                &enabled as *const _ as _,
                std::mem::size_of_val(&enabled) as u32,
            )?;
        }
        let compositor = Compositor::new()?;
        let interop: ICompositorDesktopInterop = compositor.cast()?;
        let target = unsafe { interop.CreateDesktopWindowTarget(surface.handle, false)? };
        let geometry = compositor.CreateRoundedRectangleGeometry()?;
        let visual = compositor.CreateSpriteVisual()?;
        visual.SetBrush(&compositor.CreateHostBackdropBrush()?)?;
        visual.SetClip(&compositor.CreateGeometricClipWithGeometry(&geometry)?)?;
        target.SetRoot(&visual)?;
        let state = Self {
            owner,
            surface,
            target,
            visual,
            geometry,
            compositor,
            _queue: queue,
        };
        state.sync()?;
        Ok(state)
    }

    fn sync(&self) -> Result<()> {
        unsafe {
            if IsIconic(self.owner).as_bool() || !IsWindowVisible(self.owner).as_bool() {
                let _ = ShowWindow(self.surface.handle, SW_HIDE);
                return Ok(());
            }
            let mut bounds = RECT::default();
            GetWindowRect(self.owner, &mut bounds)?;
            let width = (bounds.right - bounds.left).max(1);
            let height = (bounds.bottom - bounds.top).max(1);
            let size = Vector2 {
                X: width as f32,
                Y: height as f32,
            };
            let radius = 14.0 * (width as f32 / 1000.0).min(height as f32 / 714.0);
            self.visual.SetSize(size)?;
            self.geometry.SetSize(size)?;
            self.geometry.SetCornerRadius(Vector2 {
                X: radius,
                Y: radius,
            })?;
            // No background is painted outside this geometry, including DWM's blur itself.
            SetWindowPos(
                self.surface.handle,
                Some(self.owner),
                bounds.left,
                bounds.top,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )?;
        }
        Ok(())
    }
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        let _ = self.target.Close();
        let _ = self.compositor.Close();
    }
}

#[derive(Default)]
pub struct State {
    backdrop: Option<Backdrop>,
}
impl State {
    pub fn enable(&mut self, owner: HWND) -> Result<()> {
        if self.backdrop.as_ref().is_some_and(|v| v.owner != owner) {
            self.backdrop.take();
        }
        if self.backdrop.is_none() {
            self.backdrop = Some(Backdrop::new(owner)?);
        }
        self.backdrop.as_ref().unwrap().sync()
    }
    pub fn sync(&self) {
        if let Some(state) = &self.backdrop {
            let _ = state.sync();
        }
    }
    pub fn disable(&mut self) {
        self.backdrop.take();
    }
}
