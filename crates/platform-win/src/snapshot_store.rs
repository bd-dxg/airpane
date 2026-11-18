//! 快照文件的读写。
//!
//! 快照是「移动窗口前必须落盘」的落地形式（PRD 11.3 第 3 条、D-005）。写入用
//! 临时文件 + 改名，保证读到的要么是完整快照、要么什么都没有。

use std::fs;
use std::io;
use std::path::Path;

use airpane_core::snapshot::{SNAPSHOT_VERSION, SnapshotFile};

use crate::{log_debug, log_warn};

/// 读取快照。文件不存在、损坏或版本不符时返回 `None`。
pub fn read(path: &Path) -> Option<SnapshotFile> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
        Err(err) => {
            log_warn!("读取快照失败（{}）: {err}", path.display());
            return None;
        }
    };

    match serde_json::from_str::<SnapshotFile>(&text) {
        Ok(snapshot) if snapshot.version == SNAPSHOT_VERSION => Some(snapshot),
        Ok(snapshot) => {
            log_warn!(
                "快照版本不符（文件 v{}，程序 v{}），忽略：{}",
                snapshot.version,
                SNAPSHOT_VERSION,
                path.display()
            );
            None
        }
        Err(err) => {
            log_warn!("快照解析失败，忽略（{}）: {err}", path.display());
            None
        }
    }
}

/// 原子写入快照。
pub fn write(path: &Path, snapshot: &SnapshotFile) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(snapshot).map_err(io::Error::other)?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, text)?;
    // Windows 上 `fs::rename` 使用 MoveFileEx(MOVEFILE_REPLACE_EXISTING)，可直接覆盖。
    fs::rename(&temp, path)
}

/// 删除快照文件，不存在时不算错误。
pub fn clear(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => log_debug!("已清除快照：{}", path.display()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => log_warn!("清除快照失败（{}）: {err}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use airpane_core::snapshot::{Rect, ShowState, WindowSnapshot};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("airpane-snap-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn sample() -> SnapshotFile {
        SnapshotFile::new(
            1_760_000_000,
            vec![WindowSnapshot {
                hwnd: 0x1234,
                process_name: "notepad.exe".to_string(),
                class_name: "Notepad".to_string(),
                rect: Rect {
                    left: 100,
                    top: 50,
                    right: 900,
                    bottom: 650,
                },
                show_state: ShowState::Normal,
            }],
        )
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("snapshot.json");
        let original = sample();

        write(&path, &original).expect("写入快照");
        assert_eq!(read(&path), Some(original));
        assert!(
            !path.with_extension("json.tmp").exists(),
            "临时文件应当被改名"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_reads_as_none() {
        let dir = temp_dir("missing");
        assert_eq!(read(&dir.join("snapshot.json")), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_mismatch_is_ignored() {
        let dir = temp_dir("version");
        let path = dir.join("snapshot.json");
        fs::create_dir_all(&dir).expect("创建测试目录");
        let mut file = sample();
        file.version = SNAPSHOT_VERSION + 1;
        fs::write(&path, serde_json::to_string(&file).expect("序列化")).expect("写入");

        assert_eq!(read(&path), None, "版本不符应当忽略");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_file_is_ignored() {
        let dir = temp_dir("broken");
        let path = dir.join("snapshot.json");
        fs::create_dir_all(&dir).expect("创建测试目录");
        fs::write(&path, "{ 不是 JSON").expect("写入损坏文件");

        assert_eq!(read(&path), None);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_removes_file_and_tolerates_missing() {
        let dir = temp_dir("clear");
        let path = dir.join("snapshot.json");
        write(&path, &sample()).expect("写入快照");
        assert!(path.exists());

        clear(&path);
        assert!(!path.exists());
        clear(&path);

        let _ = fs::remove_dir_all(&dir);
    }
}
