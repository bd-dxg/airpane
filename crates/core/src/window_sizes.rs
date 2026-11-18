//! 窗口在 Stage 模式下的尺寸记忆（D-025）。
//!
//! 会话内按 hwnd 精确记录，持久化时按 `(process_name, class_name)` 合并——同一应用
//! 的多个窗口在重启后共享一条尺寸记录。只记尺寸，不记位置：位置每次由舞台区几何
//! 重新算，屏幕或预览区宽度变化时窗口不会偏出舞台区。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 尺寸记忆文件的格式版本。
pub const WINDOW_SIZES_VERSION: u32 = 1;

/// 窗口尺寸，物理像素。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub width: i32,
    pub height: i32,
}

/// 一条持久化的尺寸记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeEntry {
    pub process_name: String,
    pub class_name: String,
    pub size: Size,
}

/// `window-sizes.json` 的根结构。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowSizesFile {
    pub version: u32,
    pub entries: Vec<SizeEntry>,
}

impl Default for WindowSizesFile {
    fn default() -> Self {
        Self {
            version: WINDOW_SIZES_VERSION,
            entries: Vec::new(),
        }
    }
}

/// 尺寸记忆表。
#[derive(Debug, Clone, Default)]
pub struct WindowSizeStore {
    /// 本次运行内按句柄记录，能区分同一应用的多个窗口。
    session: HashMap<i64, Size>,
    /// 跨运行保留的记录，按 `(process_name, class_name)` 合并。
    entries: Vec<SizeEntry>,
    dirty: bool,
}

impl WindowSizeStore {
    pub fn from_file(file: WindowSizesFile) -> Self {
        Self {
            session: HashMap::new(),
            entries: file.entries,
            dirty: false,
        }
    }

    /// 导出为可持久化的结构。
    pub fn to_file(&self) -> WindowSizesFile {
        WindowSizesFile {
            version: WINDOW_SIZES_VERSION,
            entries: self.entries.clone(),
        }
    }

    /// 查询某个窗口该用多大的尺寸。
    pub fn size_for(&self, hwnd: i64, process_name: &str, class_name: &str) -> Option<Size> {
        self.session
            .get(&hwnd)
            .copied()
            .or_else(|| self.entry_size(process_name, class_name))
    }

    /// 记录窗口的当前尺寸。值没有变化时不产生写盘需求。
    pub fn record(&mut self, hwnd: i64, process_name: &str, class_name: &str, size: Size) {
        if size.width <= 0 || size.height <= 0 {
            return;
        }
        if self.session.get(&hwnd) != Some(&size) {
            self.session.insert(hwnd, size);
            self.dirty = true;
        }
        match self
            .entries
            .iter_mut()
            .find(|entry| same_window(entry, process_name, class_name))
        {
            Some(entry) if entry.size != size => {
                entry.size = size;
                self.dirty = true;
            }
            Some(_) => {}
            None => {
                self.entries.push(SizeEntry {
                    process_name: process_name.to_string(),
                    class_name: class_name.to_string(),
                    size,
                });
                self.dirty = true;
            }
        }
    }

    /// 忘记某个已消失的窗口的会话内记录。
    pub fn forget(&mut self, hwnd: i64) {
        self.session.remove(&hwnd);
    }

    /// 取出并清除「需要写盘」标记。
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// 记录的条数，用于日志与测试。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn entry_size(&self, process_name: &str, class_name: &str) -> Option<Size> {
        self.entries
            .iter()
            .find(|entry| same_window(entry, process_name, class_name))
            .map(|entry| entry.size)
    }
}

fn same_window(entry: &SizeEntry, process_name: &str, class_name: &str) -> bool {
    entry.process_name.eq_ignore_ascii_case(process_name)
        && entry.class_name.eq_ignore_ascii_case(class_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: i32, height: i32) -> Size {
        Size { width, height }
    }

    #[test]
    fn session_record_is_preferred_over_persisted_entry() {
        let mut store = WindowSizeStore::default();
        store.record(1, "code.exe", "Chrome_WidgetWin_1", size(1000, 700));
        store.record(2, "code.exe", "Chrome_WidgetWin_1", size(400, 300));

        assert_eq!(
            store.size_for(2, "code.exe", "Chrome_WidgetWin_1"),
            Some(size(400, 300)),
            "第二窗口用会话内记录，不串到第一条"
        );
        assert_eq!(
            store.size_for(99, "code.exe", "Chrome_WidgetWin_1"),
            Some(size(400, 300)),
            "没有句柄记录时用进程 + 类名回退"
        );
    }

    #[test]
    fn unknown_window_has_no_size() {
        let store = WindowSizeStore::default();
        assert_eq!(store.size_for(1, "notepad.exe", "Notepad"), None);
    }

    #[test]
    fn recording_the_same_size_twice_is_not_dirty() {
        let mut store = WindowSizeStore::default();
        store.record(1, "notepad.exe", "Notepad", size(800, 600));
        assert!(store.take_dirty(), "首次记录应当需要写盘");

        store.record(1, "notepad.exe", "Notepad", size(800, 600));
        assert!(!store.take_dirty(), "重复记录同一尺寸不应写盘");
    }

    #[test]
    fn resizing_marks_dirty_and_updates_both_keys() {
        let mut store = WindowSizeStore::default();
        store.record(1, "notepad.exe", "Notepad", size(800, 600));
        store.take_dirty();

        store.record(1, "notepad.exe", "Notepad", size(900, 650));
        assert!(store.take_dirty());
        assert_eq!(
            store.size_for(1, "notepad.exe", "Notepad"),
            Some(size(900, 650))
        );
        assert_eq!(store.len(), 1, "同一进程 + 类名只有一条持久化记录");
    }

    #[test]
    fn non_positive_size_is_ignored() {
        let mut store = WindowSizeStore::default();
        store.record(1, "notepad.exe", "Notepad", size(0, 600));
        assert!(!store.take_dirty());
        assert!(store.is_empty());
    }

    #[test]
    fn file_round_trip_keeps_entries() {
        let mut store = WindowSizeStore::default();
        store.record(1, "a.exe", "A", size(700, 500));
        let file = store.to_file();

        let json = serde_json::to_string(&file).expect("序列化");
        let restored: WindowSizesFile = serde_json::from_str(&json).expect("反序列化");
        assert_eq!(restored.version, WINDOW_SIZES_VERSION);
        assert_eq!(restored.entries.len(), 1);
        assert_eq!(restored.entries[0].size, size(700, 500));

        let mut store = WindowSizeStore::from_file(restored);
        assert!(!store.take_dirty(), "刚加载不应需要写盘");
        assert_eq!(store.size_for(42, "a.exe", "A"), Some(size(700, 500)));
    }

    #[test]
    fn matching_ignores_case() {
        let mut store = WindowSizeStore::default();
        store.record(1, "Notepad.EXE", "Notepad", size(800, 600));
        assert_eq!(
            store.size_for(2, "notepad.exe", "notepad"),
            Some(size(800, 600))
        );
    }

    #[test]
    fn missing_fields_fall_back_to_empty_file() {
        let file: WindowSizesFile = serde_json::from_str("{}").expect("空对象应当可解析");
        assert_eq!(file.version, WINDOW_SIZES_VERSION);
        assert!(file.entries.is_empty());
    }
}
