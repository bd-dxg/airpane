//! 窗口所属进程的解析：可执行文件路径、进程名与 UWP 的真实应用进程。

use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;
use windows::core::{BOOL, PWSTR};

use super::query::window_pid;

/// 进程可执行文件路径的缓冲区长度。
const PROCESS_PATH_LEN: usize = 512;

/// UWP 应用的顶层窗口由 `ApplicationFrameHost.exe` 承载，真实应用进程在其子窗口里。
const APPLICATION_FRAME_WINDOW_CLASS: &str = "ApplicationFrameWindow";

/// 进程可执行文件的完整路径。拿不到时返回 `None`。
///
/// 预览区按 exe 取文件图标时会用到（D-046）。
pub fn process_path(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let mut buf = [0u16; PROCESS_PATH_LEN];
    let mut size = buf.len() as u32;
    // SAFETY: 以查询权限打开进程，缓冲区长度如实传入；失败时返回 None。
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);
        result.ok()?;
    }
    Some(String::from_utf16_lossy(&buf[..size as usize]))
}

/// 取进程可执行文件名（不含路径）。拿不到时返回空串，不影响其他过滤规则。
pub(super) fn query_process_name(pid: u32) -> String {
    match process_path(pid) {
        Some(path) => Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or(path),
        None => String::new(),
    }
}

/// 对 UWP 的 `ApplicationFrameWindow` 返回真实应用进程；其他窗口直接返回自身 PID。
pub(super) fn resolve_app_pid(hwnd: HWND, class_name: &str, host_pid: u32) -> u32 {
    if !class_name.eq_ignore_ascii_case(APPLICATION_FRAME_WINDOW_CLASS) {
        return host_pid;
    }
    let mut child_pids: Vec<u32> = Vec::new();
    // SAFETY: 回调只把 pid 压入 `child_pids`，该 Vec 在调用期间保持有效。
    unsafe {
        let _ = EnumChildWindows(
            Some(hwnd),
            Some(collect_child_pid),
            LPARAM(&mut child_pids as *mut Vec<u32> as isize),
        );
    }
    child_pids
        .into_iter()
        .find(|pid| *pid != 0 && *pid != host_pid)
        .unwrap_or(host_pid)
}

/// `EnumChildWindows` 的回调，收集子窗口所属进程。
///
/// 安全前提：`lparam` 必须指向存活的 `Vec<u32>`。
unsafe extern "system" fn collect_child_pid(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: 见函数文档。
    let pids = unsafe { &mut *(lparam.0 as *mut Vec<u32>) };
    pids.push(window_pid(hwnd));
    BOOL::from(true)
}
