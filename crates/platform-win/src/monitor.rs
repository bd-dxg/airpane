//! 主显示器几何与全屏判定。
//!
//! M1 只在主显示器上启用 Stage 模式（PRD 4.1）。全屏判定是 M1 的最小保护：前台
//! 窗口覆盖整个显示器或处于最大化时，不移动、不缩放它（D-026）。完整的进程名单与
//! 暂停防抖留给 M2。

use core::ffi::c_void;

use airpane_core::snapshot::Rect;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::UI::WindowsAndMessaging::{
    IsZoomed, SPI_GETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};

use crate::window::{extended_frame_bounds, is_window};

/// 主显示器的可用区域（已扣除任务栏）。
pub fn primary_work_area() -> Rect {
    let mut rect = RECT::default();
    // SAFETY: 输出缓冲是本栈帧上的 `RECT`，类型与大小一致。
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rect as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_ok() {
        return to_rect(rect);
    }
    // 回退：主显示器整屏再手动扣任务栏的常见高度。
    primary_monitor_rect()
}

/// 主显示器的完整矩形（含任务栏区域）。
pub fn primary_monitor_rect() -> Rect {
    // SAFETY: 坐标点 (0,0) 与标志都是纯输入，不需要额外的资源。
    let monitor = unsafe {
        MonitorFromPoint(
            windows::Win32::Foundation::POINT { x: 0, y: 0 },
            MONITOR_DEFAULTTOPRIMARY,
        )
    };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info.cbSize` 已按结构体大小填入，输出缓冲存活于本栈帧。
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return to_rect(info.rcMonitor);
    }
    Rect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    }
}

/// 窗口是否处于最大化。
pub fn is_maximized(hwnd: isize) -> bool {
    if !is_window(hwnd) {
        return false;
    }
    // SAFETY: 只读查询，对任意句柄安全。
    unsafe { IsZoomed(windows::Win32::Foundation::HWND(hwnd as *mut c_void)).as_bool() }
}

/// 窗口的可见边界是否覆盖了整个主显示器。
///
/// 允许 1 像素误差：全屏播放器与游戏常用无边框全屏，边界可能正好差 1px。
pub fn covers_primary_monitor(hwnd: isize) -> bool {
    let Some(bounds) = extended_frame_bounds(hwnd) else {
        return false;
    };
    let monitor = primary_monitor_rect();
    bounds.left <= monitor.left + 1
        && bounds.top <= monitor.top + 1
        && bounds.right >= monitor.right - 1
        && bounds.bottom >= monitor.bottom - 1
}

/// M1 的最小全屏保护：命中时不移动、不缩放该窗口。
pub fn should_skip_placement(hwnd: isize) -> bool {
    is_maximized(hwnd) || covers_primary_monitor(hwnd)
}

fn to_rect(rect: RECT) -> Rect {
    Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_area_is_not_empty() {
        let area = primary_work_area();
        assert!(area.width() > 0, "工作区宽度应当为正");
        assert!(area.height() > 0, "工作区高度应当为正");
    }

    #[test]
    fn invalid_window_is_never_maximized_or_fullscreen() {
        assert!(!is_maximized(0));
        assert!(!covers_primary_monitor(0));
        assert!(!should_skip_placement(0));
    }
}
