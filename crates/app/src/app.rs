//! Stage 模式的控制器。
//!
//! 规则（D-022 / D-023）：活跃窗口 = 前台窗口；任何可管理窗口成为前台时把它搬进
//! 舞台区，其余窗口留在原地。每次移动之前先把原始状态落盘（D-005），退出时按快照
//! 还原并把快照清掉（D-024）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use airpane_core::registry::{TrackedWindow, WindowRegistry};
use airpane_core::snapshot::{Rect, ShowState, SnapshotFile, WindowSnapshot};
use airpane_core::stage::{self, Mode};
use airpane_core::window_sizes::{Size, WindowSizeStore, WindowSizesFile};
use airpane_platform_win::events::WindowEvent;
use airpane_platform_win::foreground::foreground_window;
use airpane_platform_win::monitor;
use airpane_platform_win::placement::move_window;
use airpane_platform_win::window::{WindowInfo, enum_managed_windows, inspect_window, is_window};
use airpane_platform_win::{log_debug, log_info, log_warn, size_store, snapshot_store};
use airpane_strip::window::{Strip, StripItem};

/// Stage 模式的运行时状态。
pub struct App {
    mode: Mode,
    registry: WindowRegistry,
    sizes: WindowSizeStore,
    active: Option<i64>,
    /// 被本程序移动过的窗口及其原始状态，退出时按它还原。
    moved: BTreeMap<i64, WindowSnapshot>,
    snapshot_path: PathBuf,
    sizes_path: PathBuf,
}

impl App {
    pub fn new(sizes: WindowSizesFile, snapshot_path: PathBuf, sizes_path: PathBuf) -> Self {
        Self {
            mode: Mode::Normal,
            registry: WindowRegistry::new(),
            sizes: WindowSizeStore::from_file(sizes),
            active: None,
            moved: BTreeMap::new(),
            snapshot_path,
            sizes_path,
        }
    }

    /// 进入 Stage 模式：刷新窗口列表，并把当前前台窗口搬进舞台区。
    pub fn enter_stage(&mut self, strip: &mut Strip) {
        if self.mode == Mode::Stage {
            return;
        }
        self.mode = Mode::Stage;
        log_info!("进入 Stage 模式（预览区宽 {} 像素）", strip.width());
        self.refresh_registry();
        self.focus_window(foreground_window(), strip);
        self.sync_strip(strip);
    }

    /// 处理一个窗口事件。
    pub fn on_event(&mut self, event: WindowEvent, strip: &mut Strip) {
        match event {
            WindowEvent::Foreground { hwnd } => {
                self.refresh_registry();
                self.focus_window(hwnd, strip);
            }
            // DESTROY 事件在进程退出时不会投递（D-020），这里只是尽力而为，
            // 1Hz 的 reconcile 才是兜底。
            WindowEvent::Destroyed { hwnd } | WindowEvent::Hidden { hwnd } => {
                self.forget_window(hwnd);
                self.sync_strip(strip);
            }
            WindowEvent::Created { hwnd }
            | WindowEvent::Shown { hwnd }
            | WindowEvent::TitleChanged { hwnd }
            | WindowEvent::Minimized { hwnd }
            | WindowEvent::Restored { hwnd } => {
                self.update_window(hwnd);
                self.sync_strip(strip);
            }
        }
    }

    /// 1Hz 校准：重新枚举、记录活跃窗口尺寸、修正预览区。
    pub fn reconcile(&mut self, strip: &mut Strip) {
        self.refresh_registry();
        self.remember_active_size(strip.width());
        if let Some(active) = self.active
            && !self.registry.contains(active)
        {
            log_debug!("活跃窗口 {:#x} 已消失", active);
            self.active = None;
        }
        self.sync_strip(strip);
    }

    /// 把前台窗口搬进舞台区。
    fn focus_window(&mut self, hwnd: isize, strip: &mut Strip) {
        if self.mode != Mode::Stage || hwnd == 0 {
            return;
        }
        let Some(info) = inspect_window(hwnd) else {
            self.sync_strip(strip);
            return;
        };
        if !info.is_managed() {
            self.sync_strip(strip);
            return;
        }
        let hwnd = info.hwnd as i64;
        self.registry.upsert(tracked(&info), true);

        if self.active == Some(hwnd) {
            // 同一窗口重新获得焦点：只顺手记录尺寸，不重复搬动。
            self.remember_size(&info, strip.width());
            self.sync_strip(strip);
            return;
        }

        // 先记下上一个活跃窗口的尺寸，再换人。
        self.remember_active_size(strip.width());

        if monitor::should_skip_placement(info.hwnd) {
            log_debug!("{} 处于最大化或全屏，保持原位（D-026）", info.process_name);
            self.active = Some(hwnd);
            self.sync_strip(strip);
            return;
        }

        self.ensure_snapshot(&info);
        let target = self.target_rect(&info, strip.width());
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
            }
            Err(err) => log_warn!("移动 {} 失败：{err}", info.process_name),
        }
        self.sync_strip(strip);
    }

    /// 计算窗口在舞台区里的目标矩形。
    fn target_rect(&self, info: &WindowInfo, strip_width: i32) -> Rect {
        let work_area = monitor::primary_work_area();
        let stage = stage::stage_rect(work_area, strip_width);
        match self
            .sizes
            .size_for(info.hwnd as i64, &info.process_name, &info.class_name)
        {
            Some(size) => stage::centered_rect(stage, size.width, size.height),
            None => stage,
        }
    }

    /// 若该窗口还没有快照，记录下来并落盘。移动之前必须先调用（D-005）。
    fn ensure_snapshot(&mut self, info: &WindowInfo) {
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
                show_state: ShowState::Normal,
            },
        );
        self.persist_snapshot();
    }

    fn remember_active_size(&mut self, strip_width: i32) {
        let Some(active) = self.active else {
            return;
        };
        let Some(info) = inspect_window(active as isize) else {
            return;
        };
        self.remember_size(&info, strip_width);
    }

    fn remember_size(&mut self, info: &WindowInfo, strip_width: i32) {
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
        if self.sizes.take_dirty() {
            let file = self.sizes.to_file();
            if let Err(err) = size_store::write(&self.sizes_path, &file) {
                log_warn!("写尺寸记忆失败：{err}");
            }
        }
    }

    fn persist_snapshot(&self) {
        if self.moved.is_empty() {
            return;
        }
        let file = SnapshotFile::new(unix_now(), self.moved.values().cloned().collect());
        if let Err(err) = snapshot_store::write(&self.snapshot_path, &file) {
            log_warn!("写快照失败：{err}");
        }
    }

    /// 把记录下来的窗口摆回原位并清空快照。
    fn restore_moved(&mut self) {
        let moved = std::mem::take(&mut self.moved);
        if moved.is_empty() {
            return;
        }
        for snapshot in moved.into_values() {
            if !is_window(snapshot.hwnd as isize) {
                continue;
            }
            match move_window(snapshot.hwnd as isize, snapshot.rect) {
                Ok(()) => log_info!("已还原 {} 的位置", snapshot.process_name),
                Err(err) => log_warn!("还原 {} 失败：{err}", snapshot.process_name),
            }
        }
        snapshot_store::clear(&self.snapshot_path);
        self.active = None;
    }

    fn refresh_registry(&mut self) {
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

    fn update_window(&mut self, hwnd: isize) {
        match inspect_window(hwnd) {
            Some(info) if info.is_managed() => {
                let activate = self.active == Some(info.hwnd as i64);
                self.registry.upsert(tracked(&info), activate);
            }
            _ => self.forget_window(hwnd),
        }
    }

    fn forget_window(&mut self, hwnd: isize) {
        let hwnd = hwnd as i64;
        self.registry.remove(hwnd);
        self.sizes.forget(hwnd);
        if self.active == Some(hwnd) {
            self.active = None;
        }
    }

    /// 预览区只列非活跃窗口，顺序即最近使用顺序。
    fn sync_strip(&mut self, strip: &mut Strip) {
        let mut items = Vec::with_capacity(self.registry.len().saturating_sub(1));
        for window in self.registry.iter() {
            if Some(window.hwnd) == self.active {
                continue;
            }
            if let Some(info) = inspect_window(window.hwnd as isize) {
                items.push(StripItem {
                    window: info.hwnd,
                    rect: info.rect,
                });
            }
        }
        if let Err(err) = strip.sync(&items) {
            log_warn!("同步预览区失败：{err}");
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // 正常退出（包括 panic unwind）先还原窗口。被强杀时走不到这里，
        // 快照会留在磁盘上，下次启动由 recovery 处理（D-024）。
        self.restore_moved();
    }
}

fn tracked(info: &WindowInfo) -> TrackedWindow {
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
