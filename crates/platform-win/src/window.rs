//! 窗口枚举与句柄查询。
//!
//! 枚举顶层窗口，收集过滤所需的事实，交给 `airpane_core::window::classify` 判定，
//! 返回可被 AirPane 管理的窗口列表。

use airpane_core::snapshot::Rect;
use airpane_core::window::{RejectReason, WindowFacts, classify};
use core::ffi::c_void;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GWL_EXSTYLE, GetClassNameW, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextW, GetWindowThreadProcessId, IsWindow, IsWindowVisible, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, PWSTR};

const CLASS_NAME_LEN: usize = 256;
const PROCESS_PATH_LEN: usize = 512;

/// 一个顶层窗口及其过滤判定结果，供诊断与筛选使用。
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub pid: u32,
    pub process_name: String,
    pub class_name: String,
    pub title: String,
    /// 窗口矩形，物理像素。取 `GetWindowRect`，含窗口阴影边框。
    pub rect: Rect,
    pub facts: WindowFacts,
    /// `Ok(())` 表示可被管理。
    pub verdict: Result<(), RejectReason>,
}

impl WindowInfo {
    /// 是否可被 AirPane 管理。
    pub fn is_managed(&self) -> bool {
        self.verdict.is_ok()
    }
}

/// 判断一个原始整数句柄是否仍指向存在的窗口。
///
/// 安全前提：`IsWindow` 只读取句柄表，对任意取值都是安全查询，不会解引用调用方
/// 提供的指针。
pub fn is_window(hwnd: isize) -> bool {
    let hwnd = HWND(hwnd as *mut c_void);
    // SAFETY: 见函数文档，查询本身对无效句柄安全。
    unsafe { IsWindow(Some(hwnd)) }.as_bool()
}

/// 枚举当前虚拟桌面上可被管理的顶层窗口。
pub fn enum_managed_windows() -> Vec<WindowInfo> {
    enum_top_level_windows()
        .into_iter()
        .filter(WindowInfo::is_managed)
        .collect()
}

/// 枚举所有顶层窗口及其过滤判定结果，用于诊断过滤规则。
pub fn enum_top_level_windows() -> Vec<WindowInfo> {
    let mut handles: Vec<HWND> = Vec::new();
    // SAFETY: `collect_handle` 只把句柄压入 `handles`，`handles` 在调用期间不会被移动。
    unsafe {
        let _ = EnumWindows(
            Some(collect_handle),
            LPARAM(&mut handles as *mut Vec<HWND> as isize),
        );
    }
    handles.iter().filter_map(|hwnd| inspect(*hwnd)).collect()
}

/// `EnumWindows` 的回调，把所有顶层窗口句柄收进 `lparam` 指向的 `Vec`。
///
/// 安全前提：`lparam` 必须是调用方传入的、指向存活 `Vec<HWND>` 的指针。
unsafe extern "system" fn collect_handle(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: 见函数文档，指针由 `enum_managed_windows` 保证有效。
    let handles = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
    handles.push(hwnd);
    BOOL::from(true)
}

/// UWP 应用的顶层窗口由 `ApplicationFrameHost.exe` 承载，真实应用进程在其子窗口里。
const APPLICATION_FRAME_WINDOW_CLASS: &str = "ApplicationFrameWindow";

/// 读取单个窗口的信息与过滤判定。
fn inspect(hwnd: HWND) -> Option<WindowInfo> {
    let host_pid = window_pid(hwnd);
    let class_name = window_class_name(hwnd);
    let pid = resolve_app_pid(hwnd, &class_name, host_pid);
    let facts = WindowFacts {
        visible: is_visible(hwnd),
        tool_window: has_tool_window_style(hwnd),
        cloaked: is_cloaked(hwnd),
        class_name: class_name.clone(),
        own_process: host_pid == std::process::id(),
    };
    let verdict = classify(&facts);

    Some(WindowInfo {
        hwnd: hwnd.0 as isize,
        pid,
        process_name: query_process_name(pid),
        class_name,
        title: window_title(hwnd),
        rect: window_rect(hwnd),
        facts,
        verdict,
    })
}

/// 对 UWP 的 `ApplicationFrameWindow` 返回真实应用进程；其他窗口直接返回自身 PID。
fn resolve_app_pid(hwnd: HWND, class_name: &str, host_pid: u32) -> u32 {
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

fn is_visible(hwnd: HWND) -> bool {
    // SAFETY: 只读查询，对任意句柄安全。
    unsafe { IsWindowVisible(hwnd) }.as_bool()
}

fn has_tool_window_style(hwnd: HWND) -> bool {
    // SAFETY: 只读查询，对任意句柄安全。
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    (ex_style & WS_EX_TOOLWINDOW.0 as isize) != 0
}

fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked: u32 = 0;
    // SAFETY: 输出缓冲是本栈帧上的 `u32`，大小如实传入。
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            size_of::<u32>() as u32,
        )
    };
    result.is_ok() && cloaked != 0
}

fn window_pid(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: 输出参数是本栈帧上的 `u32`。
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

fn window_class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; CLASS_NAME_LEN];
    // SAFETY: 缓冲区长度如实传入，函数最多写入该长度。
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

fn window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; CLASS_NAME_LEN];
    // SAFETY: 缓冲区长度如实传入，函数最多写入该长度。
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

fn window_rect(hwnd: HWND) -> Rect {
    let mut rect = windows::Win32::Foundation::RECT::default();
    // SAFETY: 输出参数是本栈帧上的 `RECT`。
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
    }
    Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

/// 取进程可执行文件名（不含路径）。拿不到时返回空串，不影响其他过滤规则。
fn query_process_name(pid: u32) -> String {
    if pid == 0 {
        return String::new();
    }
    let mut buf = [0u16; PROCESS_PATH_LEN];
    let mut size = buf.len() as u32;
    // SAFETY: 以查询权限打开进程，缓冲区长度如实传入；失败时直接返回空串。
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);
        if result.is_err() {
            return String::new();
        }
    }
    let path = String::from_utf16_lossy(&buf[..size as usize]);
    Path::new(&path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_handle_is_not_a_window() {
        assert!(!is_window(0));
    }

    #[test]
    fn enumeration_does_not_panic() {
        // CI 的 runner 可能没有可见窗口，这里只要求枚举过程不 panic 且自己不在列表里。
        let windows = enum_managed_windows();
        assert!(windows.iter().all(|w| w.pid != std::process::id()));
    }
}
