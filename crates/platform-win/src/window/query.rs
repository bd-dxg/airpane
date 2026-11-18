//! 单个窗口的底层字段读取：可见性、样式、类名、标题、矩形。

use airpane_core::snapshot::Rect;
use core::ffi::c_void;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GW_OWNER, GWL_EXSTYLE, GetAncestor, GetClassNameW, GetWindow, GetWindowLongPtrW,
    GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};

/// 类名与标题缓冲区长度。Windows 的类名上限是 256 个字符。
pub(super) const CLASS_NAME_LEN: usize = 256;

pub(super) fn is_visible(hwnd: HWND) -> bool {
    // SAFETY: 只读查询，对任意句柄安全。
    unsafe { IsWindowVisible(hwnd) }.as_bool()
}

pub(super) fn has_ex_style(hwnd: HWND, style: isize) -> bool {
    // SAFETY: 只读查询，对任意句柄安全。
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    (ex_style & style) != 0
}

/// 顶层窗口的根祖先就是它自己。子窗口不应进入预览区。
pub(super) fn is_top_level(hwnd: HWND) -> bool {
    // SAFETY: 只读查询，对任意句柄安全。
    unsafe { GetAncestor(hwnd, GA_ROOT) == hwnd }
}

/// 有 owner 的窗口通常是对话框或应用自建的辅助窗口，不应进入预览区。
pub(super) fn has_owner(hwnd: HWND) -> bool {
    // SAFETY: 只读查询，对任意句柄安全。
    let owner = unsafe { GetWindow(hwnd, GW_OWNER) };
    owner.is_ok_and(|owner| !owner.0.is_null())
}

pub(super) fn is_cloaked(hwnd: HWND) -> bool {
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

pub(super) fn window_pid(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: 输出参数是本栈帧上的 `u32`。
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

pub(super) fn window_class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; CLASS_NAME_LEN];
    // SAFETY: 缓冲区长度如实传入，函数最多写入该长度。
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

pub(super) fn window_title(hwnd: HWND) -> String {
    let mut buf = [0u16; CLASS_NAME_LEN];
    // SAFETY: 缓冲区长度如实传入，函数最多写入该长度。
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

pub(super) fn window_rect(hwnd: HWND) -> Rect {
    // 优先用 DWM 的可见边界，它才是用户看到的矩形；摆位与快照都以此为准。
    if let Some(rect) = super::extended_frame_bounds(hwnd.0 as isize) {
        return rect;
    }
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
