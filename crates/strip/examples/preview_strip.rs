//! 预览区窗口壳子验收：贴左显示一条 200 DIP 宽的深色窄栏，指定秒数后自动退出。
//!
//! ```powershell
//! cargo run -p airpane-strip --example preview_strip -- --seconds 10
//! ```

use std::time::Duration;

use airpane_strip::window::{Strip, enable_per_monitor_v2, run_message_loop};

fn main() {
    let seconds = std::env::args()
        .skip_while(|arg| arg != "--seconds")
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(10);

    enable_per_monitor_v2();

    let strip = match Strip::create() {
        Ok(strip) => strip,
        Err(err) => {
            eprintln!("创建预览区失败: {err}");
            std::process::exit(1);
        }
    };
    println!("预览区窗口 {:#x}，{seconds} 秒后退出", strip.hwnd());

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(seconds));
        std::process::exit(0);
    });

    run_message_loop();
}
