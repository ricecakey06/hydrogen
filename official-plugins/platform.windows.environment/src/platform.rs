use hydrogen_plugin_sdk::contracts::{ApiError, Result};
use std::path::{Path, PathBuf};

#[cfg(windows)]
pub fn available_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let directory: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0;
    unsafe {
        windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            windows::core::PCWSTR(directory.as_ptr()),
            Some(&mut available),
            None,
            None,
        )
        .ok()?;
    }
    Some(available)
}

pub fn choose(owner: isize, folder: bool, multiple: bool) -> Result<Vec<PathBuf>> {
    #[cfg(windows)]
    {
        // A modal Shell dialog needs its own STA and message loop. Keeping it off
        // the WebView event loop also lets its owner process modal window messages.
        std::thread::Builder::new()
            .name("hydrogen-file-dialog".into())
            .spawn(move || choose_sta(owner, folder, multiple))?
            .join()
            .map_err(|_| ApiError::new("DIALOG_ERROR", "系统文件选择器执行失败。"))?
    }
    #[cfg(not(windows))]
    {
        let _ = (owner, folder, multiple);
        Err(ApiError::new(
            "PLATFORM_UNSUPPORTED",
            "当前平台尚未接入原生文件选择器。",
        ))
    }
}

#[cfg(windows)]
fn choose_sta(owner: isize, folder: bool, multiple: bool) -> Result<Vec<PathBuf>> {
    unsafe {
        use windows::Win32::{
            Foundation::HWND,
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
            },
            UI::Shell::*,
        };
        CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
            .ok()
            .map_err(dialog_error)?;
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _apartment = Apartment;
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).map_err(dialog_error)?;
        let mut options = FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_FILEMUSTEXIST;
        if folder {
            options |= FOS_PICKFOLDERS;
        }
        if multiple {
            options |= FOS_ALLOWMULTISELECT;
        }
        dialog.SetOptions(options).map_err(dialog_error)?;
        // Use the inherited IFileDialog vtable. Querying IModalWindow separately
        // can expose a different Shell interface pointer on current Windows.
        if let Err(error) = dialog.Show(Some(HWND(owner as _))) {
            if error.code().0 as u32 == 0x800704c7 {
                return Ok(Vec::new());
            }
            return Err(dialog_error(error));
        }
        let items = dialog.GetResults().map_err(dialog_error)?;
        let mut paths = Vec::new();
        for index in 0..items.GetCount().map_err(dialog_error)? {
            let item = items.GetItemAt(index).map_err(dialog_error)?;
            let raw = item
                .GetDisplayName(SIGDN_FILESYSPATH)
                .map_err(dialog_error)?;
            let text = raw.to_string();
            CoTaskMemFree(Some(raw.0 as _));
            let text = text.map_err(|_| ApiError::new("INVALID_PATH", "所选路径编码无效。"))?;
            paths.push(PathBuf::from(text));
        }
        Ok(paths)
    }
}
#[cfg(windows)]
fn dialog_error(_: windows::core::Error) -> ApiError {
    ApiError::new("DIALOG_ERROR", "无法打开系统文件选择器。")
}

pub fn browser(url: &str) -> Result<()> {
    if !url::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https" && matches!(u.host_str(), Some("login.microsoftonline.com"))
    }) {
        return Err(ApiError::new(
            "INVALID_BROWSER_URL",
            "仅允许打开微软授权页面。",
        ));
    }
    open_browser(url)
}

pub fn open_about_link(link: &str) -> Result<()> {
    let url = match link {
        "developer" => "https://space.bilibili.com/365980549",
        "bmclapi" => "https://www.bangbang93.com/",
        "mcim" => "https://www.mcimirror.top/",
        "releases" => "https://github.com/ricecakey06/hydrogen/releases/latest",
        "miSansLicense" => {
            "https://hyperos.mi.com/font-download/MiSans%E5%AD%97%E4%BD%93%E7%9F%A5%E8%AF%86%E4%BA%A7%E6%9D%83%E8%AE%B8%E5%8F%AF%E5%8D%8F%E8%AE%AE.pdf"
        }
        _ => return Err(ApiError::new("INVALID_BROWSER_URL", "未知的关于页面链接。")),
    };
    open_browser(url)
}

fn open_browser(url: &str) -> Result<()> {
    #[cfg(windows)]
    unsafe {
        use windows::{
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
            core::{PCWSTR, w},
        };
        let wide: Vec<_> = url.encode_utf16().chain(Some(0)).collect();
        if ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
        .0 as usize
            <= 32
        {
            return Err(ApiError::new(
                "BROWSER_ERROR",
                "无法打开系统浏览器，请检查默认浏览器设置。",
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new(if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        })
        .arg(url)
        .spawn()?;
        Ok(())
    }
}
pub fn open_folder(path: &Path) -> Result<()> {
    if !path.is_dir() {
        return Err(ApiError::new("DIRECTORY_MISSING", "目录不存在。"));
    }
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot")
            .ok_or_else(|| ApiError::new("PLATFORM_ERROR", "Windows 系统目录不可用。"))?;
        std::process::Command::new(PathBuf::from(root).join("explorer.exe"))
            .arg(path)
            .spawn()?;
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new(if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        })
        .arg(path)
        .spawn()?;
    }
    Ok(())
}

pub fn open_text(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || !path.is_file()
        || !path.extension().is_some_and(|extension| {
            extension.eq_ignore_ascii_case("log") || extension.eq_ignore_ascii_case("txt")
        })
        || path.ancestors().any(|ancestor| {
            std::fs::symlink_metadata(ancestor).is_ok_and(|metadata| {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes() & 0x400 != 0
                }
                #[cfg(not(windows))]
                {
                    metadata.file_type().is_symlink()
                }
            })
        })
    {
        return Err(ApiError::new("LOG_MISSING", "日志文件不存在或路径无效。"));
    }
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot")
            .ok_or_else(|| ApiError::new("PLATFORM_ERROR", "Windows 系统目录不可用。"))?;
        std::process::Command::new(PathBuf::from(root).join("System32/notepad.exe"))
            .arg(path)
            .spawn()?;
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new(if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        })
        .arg(path)
        .spawn()?;
    }
    Ok(())
}

pub fn os_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        #[cfg(windows)]
        {
            #[repr(C)]
            struct Version {
                size: u32,
                major: u32,
                minor: u32,
                build: u32,
                platform: u32,
                service: [u16; 128],
            }
            #[link(name = "ntdll")]
            unsafe extern "system" {
                fn RtlGetVersion(info: *mut Version) -> i32;
            }
            let mut version = Version {
                size: std::mem::size_of::<Version>() as u32,
                major: 0,
                minor: 0,
                build: 0,
                platform: 0,
                service: [0; 128],
            };
            if unsafe { RtlGetVersion(&mut version) } == 0 {
                return format!("{}.{}.{}", version.major, version.minor, version.build);
            }
        }
        String::new()
    })
}
