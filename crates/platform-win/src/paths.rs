//! 应用数据文件的路径。
//!
//! 统一放在 `%APPDATA%\AirPane` 下（D-029）：用户设置、崩溃恢复快照、尺寸记忆与
//! 日志各占一个文件，生命周期互不干扰。`APPDATA` 缺失时退回可执行文件所在目录，
//! 保证便携运行时也能留下配置与日志。

use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

const APP_DIR_NAME: &str = "AirPane";
const LOG_DIR_NAME: &str = "logs";
const CONFIG_FILE_NAME: &str = "config.json";
const SNAPSHOT_FILE_NAME: &str = "snapshot.json";
const WINDOW_SIZES_FILE_NAME: &str = "window-sizes.json";
const STRIP_ORDER_FILE_NAME: &str = "strip-order.json";
const LOG_FILE_NAME: &str = "airpane.log";

/// 应用数据目录，不存在时连同 `logs` 子目录一起创建。
pub fn ensure_app_dir() -> io::Result<PathBuf> {
    let dir = app_dir();
    fs::create_dir_all(dir.join(LOG_DIR_NAME))?;
    Ok(dir)
}

/// 应用数据目录：优先 `%APPDATA%\AirPane`，否则回退到可执行文件同级目录。
pub fn app_dir() -> PathBuf {
    match env::var_os("APPDATA") {
        Some(appdata) if !appdata.is_empty() => PathBuf::from(appdata).join(APP_DIR_NAME),
        _ => executable_dir().join(APP_DIR_NAME),
    }
}

/// 用户配置文件。
pub fn config_file() -> PathBuf {
    app_dir().join(CONFIG_FILE_NAME)
}

/// 进入 Stage 模式前落盘的窗口快照，用于崩溃恢复。
pub fn snapshot_file() -> PathBuf {
    app_dir().join(SNAPSHOT_FILE_NAME)
}

/// 每个窗口在 Stage 模式下的尺寸记忆。
pub fn window_sizes_file() -> PathBuf {
    app_dir().join(WINDOW_SIZES_FILE_NAME)
}

/// 预览区的手动排序。
pub fn strip_order_file() -> PathBuf {
    app_dir().join(STRIP_ORDER_FILE_NAME)
}

/// 日志文件。
pub fn log_file() -> PathBuf {
    app_dir().join(LOG_DIR_NAME).join(LOG_FILE_NAME)
}

fn executable_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_live_under_the_app_directory() {
        let dir = app_dir();
        for file in [
            config_file(),
            snapshot_file(),
            window_sizes_file(),
            strip_order_file(),
            log_file(),
        ] {
            assert!(
                file.starts_with(&dir),
                "{} 应在 {} 之下",
                file.display(),
                dir.display()
            );
        }
    }

    #[test]
    fn log_file_is_inside_logs_subdirectory() {
        assert!(log_file().starts_with(app_dir().join(LOG_DIR_NAME)));
    }
}
