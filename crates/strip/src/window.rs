//! 预览区窗口的创建、缩略图渲染与点击切换。

use core::ffi::c_void;
use core::fmt;
use std::collections::HashMap;
use std::sync::mpsc::Sender;

use airpane_core::config::{DEFAULT_STRIP_WIDTH_DIP, effective_strip_width};
use airpane_core::snapshot::Rect;
use airpane_platform_win::events;
use airpane_platform_win::foreground::activate;
use airpane_platform_win::monitor::primary_work_area;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION, DWM_TNP_VISIBLE,
    DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DEFAULT_GUI_FONT, DT_CENTER, DT_SINGLELINE, DT_VCENTER,
    DeleteObject, DrawTextW, EndPaint, FillRect, GetStockObject, HBRUSH, InvalidateRect,
    PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetClientRect,
    GetMessageW, GetWindowLongPtrW, HWND_TOPMOST, MSG, PostQuitMessage, RegisterClassW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TranslateMessage, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_MOUSEWHEEL,
    WM_PAINT, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::{BOOL, HSTRING, w};

/// 预览区默认的逻辑宽度（DIP），PRD 4.5。实际生效值由 [`physical_width`] 计算。
pub const DEFAULT_WIDTH_DIP: u32 = DEFAULT_STRIP_WIDTH_DIP;
/// 预览区背景色（深灰，M0 阶段的降级视觉）。
const BACKGROUND: COLORREF = COLORREF(0x0020_2020);
/// 底部工具区背景色，比列表区域略亮。
const TOOLBAR_BACKGROUND: COLORREF = COLORREF(0x0032_3232);
/// 工具区文字颜色。
const TEXT_COLOR: COLORREF = COLORREF(0x00E0_E0E0);

/// 底部工具区的高度（DIP）。
const TOOLBAR_HEIGHT_DIP: i32 = 40;
/// 工具区两个按钮的文字。M1 硬编码，M3 引入多语言资源表时替换（D-033）。
const TOOLBAR_LABELS: [&str; 2] = ["设置", "桌面"];

// 布局常量，单位都是 DIP。
const PADDING_DIP: i32 = 8;
const GAP_DIP: i32 = 8;
const MIN_ITEM_DIP: i32 = 56;
const MAX_ITEM_DIP: i32 = 160;
/// 源窗口尺寸不可用时使用的宽高比（16:9 的倒数）。
const FALLBACK_ASPECT: f64 = 9.0 / 16.0;

const CLASS_NAME: windows::core::PCWSTR = w!("AirPaneStrip");

/// 预览区要展示的一个窗口。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StripItem {
    pub window: isize,
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
    entries: Vec<Entry>,
    /// 工具区动作的出口。
    actions: Sender<StripAction>,
}

struct Entry {
    window: isize,
    /// DWM 缩略图句柄（`HTHUMBNAIL`，在绑定里表示为 `isize`）。
    thumb: isize,
    /// 最近一次已知的源窗口矩形，用于宽高比。
    source: Rect,
    /// 缩略图在预览区客户区中的目标矩形。
    rect: RECT,
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
            let entry = match existing.remove(&item.window) {
                Some(mut entry) => {
                    entry.source = item.rect;
                    entry
                }
                None => {
                    let source = HWND(item.window as *mut c_void);
                    // SAFETY: 源窗口由调用方从枚举结果传入，句柄可能已失效，
                    // `DwmRegisterThumbnail` 会返回错误而不是崩溃。
                    let thumb = unsafe { DwmRegisterThumbnail(self.hwnd, source) }
                        .map_err(|err| StripError(format!("注册缩略图失败: {err}")))?;
                    Entry {
                        window: item.window,
                        thumb,
                        source: item.rect,
                        rect: RECT::default(),
                    }
                }
            };
            next.push(entry);
        }

        for (_, entry) in existing {
            // SAFETY: 只注销本窗口注册的缩略图。
            unsafe {
                let _ = DwmUnregisterThumbnail(entry.thumb);
            }
        }

        state.entries = next;
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
            &mut (*state).entries,
        );
        apply_thumbnails(&(*state).entries);
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

/// 按顺序纵向排列缩略图，每项高度按源窗口宽高比计算。
///
/// `scroll` 会被钳制在 `0..=(内容总高 - 可用高)` 内。超出工具区的项仍然会写进
/// `rect`，但会被窗口客户区裁掉。
fn layout_entries(dpi: u32, width: i32, available: i32, scroll: &mut i32, entries: &mut [Entry]) {
    let padding = scaled(PADDING_DIP, dpi);
    let gap = scaled(GAP_DIP, dpi);
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
        let height = ((inner as f64 * aspect) as i32).clamp(min_height, max_height);
        heights.push(height);
        total += height + gap;
    }

    let max_scroll = (total - available).max(0);
    *scroll = (*scroll).clamp(0, max_scroll);

    let mut top = padding - *scroll;
    for (entry, height) in entries.iter_mut().zip(heights) {
        entry.rect = RECT {
            left: padding,
            top,
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

fn hit_test(entries: &[Entry], x: i32, y: i32) -> Option<isize> {
    entries
        .iter()
        .find(|entry| {
            x >= entry.rect.left
                && x < entry.rect.right
                && y >= entry.rect.top
                && y < entry.rect.bottom
        })
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

fn paint(hwnd: HWND) {
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
            paint(hwnd);
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
                } else if let Some(window) = hit_test(entries, x, y) {
                    let _ = activate(window);
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
