//! 窗口注册表的维护与预览区同步。

use airpane_core::registry::{self, TrackedWindow};
use airpane_platform_win::log_warn;
use airpane_platform_win::window::{enum_managed_windows, inspect_window};
use airpane_strip::window::{Strip, StripItem};

use super::App;
use super::bookkeeping::tracked;

impl App {
    /// 只检查已跟踪的窗口：失效或不再可管理的移除，其余刷新字段。
    pub(super) fn reconcile_tracked(&mut self) {
        let hwnds: Vec<i64> = self.registry.iter().map(|window| window.hwnd).collect();
        for hwnd in hwnds {
            match inspect_window(hwnd as isize) {
                Some(info) if info.is_managed() => {
                    let activate = self.active == Some(hwnd);
                    self.registry.upsert(tracked(&info), activate);
                }
                _ => self.forget_window(hwnd as isize),
            }
        }
    }

    pub(super) fn refresh_registry(&mut self) {
        let windows = enum_managed_windows();
        let alive: Vec<i64> = windows.iter().map(|info| info.hwnd as i64).collect();

        let stale: Vec<i64> = self
            .registry
            .iter()
            .map(|window| window.hwnd)
            .filter(|hwnd| !alive.contains(hwnd))
            .collect();
        for hwnd in stale {
            self.forget_window(hwnd as isize);
        }

        for info in &windows {
            let activate = self.active == Some(info.hwnd as i64);
            self.registry.upsert(tracked(info), activate);
        }
    }

    pub(super) fn update_window(&mut self, hwnd: isize) {
        match inspect_window(hwnd) {
            Some(info) if info.is_managed() => {
                let activate = self.active == Some(info.hwnd as i64);
                self.registry.upsert(tracked(&info), activate);
            }
            _ => self.forget_window(hwnd),
        }
    }

    pub(super) fn forget_window(&mut self, hwnd: isize) {
        let hwnd = hwnd as i64;
        self.registry.remove(hwnd);
        self.sizes.forget(hwnd);
        if self.active == Some(hwnd) {
            self.active = None;
        }
    }

    /// 预览区只列非活跃窗口，按固定顺序排列（D-047）：点开一个窗口只会让它消失，
    /// 其余窗口不会因为使用历史而互换位置。列表没变化就不碰 DWM。
    pub(super) fn sync_strip(&mut self, strip: &mut Strip) {
        let mut windows: Vec<&TrackedWindow> = self
            .registry
            .iter()
            .filter(|window| Some(window.hwnd) != self.active)
            .collect();
        windows.sort_by_cached_key(|window| registry::display_key(window));

        let mut items = Vec::with_capacity(windows.len());
        for window in windows {
            if let Some(info) = inspect_window(window.hwnd as isize) {
                items.push(StripItem {
                    window: info.hwnd,
                    pid: info.pid,
                    class_name: info.class_name,
                    rect: info.rect,
                });
            }
        }
        if items == self.last_items {
            return;
        }
        if let Err(err) = strip.sync(&items) {
            log_warn!("同步预览区失败：{err}");
            return;
        }
        self.last_items = items;
    }
}
