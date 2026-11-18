//! M0 动画路线最小原型：DWM 缩略图 + 覆盖层窗口动画。
//!
//! 验证目标：在不改动源窗口位置的前提下，用置顶覆盖层承载 DWM 缩略图，靠每帧
//! `SetWindowPos` + `DwmUpdateThumbnailProperties` 把缩略图从舞台区缩小滑到预览区，
//! 并测出帧耗时与 CPU 占用。
//!
//! ```powershell
//! cargo run -p airpane-anim-prototype -- 300
//! ```

use std::process::ExitCode;
use std::time::{Duration, Instant};

use airpane_platform_win::window::enum_top_level_windows;
use windows::Win32::Foundation::{FILETIME, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_RECTDESTINATION, DWM_TNP_VISIBLE, DwmFlush,
    DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
    GetSystemMetrics, HWND_TOPMOST, MSG, PM_REMOVE, PeekMessageW, RegisterClassW, SM_CXSCREEN,
    SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, TranslateMessage, WM_DESTROY,
    WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::{BOOL, HSTRING, w};

/// 舞台矩形占可用屏幕的比例。
const STAGE_SCALE: f64 = 0.6;
/// 预览区宽度与缩略图尺寸，与 `crates/strip` 的默认值保持一致。
const STRIP_WIDTH: i32 = 200;
const THUMB_HEIGHT: i32 = 104;

#[derive(Default)]
struct FrameStats {
    frames: Vec<Duration>,
}

impl FrameStats {
    fn record(&mut self, elapsed: Duration) {
        self.frames.push(elapsed);
    }

    fn summary(&self) -> String {
        if self.frames.is_empty() {
            return "无帧".to_string();
        }
        let total: Duration = self.frames.iter().sum();
        let average = total / self.frames.len() as u32;
        let mut sorted = self.frames.clone();
        sorted.sort();
        let p95 = sorted[(sorted.len() as f64 * 0.95) as usize % sorted.len()];
        let max = *sorted.last().expect("非空");
        let fps = self.frames.len() as f64 / total.as_secs_f64();
        format!(
            "{} 帧，平均 {:.2}ms，p95 {:.2}ms，最大 {:.2}ms，等效 {:.1} FPS",
            self.frames.len(),
            average.as_secs_f64() * 1000.0,
            p95.as_secs_f64() * 1000.0,
            max.as_secs_f64() * 1000.0,
            fps
        )
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let duration_ms: u64 = args
        .iter()
        .skip(1)
        .find_map(|value| value.parse().ok())
        .unwrap_or(300);
    let flush = !args.iter().any(|arg| arg == "--no-flush");

    let Some(source) = pick_source_window() else {
        eprintln!("没有可用的源窗口，请先打开两个以上的普通窗口");
        return ExitCode::FAILURE;
    };

    // SAFETY: 无参数只读查询主屏尺寸。
    let (screen_width, screen_height) =
        unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    let stage = RECT {
        left: (screen_width as f64 * (1.0 - STAGE_SCALE) / 2.0) as i32,
        top: (screen_height as f64 * (1.0 - STAGE_SCALE) / 2.0) as i32,
        right: (screen_width as f64 * (1.0 + STAGE_SCALE) / 2.0) as i32,
        bottom: (screen_height as f64 * (1.0 + STAGE_SCALE) / 2.0) as i32,
    };
    let thumb = RECT {
        left: screen_width - STRIP_WIDTH + 8,
        top: 8,
        right: screen_width - 8,
        bottom: 8 + THUMB_HEIGHT,
    };

    let overlay = match create_overlay(&stage) {
        Ok(hwnd) => hwnd,
        Err(err) => {
            eprintln!("创建覆盖层失败: {err}");
            return ExitCode::FAILURE;
        }
    };

    // SAFETY: 覆盖层是刚创建的本进程窗口，源窗口来自枚举结果。
    let thumbnail = match unsafe { DwmRegisterThumbnail(overlay, HWND(source as *mut _)) } {
        Ok(handle) => handle,
        Err(err) => {
            eprintln!("注册缩略图失败: {err}");
            return ExitCode::FAILURE;
        }
    };

    let cpu_before = process_cpu_time();
    let wall_before = Instant::now();

    let mut stats = FrameStats::default();
    animate(
        &mut stats,
        overlay,
        thumbnail,
        stage,
        thumb,
        duration_ms,
        flush,
    );
    animate(
        &mut stats,
        overlay,
        thumbnail,
        thumb,
        stage,
        duration_ms,
        flush,
    );

    let wall = wall_before.elapsed();
    let cpu = process_cpu_time().saturating_sub(cpu_before);
    let cpu_percent = cpu.as_secs_f64() / wall.as_secs_f64() * 100.0;

    println!("源窗口 {source:#x}，覆盖层 {:#x}", overlay.0 as isize);
    println!(
        "动画时长参数 {duration_ms}ms，帧同步 {}",
        if flush { "DwmFlush" } else { "关闭" }
    );
    println!("{}", stats.summary());
    println!(
        "墙钟 {:.2}s，进程 CPU {:.2}s（{:.1}%）",
        wall.as_secs_f64(),
        cpu.as_secs_f64(),
        cpu_percent
    );

    // SAFETY: 注销本进程注册的缩略图并销毁覆盖层窗口。
    unsafe {
        let _ = DwmUnregisterThumbnail(thumbnail);
        let _ = DestroyWindow(overlay);
    }
    ExitCode::SUCCESS
}

/// 选一个不是前台的普通窗口作为动画的源。
fn pick_source_window() -> Option<isize> {
    // SAFETY: 无参数只读查询。
    let foreground = unsafe { GetForegroundWindow() };
    let foreground = foreground.0 as isize;
    enum_top_level_windows()
        .into_iter()
        .find(|info| info.is_managed() && info.hwnd != foreground)
        .map(|info| info.hwnd)
}

fn create_overlay(rect: &RECT) -> Result<HWND, String> {
    // SAFETY: 只查询本模块句柄。
    let instance = unsafe { GetModuleHandleW(None) }.map_err(|err| err.to_string())?;
    let class = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: HINSTANCE(instance.0),
        lpszClassName: w!("AirPaneAnimPrototype"),
        ..Default::default()
    };
    // SAFETY: 窗口类只注册一次。
    unsafe {
        RegisterClassW(&class);
    }
    // SAFETY: 创建本进程的覆盖层窗口。
    unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            w!("AirPaneAnimPrototype"),
            &HSTRING::from("AirPane 动画原型"),
            WS_POPUP | WS_VISIBLE,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            None,
        )
        .map_err(|err| err.to_string())
    }
}

/// 从 `from` 到 `to` 执行一次 ease-out 动画。
fn animate(
    stats: &mut FrameStats,
    overlay: HWND,
    thumbnail: isize,
    from: RECT,
    to: RECT,
    duration_ms: u64,
    flush: bool,
) {
    let started = Instant::now();
    let duration = Duration::from_millis(duration_ms);
    let mut last_frame = Instant::now();

    loop {
        let progress = (started.elapsed().as_secs_f64() / duration.as_secs_f64()).min(1.0);
        let eased = 1.0 - (1.0 - progress).powi(3);
        let rect = interpolate(from, to, eased);

        // SAFETY: 只移动本进程的覆盖层窗口。
        unsafe {
            let _ = SetWindowPos(
                overlay,
                Some(HWND_TOPMOST),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
            let _ = InvalidateRect(Some(overlay), None, false);
        }
        // 缩略图只能渲染到目标窗口的指定矩形，必须随窗口尺寸同步更新。
        update_thumbnail(thumbnail, rect.right - rect.left, rect.bottom - rect.top);

        // 对齐垂直同步，测量单帧代价。
        // SAFETY: 只等待 DWM 提交。
        if flush {
            unsafe {
                let _ = DwmFlush();
            }
        }
        let now = Instant::now();
        stats.record(now - last_frame);
        last_frame = now;

        if progress >= 1.0 {
            break;
        }
        pump_messages();
    }
}

fn interpolate(from: RECT, to: RECT, t: f64) -> RECT {
    let lerp = |a: i32, b: i32| (a as f64 + (b - a) as f64 * t) as i32;
    RECT {
        left: lerp(from.left, to.left),
        top: lerp(from.top, to.top),
        right: lerp(from.right, to.right),
        bottom: lerp(from.bottom, to.bottom),
    }
}

fn update_thumbnail(thumbnail: isize, width: i32, height: i32) {
    let props = DWM_THUMBNAIL_PROPERTIES {
        dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE,
        rcDestination: RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        },
        fVisible: BOOL::from(true),
        ..Default::default()
    };
    // SAFETY: 缩略图句柄由本进程持有。
    unsafe {
        let _ = DwmUpdateThumbnailProperties(thumbnail, &props);
    }
}

fn pump_messages() {
    let mut msg = MSG::default();
    // SAFETY: `msg` 是本栈帧上的 MSG。
    while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
        // SAFETY: 消息由 PeekMessageW 填充。
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// 进程累计 CPU 时间（用户态 + 内核态）。
fn process_cpu_time() -> Duration {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: 输出参数都是本栈帧上的 FILETIME。
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok.is_err() {
        return Duration::ZERO;
    }
    let to_duration = |time: FILETIME| {
        let ticks = ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64;
        // FILETIME 以 100ns 为单位。
        Duration::from_nanos(ticks * 100)
    };
    to_duration(kernel) + to_duration(user)
}

/// 覆盖层窗口过程：原型只需能创建与销毁。
///
/// 安全前提：由系统按窗口过程签名调用。
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DESTROY => LRESULT(0),
        _ => {
            // SAFETY: 未处理的消息交回系统默认处理。
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}
