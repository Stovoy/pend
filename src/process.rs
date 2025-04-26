//! Small cross-platform helper to query whether a given PID is currently alive.

#[cfg(unix)]
pub(crate) fn process_is_alive(pid: u32) -> bool {
    // Safety: kill with signal 0 performs error checking only, no signal sent.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(windows)]
pub(crate) fn process_is_alive(pid: u32) -> bool {
    // Fall back to a conservative answer on Windows – if we cannot confirm
    // the process has exited we assume it is still running so that callers
    // play it safe.
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, WaitForSingleObject, INFINITE,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle == 0 {
            return false;
        }
        // Query immediately whether the process has already exited.
        const WAIT_TIMEOUT: u32 = 0x00000102;
        let status = WaitForSingleObject(handle, 0);
        let alive = status == WAIT_TIMEOUT;
        CloseHandle(handle);
        alive
    }
}
