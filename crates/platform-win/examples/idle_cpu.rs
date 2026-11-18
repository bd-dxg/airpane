//! 空闲 CPU 验收：采样运行中的 AirPane 进程，输出 CPU 占用与可管理窗口数。
//!
//! ```powershell
//! cargo run -p airpane-platform-win --example idle_cpu -- --seconds 20
//! ```
//!
//! 进程按预览区窗口的类名定位，不需要手动传 PID。窗口数 ≥ 20 且 CPU ≥ 1% 时以
//! 非零退出码结束，方便当成验收判据。

use std::process::ExitCode;
use std::time::{Duration, Instant};

use airpane_platform_win::window::enum_managed_windows;
use windows::Win32::Foundation::{CloseHandle, FILETIME, HWND, LPARAM};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowThreadProcessId,
};
use windows::core::BOOL;

/// 预览区窗口的类名，与 `airpane-strip` 的 `CLASS_NAME` 保持一致。
const STRIP_CLASS: &str = "AirPaneStrip";

fn main() -> ExitCode {
    let seconds = arg_value("--seconds")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(20);

    let Some(pid) = find_airpane_pid() else {
        println!("没找到正在运行的 AirPane（需要先启动主程序）。");
        return ExitCode::FAILURE;
    };
    let windows = enum_managed_windows().len();
    let Some(start) = cpu_time(pid) else {
        println!("读取进程 {pid} 的 CPU 时间失败。");
        return ExitCode::FAILURE;
    };

    println!("采样 {seconds} 秒：pid={pid}，可管理窗口 {windows} 个");
    let started = Instant::now();
    std::thread::sleep(Duration::from_secs(seconds));
    let elapsed = started.elapsed();

    let Some(end) = cpu_time(pid) else {
        println!("读取进程 {pid} 的 CPU 时间失败。");
        return ExitCode::FAILURE;
    };
    let used = end.saturating_sub(start);
    let percent = used.as_secs_f64() / elapsed.as_secs_f64() * 100.0;
    println!(
        "空闲 CPU 占用：{percent:.2}%（窗口 {windows} 个，采样 {:.1} 秒）",
        elapsed.as_secs_f64()
    );

    if windows >= 20 && percent >= 1.0 {
        println!("验收不通过：20 个窗口时空闲 CPU 应低于 1%。");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn arg_value(flag: &str) -> Option<String> {
    let mut args = std::env::args().skip_while(|arg| arg != flag);
    args.next()?;
    args.next()
}

/// 找到预览区窗口所属的进程。
fn find_airpane_pid() -> Option<u32> {
    let mut found: Option<u32> = None;
    // SAFETY: 回调只把 pid 写进 `found`，该变量在调用期间存活。
    unsafe {
        let _ = EnumWindows(
            Some(find_strip_process),
            LPARAM(&mut found as *mut Option<u32> as isize),
        );
    }
    found
}

/// `EnumWindows` 的回调：命中预览区类名时记录进程并停止枚举。
///
/// 安全前提：`lparam` 必须指向存活的 `Option<u32>`。
unsafe extern "system" fn find_strip_process(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let mut buf = [0u16; 64];
    // SAFETY: 缓冲区长度如实传入。
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 || String::from_utf16_lossy(&buf[..len as usize]) != STRIP_CLASS {
        return BOOL::from(true);
    }
    let mut pid = 0u32;
    // SAFETY: 输出参数是本栈帧上的 `u32`。
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    // SAFETY: 见函数文档。
    let slot = unsafe { &mut *(lparam.0 as *mut Option<u32>) };
    *slot = Some(pid);
    BOOL::from(false)
}

/// 进程累计的 CPU 时间（内核 + 用户）。
fn cpu_time(pid: u32) -> Option<Duration> {
    // SAFETY: 以查询权限打开进程；四个 `FILETIME` 都是本栈帧上的局部变量。
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let result = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user);
        let _ = CloseHandle(handle);
        if result.is_err() {
            return None;
        }
        let total = filetime_nanos(&kernel) + filetime_nanos(&user);
        Some(Duration::from_nanos(total))
    }
}

/// `FILETIME` 以 100 纳秒为单位。
fn filetime_nanos(time: &FILETIME) -> u64 {
    let ticks = (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    ticks.saturating_mul(100)
}
