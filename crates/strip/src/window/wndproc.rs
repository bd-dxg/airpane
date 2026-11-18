//! 预览区窗口的窗口过程与消息循环。

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DispatchMessageW, GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, MSG,
    PostQuitMessage, TranslateMessage, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT,
};

use airpane_platform_win::events;
use airpane_platform_win::foreground::activate;

use super::StripAction;
use super::layout::{
    apply_layout, client_height, client_width, contains, hit_test, relayout_window, scaled,
    toolbar_buttons, toolbar_height,
};
use super::paint::paint;
use super::state::StripState;

/// `WM_MOUSELEAVE` 在绑定里属于 `Win32_UI_Controls`，为一个常量不值得加 feature。
const WM_MOUSELEAVE: u32 = 0x02A3;

/// 打开 Per-Monitor V2 DPI 感知。进程内只需调用一次，重复调用会失败并被忽略。
pub fn enable_per_monitor_v2() {
    // SAFETY: 进程级设置，无参数指针；失败通常表示已经设置过。
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// 在预览区线程上运行消息循环，直到窗口销毁。
pub fn run_message_loop() {
    let mut msg = MSG::default();
    // SAFETY: `msg` 是本栈帧上的 MSG。
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        // SAFETY: 消息由 GetMessageW 填充。
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// 预览区窗口的消息处理。
///
/// 安全前提：由系统按窗口过程签名调用，句柄与用户数据都属于本进程。
pub(super) unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: 用户数据由本进程写入，窗口存续期间保持有效。
    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut StripState;

    match msg {
        WM_PAINT => {
            paint(hwnd, state);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_DPICHANGED => {
            // SAFETY: 状态指针由本进程维护。
            unsafe {
                relayout_window(hwnd, state);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if !state.is_null() {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                // SAFETY: 状态指针由本进程维护，只在窗口存续期间使用。
                let state_ref = unsafe { &mut *state };
                if !state_ref.tracking_leave {
                    state_ref.tracking_leave = true;
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    // SAFETY: `track` 是本栈帧上的结构体，字段已按 API 要求填好。
                    unsafe {
                        let _ = TrackMouseEvent(&mut track);
                    }
                }
                // 工具区上的移动不算悬停列表项。
                let list_bottom = client_height(hwnd) - toolbar_height(state_ref.dpi);
                let hovered = if y < list_bottom {
                    hit_test(&state_ref.entries, x, y).map(|entry| entry.window)
                } else {
                    None
                };
                if hovered != state_ref.hovered {
                    state_ref.hovered = hovered;
                    // 悬停只影响绘制，不重排，列表不会上下滑动（D-049）。
                    // SAFETY: 只重画本程序自己的窗口。
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, true);
                    }
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if !state.is_null() {
                // SAFETY: 状态指针由本进程维护，只在窗口存续期间使用。
                let state_ref = unsafe { &mut *state };
                state_ref.tracking_leave = false;
                if state_ref.hovered.take().is_some() {
                    // SAFETY: 只重画本程序自己的窗口。
                    unsafe {
                        let _ = InvalidateRect(Some(hwnd), None, true);
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            if !state.is_null() {
                // SAFETY: 状态指针由本进程维护，只在窗口存续期间使用。
                let (actions, entries) = unsafe {
                    let state = &*state;
                    (state.actions.clone(), &state.entries)
                };
                let dpi = unsafe { (*state).dpi };
                let buttons = toolbar_buttons(dpi, client_width(hwnd), client_height(hwnd));

                if contains(&buttons[0], x, y) {
                    let _ = actions.send(StripAction::OpenSettings);
                    events::wake();
                } else if contains(&buttons[1], x, y) {
                    let _ = actions.send(StripAction::ToggleMode);
                    events::wake();
                } else if y < client_height(hwnd) - toolbar_height(dpi) {
                    // 列表被工具区遮住的部分不参与点击；点当前窗口不做事。
                    if let Some(entry) = hit_test(entries, x, y) {
                        if !entry.active {
                            let _ = activate(entry.window);
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            if !state.is_null() {
                // SAFETY: 状态指针由本进程维护，只在窗口存续期间使用。
                let dpi = unsafe { (*state).dpi };
                // 滚轮增量在高 16 位，一格是 120。
                let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
                if delta != 0 {
                    let step = scaled(60, dpi);
                    // SAFETY: 同上。
                    unsafe {
                        (*state).scroll -= (delta as i32 / 120) * step;
                        apply_layout(hwnd, state);
                    }
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: 标准退出流程。
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => {
            // SAFETY: 未处理的消息交回系统默认处理。
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}
