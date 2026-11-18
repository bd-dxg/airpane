//! 窗口快照。
//!
//! 依据 `docs/DECISIONS.md` 的 D-005：任何移动或缩放他人窗口的操作之前，必须先把
//! 该窗口的原始位置与状态写入磁盘，用于崩溃后恢复。本模块只提供数据结构与序列化，
//! 写盘与恢复动作在 `airpane` 主程序里完成。

use serde::{Deserialize, Serialize};

/// 快照文件的格式版本。结构发生不兼容变化时递增。
pub const SNAPSHOT_VERSION: u32 = 1;

/// 窗口矩形，单位为物理像素，与 Win32 的 `RECT` 语义一致（右下开区间）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

/// 窗口的显示状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowState {
    Normal,
    Minimized,
    Maximized,
}

/// 单个窗口在被本程序改动之前的原始状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSnapshot {
    /// 原始 Win32 句柄，仅用于本次会话内的比对；跨进程重启后不再有效。
    pub hwnd: i64,
    /// 进程可执行文件名，用于崩溃重启后按进程重新匹配窗口。
    pub process_name: String,
    pub rect: Rect,
    pub show_state: ShowState,
}

/// 一次进入 Stage 模式时落盘的完整快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotFile {
    pub version: u32,
    /// Unix 时间戳（秒）。
    pub captured_at: u64,
    pub windows: Vec<WindowSnapshot>,
}

impl SnapshotFile {
    pub fn new(captured_at: u64, windows: Vec<WindowSnapshot>) -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            captured_at,
            windows,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SnapshotFile {
        SnapshotFile::new(
            1_760_000_000,
            vec![WindowSnapshot {
                hwnd: 0x1234,
                process_name: "notepad.exe".to_string(),
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
    fn rect_reports_size() {
        let rect = sample().windows[0].rect;
        assert_eq!(rect.width(), 800);
        assert_eq!(rect.height(), 600);
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let original = sample();
        let json = serde_json::to_string(&original).expect("序列化应当成功");
        let restored: SnapshotFile = serde_json::from_str(&json).expect("反序列化应当成功");
        assert_eq!(restored, original);
        assert_eq!(restored.version, SNAPSHOT_VERSION);
    }
}
