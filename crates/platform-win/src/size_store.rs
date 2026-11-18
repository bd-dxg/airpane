//! 尺寸记忆文件的读写（`window-sizes.json`）。

use std::fs;
use std::io;
use std::path::Path;

use airpane_core::window_sizes::{WINDOW_SIZES_VERSION, WindowSizesFile};

use crate::{json_io, log_warn};

/// 读取尺寸记忆。文件不存在时返回 `None`，损坏或版本不符时警告并返回 `None`。
pub fn read(path: &Path) -> Option<WindowSizesFile> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
        Err(err) => {
            log_warn!("读取尺寸记忆失败（{}）: {err}", path.display());
            return None;
        }
    };

    match serde_json::from_str::<WindowSizesFile>(&text) {
        Ok(file) if file.version == WINDOW_SIZES_VERSION => Some(file),
        Ok(file) => {
            log_warn!(
                "尺寸记忆版本不符（文件 v{}，程序 v{}），忽略：{}",
                file.version,
                WINDOW_SIZES_VERSION,
                path.display()
            );
            None
        }
        Err(err) => {
            log_warn!("尺寸记忆解析失败，忽略（{}）: {err}", path.display());
            None
        }
    }
}

/// 原子写入尺寸记忆。
pub fn write(path: &Path, file: &WindowSizesFile) -> io::Result<()> {
    json_io::write_atomic(path, file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use airpane_core::window_sizes::{Size, SizeEntry};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("airpane-sizes-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn sample() -> WindowSizesFile {
        WindowSizesFile {
            version: WINDOW_SIZES_VERSION,
            entries: vec![SizeEntry {
                process_name: "notepad.exe".to_string(),
                class_name: "Notepad".to_string(),
                size: Size {
                    width: 800,
                    height: 600,
                },
            }],
        }
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("window-sizes.json");
        write(&path, &sample()).expect("写入");

        assert_eq!(read(&path), Some(sample()));
        assert!(!path.with_extension("json.tmp").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_reads_as_none() {
        let dir = temp_dir("missing");
        assert_eq!(read(&dir.join("window-sizes.json")), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_file_is_ignored() {
        let dir = temp_dir("broken");
        let path = dir.join("window-sizes.json");
        fs::create_dir_all(&dir).expect("创建目录");
        fs::write(&path, "{ 不是 JSON").expect("写入");

        assert_eq!(read(&path), None);

        let _ = fs::remove_dir_all(&dir);
    }
}
