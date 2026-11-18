//! 预览区的手动排序表（`strip-order.json`，D-053）。
//!
//! 用户拖动缩略图重新排序，顺序按「进程名 + 类名」记录，跨重启生效。没记录过的窗口
//! 排在已记录项之后，组内仍按 D-047 的固定顺序。纯逻辑，不碰文件。

use serde::{Deserialize, Serialize};

use crate::registry::TrackedWindow;

/// 排序文件格式版本。
pub const STRIP_ORDER_VERSION: u32 = 1;

/// 排序单位：进程名 + 类名。
///
/// 同一个应用的同类窗口视为一组一起移动 —— 句柄跨重启会变，标题又经常变，只有这两个
/// 字段既能稳定标识又不会天天变。两份都转小写：Windows 的进程名与类名不区分大小写。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StripKey {
    pub process_name: String,
    pub class_name: String,
}

impl StripKey {
    /// 窗口对应的排序单位。
    pub fn of(window: &TrackedWindow) -> Self {
        Self {
            process_name: window.process_name.to_lowercase(),
            class_name: window.class_name.to_lowercase(),
        }
    }
}

/// `strip-order.json` 的内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StripOrderFile {
    pub version: u32,
    /// 手动拖出来的顺序，索引即显示顺序。
    pub keys: Vec<StripKey>,
}

impl Default for StripOrderFile {
    fn default() -> Self {
        Self {
            version: STRIP_ORDER_VERSION,
            keys: Vec::new(),
        }
    }
}

impl StripOrderFile {
    /// 排序键：先按手动顺序的排名，没记录的排最后；同排名再按进程名、类名、句柄。
    pub fn sort_key(&self, window: &TrackedWindow) -> (usize, String, String, i64) {
        let key = StripKey::of(window);
        let rank = self.position(&key).unwrap_or(usize::MAX);
        (rank, key.process_name, key.class_name, window.hwnd)
    }

    /// 把 `key` 在 `display`（当前显示顺序）里移到 `before` 之前，记为新的手动顺序。
    ///
    /// `before` 为 `None` 表示移到末尾。第一次拖动时会把当前显示顺序整体落表，这样
    /// 只有被拖的那一项换位置，其余都留在原地。什么都没变时不写盘，返回 false。
    pub fn apply_drag(
        &mut self,
        display: &[StripKey],
        key: &StripKey,
        before: Option<&StripKey>,
    ) -> bool {
        // 同一应用的同类窗口共享一个 key：落点只能是它自己，拖动不成任何变化。
        if before == Some(key) {
            return false;
        }
        let baseline = self.merged(display);
        let mut next = baseline.clone();
        next.retain(|existing| existing != key);
        let index = match before {
            Some(before) if before != key => next.iter().position(|existing| existing == before),
            _ => None,
        };
        let index = index.unwrap_or(next.len());
        next.insert(index, key.clone());
        if next == baseline {
            return false;
        }
        self.keys = next;
        true
    }

    /// 手动顺序 + 当前显示里还没记过的 key（按显示顺序追加）。
    fn merged(&self, display: &[StripKey]) -> Vec<StripKey> {
        let mut result = self.keys.clone();
        for key in display {
            if !result.contains(key) {
                result.push(key.clone());
            }
        }
        result
    }

    fn position(&self, key: &StripKey) -> Option<usize> {
        self.keys.iter().position(|existing| existing == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(process: &str, class: &str, hwnd: i64) -> TrackedWindow {
        TrackedWindow {
            hwnd,
            pid: 1,
            process_name: process.to_string(),
            class_name: class.to_string(),
            title: String::new(),
        }
    }

    fn order_of(order: &StripOrderFile, windows: &[TrackedWindow]) -> Vec<i64> {
        let mut windows: Vec<&TrackedWindow> = windows.iter().collect();
        windows.sort_by_cached_key(|window| order.sort_key(window));
        windows.iter().map(|window| window.hwnd).collect()
    }

    fn sample_windows() -> Vec<TrackedWindow> {
        vec![
            window("msedge.exe", "Chrome_WidgetWin_1", 10),
            window("Code.exe", "Chrome_WidgetWin_1", 20),
            window("explorer.exe", "CabinetWClass", 30),
        ]
    }

    #[test]
    fn empty_order_falls_back_to_alphabetical() {
        let order = StripOrderFile::default();
        // Code.exe < explorer.exe < msedge.exe
        assert_eq!(order_of(&order, &sample_windows()), vec![20, 30, 10]);
    }

    #[test]
    fn dragged_key_moves_to_the_requested_slot() {
        let mut order = StripOrderFile::default();
        let windows = sample_windows();
        let display = vec![
            StripKey::of(&windows[1]), // Code
            StripKey::of(&windows[2]), // explorer
            StripKey::of(&windows[0]), // msedge
        ];

        // 把 msedge 拖到最前面。
        assert!(order.apply_drag(&display, &StripKey::of(&windows[0]), Some(&display[0])));
        assert_eq!(order_of(&order, &windows), vec![10, 20, 30]);
        assert_eq!(order.keys.len(), 3, "第一次拖动会把完整顺序落表");
    }

    #[test]
    fn dragging_to_the_end_uses_none() {
        let mut order = StripOrderFile::default();
        let windows = sample_windows();
        let display: Vec<StripKey> = vec![
            StripKey::of(&windows[1]),
            StripKey::of(&windows[2]),
            StripKey::of(&windows[0]),
        ];

        assert!(order.apply_drag(&display, &StripKey::of(&windows[1]), None));
        assert_eq!(order_of(&order, &windows), vec![30, 10, 20]);
    }

    #[test]
    fn dragging_onto_itself_changes_nothing() {
        let mut order = StripOrderFile::default();
        let windows = sample_windows();
        let display = vec![StripKey::of(&windows[0])];

        assert!(!order.apply_drag(&display, &StripKey::of(&windows[0]), Some(&display[0])));
        assert!(order.keys.is_empty());
    }

    #[test]
    fn dragging_before_its_own_group_is_a_no_op() {
        let mut order = StripOrderFile::default();
        let first = window("msedge.exe", "Chrome_WidgetWin_1", 10);
        let second = window("msedge.exe", "Chrome_WidgetWin_1", 11);
        let display = vec![StripKey::of(&first)];

        // 同类窗口共享同一个 key，落点只能是它自己，不应该被当成「移到末尾」。
        assert!(!order.apply_drag(
            &display,
            &StripKey::of(&first),
            Some(&StripKey::of(&second))
        ));
        assert!(order.keys.is_empty());
    }

    #[test]
    fn unknown_windows_go_after_known_ones() {
        let windows = sample_windows();
        let order = StripOrderFile {
            keys: vec![StripKey::of(&windows[0])], // msedge 置顶
            ..Default::default()
        };

        assert_eq!(order_of(&order, &windows), vec![10, 20, 30]);
    }

    #[test]
    fn keys_ignore_case() {
        let upper = window("MSEDGE.EXE", "CHROME_WIDGETWIN_1", 40);
        let lower = window("msedge.exe", "Chrome_WidgetWin_1", 41);
        assert_eq!(StripKey::of(&upper), StripKey::of(&lower));

        // 用大写形式记录，小写窗口同样能命中。
        let order = StripOrderFile {
            keys: vec![StripKey::of(&upper)],
            ..Default::default()
        };
        let windows = vec![lower, window("Code.exe", "Chrome_WidgetWin_1", 20)];
        assert_eq!(order_of(&order, &windows), vec![41, 20]);
    }

    #[test]
    fn moving_a_group_moves_all_its_windows_together() {
        let windows = vec![
            window("msedge.exe", "Chrome_WidgetWin_1", 10),
            window("msedge.exe", "Chrome_WidgetWin_1", 11),
            window("Code.exe", "Chrome_WidgetWin_1", 20),
        ];
        let order = StripOrderFile {
            keys: vec![StripKey::of(&windows[0])],
            ..Default::default()
        };

        assert_eq!(order_of(&order, &windows), vec![10, 11, 20]);
    }
}
