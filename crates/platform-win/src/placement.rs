//! 移动他人窗口的唯一入口。
//!
//! PRD 11.3 第 3 条与 D-005：调用方必须先落盘快照，才能调用本模块。这里只负责
//! 计算与执行，不负责记录与回滚。

use core::ffi::c_void;
use core::fmt;

use airpane_core::snapshot::Rect;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
};

use crate::window::extended_frame_bounds;

/// 移动窗口失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlacementError {
    /// 句柄已失效。
    InvalidWindow,
    /// 系统拒绝移动，附带 Win32 错误码。
    Rejected(String),
}

impl fmt::Display for PlacementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWindow => f.write_str("窗口句柄已失效"),
            Self::Rejected(reason) => write!(f, "移动窗口失败: {reason}"),
        }
    }
}

/// 把窗口的**可见边界**摆到 `target`。
///
/// `SetWindowPos` 操作的是含阴影的外框，所以这里先量出可见边界与外框的差值再补偿，
/// 保证摆完之后用户看到的矩形等于 `target`。不改变 Z 序、不抢焦点。
///
/// `target` 必须来自 [`extended_frame_bounds`] 或 `WindowInfo::rect`，两者语义相同。
pub fn move_window(hwnd: isize, target: Rect) -> Result<(), PlacementError> {
    if !crate::window::is_window(hwnd) {
        return Err(PlacementError::InvalidWindow);
    }
    let window = HWND(hwnd as *mut c_void);
    let mut frame = RECT::default();
    // SAFETY: 输出参数是本栈帧上的 `RECT`；句柄无效时返回错误。
    unsafe { GetWindowRect(window, &mut frame) }
        .map_err(|err| PlacementError::Rejected(err.message()))?;

    let visible = extended_frame_bounds(hwnd).unwrap_or(Rect {
        left: frame.left,
        top: frame.top,
        right: frame.right,
        bottom: frame.bottom,
    });

    let left_pad = visible.left - frame.left;
    let top_pad = visible.top - frame.top;
    let right_pad = frame.right - visible.right;
    let bottom_pad = frame.bottom - visible.bottom;

    let x = target.left - left_pad;
    let y = target.top - top_pad;
    let width = (target.width() + left_pad + right_pad).max(1);
    let height = (target.height() + top_pad + bottom_pad).max(1);

    // SAFETY: 只调整目标窗口的位置与大小；不改变 Z 序、不激活，不会抢走用户焦点。
    unsafe {
        SetWindowPos(
            window,
            None,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )
    }
    .map_err(|err| PlacementError::Rejected(err.message()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::extended_frame_bounds;

    #[test]
    fn invalid_handle_is_rejected() {
        assert!(
            move_window(
                0,
                Rect {
                    left: 0,
                    top: 0,
                    right: 100,
                    bottom: 100,
                }
            )
            .is_err()
        );
        assert!(extended_frame_bounds(0).is_none());
    }
}
