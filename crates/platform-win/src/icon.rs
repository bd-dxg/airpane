//! 应用图标：三级兜底获取与句柄所有权管理（D-046）。
//!
//! 顺序是「窗口自己的图标 → 窗口类图标 → exe 文件图标」。前两级拿到的是系统里
//! 已有的句柄，不能销毁；第三级由 `SHGetFileInfoW` 新分配，必须 `DestroyIcon`。
//! 这个差别由 [`AppIcon`] 的 `owned` 标记记住，Drop 时只释放自己的。

use core::ffi::c_void;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGetFileInfoW};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, GCLP_HICON, GetClassLongPtrW, HICON, ICON_BIG, ICON_SMALL, ICON_SMALL2,
    SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_GETICON,
};
use windows::core::PCWSTR;

use crate::window::process_path;

/// 一次查询（或分配）得到的应用图标。
pub struct AppIcon {
    handle: HICON,
    /// 句柄是否由本进程分配。`true` 时 Drop 会调 `DestroyIcon`。
    owned: bool,
}

impl AppIcon {
    pub fn handle(&self) -> HICON {
        self.handle
    }
}

impl Drop for AppIcon {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: 只有 `SHGetFileInfoW` 返回的句柄由本进程持有，销毁一次。
            unsafe {
                let _ = DestroyIcon(self.handle);
            }
        }
    }
}

/// 取窗口所属应用的图标，三级兜底。
pub fn app_icon(hwnd: isize, pid: u32) -> Option<AppIcon> {
    window_icon(hwnd)
        .or_else(|| class_icon(hwnd))
        .or_else(|| file_icon(pid))
}

/// 第一级：`WM_GETICON`。大图标优先，其次任务栏用的 `ICON_SMALL2`，最后 `ICON_SMALL`。
fn window_icon(hwnd: isize) -> Option<AppIcon> {
    let window = HWND(hwnd as *mut c_void);
    for kind in [ICON_BIG, ICON_SMALL2, ICON_SMALL] {
        let mut result: usize = 0;
        // SAFETY: `WM_GETICON` 由目标窗口的消息循环回答，`SMTO_ABORTIFHUNG`
        // 保证目标进程无响应时 100ms 后返回而不是挂住本线程。三级都超时
        // 也会拖 300ms，但这只在每个应用第一次进列表时付一次。
        let sent = unsafe {
            SendMessageTimeoutW(
                window,
                WM_GETICON,
                WPARAM(kind as usize),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                100,
                Some(&mut result),
            )
        };
        if sent.0 != 0 && result != 0 {
            return Some(AppIcon {
                handle: HICON(result as *mut c_void),
                owned: false,
            });
        }
    }
    None
}

/// 第二级：窗口类注册时设置的大图标。
fn class_icon(hwnd: isize) -> Option<AppIcon> {
    let window = HWND(hwnd as *mut c_void);
    // SAFETY: 只读查询类信息，对任意句柄安全。
    let raw = unsafe { GetClassLongPtrW(window, GCLP_HICON) };
    (raw != 0).then_some(AppIcon {
        handle: HICON(raw as *mut c_void),
        owned: false,
    })
}

/// 第三级：按 exe 路径取文件图标，覆盖前两级都没设图标的窗口。
fn file_icon(pid: u32) -> Option<AppIcon> {
    let path = process_path(pid)?;
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut info = SHFILEINFOW::default();
    // SAFETY: 路径以 NUL 结尾且在调用期间存活；`info` 是本栈帧上的输出缓冲，
    // 结构体大小如实传入。返回的 hIcon 归调用方所有，由 `AppIcon` 负责销毁。
    let ok = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            Default::default(),
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_SMALLICON,
        )
    };
    (ok != 0 && !info.hIcon.0.is_null()).then_some(AppIcon {
        handle: info.hIcon,
        owned: true,
    })
}
