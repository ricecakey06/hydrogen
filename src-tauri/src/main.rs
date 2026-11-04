#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = hydrogen::desktop::run() {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
            let text: Vec<u16> = format!("{error}\n\nWebView2 Runtime 安装说明：https://developer.microsoft.com/microsoft-edge/webview2/\n程序不会自动下载或安装运行时。")
                .encode_utf16().chain(Some(0)).collect();
            let title: Vec<u16> = "Hydrogen 启动检查".encode_utf16().chain(Some(0)).collect();
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_ICONERROR | MB_OK,
            );
        }
        eprintln!("{error}");
        std::process::exit(1);
    }
}
