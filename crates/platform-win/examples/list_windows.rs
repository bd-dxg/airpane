//! 打印当前可被 AirPane 管理的窗口列表，用于与任务栏逐个人工对照。
//!
//! ```powershell
//! cargo run -p airpane-platform-win --example list_windows
//! cargo run -p airpane-platform-win --example list_windows -- --all
//! ```

use airpane_platform_win::window::{WindowInfo, enum_top_level_windows};

fn main() {
    let show_all = std::env::args().any(|arg| arg == "--all");
    let windows: Vec<WindowInfo> = enum_top_level_windows();
    let managed: Vec<&WindowInfo> = windows.iter().filter(|w| w.is_managed()).collect();

    println!(
        "顶层窗口 {} 个，其中可管理 {} 个{}",
        windows.len(),
        managed.len(),
        if show_all {
            "（含被排除项）"
        } else {
            ""
        }
    );

    for (index, window) in windows.iter().enumerate() {
        if !show_all && !window.is_managed() {
            continue;
        }
        let verdict = match &window.verdict {
            Ok(()) => "managed".to_string(),
            Err(reason) => format!("rejected: {reason}"),
        };
        println!(
            "{:>3}. {:#010x} pid={:<6} {:<26} {} {}x{} @({},{}) [{}] {}",
            index + 1,
            window.hwnd,
            window.pid,
            window.process_name,
            verdict,
            window.rect.width(),
            window.rect.height(),
            window.rect.left,
            window.rect.top,
            window.class_name,
            window.title,
        );
    }
}
