//! 写一份「上次没正常退出」的快照并挪开窗口，用来验证启动恢复。
//!
//! 用法：
//!
//! ```powershell
//! cargo run -p airpane-platform-win --example snapshot_check
//! cargo run --bin airpane
//! ```
//!
//! 第一步把第一个可管理窗口的当前位置写进 `snapshot.json`（`clean_exit = false`），
//! 再把它向左上挪 80px；第二步启动主程序，它读到未清理的快照后应当把窗口移回原位。

use std::time::{SystemTime, UNIX_EPOCH};

use airpane_core::snapshot::{Rect, ShowState, SnapshotFile, WindowSnapshot};
use airpane_platform_win::placement::move_window;
use airpane_platform_win::window::{enum_managed_windows, show_state};
use airpane_platform_win::{paths, snapshot_store};

fn main() {
    let windows = enum_managed_windows();
    // 只在普通状态的窗口上验证：最大化与最小化的窗口不会被 Stage 搬动（D-026）。
    let Some(window) = windows
        .iter()
        .find(|window| show_state(window.hwnd) == ShowState::Normal)
    else {
        println!("没有普通状态的可管理窗口，先打开一个记事本再试。");
        return;
    };

    let snapshot = SnapshotFile::new(
        unix_now(),
        vec![WindowSnapshot {
            hwnd: window.hwnd as i64,
            process_name: window.process_name.clone(),
            class_name: window.class_name.clone(),
            rect: window.rect,
            show_state: show_state(window.hwnd),
        }],
    );

    let path = paths::snapshot_file();
    if let Err(err) = snapshot_store::write(&path, &snapshot) {
        println!("写入快照失败: {err}");
        return;
    }
    println!("已写入快照：{}", path.display());
    println!("目标窗口：{}（{}）", window.title, window.process_name);
    println!("快照位置：{:?}", window.rect);

    // 整体左上平移，否则只改 left/top 会把窗口拉大。
    let moved = Rect {
        left: window.rect.left - 80,
        top: window.rect.top - 80,
        right: window.rect.right - 80,
        bottom: window.rect.bottom - 80,
    };
    match move_window(window.hwnd, moved) {
        Ok(()) => println!(
            "已把窗口向左上挪 80px（{:?}）。现在运行 cargo run --bin airpane 验证恢复。",
            moved
        ),
        Err(err) => println!("挪动窗口失败: {err}"),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
