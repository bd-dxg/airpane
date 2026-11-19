//! 合成层的即时重绘辅助。

use core::ffi::c_void;

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{RDW_ALLCHILDREN, RDW_INVALIDATE, RedrawWindow};

/// 让 `hwnd` 重画，并等下一帧合成完成。
///
/// 隐藏置顶的预览区后，它盖住的区域要由下层窗口补画；不催一下时偶尔会露一帧桌面。
/// 故意不用 `RDW_UPDATENOW`：那会同步给别的进程发 `WM_PAINT`，对方卡住时本线程跟着卡。
pub fn redraw_now(hwnd: isize) {
    if hwnd == 0 {
        return;
    }
    let window = HWND(hwnd as *mut c_void);
    // SAFETY: 只请求重画，不读写调用方内存；句柄无效时 API 返回失败。
    unsafe {
        let _ = RedrawWindow(Some(window), None, None, RDW_INVALIDATE | RDW_ALLCHILDREN);
        let _ = DwmFlush();
    }
}
