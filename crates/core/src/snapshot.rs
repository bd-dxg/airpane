//! 窗口快照。
//!
//! 依据 `docs/DECISIONS.md` 的 D-005：任何移动或缩放他人窗口的操作之前，必须先把
//! 该窗口的原始位置与状态写入磁盘，用于崩溃后恢复。本模块只提供数据结构、
//! 序列化与配对算法；写盘与恢复动作在 `airpane-platform-win` 与主程序里完成。

use serde::{Deserialize, Serialize};

/// 快照文件的格式版本。结构发生不兼容变化时递增。
///
/// v2 相比 v1 增加了 `SnapshotFile::clean_exit` 与 `WindowSnapshot::class_name`。
pub const SNAPSHOT_VERSION: u32 = 2;

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
    /// 原始 Win32 句柄。同一次会话内可精确配对，跨进程重启后失效。
    pub hwnd: i64,
    /// 进程可执行文件名，用于崩溃重启后按进程重新匹配窗口。
    pub process_name: String,
    /// 窗口类名，与进程名一起构成跨重启匹配的键。
    pub class_name: String,
    pub rect: Rect,
    pub show_state: ShowState,
}

/// 一次进入 Stage 模式时落盘的完整快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotFile {
    pub version: u32,
    /// 正常退出时置为 `true`。启动时读到 `false` 说明上次没走完还原流程。
    pub clean_exit: bool,
    /// Unix 时间戳（秒）。
    pub captured_at: u64,
    pub windows: Vec<WindowSnapshot>,
}

impl SnapshotFile {
    pub fn new(captured_at: u64, windows: Vec<WindowSnapshot>) -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            clean_exit: false,
            captured_at,
            windows,
        }
    }
}

/// 一个当前存在的、可参与配对的窗口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetWindow {
    pub hwnd: i64,
    pub process_name: String,
    pub class_name: String,
    pub rect: Rect,
}

/// 把快照条目与现存窗口配对，返回 `(快照下标, 窗口句柄)`，按快照下标升序。
///
/// 两轮匹配：
/// 1. hwnd 与进程名都相同：同一次会话内的恢复，最可靠。
/// 2. 剩余条目按 `(process_name, class_name)` 分组，组内按矩形左上角距离最近一对一
///    配对。进程名与类名比较不区分大小写。
///
/// 配不上的条目直接丢弃：窗口可能已经关闭，这不该阻止其他窗口恢复。
pub fn match_targets(snapshots: &[WindowSnapshot], targets: &[TargetWindow]) -> Vec<(usize, i64)> {
    let mut used = vec![false; targets.len()];
    let mut matched = vec![false; snapshots.len()];
    let mut result = Vec::new();

    for (index, snapshot) in snapshots.iter().enumerate() {
        let found = targets
            .iter()
            .enumerate()
            .find(|(target_index, target)| {
                !used[*target_index]
                    && target.hwnd == snapshot.hwnd
                    && target
                        .process_name
                        .eq_ignore_ascii_case(&snapshot.process_name)
            })
            .map(|(target_index, _)| target_index);
        if let Some(target_index) = found {
            used[target_index] = true;
            matched[index] = true;
            result.push((index, targets[target_index].hwnd));
        }
    }

    for (index, snapshot) in snapshots.iter().enumerate() {
        if matched[index] {
            continue;
        }
        let found = targets
            .iter()
            .enumerate()
            .filter(|(target_index, target)| {
                !used[*target_index]
                    && target
                        .process_name
                        .eq_ignore_ascii_case(&snapshot.process_name)
                    && target.class_name.eq_ignore_ascii_case(&snapshot.class_name)
            })
            .min_by_key(|(_, target)| rect_distance(snapshot.rect, target.rect))
            .map(|(target_index, _)| target_index);
        if let Some(target_index) = found {
            used[target_index] = true;
            result.push((index, targets[target_index].hwnd));
        }
    }

    result.sort_by_key(|(index, _)| *index);
    result
}

/// 两个矩形左上角之间的平方距离，用于"哪个窗口最像这个快照"。
fn rect_distance(a: Rect, b: Rect) -> i64 {
    let dx = i64::from(a.left - b.left);
    let dy = i64::from(a.top - b.top);
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32) -> Rect {
        Rect {
            left,
            top,
            right: left + 800,
            bottom: top + 600,
        }
    }

    fn snapshot(hwnd: i64, process: &str, class: &str, left: i32) -> WindowSnapshot {
        WindowSnapshot {
            hwnd,
            process_name: process.to_string(),
            class_name: class.to_string(),
            rect: rect(left, left),
            show_state: ShowState::Normal,
        }
    }

    fn target(hwnd: i64, process: &str, class: &str, left: i32) -> TargetWindow {
        TargetWindow {
            hwnd,
            process_name: process.to_string(),
            class_name: class.to_string(),
            rect: rect(left, left),
        }
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
        assert!(!restored.clean_exit, "新建快照默认未正常退出");
    }

    #[test]
    fn same_hwnd_is_matched_first() {
        let snapshots = [snapshot(0x10, "notepad.exe", "Notepad", 0)];
        let targets = [
            target(0x20, "notepad.exe", "Notepad", 500),
            target(0x10, "notepad.exe", "Notepad", 900),
        ];
        assert_eq!(match_targets(&snapshots, &targets), vec![(0, 0x10)]);
    }

    #[test]
    fn falls_back_to_process_and_class_then_nearest_rect() {
        // hwnd 全变了（进程重启），两个同进程同类的窗口按位置就近配对。
        let snapshots = [
            snapshot(0x10, "Code.exe", "Chrome_WidgetWin_1", 0),
            snapshot(0x11, "Code.exe", "Chrome_WidgetWin_1", 1000),
        ];
        let targets = [
            target(0x21, "Code.exe", "Chrome_WidgetWin_1", 1010),
            target(0x20, "Code.exe", "Chrome_WidgetWin_1", 10),
        ];

        assert_eq!(
            match_targets(&snapshots, &targets),
            vec![(0, 0x20), (1, 0x21)]
        );
    }

    #[test]
    fn matching_is_case_insensitive() {
        let snapshots = [snapshot(0x10, "NOTEPAD.EXE", "notepad", 0)];
        let targets = [target(0x99, "notepad.exe", "Notepad", 500)];
        assert_eq!(match_targets(&snapshots, &targets), vec![(0, 0x99)]);
    }

    #[test]
    fn each_target_is_consumed_only_once() {
        let snapshots = [
            snapshot(0x10, "a.exe", "A", 0),
            snapshot(0x11, "a.exe", "A", 10),
        ];
        let targets = [target(0x20, "a.exe", "A", 0)];

        let matches = match_targets(&snapshots, &targets);
        assert_eq!(matches, vec![(0, 0x20)], "第二个快照没有可配对的窗口");
    }

    #[test]
    fn unmatched_snapshots_are_dropped() {
        let snapshots = [snapshot(0x10, "gone.exe", "Gone", 0)];
        let targets = [target(0x20, "other.exe", "Other", 0)];
        assert!(match_targets(&snapshots, &targets).is_empty());
    }

    #[test]
    fn different_class_does_not_match_by_process_only() {
        let snapshots = [snapshot(0x10, "a.exe", "Dialog", 0)];
        let targets = [target(0x20, "a.exe", "Main", 0)];
        assert!(
            match_targets(&snapshots, &targets).is_empty(),
            "类名不同不应配对，避免把对话框当成主窗口移动"
        );
    }
}
