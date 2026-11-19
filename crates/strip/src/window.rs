//! 预览区窗口：创建、生命周期与缩略图同步。
//!
//! 布局（`layout`）、绘制（`paint`）、消息处理（`wndproc`）与运行时状态（`state`）
//! 各占一个子模块，单文件控制在 300 行以内（AGENTS.md）。

mod layout;
mod paint;
mod state;
mod wndproc;

use core::ffi::c_void;
use core::fmt;
use std::collections::HashMap;
use std::sync::mpsc::Sender;

use airpane_core::config::DEFAULT_STRIP_WIDTH_DIP;
use airpane_core::snapshot::Rect;
use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::Graphics::Dwm::{DwmRegisterThumbnail, DwmUnregisterThumbnail};
use windows::Win32::Graphics::Gdi::{InvalidateRect, UpdateWindow};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GWLP_USERDATA, IDC_ARROW, LoadCursorW, LoadIconW,
    RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SetWindowLongPtrW, ShowWindow, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::{HSTRING, PCWSTR, w};

use layout::{apply_layout, client_width, physical_width, relayout_window, work_area};
use state::{Entry, IconCache, StripState};
use wndproc::wnd_proc;

pub use wndproc::{enable_per_monitor_v2, run_message_loop};

/// 预览区默认的逻辑宽度（DIP），PRD 4.5。实际生效值由 [`physical_width`] 计算。
pub const DEFAULT_WIDTH_DIP: u32 = DEFAULT_STRIP_WIDTH_DIP;

const CLASS_NAME: windows::core::PCWSTR = w!("AirPaneStrip");
/// 应用图标资源 id，对应 `assets/airpane.rc` 里的 `1 ICON`。
///
/// 资源由 `crates/app/build.rs` 编进 exe；`preview_strip` 这类示例没有这份资源，
/// 加载失败就退化成没有窗口图标。
const APP_ICON_ID: usize = 1;

/// 预览区要展示的一个窗口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripItem {
    pub window: isize,
    /// 所属进程，与 `class_name` 一起决定图标缓存键（D-046）。
    pub pid: u32,
    pub class_name: String,
    /// 用于计算宽高比的矩形。活跃窗口传入搬动前的矩形，否则项高会在切换时跳变（D-051）。
    pub rect: Rect,
    /// 是否是当前在舞台区的窗口。活跃项高亮显示，点击它不做事。
    pub active: bool,
}

/// 预览区发出的动作。只有工具区按钮与拖拽走这里；缩略图点击直接调前台切换。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripAction {
    /// 切回普通桌面 / 重新进入 Stage 模式。
    ToggleMode,
    /// 打开设置面板（M1 只弹提示，M3 接真正的面板）。
    OpenSettings,
    /// 把 `window` 挪到 `before` 之前，`before` 为 `None` 表示移到末尾（D-053）。
    Reorder {
        window: isize,
        before: Option<isize>,
    },
}

/// 预览区窗口的创建错误。
#[derive(Debug, Clone)]
pub struct StripError(pub String);

impl fmt::Display for StripError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 常驻左侧的预览区窗口。
pub struct Strip {
    hwnd: HWND,
    /// 指向挂在窗口 `GWLP_USERDATA` 上的状态，窗口过程与实例共享。
    state: *mut StripState,
}

impl Strip {
    /// 创建预览区窗口并贴到主显示器工作区的左侧。
    ///
    /// `width_dip` 是配置里的设定值，实际宽度会按 DPI 与可用宽度收敛。
    /// 工具区按钮的点击通过 `actions` 交给调用方。
    /// 调用前应先执行 [`enable_per_monitor_v2`]，否则窗口会按系统缩放被拉伸。
    pub fn create(width_dip: u32, actions: Sender<StripAction>) -> Result<Self, StripError> {
        // SAFETY: 只查询本模块句柄。
        let instance = unsafe { GetModuleHandleW(None) }
            .map_err(|err| StripError(format!("GetModuleHandleW 失败: {err}")))?;

        let class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: HINSTANCE(instance.0),
            lpszClassName: CLASS_NAME,
            // 不设光标时 `DefWindowProc` 不会主动设置，鼠标形状会继承上一个窗口
            // 留下的状态（例如应用启动中的等待光标）。显式指定标准箭头。
            // SAFETY: `LoadCursorW` 只返回系统预定义光标的句柄，不会失败。
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(),
            // SAFETY: 图标资源编在本 exe 里，句柄由系统共享，不需要释放。
            hIcon: unsafe {
                LoadIconW(
                    Some(HINSTANCE(instance.0)),
                    PCWSTR(APP_ICON_ID as *const u16),
                )
            }
            .unwrap_or_default(),
            ..Default::default()
        };
        // SAFETY: 窗口类只注册一次；重复注册会返回 0 并设置 ERROR_CLASS_ALREADY_EXISTS。
        unsafe {
            RegisterClassW(&class);
        }

        let work = work_area();
        let initial_width = physical_width(width_dip, 96, work.right - work.left);
        // SAFETY: 创建本程序自己的窗口，父窗口与菜单为空。
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                CLASS_NAME,
                &HSTRING::from("AirPane"),
                WS_POPUP | WS_VISIBLE,
                work.left,
                work.top,
                initial_width,
                work.bottom - work.top,
                None,
                None,
                Some(HINSTANCE(instance.0)),
                None,
            )
        }
        .map_err(|err| StripError(format!("CreateWindowExW 失败: {err}")))?;

        // SAFETY: `GetDpiForWindow` 只读查询本窗口 DPI。
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        let state = Box::into_raw(Box::new(StripState {
            dpi: if dpi == 0 { 96 } else { dpi },
            width_dip,
            scroll: 0,
            hovered: None,
            tracking_leave: false,
            scheme: airpane_platform_win::theme::system_color_scheme(),
            drag: None,
            icons: IconCache::default(),
            entries: Vec::new(),
            actions,
        }));
        // SAFETY: 把状态指针挂到本窗口，窗口过程只在使用期间解引用。
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        }

        let strip = Self { hwnd, state };
        strip.relayout();
        Ok(strip)
    }

    pub fn hwnd(&self) -> isize {
        self.hwnd.0 as isize
    }

    /// 预览区的实际宽度（物理像素，已按 DPI 与可用宽度收敛）。
    pub fn width(&self) -> i32 {
        client_width(self.hwnd)
    }

    /// 显示或隐藏预览区。隐藏时不抢焦点，也不改变 Z 序以外的东西。
    pub fn set_visible(&self, visible: bool) {
        // SAFETY: 只改变本程序自己窗口的可见性；`SW_SHOWNOACTIVATE` 不会抢焦点。
        unsafe {
            if visible {
                // 先把内容画好再显示：隐藏期间窗口表面可能被系统丢掉，直接显示会
                // 先露一帧桌面（D-055）。
                let _ = InvalidateRect(Some(self.hwnd), None, false);
                let _ = UpdateWindow(self.hwnd);
            }
            let _ = ShowWindow(self.hwnd, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
    }

    /// 同步要展示的窗口列表，顺序即最近使用顺序（最前的最靠上）。
    ///
    /// 已不在列表中的窗口会被注销缩略图，新窗口注册缩略图，然后重新布局。
    pub fn sync(&mut self, items: &[StripItem]) -> Result<(), StripError> {
        // SAFETY: `state` 由 `Strip::create` 分配，生命周期跟随 `Strip`。
        let state = unsafe { &mut *self.state };

        let mut existing: HashMap<isize, Entry> = state
            .entries
            .drain(..)
            .map(|entry| (entry.window, entry))
            .collect();

        let mut next = Vec::with_capacity(items.len());
        for item in items {
            let thumb = match existing.remove(&item.window) {
                Some(entry) => entry.thumb,
                None => {
                    let source = HWND(item.window as *mut c_void);
                    // SAFETY: 源窗口由调用方从枚举结果传入，句柄可能已失效，
                    // `DwmRegisterThumbnail` 会返回错误而不是崩溃。
                    unsafe { DwmRegisterThumbnail(self.hwnd, source) }
                        .map_err(|err| StripError(format!("注册缩略图失败: {err}")))?
                }
            };
            let icon = state.icons.handle(item.window, item.pid, &item.class_name);
            next.push(Entry {
                window: item.window,
                pid: item.pid,
                class_name: item.class_name.clone(),
                icon,
                thumb,
                source: item.rect,
                active: item.active,
                rect: Default::default(),
                bar: Default::default(),
            });
        }

        for (_, entry) in existing {
            // SAFETY: 只注销本窗口注册的缩略图。
            unsafe {
                let _ = DwmUnregisterThumbnail(entry.thumb);
            }
        }

        state.entries = next;
        state.icons.retain_windows(&state.entries);
        // 列表可能在拖拽过程中变化：被拖的窗口消失就放弃这次拖拽，否则把落点钳回
        // 范围，避免松手时作用到别的窗口上。
        if let Some(window) = state.drag.map(|drag| drag.window) {
            if state.entries.iter().any(|entry| entry.window == window) {
                if let Some(drag) = state.drag.as_mut() {
                    drag.target = drag.target.min(state.entries.len());
                }
            } else {
                state.drag = None;
            }
        }
        // 悬停的窗口可能已经被切到前台而离开列表。
        let hovered = state.hovered;
        if hovered.is_some()
            && !state
                .entries
                .iter()
                .any(|entry| Some(entry.window) == hovered)
        {
            state.hovered = None;
        }
        // SAFETY: `state` 由 `Strip::create` 分配，只在 `Strip` 存续期间使用。
        unsafe {
            apply_layout(self.hwnd, self.state);
        }
        Ok(())
    }

    /// 按当前显示器 DPI 重新计算位置与宽度，并保持置顶。
    pub fn relayout(&self) {
        // SAFETY: `state` 由 `Strip::create` 分配。
        unsafe {
            relayout_window(self.hwnd, self.state);
        }
    }
}

impl Drop for Strip {
    fn drop(&mut self) {
        // SAFETY: 只销毁本程序创建的窗口，并取回挂在窗口上的状态。
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            let ptr = SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            if ptr != 0 {
                drop(Box::from_raw(ptr as *mut StripState));
            }
        }
    }
}
