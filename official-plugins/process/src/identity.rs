pub fn process_stamp(pid: u32) -> Option<u64> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::{
            Foundation::{CloseHandle, FILETIME},
            System::Threading::{
                GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let mut code = 0;
        let valid = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user)
            .is_ok()
            && GetExitCodeProcess(handle, &mut code).is_ok()
            && code == 259;
        let _ = CloseHandle(handle);
        valid.then_some((created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64)
    }
    #[cfg(not(windows))]
    {
        std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()?
            .split_whitespace()
            .nth(21)?
            .parse()
            .ok()
    }
}
