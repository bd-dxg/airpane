//! 预览区手动排序文件的读写（`strip-order.json`）。

use std::fs;
use std::io;
use std::path::Path;

use airpane_core::strip_order::{STRIP_ORDER_VERSION, StripOrderFile};

use crate::{json_io, log_warn};

/// 读取手动排序。文件不存在时返回 `None`，损坏或版本不符时警告并返回 `None`。
pub fn read(path: &Path) -> Option<StripOrderFile> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
        Err(err) => {
            log_warn!("读取预览区顺序失败（{}）: {err}", path.display());
            return None;
        }
    };

    match serde_json::from_str::<StripOrderFile>(&text) {
        Ok(file) if file.version == STRIP_ORDER_VERSION => Some(file),
        Ok(file) => {
            log_warn!(
                "预览区顺序版本不符（文件 v{}，程序 v{}），忽略：{}",
                file.version,
                STRIP_ORDER_VERSION,
                path.display()
            );
            None
        }
        Err(err) => {
            log_warn!("预览区顺序解析失败，忽略（{}）: {err}", path.display());
            None
        }
    }
}

/// 原子写入手动排序。
pub fn write(path: &Path, file: &StripOrderFile) -> io::Result<()> {
    json_io::write_atomic(path, file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use airpane_core::strip_order::StripKey;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("airpane-order-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn sample() -> StripOrderFile {
        StripOrderFile {
            version: STRIP_ORDER_VERSION,
            keys: vec![
                StripKey {
                    process_name: "code.exe".to_string(),
                    class_name: "chrome_widgetwin_1".to_string(),
                },
                StripKey {
                    process_name: "msedge.exe".to_string(),
                    class_name: "chrome_widgetwin_1".to_string(),
                },
            ],
        }
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("strip-order.json");
        write(&path, &sample()).expect("写入");

        assert_eq!(read(&path), Some(sample()));
        assert!(!path.with_extension("json.tmp").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_reads_as_none() {
        let dir = temp_dir("missing");
        assert_eq!(read(&dir.join("strip-order.json")), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_file_is_ignored() {
        let dir = temp_dir("broken");
        let path = dir.join("strip-order.json");
        fs::create_dir_all(&dir).expect("创建目录");
        fs::write(&path, "{ 不是 JSON").expect("写入");

        assert_eq!(read(&path), None);

        let _ = fs::remove_dir_all(&dir);
    }
}
