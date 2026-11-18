//! 极简文件日志。
//!
//! 依据 D-027：M1 不引入 `tracing`。单文件按大小滚动，写入失败时静默丢弃，不给主
//! 流程制造新的失败点；异常退出后能靠这份日志定位到崩溃前最后几步。
//!
//! **隐私约定**：日志只写 hwnd、pid、进程名与错误码。窗口标题与窗口内容不落盘
//! （PRD 11.3 第 5 条：窗口画面只用于 DWM 缩略图实时显示，不截图落盘）。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use windows::Win32::System::SystemInformation::GetLocalTime;

pub use airpane_core::config::LogLevel;

/// 单个日志文件的大小上限。
const DEFAULT_MAX_BYTES: u64 = 1024 * 1024;
/// 保留的历史文件数量：`airpane.1.log` 到 `airpane.3.log`。
const MAX_BACKUPS: u32 = 3;

static LOGGER: OnceLock<Mutex<Option<Logger>>> = OnceLock::new();

/// 初始化全局日志器。重复调用会替换掉之前的实例。
pub fn init(level: LogLevel, path: &Path) -> Result<(), String> {
    init_with_limit(level, path, DEFAULT_MAX_BYTES)
}

/// 写入一条日志。日志器未初始化时静默丢弃。
pub fn log(level: LogLevel, message: &str) {
    let Some(cell) = LOGGER.get() else {
        return;
    };
    let Ok(mut guard) = cell.lock() else {
        return;
    };
    if let Some(logger) = guard.as_mut() {
        logger.write(level, message);
    }
}

fn init_with_limit(level: LogLevel, path: &Path, max_bytes: u64) -> Result<(), String> {
    let logger = Logger::open(level, path, max_bytes)
        .map_err(|err| format!("无法打开日志文件 {}: {err}", path.display()))?;
    let cell = LOGGER.get_or_init(|| Mutex::new(None));
    *cell.lock().map_err(|_| "日志锁已被污染".to_string())? = Some(logger);
    Ok(())
}

struct Logger {
    level: LogLevel,
    path: PathBuf,
    file: Option<File>,
    max_bytes: u64,
}

impl Logger {
    fn open(level: LogLevel, path: &Path, max_bytes: u64) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            level,
            path: path.to_path_buf(),
            file: Some(file),
            max_bytes: max_bytes.max(1),
        })
    }

    fn write(&mut self, level: LogLevel, message: &str) {
        if level > self.level {
            return;
        }
        let line = format!("{} {:<5} {}\n", timestamp(), level.as_str(), message);
        let size = self
            .file
            .as_ref()
            .and_then(|file| file.metadata().ok())
            .map_or(0, |meta| meta.len());
        if size + line.len() as u64 > self.max_bytes {
            self.roll();
        }
        if let Some(file) = self.file.as_mut() {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    }

    /// 关闭当前文件并把它挪到 `.1`，其余历史文件顺延。
    ///
    /// Windows 上被打开的文件不能改名，所以必须先释放句柄；挪动失败（比如历史文件
    /// 不存在）直接忽略，日志本身不该成为错误来源。
    fn roll(&mut self) {
        self.file = None;
        for index in (1..MAX_BACKUPS).rev() {
            let _ = fs::rename(
                backup_path(&self.path, index),
                backup_path(&self.path, index + 1),
            );
        }
        let _ = fs::rename(&self.path, backup_path(&self.path, 1));
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .ok();
    }
}

fn backup_path(path: &Path, index: u32) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = match path
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned())
    {
        Some(ext) if !ext.is_empty() => format!("{stem}.{index}.{ext}"),
        _ => format!("{stem}.{index}"),
    };
    path.with_file_name(name)
}

fn timestamp() -> String {
    // SAFETY: `GetLocalTime` 只读取当前本地时间，不接收指针参数，也不会失败。
    let time = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        time.wYear,
        time.wMonth,
        time.wDay,
        time.wHour,
        time.wMinute,
        time.wSecond,
        time.wMilliseconds,
    )
}

/// 写一条 ERROR 日志。
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::logging::log($crate::logging::LogLevel::Error, &format!($($arg)*))
    };
}

/// 写一条 WARN 日志。
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::logging::log($crate::logging::LogLevel::Warn, &format!($($arg)*))
    };
}

/// 写一条 INFO 日志。
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::logging::log($crate::logging::LogLevel::Info, &format!($($arg)*))
    };
}

/// 写一条 DEBUG 日志。
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        $crate::logging::log($crate::logging::LogLevel::Debug, &format!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("airpane-log-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn writes_level_and_message() {
        let dir = temp_dir("write");
        let path = dir.join("airpane.log");
        let mut logger = Logger::open(LogLevel::Info, &path, DEFAULT_MAX_BYTES).expect("打开日志");

        logger.write(LogLevel::Info, "窗口已移入舞台区");
        let text = fs::read_to_string(&path).expect("读取日志");
        assert!(text.contains("INFO"));
        assert!(text.contains("窗口已移入舞台区"));
        assert!(text.starts_with("20"), "应当以年份开头: {text}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn filters_messages_below_configured_level() {
        let dir = temp_dir("filter");
        let path = dir.join("airpane.log");
        let mut logger = Logger::open(LogLevel::Warn, &path, DEFAULT_MAX_BYTES).expect("打开日志");

        logger.write(LogLevel::Info, "不该出现");
        logger.write(LogLevel::Error, "应当出现");
        let text = fs::read_to_string(&path).expect("读取日志");
        assert!(!text.contains("不该出现"));
        assert!(text.contains("应当出现"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rolls_when_size_limit_is_reached() {
        let dir = temp_dir("roll");
        let path = dir.join("airpane.log");
        let mut logger = Logger::open(LogLevel::Debug, &path, 200).expect("打开日志");

        for index in 0..20 {
            logger.write(LogLevel::Info, &format!("第 {index} 条日志"));
        }

        assert!(dir.join("airpane.1.log").exists(), "应当生成备份文件");
        let current = fs::read_to_string(&path).expect("当前日志应当存在");
        assert!(
            current.len() <= 200,
            "当前文件不应超过限制: {}",
            current.len()
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_name_keeps_extension() {
        let path = Path::new("C:\\logs\\airpane.log");
        assert_eq!(
            backup_path(path, 2),
            PathBuf::from("C:\\logs\\airpane.2.log")
        );
    }
}
