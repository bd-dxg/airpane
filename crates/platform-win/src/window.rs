//! 窗口句柄的最小封装。后续的枚举与事件同步任务在此基础上扩展。

use core::ffi::c_void;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::IsWindow;

/// 判断一个原始整数句柄是否仍指向存在的窗口。
///
/// 安全前提：`IsWindow` 只读取句柄表，对任意取值都是安全查询，不会解引用调用方
/// 提供的指针。
pub fn is_window(hwnd: isize) -> bool {
    let hwnd = HWND(hwnd as *mut c_void);
    // SAFETY: 见函数文档，查询本身对无效句柄安全。
    unsafe { IsWindow(Some(hwnd)) }.as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_handle_is_not_a_window() {
        assert!(!is_window(0));
    }
}
