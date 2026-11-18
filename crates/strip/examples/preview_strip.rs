//! 预览区缩略图验收：显示当前非活跃窗口的 DWM 缩略图，点击缩略图切换窗口。
//!
//! ```powershell
//! cargo run -p airpane-strip --example preview_strip -- --seconds 30
//! ```

use std::process::ExitCode;
use std::time::{Duration, Instant};

use airpane_platform_win::foreground::foreground_window;
use airpane_platform_win::window::enum_top_level_windows;
use airpane_strip::window::{Strip, StripItem, enable_per_monitor_v2};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};

const REFRESH_INTERVAL: Duration = Duration::from_millis(500);

fn collect_items() -> Vec<StripItem> {
    let foreground = foreground_window();
    enum_top_level_windows()
        .into_iter()
        .filter(|info| info.is_managed() && info.hwnd != foreground)
        .map(|info| StripItem {
            window: info.hwnd,
            rect: info.rect,
        })
        .collect()
}

fn main() -> ExitCode {
    let seconds = std::env::args()
        .skip_while(|arg| arg != "--seconds")
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(30);

    enable_per_monitor_v2();

    let mut strip = match Strip::create() {
        Ok(strip) => strip,
        Err(err) => {
            eprintln!("创建预览区失败: {err}");
            return ExitCode::FAILURE;
        }
    };
    println!("预览区窗口 {:#x}，运行 {seconds} 秒", strip.hwnd());

    let mut last_refresh = Instant::now() - REFRESH_INTERVAL;
    let started = Instant::now();
    let mut msg = MSG::default();

    while started.elapsed() < Duration::from_secs(seconds) {
        // SAFETY: `msg` 是本栈帧上的 MSG。
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            // SAFETY: 消息由 PeekMessageW 填充。
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        if last_refresh.elapsed() >= REFRESH_INTERVAL {
            last_refresh = Instant::now();
            let items = collect_items();
            if let Err(err) = strip.sync(&items) {
                eprintln!("同步缩略图失败: {err}");
            }
        }

        std::thread::sleep(Duration::from_millis(10));
    }

    ExitCode::SUCCESS
}
