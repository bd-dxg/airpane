//! 窗口枚举与句柄查询。
//!
//! 枚举顶层窗口，收集过滤所需的事实，交给 `airpane_core::window::classify` 判定，
//! 返回可被 AirPane 管理的窗口列表。
//!
//! 文件拆分（AGENTS.md）：底层字段读取在 `query`，进程信息在 `process`，
//! 枚举与注册表校准在 `enumerate`；本文件只留对外接口与单窗口判定。

mod enumerate;
mod process;
mod query;

use airpane_core::snapshot::{Rect, ShowState};
use airpane_core::window::{RejectReason, WindowFacts, classify};
use core::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_STYLE, GetWindowLongPtrW, IsIconic, IsWindow, IsZoomed, WS_EX_APPWINDOW,
    WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

pub use enumerate::{
    enum_managed_windows, enum_top_level_windows, prune_dead_windows, reconcile_registry,
};
pub use process::process_path;

use process::{query_process_name, resolve_app_pid};
use query::{
    has_ex_style, has_owner, is_cloaked, is_top_level, is_visible, window_class_name, window_pid,
    window_rect, window_title,
};

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

/// 读取窗口的可见边界（不含 DWM 在窗口外扩的阴影）。
///
/// Win11 起 `GetWindowRect` 返回的外框比看得见的窗口大一圈阴影，直接按它摆位会让
/// 窗口看起来偏内。`DWMWA_EXTENDED_FRAME_BOUNDS` 才是用户眼里的边界，项目内所有
/// 位置与尺寸计算都以它为准。
pub fn extended_frame_bounds(hwnd: isize) -> Option<Rect> {
    let window = HWND(hwnd as *mut c_void);
    let mut rect = windows::Win32::Foundation::RECT::default();
    // SAFETY: 输出缓冲是本栈帧上的 `RECT`，大小如实传入；句柄无效时返回错误。
    let result = unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut windows::Win32::Foundation::RECT as *mut c_void,
            size_of::<windows::Win32::Foundation::RECT>() as u32,
        )
    };
    result.is_ok().then_some(Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    })
}

/// 读取窗口的显示状态。句柄失效时按普通状态处理。
pub fn show_state(hwnd: isize) -> ShowState {
    if !is_window(hwnd) {
        return ShowState::Normal;
    }
    let window = HWND(hwnd as *mut c_void);
    // SAFETY: 只读查询，对任意句柄安全。
    let (iconic, zoomed) = unsafe { (IsIconic(window).as_bool(), IsZoomed(window).as_bool()) };
    if iconic {
        ShowState::Minimized
    } else if zoomed {
        ShowState::Maximized
    } else {
        ShowState::Normal
    }
}

/// 按窗口类名查找第一个顶层窗口的句柄。
pub fn find_window_by_class(class_name: &str) -> Option<isize> {
    struct Search<'a> {
        class_name: &'a str,
        found: Option<isize>,
    }

    /// `EnumWindows` 的回调：命中类名时记录句柄并停止枚举。
    ///
    /// 安全前提：`lparam` 必须指向存活的 `Search`。
    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: 见函数文档。
        let search = unsafe { &mut *(lparam.0 as *mut Search) };
        let class = window_class_name(hwnd);
        if !class.is_empty() && class.eq_ignore_ascii_case(search.class_name) {
            search.found = Some(hwnd.0 as isize);
            return BOOL::from(false);
        }
        BOOL::from(true)
    }

    let mut search = Search {
        class_name,
        found: None,
    };
    // SAFETY: 回调只写 `search`，该变量在调用期间存活。
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(&mut search as *mut Search as isize));
    }
    search.found
}

/// 窗口是否带有指定的窗口样式位（如 `WS_CAPTION`、`WS_MINIMIZEBOX`）。
pub fn has_window_style(hwnd: isize, style: i32) -> bool {
    if !is_window(hwnd) {
        return false;
    }
    // SAFETY: 只读查询，对任意句柄安全。
    let value = unsafe { GetWindowLongPtrW(HWND(hwnd as *mut c_void), GWL_STYLE) };
    (value as i32 & style) != 0
}

/// 读取窗口的类名。句柄失效时返回 `None`。
pub fn window_class(hwnd: isize) -> Option<String> {
    if !is_window(hwnd) {
        return None;
    }
    let class_name = window_class_name(HWND(hwnd as *mut c_void));
    (!class_name.is_empty()).then_some(class_name)
}

/// 判断窗口是否属于桌面（`Progman` / `WorkerW`）。
///
/// 按 Win+D 或点“显示桌面”后，前台会变成这两个类名的窗口。
pub fn is_desktop_window(hwnd: isize) -> bool {
    window_class(hwnd).is_some_and(|class| {
        class.eq_ignore_ascii_case("Progman") || class.eq_ignore_ascii_case("WorkerW")
    })
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

/// 读取单个原始句柄的窗口信息与过滤判定。句柄已失效时返回 `None`。
pub fn inspect_window(hwnd: isize) -> Option<WindowInfo> {
    if !is_window(hwnd) {
        return None;
    }
    inspect(HWND(hwnd as *mut c_void))
}

/// 读取单个窗口的信息与过滤判定。
fn inspect(hwnd: HWND) -> Option<WindowInfo> {
    let host_pid = window_pid(hwnd);
    let class_name = window_class_name(hwnd);
    let pid = resolve_app_pid(hwnd, &class_name, host_pid);
    let process_name = query_process_name(pid);
    let facts = WindowFacts {
        visible: is_visible(hwnd),
        tool_window: has_ex_style(hwnd, WS_EX_TOOLWINDOW.0 as isize),
        app_window: has_ex_style(hwnd, WS_EX_APPWINDOW.0 as isize),
        owned: has_owner(hwnd),
        cloaked: is_cloaked(hwnd),
        class_name: class_name.clone(),
        process_name: process_name.clone(),
        top_level: is_top_level(hwnd),
        own_process: host_pid == std::process::id(),
    };
    let verdict = classify(&facts);

    Some(WindowInfo {
        hwnd: hwnd.0 as isize,
        pid,
        process_name,
        class_name,
        title: window_title(hwnd),
        rect: window_rect(hwnd),
        facts,
        verdict,
    })
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
