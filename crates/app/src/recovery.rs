//! 崩溃恢复。
//!
//! 启动时检查快照：`clean_exit == false` 说明上次没走完还原流程（崩溃或被强杀），
//! 此时按快照把窗口摆回原位（D-024）。

use std::path::Path;

use airpane_core::snapshot::{ShowState, TargetWindow, match_targets};
use airpane_platform_win::placement::move_window;
use airpane_platform_win::window::{enum_managed_windows, show_state};
use airpane_platform_win::{log_debug, log_info, log_warn, snapshot_store};

/// 需要恢复时把窗口摆回快照位置，返回真正移动过的窗口数。
///
/// 快照不存在或已标记正常退出时直接返回 0；处理完（无论成功与否）都清掉快照，
/// 避免同一次异常在下次启动时被重复恢复。
pub fn recover_if_needed(snapshot_path: &Path) -> usize {
    let Some(snapshot) = snapshot_store::read(snapshot_path) else {
        return 0;
    };
    if snapshot.clean_exit {
        snapshot_store::clear(snapshot_path);
        return 0;
    }

    let targets: Vec<TargetWindow> = enum_managed_windows()
        .into_iter()
        .map(|info| TargetWindow {
            hwnd: info.hwnd as i64,
            process_name: info.process_name,
            class_name: info.class_name,
            rect: info.rect,
        })
        .collect();

    let mut recovered = 0;
    for (index, hwnd) in match_targets(&snapshot.windows, &targets) {
        let entry = &snapshot.windows[index];
        if entry.show_state != ShowState::Normal {
            // 最大化与最小化的窗口不在 Stage 里被搬动（D-026），没有位置可恢复。
            log_debug!(
                "跳过 {} 的恢复：快照状态为 {:?}",
                entry.process_name,
                entry.show_state
            );
            continue;
        }
        if show_state(hwnd as isize) != ShowState::Normal {
            log_debug!("跳过 {} 的恢复：窗口当前不是普通状态", entry.process_name);
            continue;
        }

        let already_in_place = targets
            .iter()
            .find(|target| target.hwnd == hwnd)
            .is_some_and(|target| target.rect == entry.rect);
        if already_in_place {
            log_debug!("{} 已在原位，无需移动", entry.process_name);
            continue;
        }

        match move_window(hwnd as isize, entry.rect) {
            Ok(()) => {
                recovered += 1;
                log_info!("已恢复 {} 的位置", entry.process_name);
            }
            Err(err) => log_warn!("恢复 {} 失败：{err}", entry.process_name),
        }
    }

    snapshot_store::clear(snapshot_path);
    log_info!(
        "崩溃恢复完成：恢复 {recovered} 个，快照记录 {} 个",
        snapshot.windows.len()
    );
    recovered
}
