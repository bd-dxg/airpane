//! 主线程消息循环。
//!
//! 窗口消息由系统派发；事件线程的唤醒消息与周期定时器由这里转成回调，避免每个
//! 调用方各写一遍 `GetMessageW` 循环。

use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, MSG, SetTimer, TranslateMessage, WM_APP, WM_TIMER,
};

use crate::events;

/// 事件监听线程用来唤醒主线程的消息号。
pub const WAKEUP_MESSAGE: u32 = WM_APP + 1;

/// 消息循环需要调用方处理的两种事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMessage {
    /// 事件线程投递了新事件，调用方应当把 channel 里的东西取空。
    Wakeup,
    /// 周期定时器触发了。
    Timer,
}

/// 在调用线程上运行消息循环，直到收到 `WM_QUIT`。
///
/// 定时器是线程定时器，不依赖任何窗口；窗口消息照常派发。
pub fn run(timer_ms: u32, mut on_message: impl FnMut(LoopMessage)) {
    // SAFETY: `GetCurrentThreadId` 无参数、无副作用。
    let thread_id = unsafe { GetCurrentThreadId() };
    events::set_wakeup(thread_id, WAKEUP_MESSAGE);

    // SAFETY: 传 `None` 表示创建线程定时器；不用回调，靠消息通知。
    let timer = unsafe { SetTimer(None, 0, timer_ms, None) };

    let mut msg = MSG::default();
    loop {
        // SAFETY: `msg` 是本栈帧上初始化的 MSG，窗口过滤传 None。
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        // 0 表示收到 WM_QUIT，-1 表示出错，两者都退出循环。
        if result.0 <= 0 {
            break;
        }
        match msg.message {
            WAKEUP_MESSAGE => on_message(LoopMessage::Wakeup),
            WM_TIMER if timer != 0 && msg.wParam.0 == timer => on_message(LoopMessage::Timer),
            _ => {
                // SAFETY: 消息已由 `GetMessageW` 填充。
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
    }

    if timer != 0 {
        // SAFETY: 只结束本线程自己创建的定时器。
        unsafe {
            let _ = KillTimer(None, timer);
        }
    }
}
