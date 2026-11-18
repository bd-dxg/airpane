//! 预览区窗口的创建、缩略图渲染与点击切换。

use core::ffi::c_void;
use core::fmt;
use std::collections::HashMap;

use airpane_core::config::{DEFAULT_STRIP_WIDTH_DIP, effective_strip_width};
use airpane_core::snapshot::Rect;
use airpane_platform_win::foreground::activate;
use airpane_platform_win::monitor::primary_work_area;
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION, DWM_TNP_VISIBLE,
    DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HBRUSH, InvalidateRect,
    PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetClientRect,
    GetMessageW, GetWindowLongPtrW, HWND_TOPMOST, MSG, PostQuitMessage, RegisterClassW,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowLongPtrW, SetWindowPos, TranslateMessage, WM_DESTROY,
    WM_DPICHANGED, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_PAINT, WNDCLASSW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::{BOOL, HSTRING, w};

/// 预览区默认的逻辑宽度（DIP），PRD 4.5。实际生效值由 [`physical_width`] 计算。
pub const DEFAULT_WIDTH_DIP: u32 = DEFAULT_STRIP_WIDTH_DIP;
/// 预览区背景色（深灰，M0 阶段的降级视觉）。
const BACKGROUND: COLORREF = COLORREF(0x0020_2020);

// 布局常量，单位都是 DIP。
const PADDING_DIP: i32 = 8;
const GAP_DIP: i32 = 8;
const MIN_ITEM_DIP: i32 = 56;
const MAX_ITEM_DIP: i32 = 160;
/// 源窗口尺寸不可用时使用的宽高比（16:9 的倒数）。
const FALLBACK_ASPECT: f64 = 9.0 / 16.0;

const CLASS_NAME: windows::core::PCWSTR = w!("AirPaneStrip");

/// 预览区要展示的一个窗口。
#[derive(Debug, Clone, Copy)]
pub struct StripItem {
    pub window: isize,
    /// 源窗口的矩形，用于计算缩略图的宽高比。
    pub rect: Rect,
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
    entries: Vec<Entry>,
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
    /// 调用前应先执行 [`enable_per_monitor_v2`]，否则窗口会按系统缩放被拉伸。
    pub fn create(width_dip: u32) -> Result<Self, StripError> {
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
            entries: Vec::new(),
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
        layout_entries(state.dpi, client_width(self.hwnd), &mut state.entries);
        apply_thumbnails(&state.entries);
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
        let width = client_width(hwnd);
        layout_entries(dpi, width, &mut (*state).entries);
        apply_thumbnails(&(*state).entries);
    }
}

/// 按顺序纵向排列缩略图，每项高度按源窗口宽高比计算。
fn layout_entries(dpi: u32, width: i32, entries: &mut [Entry]) {
    let padding = scaled(PADDING_DIP, dpi);
    let gap = scaled(GAP_DIP, dpi);
    let inner = (width - padding * 2).max(1);
    let min_height = scaled(MIN_ITEM_DIP, dpi);
    let max_height = scaled(MAX_ITEM_DIP, dpi);
    let mut top = padding;

    for entry in entries.iter_mut() {
        let aspect = if entry.source.width() > 0 {
            entry.source.height() as f64 / entry.source.width() as f64
        } else {
            FALLBACK_ASPECT
        };
        let height = ((inner as f64 * aspect) as i32).clamp(min_height, max_height);
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
    // SAFETY: `ps` 是本栈帧上的 PAINTSTRUCT，BeginPaint/EndPaint 必须成对调用。
    unsafe {
        let hdc = BeginPaint(hwnd, &mut ps);
        let brush: HBRUSH = CreateSolidBrush(BACKGROUND);
        FillRect(hdc, &ps.rcPaint, brush);
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
                let entries = unsafe { &(*state).entries };
                if let Some(window) = hit_test(entries, x, y) {
                    let _ = activate(window);
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
