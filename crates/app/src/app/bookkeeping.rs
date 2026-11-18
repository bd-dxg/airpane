//! 快照落盘、还原与尺寸记忆。

use std::time::{SystemTime, UNIX_EPOCH};

use airpane_core::registry::TrackedWindow;
use airpane_core::snapshot::{ShowState, SnapshotFile, WindowSnapshot};
use airpane_core::stage;
use airpane_core::window_sizes::Size;
use airpane_platform_win::foreground;
use airpane_platform_win::monitor;
use airpane_platform_win::placement::move_window;
use airpane_platform_win::window::{WindowInfo, inspect_window, is_window, show_state};
use airpane_platform_win::{log_info, log_warn, size_store, snapshot_store};

use super::App;

impl App {
    /// 若该窗口还没有快照，记录下来并落盘。移动之前必须先调用（D-005）。
    pub(super) fn ensure_snapshot(&mut self, info: &WindowInfo, state: ShowState) {
        let hwnd = info.hwnd as i64;
        if self.moved.contains_key(&hwnd) {
            return;
        }
        self.moved.insert(
            hwnd,
            WindowSnapshot {
                hwnd,
                process_name: info.process_name.clone(),
                class_name: info.class_name.clone(),
                rect: info.rect,
                show_state: state,
            },
        );
        self.persist_snapshot();
    }

    pub(super) fn remember_active_size(&mut self, strip_width: i32) {
        let Some(active) = self.active else {
            return;
        };
        let Some(info) = inspect_window(active as isize) else {
            return;
        };
        self.remember_size(&info, strip_width);
    }

    pub(super) fn remember_size(&mut self, info: &WindowInfo, strip_width: i32) {
        // 最小化与最大化窗口的矩形不是用户摆出来的，记下只会污染舞台区尺寸。
        if show_state(info.hwnd) != ShowState::Normal {
            return;
        }
        if info.rect.width() <= 0 || info.rect.height() <= 0 {
            return;
        }
        // 占满舞台区的尺寸没有记忆价值：记下来只会让窗口永远满屏。
        let stage = stage::stage_rect(monitor::primary_work_area(), strip_width);
        if info.rect.width() >= stage.width() && info.rect.height() >= stage.height() {
            return;
        }
        self.sizes.record(
            info.hwnd as i64,
            &info.process_name,
            &info.class_name,
            Size {
                width: info.rect.width(),
                height: info.rect.height(),
            },
        );
        self.flush_sizes();
    }

    /// 把尺寸记忆写盘（只在有变化时）。
    pub(super) fn flush_sizes(&mut self) {
        if !self.sizes.take_dirty() {
            return;
        }
        let file = self.sizes.to_file();
        if let Err(err) = size_store::write(&self.sizes_path, &file) {
            log_warn!("写尺寸记忆失败：{err}");
        }
    }

    pub(super) fn persist_snapshot(&self) {
        if self.moved.is_empty() {
            return;
        }
        let file = SnapshotFile::new(unix_now(), self.moved.values().cloned().collect());
        if let Err(err) = snapshot_store::write(&self.snapshot_path, &file) {
            log_warn!("写快照失败：{err}");
        }
    }

    /// 把记录下来的窗口摆回原位并清空快照。
    pub(super) fn restore_moved(&mut self) {
        let moved = std::mem::take(&mut self.moved);
        if moved.is_empty() {
            return;
        }
        for snapshot in moved.into_values() {
            if !is_window(snapshot.hwnd as isize) {
                continue;
            }
            let restored = match snapshot.show_state {
                // 最大化窗口的矩形由系统决定，直接恢复最大化状态即可。
                ShowState::Maximized => foreground::maximize(snapshot.hwnd as isize),
                _ => move_window(snapshot.hwnd as isize, snapshot.rect).is_ok(),
            };
            if restored {
                log_info!("已还原 {} 的状态", snapshot.process_name);
            } else {
                log_warn!("还原 {} 失败", snapshot.process_name);
            }
        }
        snapshot_store::clear(&self.snapshot_path);
        self.active = None;
    }
}

pub(super) fn tracked(info: &WindowInfo) -> TrackedWindow {
    TrackedWindow {
        hwnd: info.hwnd as i64,
        pid: info.pid,
        process_name: info.process_name.clone(),
        class_name: info.class_name.clone(),
        title: info.title.clone(),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}
