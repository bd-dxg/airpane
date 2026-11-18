//! 观察期纠正与最大化纠偏。

use airpane_core::snapshot::ShowState;
use airpane_platform_win::foreground;
use airpane_platform_win::log_debug;
use airpane_platform_win::monitor;
use airpane_platform_win::placement::move_window;
use airpane_platform_win::window::{inspect_window, show_state};
use airpane_strip::window::Strip;

use std::time::Instant;

use super::App;

impl App {
    /// 观察期内持续把活跃窗口拉回舞台区。
    ///
    /// 新程序启动时，应用自己的初始化代码常常在我们的 `SetWindowPos` 之后又把窗口
    /// 改回默认尺寸，表现为「刚打开是小窗口，切走再切回才铺满」。观察期结束后不再
    /// 干预，用户手动调尺寸仍然有效。
    pub(super) fn enforce_settle(&mut self, strip: &mut Strip) {
        let Some(deadline) = self.settle_until else {
            return;
        };
        if Instant::now() >= deadline {
            self.settle_until = None;
            return;
        }
        let Some(active) = self.active else {
            self.settle_until = None;
            return;
        };
        let Some(info) = inspect_window(active as isize) else {
            return;
        };
        if show_state(info.hwnd) != ShowState::Normal {
            return;
        }
        let target = self.target_rect(&info, strip.width(), false);
        if info.rect == target {
            return;
        }
        if move_window(active as isize, target).is_ok() {
            log_debug!("{} 在观察期内被改小，已重新摆到舞台区", info.process_name);
            self.last_items.clear();
            self.sync_strip(strip);
        }
    }

    /// 活跃窗口被最大化时重新摆回舞台区。
    ///
    /// 在舞台区窗口上点最大化不会产生前台事件，只能靠 1Hz 校准发现（D-037）。
    /// 用户只是手动调尺寸时不会走到这里：只有 `IsZoomed` 才会触发。
    pub(super) fn enforce_active_bounds(&mut self, strip: &mut Strip) {
        let Some(active) = self.active else {
            return;
        };
        if !monitor::is_maximized(active as isize) {
            return;
        }
        if !foreground::restore(active as isize) {
            return;
        }
        let Some(info) = inspect_window(active as isize) else {
            return;
        };
        let target = self.target_rect(&info, strip.width(), true);
        if move_window(active as isize, target).is_ok() {
            self.sizes
                .remove(active, &info.process_name, &info.class_name);
            self.flush_sizes();
            log_debug!("{} 被最大化，已摆回舞台区满屏", info.process_name);
            self.last_items.clear();
            self.sync_strip(strip);
        }
    }
}
