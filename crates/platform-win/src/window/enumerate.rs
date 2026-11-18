//! 顶层窗口枚举与注册表校准。

use airpane_core::registry::{TrackedWindow, WindowRegistry};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
use windows::core::BOOL;

use super::WindowInfo;
use super::is_window;

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
    handles
        .iter()
        .filter_map(|hwnd| super::inspect(*hwnd))
        .collect()
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

/// 用实时窗口信息校准注册表：移除已失效或不再可管理的窗口，并刷新其余窗口的字段。
///
/// 必需的兑底：实测 `EVENT_OBJECT_DESTROY` 在进程退出时不会投递，`WS_EX_*` 与 owner
/// 的变化也不产生事件（见 `docs/DECISIONS.md` D-020）。事件负责秒内响应，本函数负责
/// 纠偏。返回发生变化的窗口数量。
pub fn reconcile_registry(registry: &mut WindowRegistry) -> usize {
    let mut removals: Vec<i64> = Vec::new();
    let mut updates = Vec::new();

    for window in registry.iter() {
        match super::inspect_window(window.hwnd as isize) {
            Some(info) if info.is_managed() => updates.push(TrackedWindow {
                hwnd: info.hwnd as i64,
                pid: info.pid,
                process_name: info.process_name,
                class_name: info.class_name,
                title: info.title,
            }),
            _ => removals.push(window.hwnd),
        }
    }

    let mut changed = 0;
    for window in updates {
        let before = registry.iter().find(|w| w.hwnd == window.hwnd);
        if before != Some(&window) {
            changed += 1;
        }
        registry.upsert(window, false);
    }
    for hwnd in &removals {
        if registry.remove(*hwnd).is_some() {
            changed += 1;
        }
    }
    changed
}

/// 删除注册表中已失效的窗口，返回删除数量。
///
/// 必需的兑底：实测系统不会为进程退出而销毁的窗口发送 `EVENT_OBJECT_DESTROY`，
/// 只能靠 `IsWindow` 校验发现（见 `docs/DECISIONS.md` D-020）。
pub fn prune_dead_windows(registry: &mut WindowRegistry) -> usize {
    let dead: Vec<i64> = registry
        .iter()
        .map(|window| window.hwnd)
        .filter(|hwnd| !is_window(*hwnd as isize))
        .collect();
    for hwnd in &dead {
        registry.remove(*hwnd);
    }
    dead.len()
}
