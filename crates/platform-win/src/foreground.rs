//! 可靠地把目标窗口切到前台。
//!
//! `SetForegroundWindow` 受前台锁限制：调用进程不是前台进程时，系统可能只让任务栏按钮
//! 闪烁而不切窗口。这里先直接尝试，失败后用 `AttachThreadInput` 共享输入状态再试一次；
//! 两次都失败就把错误交给调用方，不做反复重试，避免任务栏持续闪烁。

use core::ffi::c_void;
use core::fmt;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsZoomed,
    SW_MAXIMIZE, SW_RESTORE, SetForegroundWindow, ShowWindow, ShowWindowAsync,
};

use crate::window::is_window;

/// 前台切换失败的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivateError {
    /// 句柄已失效。
    InvalidWindow,
    /// 系统拒绝把该窗口切到前台。
    Rejected,
}

impl fmt::Display for ActivateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWindow => f.write_str("窗口句柄已失效"),
            Self::Rejected => f.write_str("系统拒绝切换前台"),
        }
    }
}

/// 当前的前台窗口。
pub fn foreground_window() -> isize {
    // SAFETY: 无参数只读查询。
    unsafe { GetForegroundWindow().0 as isize }
}

/// 把窗口改为最大化状态。
///
/// 用于退出 Stage 模式时恢复原本最大化的窗口。
pub fn maximize(hwnd: isize) -> bool {
    if !is_window(hwnd) {
        return false;
    }
    let target = HWND(hwnd as *mut c_void);
    // SAFETY: 只对目标窗口调 `SW_MAXIMIZE`，不抢焦点。
    unsafe {
        let _ = ShowWindow(target, SW_MAXIMIZE);
    }
    // SAFETY: 只读查询。
    unsafe { IsZoomed(target) }.as_bool()
}

/// 同步还原最小化窗口，返回它是否已经不再是图标状态。
///
/// 与 [`activate`] 里的 `ShowWindowAsync` 不同，这个调用会等窗口完成还原。需要用
/// 同步版本时，是因为调用方紧接着就要读窗口的真实矩形（异步还原会读到旧值）。
pub fn restore(hwnd: isize) -> bool {
    if !is_window(hwnd) {
        return false;
    }
    let target = HWND(hwnd as *mut c_void);
    // SAFETY: 只对目标窗口调 `SW_RESTORE`，不抢焦点、不改 Z 序。
    unsafe {
        let _ = ShowWindow(target, SW_RESTORE);
    }
    // SAFETY: 只读查询。
    !unsafe { IsIconic(target) }.as_bool()
}

/// 把目标窗口切到前台并获取焦点。已在前台时直接返回成功。
pub fn activate(hwnd: isize) -> Result<(), ActivateError> {
    if !is_window(hwnd) {
        return Err(ActivateError::InvalidWindow);
    }
    let target = HWND(hwnd as *mut c_void);

    if foreground_window() == hwnd {
        return Ok(());
    }

    // SAFETY: 句柄已校验；最小化窗口先还原，否则无法获得焦点。
    unsafe {
        if IsIconic(target).as_bool() {
            let _ = ShowWindowAsync(target, SW_RESTORE);
        }
    }

    // 只有本进程已是前台进程时，直接调用才不会让任务栏按钮闪烁。
    if foreground_belongs_to_current_process() {
        // SAFETY: 句柄已校验。
        unsafe {
            let _ = SetForegroundWindow(target);
        }
        if foreground_window() == hwnd {
            return Ok(());
        }
    }

    // 兜底：把本线程的输入状态挂到当前前台线程上，绕过前台锁。
    // 直接挂到目标线程无效，因为锁是由前台线程持有的。
    // SAFETY: `AttachThreadInput` 只影响输入队列归属，配对解除后恢复原状。
    unsafe {
        let current_thread = GetCurrentThreadId();
        let foreground = GetForegroundWindow();
        let foreground_thread = if foreground.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(foreground, None)
        };
        let attached = foreground_thread != 0
            && foreground_thread != current_thread
            && AttachThreadInput(current_thread, foreground_thread, true).as_bool();

        let _ = BringWindowToTop(target);
        let _ = SetForegroundWindow(target);
        let _ = SetFocus(Some(target));

        if attached {
            let _ = AttachThreadInput(current_thread, foreground_thread, false);
        }
    }

    if foreground_window() == hwnd {
        Ok(())
    } else {
        Err(ActivateError::Rejected)
    }
}

/// 当前前台窗口是否属于本进程。
fn foreground_belongs_to_current_process() -> bool {
    // SAFETY: 无参数只读查询；`pid` 是本栈帧上的输出参数。
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.0.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(foreground, Some(&mut pid));
        pid == std::process::id()
    }
}
