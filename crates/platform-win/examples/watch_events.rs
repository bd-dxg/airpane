//! 验证事件驱动同步：启动 WinEvent 监听，维护窗口模型，并每秒与实时枚举对照。
//!
//! ```powershell
//! cargo run -p airpane-platform-win --example watch_events -- --seconds 30
//! ```
//!
//! 退出时打印不一致次数。0 表示整个观察期内事件模型与实时枚举完全一致。

use std::collections::BTreeSet;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use airpane_core::registry::{TrackedWindow, WindowRegistry};
use airpane_platform_win::events::{EventMonitor, WindowEvent};
use airpane_platform_win::window::{
    WindowInfo, enum_top_level_windows, inspect_window, reconcile_registry,
};

fn tracked(info: &WindowInfo) -> TrackedWindow {
    TrackedWindow {
        hwnd: info.hwnd as i64,
        pid: info.pid,
        process_name: info.process_name.clone(),
        class_name: info.class_name.clone(),
        title: info.title.clone(),
    }
}

fn apply(registry: &mut WindowRegistry, event: WindowEvent) {
    match event {
        WindowEvent::Foreground { hwnd } => {
            if !registry.activate(hwnd as i64) {
                if let Some(info) = inspect_window(hwnd).filter(WindowInfo::is_managed) {
                    registry.upsert(tracked(&info), true);
                }
            }
        }
        WindowEvent::Created { hwnd } | WindowEvent::Shown { hwnd } => {
            if let Some(info) = inspect_window(hwnd).filter(WindowInfo::is_managed) {
                registry.upsert(tracked(&info), false);
            }
        }
        WindowEvent::TitleChanged { hwnd } => {
            if let Some(info) = inspect_window(hwnd).filter(WindowInfo::is_managed) {
                registry.upsert(tracked(&info), false);
            }
        }
        WindowEvent::Destroyed { hwnd } | WindowEvent::Hidden { hwnd } => {
            registry.remove(hwnd as i64);
        }
        WindowEvent::Minimized { .. } | WindowEvent::Restored { .. } => {}
    }
}

fn live_handles() -> BTreeSet<i64> {
    enum_top_level_windows()
        .iter()
        .filter(|info| info.is_managed())
        .map(|info| info.hwnd as i64)
        .collect()
}

fn main() -> ExitCode {
    let seconds = std::env::args()
        .skip_while(|arg| arg != "--seconds")
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(20);

    let mut registry = WindowRegistry::new();
    for info in enum_top_level_windows()
        .iter()
        .filter(|info| info.is_managed())
    {
        registry.upsert(tracked(info), false);
    }

    println!("初始可管理窗口 {} 个，观察 {seconds} 秒", registry.len());

    let (_monitor, events) = match EventMonitor::start() {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("启动事件监听失败: {err}");
            return ExitCode::FAILURE;
        }
    };

    let verbose = std::env::args().any(|arg| arg == "--verbose");
    let started = Instant::now();
    let mut checked = Instant::now();
    let mut mismatches = 0u32;

    while started.elapsed() < Duration::from_secs(seconds) {
        match events.recv_timeout(Duration::from_millis(200)) {
            Ok(event) => {
                if verbose {
                    println!("事件: {event:?}");
                }
                apply(&mut registry, event);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }

        if checked.elapsed() < Duration::from_secs(1) {
            continue;
        }
        checked = Instant::now();

        reconcile_registry(&mut registry);

        let live = live_handles();
        let modeled: BTreeSet<i64> = registry.iter().map(|window| window.hwnd).collect();
        if live != modeled {
            mismatches += 1;
            let only_modeled: Vec<_> = modeled.difference(&live).copied().collect();
            let only_live: Vec<_> = live.difference(&modeled).copied().collect();
            println!(
                "不一致 #{mismatches}：事件模型多出 {only_modeled:?}，实时枚举多出 {only_live:?}"
            );
        }
    }

    println!(
        "观察结束：{} 次对比，{} 次不一致，最终 {} 个窗口",
        seconds,
        mismatches,
        registry.len()
    );
    for (index, window) in registry.iter().enumerate() {
        println!(
            "{:>2}. {:#010x} {:<24} {}",
            index + 1,
            window.hwnd,
            window.process_name,
            window.title
        );
    }

    if mismatches == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
