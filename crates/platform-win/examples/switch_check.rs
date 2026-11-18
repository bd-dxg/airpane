//! 前台切换可靠性验收：轮流把可管理窗口切到前台，统计成功率。
//!
//! ```powershell
//! cargo run -p airpane-platform-win --example switch_check -- --rounds 200
//! ```

use std::process::ExitCode;
use std::time::Duration;

use airpane_platform_win::foreground::{activate, foreground_window};
use airpane_platform_win::window::{WindowInfo, enum_top_level_windows};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let rounds = value_after(&args, "--rounds")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(200);
    let delay_ms = value_after(&args, "--delay-ms")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(120);

    let windows: Vec<WindowInfo> = enum_top_level_windows()
        .into_iter()
        .filter(WindowInfo::is_managed)
        .collect();

    if windows.len() < 2 {
        eprintln!("需要至少两个可管理窗口，当前 {} 个", windows.len());
        return ExitCode::FAILURE;
    }

    println!(
        "在 {} 个窗口间切换 {rounds} 次，每次间隔 {delay_ms}ms",
        windows.len()
    );

    let mut ok = 0usize;
    let mut failures: Vec<(isize, isize)> = Vec::new();

    for round in 0..rounds {
        let target = windows[round % windows.len()].hwnd;
        match activate(target) {
            Ok(()) => ok += 1,
            Err(_) => failures.push((target, foreground_window())),
        }
        std::thread::sleep(Duration::from_millis(delay_ms));
    }

    let rate = ok as f64 / rounds as f64 * 100.0;
    println!("成功 {ok}/{rounds}（{rate:.2}%）");
    if !failures.is_empty() {
        println!("失败记录（目标句柄, 实际前台）：");
        for (target, actual) in failures.iter().take(20) {
            println!("  {target:#010x} -> {actual:#010x}");
        }
    }

    if rate >= 99.0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
}
