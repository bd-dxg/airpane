//! 预览区窗口的创建、缩略图渲染与点击切换。

use core::ffi::c_void;
use core::fmt;
use std::collections::HashMap;
use std::sync::mpsc::Sender;

use airpane_core::config::{DEFAULT_STRIP_WIDTH_DIP, effective_strip_width};
use airpane_core::snapshot::Rect;
use airpane_core::title::short_title;
use airpane_platform_win::events;
use airpane_platform_win::foreground::activate;
use airpane_platform_win::icon::{AppIcon, app_icon};
use airpane_platform_win::monitor::primary_work_area;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION, DWM_TNP_VISIBLE,
    DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DEFAULT_GUI_FONT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint, FillRect,
    GetStockObject, HBRUSH, InvalidateRect, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DI_NORMAL, DefWindowProcW, DestroyWindow, DispatchMessageW, DrawIconEx,
    GWLP_USERDATA, GetClientRect, GetMessageW, GetWindowLongPtrW, HICON, HWND_TOPMOST, IDC_ARROW,
    LoadCursorW, MSG, PostQuitMessage, RegisterClassW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, WM_DESTROY,
    WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::{BOOL, HSTRING, w};

/// 预览区默认的逻辑宽度（DIP），PRD 4.5。实际生效值由 [`physical_width`] 计算。
pub const DEFAULT_WIDTH_DIP: u32 = DEFAULT_STRIP_WIDTH_DIP;
/// 预览区背景色（深灰，M0 阶段的降级视觉）。
const BACKGROUND: COLORREF = COLORREF(0x0020_2020);
/// 底部工具区背景色，比列表区域略亮。
const TOOLBAR_BACKGROUND: COLORREF = COLORREF(0x0032_3232);
/// 缩略图下方信息条背景色，在列表背景上留出一条能分辨的分隔。
const INFO_BAR_BACKGROUND: COLORREF = COLORREF(0x002B_2B2B);
/// 工具区与信息条的文字颜色。
const TEXT_COLOR: COLORREF = COLORREF(0x00E0_E0E0);

/// 底部工具区的高度（DIP）。
const TOOLBAR_HEIGHT_DIP: i32 = 40;
/// 工具区两个按钮的文字。M1 硬编码，M3 引入多语言资源表时替换（D-033）。
const TOOLBAR_LABELS: [&str; 2] = ["设置", "桌面"];

// 布局常量，单位都是 DIP。数值在实现时按实际观感调过（D-045）。
/// 列表左右与顶部内边距。
const PADDING_DIP: i32 = 8;
/// 相邻两项之间的间距。
const GAP_DIP: i32 = 6;
/// 每项底部信息条的高度。
const INFO_BAR_DIP: i32 = 20;
/// 信息条里应用图标的边长。
const ICON_DIP: i32 = 16;
/// 图标距信息条左边缘的距离。
const ICON_INSET_DIP: i32 = 2;
/// 图标与标题之间的间距。
const ICON_GAP_DIP: i32 = 6;
/// 标题距信息条右边缘的距离。
const TITLE_INSET_DIP: i32 = 4;
/// 缩略图区的最小高度。
const MIN_ITEM_DIP: i32 = 56;
/// 缩略图区的最大高度，悬停放大也不超过它。
const MAX_ITEM_DIP: i32 = 160;
/// 悬停时缩略图区按 6/5 放大（+20%），已达上限的不再放大（D-045）。
const HOVER_NUM: i32 = 6;
const HOVER_DEN: i32 = 5;
/// 源窗口尺寸不可用时使用的宽高比（16:9 的倒数）。
const FALLBACK_ASPECT: f64 = 9.0 / 16.0;

/// `WM_MOUSELEAVE` 在绑定里属于 `Win32_UI_Controls`，为一个常量不值得加 feature。
const WM_MOUSELEAVE: u32 = 0x02A3;

const CLASS_NAME: windows::core::PCWSTR = w!("AirPaneStrip");

/// 预览区要展示的一个窗口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StripItem {
    pub window: isize,
    /// 所属进程，与 `class_name` 一起决定图标缓存键（D-046）。
    pub pid: u32,
    pub class_name: String,
    /// 进程名，标题为空时作为兜底显示。
    pub process_name: String,
    /// 原始窗口标题，展示前会按 D-044 压成短标题。
    pub title: String,
    /// 源窗口的矩形，用于计算缩略图的宽高比。
    pub rect: Rect,
}

/// 预览区发出的动作。只有工具区按钮走这里；缩略图点击直接调前台切换。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripAction {
    /// 切回普通桌面 / 重新进入 Stage 模式。
    ToggleMode,
    /// 打开设置面板（M1 只弹提示，M3 接真正的面板）。
    OpenSettings,
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

/// 窗口过程需要访问的状态。
struct StripState {
    dpi: u32,
    /// 配置里的预览区宽度（DIP）。
    width_dip: u32,
    /// 缩略图列表已滚动的距离（物理像素）。
    scroll: i32,
    /// 当前悬停项的窗口句柄。用句柄而不是下标，列表重排后依然对得上。
    hovered: Option<isize>,
    /// 是否已向系统注册过 `WM_MOUSELEAVE`，避免每次移动都重复注册。
    tracking_leave: bool,
    /// 应用图标缓存，同一应用的多个窗口共用。
    icons: IconCache,
    entries: Vec<Entry>,
    /// 工具区动作的出口。
    actions: Sender<StripAction>,
}

struct Entry {
    window: isize,
    pid: u32,
    class_name: String,
    /// 展示用的短标题（D-044）。
    title: String,
    /// 应用图标句柄，生命周期由 `StripState::icons` 持有。
    icon: Option<isize>,
    /// DWM 缩略图句柄（`HTHUMBNAIL`，在绑定里表示为 `isize`）。
    thumb: isize,
    /// 最近一次已知的源窗口矩形，用于宽高比。
    source: Rect,
    /// 缩略图区域（不含信息条）。
    rect: RECT,
    /// 底部信息条：左侧图标，右侧标题。
    bar: RECT,
}

/// 应用图标缓存：key 是 `(pid, 类名)`，同一个进程的不同窗口类可能挂不同图标（D-046）。
#[derive(Default)]
struct IconCache {
    icons: HashMap<(u32, String), Option<AppIcon>>,
}

impl IconCache {
    /// 取 `(pid, 类名)` 的图标句柄，没缓存过就加载一次。
    ///
    /// 返回的句柄由缓存持有，只在对应缓存条目存在期间有效。
    fn handle(&mut self, hwnd: isize, pid: u32, class_name: &str) -> Option<isize> {
        let key = (pid, class_name.to_string());
        self.icons
            .entry(key)
            .or_insert_with(|| app_icon(hwnd, pid))
            .as_ref()
            .map(|icon| icon.handle().0 as isize)
    }

    /// 丢掉当前列表里已经没有的条目，避免长时间运行时缓存无界增长。
    fn retain_windows(&mut self, entries: &[Entry]) {
        self.icons.retain(|(pid, class_name), _| {
            entries
                .iter()
                .any(|entry| entry.pid == *pid && &entry.class_name == class_name)
        });
    }
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
                title: short_title(&item.title, &item.process_name),
                icon,
                thumb,
                source: item.rect,
                rect: RECT::default(),
                bar: RECT::default(),
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

/// 打开 Per-Monitor V2 DPI 感知。进程内只需调用一次，重复调用会失败并被忽略。
pub fn enable_per_monitor_v2() {
    // SAFETY: 进程级设置，无参数指针；失败通常表示已经设置过。
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// 在预览区线程上运行消息循环，直到窗口销毁。
pub fn run_message_loop() {
    let mut msg = MSG::default();
    // SAFETY: `msg` 是本栈帧上的 MSG。
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        // SAFETY: 消息由 GetMessageW 填充。
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// 重新计算窗口位置、DPI 与缩略图布局。
///
/// 安全前提：`state` 要么为空，要么指向存活的 `StripState`。
unsafe fn relayout_window(hwnd: HWND, state: *mut StripState) {
    let work = work_area();
    // SAFETY: 只查询本窗口所在显示器的 DPI。
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi == 0 { 96 } else { dpi };
    let width_dip = if state.is_null() {
        DEFAULT_WIDTH_DIP
    } else {
        // SAFETY: 见函数文档，`state` 只在窗口存续期间使用。
        unsafe { (*state).width_dip }
    };
    let width = physical_width(width_dip, dpi, work.right - work.left);

    // SAFETY: 只移动本程序自己的窗口，并保持置顶与不抢焦点。
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            work.left,
            work.top,
            width,
            work.bottom - work.top,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(Some(hwnd), None, true);
    }

    if state.is_null() {
        return;
    }
    // SAFETY: 见函数文档。
    unsafe {
        (*state).dpi = dpi;
        apply_layout(hwnd, state);
    }
}

/// 按当前 DPI、客户区大小与滚动位置重新排列缩略图，并推送到 DWM。
///
/// 安全前提：`state` 要么为空，要么指向存活的 `StripState`。
unsafe fn apply_layout(hwnd: HWND, state: *mut StripState) {
    if state.is_null() {
        return;
    }
    // SAFETY: 见函数文档。
    unsafe {
        let dpi = (*state).dpi;
        let width = client_width(hwnd);
        let available = client_height(hwnd) - toolbar_height(dpi);
        layout_entries(
            dpi,
            width,
            available,
            &mut (*state).scroll,
            (*state).hovered,
            &mut (*state).entries,
        );
        apply_thumbnails(&(*state).entries);
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

/// 按顺序纵向排列缩略图，每项是「缩略图 + 信息条」，高度按源窗口宽高比计算。
///
/// `scroll` 会被钳制在 `0..=(内容总高 - 可用高)` 内。超出工具区的项仍然会写进
/// `rect` 与 `bar`，但会被窗口客户区裁掉。悬停项按 `HOVER_NUM / HOVER_DEN` 放大，
/// 下面的项顺移。
fn layout_entries(
    dpi: u32,
    width: i32,
    available: i32,
    scroll: &mut i32,
    hovered: Option<isize>,
    entries: &mut [Entry],
) {
    let padding = scaled(PADDING_DIP, dpi);
    let gap = scaled(GAP_DIP, dpi);
    let bar_height = scaled(INFO_BAR_DIP, dpi);
    let inner = (width - padding * 2).max(1);
    let min_height = scaled(MIN_ITEM_DIP, dpi);
    let max_height = scaled(MAX_ITEM_DIP, dpi);

    let mut heights = Vec::with_capacity(entries.len());
    let mut total = padding;
    for entry in entries.iter() {
        let aspect = if entry.source.width() > 0 {
            entry.source.height() as f64 / entry.source.width() as f64
        } else {
            FALLBACK_ASPECT
        };
        let mut height = ((inner as f64 * aspect) as i32).clamp(min_height, max_height);
        if hovered == Some(entry.window) {
            height = (height * HOVER_NUM / HOVER_DEN).min(max_height);
        }
        heights.push(height + bar_height);
        total += height + bar_height + gap;
    }

    let max_scroll = (total - available).max(0);
    *scroll = (*scroll).clamp(0, max_scroll);

    let mut top = padding - *scroll;
    for (entry, height) in entries.iter_mut().zip(heights) {
        let thumb_bottom = top + height - bar_height;
        entry.rect = RECT {
            left: padding,
            top,
            right: padding + inner,
            bottom: thumb_bottom,
        };
        entry.bar = RECT {
            left: padding,
            top: thumb_bottom,
            right: padding + inner,
            bottom: top + height,
        };
        top += height + gap;
    }
}

fn apply_thumbnails(entries: &[Entry]) {
    for entry in entries {
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY,
            rcDestination: entry.rect,
            opacity: 255,
            fVisible: BOOL::from(true),
            ..Default::default()
        };
        // SAFETY: 缩略图句柄由本窗口持有，属性结构在栈上有效。
        unsafe {
            let _ = DwmUpdateThumbnailProperties(entry.thumb, &props);
        }
    }
}

/// 命中测试：缩略图与信息条都算在项内。
fn hit_test(entries: &[Entry], x: i32, y: i32) -> Option<isize> {
    entries
        .iter()
        .find(|entry| contains(&entry.rect, x, y) || contains(&entry.bar, x, y))
        .map(|entry| entry.window)
}

/// 工具区高度（物理像素）。
fn toolbar_height(dpi: u32) -> i32 {
    scaled(TOOLBAR_HEIGHT_DIP, dpi)
}

/// 工具区本身占用的矩形，贴客户区底部。
fn toolbar_rect(dpi: u32, width: i32, height: i32) -> RECT {
    let bar = toolbar_height(dpi);
    RECT {
        left: 0,
        top: height - bar,
        right: width,
        bottom: height,
    }
}

/// 工具区里两个按钮的矩形。
fn toolbar_buttons(dpi: u32, width: i32, height: i32) -> [RECT; 2] {
    let bar = toolbar_rect(dpi, width, height);
    let padding = scaled(PADDING_DIP, dpi);
    let gap = scaled(GAP_DIP, dpi);
    let inner = (width - padding * 2 - gap).max(2) / 2;
    let top = bar.top + scaled(6, dpi);
    let bottom = bar.bottom - scaled(6, dpi);
    [
        RECT {
            left: padding,
            top,
            right: padding + inner,
            bottom,
        },
        RECT {
            left: padding + inner + gap,
            top,
            right: padding + inner + gap + inner,
            bottom,
        },
    ]
}

fn contains(rect: &RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

fn scaled(value: i32, dpi: u32) -> i32 {
    value * dpi as i32 / 96
}

/// 把配置里的 DIP 宽度换算成物理像素，并应用 D-012 的钳制规则。
fn physical_width(width_dip: u32, dpi: u32, available_px: i32) -> i32 {
    let dpi = dpi.max(1) as i32;
    let available_dip = available_px * 96 / dpi;
    let effective = effective_strip_width(width_dip, available_dip);
    effective as i32 * dpi / 96
}

fn client_width(hwnd: HWND) -> i32 {
    let mut rect = RECT::default();
    // SAFETY: 输出参数是本栈帧上的 RECT。
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    rect.right - rect.left
}

fn client_height(hwnd: HWND) -> i32 {
    let mut rect = RECT::default();
    // SAFETY: 输出参数是本栈帧上的 RECT。
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    rect.bottom - rect.top
}

/// 主显示器工作区（扣除任务栏）。
fn work_area() -> RECT {
    let area = primary_work_area();
    RECT {
        left: area.left,
        top: area.top,
        right: area.right,
        bottom: area.bottom,
    }
}

/// 绘制背景、工具区，以及每一项底部的信息条（图标 + 标题）。
///
/// 安全前提：`state` 要么为空，要么指向存活的 `StripState`。
fn paint(hwnd: HWND, state: *mut StripState) {
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: `ps` 是本栈帧上的 PAINTSTRUCT，BeginPaint/EndPaint 必须成对调用；
    // `DrawTextW` 的文本缓冲区在调用期间存活，矩形也是本栈帧上的局部变量。
    unsafe {
        let hdc = BeginPaint(hwnd, &mut ps);
        let brush: HBRUSH = CreateSolidBrush(BACKGROUND);
        FillRect(hdc, &ps.rcPaint, brush);

        // SAFETY: 只查询本窗口 DPI。
        let dpi = GetDpiForWindow(hwnd);
        let dpi = if dpi == 0 { 96 } else { dpi };
        let width = client_width(hwnd);
        let height = client_height(hwnd);
        let toolbar = toolbar_rect(dpi, width, height);
        let toolbar_brush: HBRUSH = CreateSolidBrush(TOOLBAR_BACKGROUND);
        FillRect(hdc, &toolbar, toolbar_brush);

        let font = GetStockObject(DEFAULT_GUI_FONT);
        let previous = SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, TEXT_COLOR);

        for (mut rect, label) in toolbar_buttons(dpi, width, height)
            .into_iter()
            .zip(TOOLBAR_LABELS)
        {
            let mut text: Vec<u16> = label.encode_utf16().collect();
            DrawTextW(
                hdc,
                &mut text,
                &mut rect as *mut RECT,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
        }

        // 信息条：左侧应用图标，右侧短标题。没有图标的项也保留图标的位置，
        // 这样同一列标题的左边缘始终对齐。
        if !state.is_null() {
            // SAFETY: 见函数文档，`state` 只在窗口存续期间使用。
            let state = &*state;
            let bar_brush: HBRUSH = CreateSolidBrush(INFO_BAR_BACKGROUND);
            let icon_size = scaled(ICON_DIP, dpi);
            let icon_inset = scaled(ICON_INSET_DIP, dpi);
            let title_left = icon_inset + icon_size + scaled(ICON_GAP_DIP, dpi);
            let title_inset = scaled(TITLE_INSET_DIP, dpi);

            for entry in &state.entries {
                FillRect(hdc, &entry.bar, bar_brush);
                if let Some(icon) = entry.icon {
                    let top = entry.bar.top + (entry.bar.bottom - entry.bar.top - icon_size) / 2;
                    let _ = DrawIconEx(
                        hdc,
                        entry.bar.left + icon_inset,
                        top,
                        HICON(icon as *mut c_void),
                        icon_size,
                        icon_size,
                        0,
                        None,
                        DI_NORMAL,
                    );
                }
                let mut text_rect = RECT {
                    left: entry.bar.left + title_left,
                    top: entry.bar.top,
                    right: entry.bar.right - title_inset,
                    bottom: entry.bar.bottom,
                };
                let mut text: Vec<u16> = entry.title.encode_utf16().collect();
                DrawTextW(
                    hdc,
                    &mut text,
                    &mut text_rect as *mut RECT,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
                );
            }

            let _ = DeleteObject(bar_brush.into());
        }

        SelectObject(hdc, previous);
        let _ = DeleteObject(toolbar_brush.into());
        let _ = DeleteObject(brush.into());
        let _ = EndPaint(hwnd, &ps);
    }
}

/// 预览区窗口的消息处理。
///
/// 安全前提：由系统按窗口过程签名调用，句柄与用户数据都属于本进程。
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: 用户数据由本进程写入，窗口存续期间保持有效。
    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut StripState;

    match msg {
        WM_PAINT => {
            paint(hwnd, state);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_DPICHANGED => {
            // SAFETY: 状态指针由本进程维护。
            unsafe {
                relayout_window(hwnd, state);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if !state.is_null() {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                // SAFETY: 状态指针由本进程维护，只在窗口存续期间使用。
                let state_ref = unsafe { &mut *state };
                if !state_ref.tracking_leave {
                    state_ref.tracking_leave = true;
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    // SAFETY: `track` 是本栈帧上的结构体，字段已按 API 要求填好。
                    unsafe {
                        let _ = TrackMouseEvent(&mut track);
                    }
                }
                // 工具区上的移动不算悬停列表项。
                let list_bottom = client_height(hwnd) - toolbar_height(state_ref.dpi);
                let hovered = if y < list_bottom {
                    hit_test(&state_ref.entries, x, y)
                } else {
                    None
                };
                if hovered != state_ref.hovered {
                    state_ref.hovered = hovered;
                    // SAFETY: 状态指针由本进程维护。
                    unsafe {
                        apply_layout(hwnd, state);
                    }
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if !state.is_null() {
                // SAFETY: 状态指针由本进程维护，只在窗口存续期间使用。
                let state_ref = unsafe { &mut *state };
                state_ref.tracking_leave = false;
                if state_ref.hovered.take().is_some() {
                    // SAFETY: 状态指针由本进程维护。
                    unsafe {
                        apply_layout(hwnd, state);
                    }
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            if !state.is_null() {
                // SAFETY: 状态指针由本进程维护，只在窗口存续期间使用。
                let (actions, entries) = unsafe {
                    let state = &*state;
                    (state.actions.clone(), &state.entries)
                };
                let dpi = unsafe { (*state).dpi };
                let buttons = toolbar_buttons(dpi, client_width(hwnd), client_height(hwnd));

                if contains(&buttons[0], x, y) {
                    let _ = actions.send(StripAction::OpenSettings);
                    events::wake();
                } else if contains(&buttons[1], x, y) {
                    let _ = actions.send(StripAction::ToggleMode);
                    events::wake();
                } else if y < client_height(hwnd) - toolbar_height(dpi) {
                    // 列表被工具区遮住的部分不参与点击。
                    if let Some(window) = hit_test(entries, x, y) {
                        let _ = activate(window);
                    }
                }
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            if !state.is_null() {
                // SAFETY: 状态指针由本进程维护，只在窗口存续期间使用。
                let dpi = unsafe { (*state).dpi };
                // 滚轮增量在高 16 位，一格是 120。
                let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
                if delta != 0 {
                    let step = scaled(60, dpi);
                    // SAFETY: 同上。
                    unsafe {
                        (*state).scroll -= (delta as i32 / 120) * step;
                        apply_layout(hwnd, state);
                    }
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: 标准退出流程。
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => {
            // SAFETY: 未处理的消息交回系统默认处理。
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}
