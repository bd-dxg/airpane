//! 预览区窗口的创建与绘制。

use core::ffi::c_void;
use core::fmt;

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HBRUSH, InvalidateRect,
    PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, HWND_TOPMOST, MSG, PostQuitMessage, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN,
    SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_SHOWWINDOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SetWindowPos, SystemParametersInfoW, TranslateMessage, WM_DESTROY, WM_DPICHANGED,
    WM_ERASEBKGND, WM_PAINT, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP, WS_VISIBLE,
};
use windows::core::{HSTRING, w};

/// 预览区的逻辑宽度（DIP）。PRD 4.5 的默认值。
pub const DEFAULT_WIDTH_DIP: u32 = 200;
/// 预览区背景色（深灰，M0 阶段的降级视觉）。
const BACKGROUND: COLORREF = COLORREF(0x0020_2020);

const CLASS_NAME: windows::core::PCWSTR = w!("AirPaneStrip");

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
}

impl Strip {
    /// 创建预览区窗口并贴到主显示器工作区的左侧。
    ///
    /// 调用前应先执行 [`enable_per_monitor_v2`]，否则窗口会按系统缩放被拉伸。
    pub fn create() -> Result<Self, StripError> {
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
        // SAFETY: 创建本程序自己的窗口，父窗口与菜单为空。
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                CLASS_NAME,
                &HSTRING::from("AirPane"),
                WS_POPUP | WS_VISIBLE,
                work.left,
                work.top,
                scaled_width(96),
                work.bottom - work.top,
                None,
                None,
                Some(HINSTANCE(instance.0)),
                None,
            )
        }
        .map_err(|err| StripError(format!("CreateWindowExW 失败: {err}")))?;

        let strip = Self { hwnd };
        strip.relayout();
        Ok(strip)
    }

    pub fn hwnd(&self) -> isize {
        self.hwnd.0 as isize
    }

    /// 按当前显示器 DPI 重新计算位置与宽度，并保持置顶。
    pub fn relayout(&self) {
        relayout_window(self.hwnd);
    }
}

impl Drop for Strip {
    fn drop(&mut self) {
        // SAFETY: 只销毁本程序创建的窗口。
        unsafe {
            let _ = DestroyWindow(self.hwnd);
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

fn relayout_window(hwnd: HWND) {
    let work = work_area();
    // SAFETY: 只查询本窗口所在显示器的 DPI。
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi == 0 { 96 } else { dpi };
    // SAFETY: 只移动本程序自己的窗口，并保持置顶与不抢焦点。
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            work.left,
            work.top,
            scaled_width(dpi),
            work.bottom - work.top,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

fn scaled_width(dpi: u32) -> i32 {
    (DEFAULT_WIDTH_DIP * dpi / 96) as i32
}

/// 主显示器工作区（扣除任务栏）。拿不到时退化为整屏。
fn work_area() -> RECT {
    let mut rect = RECT::default();
    // SAFETY: 输出参数是本栈帧上的 RECT，类型与大小一致。
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rect as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_err() {
        // SAFETY: 无参数查询主屏尺寸。
        unsafe {
            rect.right = GetSystemMetrics(SM_CXSCREEN);
            rect.bottom = GetSystemMetrics(SM_CYSCREEN);
        }
    }
    rect
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
/// 安全前提：由系统按窗口过程签名调用，句柄属于本进程。
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_DPICHANGED => {
            relayout_window(hwnd);
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
