//! Stage 模式的控制器。
//!
//! 规则（D-022 / D-023）：活跃窗口 = 前台窗口；任何可管理窗口成为前台时把它搬进
//! 舞台区，其余窗口留在原地。每次移动之前先把原始状态落盘（D-005），退出时按快照
//! 还原并把快照清掉（D-024）。
//!
//! 文件拆分（AGENTS.md）：搬窗口在 `focus`，快照与尺寸记忆在 `bookkeeping`，
//! 观察期与最大化纠偏在 `enforce`，注册表与预览区同步在 `registry`。

mod bookkeeping;
mod enforce;
mod focus;
mod registry;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use airpane_core::registry::WindowRegistry;
use airpane_core::snapshot::{Rect, WindowSnapshot};
use airpane_core::stage::Mode;
use airpane_core::strip_order::StripOrderFile;
use airpane_core::window_sizes::{WindowSizeStore, WindowSizesFile};
use airpane_platform_win::events::WindowEvent;
use airpane_platform_win::foreground::foreground_window;
use airpane_platform_win::monitor;
use airpane_platform_win::placement::move_window;
use airpane_platform_win::window::{inspect_window, is_desktop_window};
use airpane_platform_win::{compose, log_debug, log_info};
use airpane_strip::window::{Strip, StripAction, StripItem};

/// Stage 模式的运行时状态。
pub struct App {
    mode: Mode,
    registry: WindowRegistry,
    sizes: WindowSizeStore,
    /// 预览区的手动排序（D-053）。
    order: StripOrderFile,
    /// 排序有变化才写盘。
    order_dirty: bool,
    active: Option<i64>,
    /// 被本程序移动过的窗口及其原始状态，退出时按它还原。
    moved: BTreeMap<i64, WindowSnapshot>,
    /// 预览区上一次同步的列表，没变化就不再碰 DWM。
    last_items: Vec<StripItem>,
    /// 1Hz 校准的计数，用于每 10 秒做一次全量枚举兜底。
    tick: u32,
    /// 预览区是否被隐藏（桌面在前台，或当前窗口盖住整屏）。
    strip_hidden: bool,
    /// 新搬入窗口的观察期截止时间。应用启动时会自己改尺寸，这段时间内持续纠正。
    settle_until: Option<Instant>,
    /// 上次已知的工作区。任务栏换边、分辨率变化都会改它（D-058）。
    work_area: Rect,
    snapshot_path: PathBuf,
    sizes_path: PathBuf,
    order_path: PathBuf,
}

/// Stage 控制器用到的状态文件路径。
pub struct AppPaths {
    pub snapshot: PathBuf,
    pub sizes: PathBuf,
    pub order: PathBuf,
}

impl App {
    pub fn new(sizes: WindowSizesFile, order: StripOrderFile, paths: AppPaths) -> Self {
        Self {
            mode: Mode::Normal,
            registry: WindowRegistry::new(),
            sizes: WindowSizeStore::from_file(sizes),
            order,
            order_dirty: false,
            active: None,
            moved: BTreeMap::new(),
            last_items: Vec::new(),
            tick: 0,
            strip_hidden: false,
            settle_until: None,
            work_area: monitor::primary_work_area(),
            snapshot_path: paths.snapshot,
            sizes_path: paths.sizes,
            order_path: paths.order,
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
        let foreground = foreground_window();
        self.focus_window(foreground, strip);
        self.update_strip_visibility(strip, foreground);
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
        self.strip_hidden = false;
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
            StripAction::Reorder { window, before } => self.reorder(window, before, strip),
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
                // 先调可见性再干别的：这里只查一次窗口矩形，而后面要枚举近 300 个
                // 窗口再搬窗口。放在后面的话，全屏视频缩回时桌面会先露几帧（D-055）。
                self.update_strip_visibility(strip, hwnd);
                // Win+D / 显示桌面：桌面不需要搬窗口。
                if is_desktop_window(hwnd) {
                    return;
                }
                self.refresh_registry();
                self.focus_window(hwnd, strip);
            }
            WindowEvent::BoundsChanged { hwnd } => {
                // 前台窗口的矩形变了：进出全屏就靠这个事件，不等 1Hz 校准（D-055）。
                self.update_strip_visibility(strip, hwnd);
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
        // 任务栏换边/自动隐藏、分辨率变化都会改工作区，靠每拍对比发现（D-058）。
        self.sync_work_area(strip);
        // 观察期内不记录尺寸：那时窗口的尺寸是应用自己初始化出来的，不是用户偏好。
        if self.settle_until.is_none() {
            self.remember_active_size(strip.width());
        }
        // 在同一个窗口里点全屏不会产生前台事件，靠 1Hz 校准补上（D-052）。
        self.update_strip_visibility(strip, foreground_window());
        if let Some(active) = self.active {
            if !self.registry.contains(active) {
                log_debug!("活跃窗口 {:#x} 已消失", active);
                self.active = None;
            }
        }
        self.sync_strip(strip);
    }

    /// 工作区变了就把预览区与舞台区重新摆一次（D-058）。
    ///
    /// 任务栏从底部换到左侧时，工作区的左边界会从 0 变成任务栏宽度；不重新摆的话
    /// 预览区会继续压着任务栏。只在真的变了时才动，平时每拍只多一次查寻。
    fn sync_work_area(&mut self, strip: &mut Strip) {
        let work_area = monitor::primary_work_area();
        if work_area == self.work_area {
            return;
        }
        self.work_area = work_area;
        log_info!(
            "工作区变化（{}x{} @({},{})），重摆预览区与舞台区",
            work_area.width(),
            work_area.height(),
            work_area.left,
            work_area.top
        );
        strip.relayout();

        // 全屏的窗口不能动，否则会把它的全屏状态破坏掉。
        let Some(active) = self.active else {
            return;
        };
        if monitor::covers_primary_monitor(active as isize) {
            return;
        }
        let Some(info) = inspect_window(active as isize) else {
            return;
        };
        let target = self.target_rect(&info, strip.width(), false);
        if move_window(active as isize, target).is_ok() {
            log_debug!("已把 {} 重新摆进新的舞台区", info.process_name);
            self.last_items.clear();
            self.sync_strip(strip);
        }
    }

    /// 根据前台窗口决定预览区要不要隐藏（D-036、D-052、D-055）。
    ///
    /// 两种情况要藏：桌面成为前台（Win+D，否则遮住桌面图标）；前台窗口的可视边界
    /// 盖住整个显示器（视频全屏、游戏、F11，否则置顶的预览区挡在画面上）。
    ///
    /// 用传进来的前台窗口而不是 `active`：调用方在 "前台刚变、还没搬窗口" 时就想
    /// 知道答案，那时 `active` 还是旧窗口。只在状态真正变化时调 `set_visible`。
    fn update_strip_visibility(&mut self, strip: &mut Strip, foreground: isize) {
        let desktop = is_desktop_window(foreground);
        let fullscreen = !desktop && monitor::covers_primary_monitor(foreground);
        let hidden = desktop || fullscreen;
        if hidden == self.strip_hidden {
            return;
        }
        self.strip_hidden = hidden;
        strip.set_visible(!hidden);
        if hidden {
            // 刚才被预览区盖住的区域要由下层窗口补画，否则会露一帧桌面（D-055）。
            compose::redraw_now(foreground);
        }
        if !hidden {
            log_info!("恢复预览区");
        } else if desktop {
            log_info!("桌面成为前台，隐藏预览区");
        } else {
            log_info!("当前窗口盖住整屏，隐藏预览区");
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
