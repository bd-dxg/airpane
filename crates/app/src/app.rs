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
use airpane_core::snapshot::WindowSnapshot;
use airpane_core::stage::Mode;
use airpane_core::window_sizes::{WindowSizeStore, WindowSizesFile};
use airpane_platform_win::events::WindowEvent;
use airpane_platform_win::foreground::foreground_window;
use airpane_platform_win::monitor;
use airpane_platform_win::window::is_desktop_window;
use airpane_platform_win::{log_debug, log_info};
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
    /// 当前窗口盖住整屏（视频全屏、F11、游戏）时隐藏了预览区（D-052）。
    fullscreen_hidden: bool,
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
            fullscreen_hidden: false,
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
        self.sync_fullscreen_visibility(strip);
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
        self.fullscreen_hidden = false;
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
                // 全屏视频/游戏/F11 会把整屏盖住，此时预览区浮在上面反而遮挡画面。
                self.sync_fullscreen_visibility(strip);
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
        // 在同一个窗口里点全屏不会产生前台事件，靠 1Hz 校准补上（D-052）。
        self.sync_fullscreen_visibility(strip);
        if let Some(active) = self.active {
            if !self.registry.contains(active) {
                log_debug!("活跃窗口 {:#x} 已消失", active);
                self.active = None;
            }
        }
        self.sync_strip(strip);
    }

    /// 当前窗口盖住整屏时隐藏预览区，回到普通窗口再恢复（D-052）。
    ///
    /// 用「可视边界盖住整个显示器」而不是 D-041 的「无标题栏 + 无最小化按钮」：
    /// 浏览器 F11 会保留标题栏样式，严格规则会漏掉；而只要窗口真的盖住整屏，
    /// 置顶的预览区就会挡在它上面。
    ///
    /// 桌面隐藏优先：Win+D 后不重复处理。
    fn sync_fullscreen_visibility(&mut self, strip: &mut Strip) {
        if self.desktop_hidden {
            return;
        }
        let fullscreen = self
            .active
            .is_some_and(|hwnd| monitor::covers_primary_monitor(hwnd as isize));
        if fullscreen == self.fullscreen_hidden {
            return;
        }
        self.fullscreen_hidden = fullscreen;
        strip.set_visible(!fullscreen);
        if fullscreen {
            log_info!("当前窗口盖住整屏，隐藏预览区");
        } else {
            log_info!("回到窗口模式，恢复预览区");
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
