//! 把前台窗口搬进舞台区。

use std::time::{Duration, Instant};

use airpane_core::snapshot::{Rect, ShowState};
use airpane_core::stage::{self, Mode};
use airpane_platform_win::foreground;
use airpane_platform_win::monitor;
use airpane_platform_win::placement::move_window;
use airpane_platform_win::window::{WindowInfo, inspect_window, show_state};
use airpane_platform_win::{log_debug, log_info, log_warn};
use airpane_strip::window::Strip;

use super::App;
use super::bookkeeping::tracked;

/// 新窗口搬入后的观察期。应用启动时的自我重排通常发生在 1–2 秒内。
const SETTLE_WINDOW: Duration = Duration::from_secs(3);

impl App {
    /// 把前台窗口搬进舞台区。
    pub(super) fn focus_window(&mut self, hwnd: isize, strip: &mut Strip) {
        if self.mode != Mode::Stage || hwnd == 0 {
            return;
        }
        let Some(mut info) = inspect_window(hwnd) else {
            self.sync_strip(strip);
            return;
        };
        if !info.is_managed() {
            self.sync_strip(strip);
            return;
        }
        let hwnd = info.hwnd as i64;
        self.registry.upsert(tracked(&info), true);

        // 先记住原始显示状态：最大化窗口要先取消最大化才能摆到舞台区，
        // 退出 Stage 时再把最大化状态恢复回去（D-037）。
        let original_state = if monitor::is_maximized(info.hwnd) {
            ShowState::Maximized
        } else {
            show_state(info.hwnd)
        };

        if original_state != ShowState::Normal {
            // 最小化与最大化都先还原成普通窗口，之后的矩形才是真实可用的。
            if !foreground::restore(info.hwnd) {
                log_warn!("无法还原窗口状态：{}", info.process_name);
                self.sync_strip(strip);
                return;
            }
            match inspect_window(info.hwnd) {
                Some(refreshed) => info = refreshed,
                None => {
                    self.sync_strip(strip);
                    return;
                }
            }
        }

        if self.active == Some(hwnd) {
            // 同一窗口重新获得焦点：只顺手记录尺寸，不重复搬动。
            // 窗口刚被从最大化/最小化还原时不记录，否则会把它的还原尺寸当成偏好。
            if original_state == ShowState::Normal {
                self.remember_size(&info, strip.width());
            }
            self.sync_strip(strip);
            return;
        }

        // 先记下上一个活跃窗口的尺寸，再换人。
        self.remember_active_size(strip.width());

        // 真全屏（非最大化的覆盖整屏）是游戏与全屏视频，不动它。
        if original_state == ShowState::Normal && monitor::is_exclusive_fullscreen(info.hwnd) {
            log_debug!("{} 处于全屏，保持原位（D-026）", info.process_name);
            self.active = Some(hwnd);
            self.sync_strip(strip);
            return;
        }

        self.ensure_snapshot(&info, original_state);
        let force_stage = original_state == ShowState::Maximized;
        if force_stage {
            // 用户明确要求最大化，旧的小尺寸记忆不再代表偏好。
            self.sizes
                .remove(hwnd, &info.process_name, &info.class_name);
            self.flush_sizes();
        }
        let target = self.target_rect(&info, strip.width(), force_stage);
        match move_window(info.hwnd, target) {
            Ok(()) => {
                log_info!(
                    "已把 {} 移入舞台区：{}x{} @({},{})",
                    info.process_name,
                    target.width(),
                    target.height(),
                    target.left,
                    target.top
                );
                self.active = Some(hwnd);
                // 应用启动时往往会在之后 1–2 秒内自己重排尺寸，观察一会儿并及时纠正。
                self.settle_until = Some(Instant::now() + SETTLE_WINDOW);
            }
            Err(err) => log_warn!("移动 {} 失败：{err}", info.process_name),
        }
        self.sync_strip(strip);
    }

    /// 计算窗口在舞台区里的目标矩形。
    ///
    /// `force_stage` 为真时忽略尺寸记忆，直接占满舞台区：用户明确点了最大化，
    /// 旧的小尺寸记忆不该把它拉回去。
    pub(super) fn target_rect(
        &self,
        info: &WindowInfo,
        strip_width: i32,
        force_stage: bool,
    ) -> Rect {
        let work_area = monitor::primary_work_area();
        let stage = stage::stage_rect(work_area, strip_width);
        if force_stage {
            return stage;
        }
        match self
            .sizes
            .size_for(info.hwnd as i64, &info.process_name, &info.class_name)
        {
            Some(size) => stage::centered_rect(stage, size.width, size.height),
            None => stage,
        }
    }
}
