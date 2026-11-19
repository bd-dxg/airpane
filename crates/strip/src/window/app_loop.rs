//! 预览区的消息循环与 DPI 感知设置。

use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, TranslateMessage,
};

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
