//! Stage 模式的控制器。
//!
//! 规则（D-022 / D-023）：活跃窗口 = 前台窗口；任何可管理窗口成为前台时把它搬进
//! 舞台区，其余窗口留在原地。每次移动之前先把原始状态落盘（D-005），退出时按快照
//! 还原并把快照清掉（D-024）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 新窗口搬入后的观察期。应用启动时的自我重排通常发生在 1–2 秒内。
const SETTLE_WINDOW: Duration = Duration::from_secs(3);

use airpane_core::registry::{TrackedWindow, WindowRegistry};
use airpane_core::snapshot::{Rect, ShowState, SnapshotFile, WindowSnapshot};
use airpane_core::stage::{self, Mode};
use airpane_core::window_sizes::{Size, WindowSizeStore, WindowSizesFile};
use airpane_platform_win::events::WindowEvent;
use airpane_platform_win::foreground::{self, foreground_window};
use airpane_platform_win::monitor;
use airpane_platform_win::placement::move_window;
use airpane_platform_win::window::{
    WindowInfo, enum_managed_windows, inspect_window, is_desktop_window, is_window, show_state,
};
use airpane_platform_win::{log_debug, log_info, log_warn, size_store, snapshot_store};
use airpane_strip::window::{Strip, StripAction, StripItem};

/// Stage 模式的运行时状态。
pub struct App {
    mode: Mode,
    registry: WindowRegistry,
    sizes: WindowSizeStore,
    active: Option<i64>,
    /// 被本程序移动过的窗口及其原始状态，退出时按它还原。
    moved: BTreeMap<i64, WindowSnapshot>,
    /// 预览区上一次同步的列表，没变化就不再碰 DWM。
    last_items: Vec<StripItem>,
    /// 1Hz 校准的计数，用于每 10 秒做一次全量枚举兜底。
    tick: u32,
    /// 前台变成桌面（Win+D）时隐藏了预览区，回到普通窗口时需要恢复。
    desktop_hidden: bool,
    /// 新搬入窗口的观察期截止时间。应用启动时会自己改尺寸，这段时间内持续纠正。
    settle_until: Option<Instant>,
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
            last_items: Vec::new(),
            tick: 0,
            desktop_hidden: false,
            settle_until: None,
            snapshot_path,
            sizes_path,
        }
    }

    /// 当前模式的显示名，用于启动日志与控制台输出。
    pub fn mode_name(&self) -> &'static str {
        match self.mode {
            Mode::Stage => "Stage 模式（预览区 + 舞台区）",
            Mode::Normal => "普通桌面模式",
        }
    }

    /// 进入 Stage 模式：显示预览区，并把当前前台窗口搬进舞台区。
    pub fn enter_stage(&mut self, strip: &mut Strip) {
        if self.mode == Mode::Stage {
            return;
        }
        self.mode = Mode::Stage;
        strip.set_visible(true);
        log_info!("进入 Stage 模式（预览区宽 {} 像素）", strip.width());
        self.refresh_registry();
        self.focus_window(foreground_window(), strip);
        self.sync_strip(strip);
    }

    /// 切回普通桌面：还原所有被移动过的窗口，隐藏预览区。
    pub fn leave_stage(&mut self, strip: &mut Strip) {
        if self.mode == Mode::Normal {
            return;
        }
        self.mode = Mode::Normal;
        log_info!("切回普通桌面模式");
        self.restore_moved();
        self.registry = WindowRegistry::new();
        self.last_items.clear();
        self.desktop_hidden = false;
        let _ = strip.sync(&[]);
        strip.set_visible(false);
    }

    /// 在两种模式之间切换。
    pub fn toggle_mode(&mut self, strip: &mut Strip) {
        match self.mode {
            Mode::Stage => self.leave_stage(strip),
            Mode::Normal => self.enter_stage(strip),
        }
    }

    /// 处理预览区工具区发出的动作。
    pub fn on_action(&mut self, action: StripAction, strip: &mut Strip) {
        match action {
            StripAction::ToggleMode => self.toggle_mode(strip),
            StripAction::OpenSettings => {
                log_info!("设置入口被点击（M1 无设置面板，M3 提供）");
                airpane_platform_win::dialog::info(
                    "AirPane",
                    "M1 还没有设置面板。\n\n配置在 %APPDATA%\\AirPane\\config.json，改完重启程序生效。模式切换默认是 Ctrl+Alt+S。",
                );
            }
        }
    }

    /// 处理一个窗口事件。
    pub fn on_event(&mut self, event: WindowEvent, strip: &mut Strip) {
        // 普通桌面模式下不跟踪窗口，也不搬东西。
        if self.mode != Mode::Stage {
            return;
        }
        match event {
            WindowEvent::Foreground { hwnd } => {
                // Win+D / 显示桌面：预览区会遮住桌面图标，暂时藏起来。
                if is_desktop_window(hwnd) {
                    if !self.desktop_hidden {
                        self.desktop_hidden = true;
                        strip.set_visible(false);
                        log_info!("桌面成为前台，隐藏预览区");
                    }
                    return;
                }
                if self.desktop_hidden {
                    self.desktop_hidden = false;
                    strip.set_visible(true);
                    log_info!("离开桌面，恢复预览区");
                }
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

    /// 1Hz 校准：检查已跟踪的窗口、记录活跃窗口尺寸、修正预览区。
    ///
    /// 默认只检查注册表里已有的窗口：新窗口由 `Create`/`Show` 事件发现（D-020）。
    /// 每 10 秒做一次全量枚举，兜住事件丢失的情况。
    pub fn reconcile(&mut self, strip: &mut Strip) {
        if self.mode != Mode::Stage {
            return;
        }
        self.tick = self.tick.wrapping_add(1);
        if self.tick % 10 == 0 {
            self.refresh_registry();
        } else {
            self.reconcile_tracked();
        }
        self.enforce_active_bounds(strip);
        self.enforce_settle(strip);
        // 观察期内不记录尺寸：那时窗口的尺寸是应用自己初始化出来的，不是用户偏好。
        if self.settle_until.is_none() {
            self.remember_active_size(strip.width());
        }
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
    fn target_rect(&self, info: &WindowInfo, strip_width: i32, force_stage: bool) -> Rect {
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

    /// 若该窗口还没有快照，记录下来并落盘。移动之前必须先调用（D-005）。
    fn ensure_snapshot(&mut self, info: &WindowInfo, state: ShowState) {
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
    fn flush_sizes(&mut self) {
        if !self.sizes.take_dirty() {
            return;
        }
        let file = self.sizes.to_file();
        if let Err(err) = size_store::write(&self.sizes_path, &file) {
            log_warn!("写尺寸记忆失败：{err}");
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

    /// 观察期内持续把活跃窗口拉回舞台区。
    ///
    /// 新程序启动时，应用自己的初始化代码常常在我们的 `SetWindowPos` 之后又把窗口
    /// 改回默认尺寸，表现为「刚打开是小窗口，切走再切回才铺满」。观察期结束后不再
    /// 干预，用户手动调尺寸仍然有效。
    fn enforce_settle(&mut self, strip: &mut Strip) {
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
    fn enforce_active_bounds(&mut self, strip: &mut Strip) {
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

    /// 只检查已跟踪的窗口：失效或不再可管理的移除，其余刷新字段。
    fn reconcile_tracked(&mut self) {
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

    /// 预览区只列非活跃窗口，顺序即最近使用顺序。列表没变化就不碰 DWM。
    fn sync_strip(&mut self, strip: &mut Strip) {
        let mut items = Vec::with_capacity(self.registry.len().saturating_sub(1));
        for window in self.registry.iter() {
            if Some(window.hwnd) == self.active {
                continue;
            }
            if let Some(info) = inspect_window(window.hwnd as isize) {
                items.push(StripItem {
                    window: info.hwnd,
                    pid: info.pid,
                    class_name: info.class_name,
                    process_name: info.process_name,
                    title: info.title,
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
