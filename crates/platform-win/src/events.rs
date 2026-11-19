//! 窗口事件监听。
//!
//! 使用 `SetWinEventHook` 的 `WINEVENT_OUTOFCONTEXT` 模式：系统把事件投递到本进程
//! 的消息循环，不向目标进程注入任何代码（PRD 11.3 第 1 条）。监听线程自己跑消息
//! 循环，事件通过 channel 交给调用方。

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, RecvError, Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, JoinHandle};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE,
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_SHOW,
    EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART,
    GetForegroundWindow, GetMessageW, MSG, OBJID_WINDOW, PostThreadMessageW, TranslateMessage,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_QUIT,
};

/// 监听线程收到的窗口事件。`hwnd` 是原始句柄值，跨进程重启后失效。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowEvent {
    /// 前台窗口变化。
    Foreground {
        hwnd: isize,
    },
    Created {
        hwnd: isize,
    },
    Destroyed {
        hwnd: isize,
    },
    Shown {
        hwnd: isize,
    },
    Hidden {
        hwnd: isize,
    },
    TitleChanged {
        hwnd: isize,
    },
    Minimized {
        hwnd: isize,
    },
    Restored {
        hwnd: isize,
    },
    /// 前台窗口的矩形变了（移动、缩放、进出全屏）。
    ///
    /// 只在前台窗口上发，否则拖一个普通窗口会源源不断地吵主线程（D-055）。
    BoundsChanged {
        hwnd: isize,
    },
}

/// 事件出口。回调由系统在监听线程上调用，拿不到自定义上下文，因此放在全局。
static EVENT_SINK: OnceLock<Mutex<Option<Sender<WindowEvent>>>> = OnceLock::new();

/// 唤醒目标：主线程 id 与消息号。事件线程每收到一个事件就投递一次。
static WAKEUP_THREAD: AtomicU32 = AtomicU32::new(0);
static WAKEUP_MESSAGE: AtomicU32 = AtomicU32::new(0);

/// 设置事件线程唤醒主线程用的目标。
pub fn set_wakeup(thread_id: u32, message: u32) {
    WAKEUP_MESSAGE.store(message, Ordering::Relaxed);
    WAKEUP_THREAD.store(thread_id, Ordering::Relaxed);
}

/// 主动踢一下主线程。
///
/// 供窗口过程使用：处理完按钮点击后让主循环立刻回来取动作，不用等下一个定时器。
pub fn wake() {
    wake_up();
}

fn sink() -> &'static Mutex<Option<Sender<WindowEvent>>> {
    EVENT_SINK.get_or_init(|| Mutex::new(None))
}

/// 事件监听器。停止时向监听线程投递 `WM_QUIT` 并等待它退出。
pub struct EventMonitor {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl EventMonitor {
    /// 启动监听线程。返回的事件接收端在监听器停止后被关闭。
    pub fn start() -> Result<(Self, Receiver<WindowEvent>), String> {
        let (event_tx, event_rx) = channel();
        *sink().lock().map_err(|_| "事件出口已被污染".to_string())? = Some(event_tx);

        let (ready_tx, ready_rx) = channel();
        let thread = thread::Builder::new()
            .name("airpane-winevent".to_string())
            .spawn(move || message_loop(ready_tx))
            .map_err(|err| format!("无法创建监听线程: {err}"))?;

        let thread_id = ready_rx
            .recv()
            .map_err(|RecvError| "监听线程在就绪前退出".to_string())??;

        Ok((
            Self {
                thread_id,
                thread: Some(thread),
            },
            event_rx,
        ))
    }

    /// 停止监听并等待线程退出。重复调用无副作用。
    pub fn stop(&mut self) {
        let Some(handle) = self.thread.take() else {
            return;
        };
        // SAFETY: 只向本进程内已知线程投递退出消息，参数类型如实传入。
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        let _ = handle.join();
    }
}

impl Drop for EventMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 监听线程的主体：注册 hook、通知调用方、跑消息循环，退出时卸载 hook。
fn message_loop(ready: Sender<Result<u32, String>>) {
    // SAFETY: `GetCurrentThreadId` 无参数、无副作用。
    let thread_id = unsafe { GetCurrentThreadId() };
    // SAFETY: 见 `install_hooks` 的安全前提。
    let hooks = unsafe { install_hooks() };
    if hooks.is_empty() {
        let _ = ready.send(Err("SetWinEventHook 全部失败".to_string()));
        return;
    }

    if ready.send(Ok(thread_id)).is_err() {
        for hook in hooks {
            // SAFETY: 卸载自己注册的 hook。
            unsafe {
                let _ = UnhookWinEvent(hook);
            }
        }
        return;
    }

    let mut msg = MSG::default();
    loop {
        // SAFETY: `msg` 是本栈帧上初始化的 MSG，窗口过滤传 None。
        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        // 0 表示收到 WM_QUIT，-1 表示出错，两者都退出循环。
        if result.0 <= 0 {
            break;
        }
        // SAFETY: 消息已由 `GetMessageW` 填充。
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    for hook in hooks {
        // SAFETY: 卸载本线程注册的 hook。
        unsafe {
            let _ = UnhookWinEvent(hook);
        }
    }
    if let Ok(mut guard) = sink().lock() {
        guard.take();
    }
}

/// 安装两个区间 hook，覆盖前台、最小化与对象增删改事件。
///
/// 安全前提：回调是 `extern "system"` 函数指针，签名为 `WINEVENTPROC`；
/// `WINEVENT_OUTOFCONTEXT` 保证回调在本进程线程上执行。
unsafe fn install_hooks() -> Vec<HWINEVENTHOOK> {
    let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
    let ranges = [
        (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MINIMIZEEND),
        (EVENT_OBJECT_CREATE, EVENT_OBJECT_NAMECHANGE),
        (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
    ];

    let mut hooks = Vec::new();
    for (min, max) in ranges {
        // SAFETY: 见函数文档，参数与实际回调签名一致。
        let hook = unsafe { SetWinEventHook(min, max, None, Some(win_event_proc), 0, 0, flags) };
        if !hook.is_invalid() {
            hooks.push(hook);
        }
    }
    hooks
}

/// 系统回调：把事件规格化成 [`WindowEvent`] 后转发，未关心的事件直接丢弃。
///
/// 安全前提：由系统按 `WINEVENTPROC` 签名调用；本函数不阻塞、不 panic，
/// 只做匹配与一次无需分配的 channel 发送。
unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    // 只关心窗口对象本身的事件，忽略子控件与光标等对象。
    if id_object != OBJID_WINDOW.0 || id_child != 0 || hwnd.0.is_null() {
        return;
    }
    let Some(window_event) = map_event(event, hwnd.0 as isize) else {
        return;
    };
    emit(window_event);
}

fn map_event(event: u32, hwnd: isize) -> Option<WindowEvent> {
    let window_event = match event {
        EVENT_SYSTEM_FOREGROUND => WindowEvent::Foreground { hwnd },
        EVENT_SYSTEM_MINIMIZESTART => WindowEvent::Minimized { hwnd },
        EVENT_SYSTEM_MINIMIZEEND => WindowEvent::Restored { hwnd },
        EVENT_OBJECT_CREATE => WindowEvent::Created { hwnd },
        EVENT_OBJECT_DESTROY => WindowEvent::Destroyed { hwnd },
        EVENT_OBJECT_SHOW => WindowEvent::Shown { hwnd },
        EVENT_OBJECT_HIDE => WindowEvent::Hidden { hwnd },
        EVENT_OBJECT_NAMECHANGE => WindowEvent::TitleChanged { hwnd },
        // 进出全屏靠这个事件：同一个窗口里改尺寸不会产生前台事件，等 1Hz 校准
        // 就晚了半秒，视频缩回去时桌面会先露出来（D-055）。
        EVENT_OBJECT_LOCATIONCHANGE if is_foreground(hwnd) => WindowEvent::BoundsChanged { hwnd },
        _ => return None,
    };
    Some(window_event)
}

/// 句柄是否就是当前前台窗口。
fn is_foreground(hwnd: isize) -> bool {
    // SAFETY: 只读查询，对任意句柄安全。
    unsafe { GetForegroundWindow() == HWND(hwnd as *mut core::ffi::c_void) }
}

fn emit(event: WindowEvent) {
    let Ok(guard) = sink().lock() else {
        return;
    };
    if let Some(sender) = guard.as_ref() {
        let _ = sender.send(event);
    }
    drop(guard);
    wake_up();
}

/// 踢一下主线程，让它在自己的消息循环里处理事件。
fn wake_up() {
    let thread_id = WAKEUP_THREAD.load(Ordering::Relaxed);
    let message = WAKEUP_MESSAGE.load(Ordering::Relaxed);
    if thread_id == 0 || message == 0 {
        return;
    }
    // SAFETY: 只向本进程内已知线程投递一条空消息；`PostThreadMessageW` 不阻塞。
    unsafe {
        let _ = PostThreadMessageW(thread_id, message, WPARAM(0), LPARAM(0));
    }
}
