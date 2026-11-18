//! 窗口模型：维护被管理窗口的集合与最近使用顺序（MRU）。
//!
//! 索引 0 是最近使用的窗口。本模块不依赖 Win32，事件与枚举结果由上层填进来，
//! 因此顺序逻辑可以单元测试。预览区不使用这个顺序，改用 [`display_key`] 的固定排序
//! （D-047）。

/// 注册表中的一个被管理窗口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedWindow {
    pub hwnd: i64,
    pub pid: u32,
    pub process_name: String,
    pub class_name: String,
    pub title: String,
}

/// 按最近使用顺序维护被管理窗口。
#[derive(Debug, Default, Clone)]
pub struct WindowRegistry {
    windows: Vec<TrackedWindow>,
}

impl WindowRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 插入或更新窗口。`activate` 为真时把它移到最前（新窗口或刚获得焦点）。
    pub fn upsert(&mut self, window: TrackedWindow, activate: bool) {
        match self.position(&window.hwnd) {
            Some(index) => {
                self.windows[index] = window;
                if activate && index > 0 {
                    self.move_to_front(index);
                }
            }
            None if activate => self.windows.insert(0, window),
            None => self.windows.push(window),
        }
    }

    /// 移除窗口，返回被移除的条目。
    pub fn remove(&mut self, hwnd: i64) -> Option<TrackedWindow> {
        let index = self.position(&hwnd)?;
        Some(self.windows.remove(index))
    }

    /// 把已有窗口移到最前，返回它是否在注册表里。
    pub fn activate(&mut self, hwnd: i64) -> bool {
        match self.position(&hwnd) {
            Some(0) => true,
            Some(index) => {
                self.move_to_front(index);
                true
            }
            None => false,
        }
    }

    pub fn contains(&self, hwnd: i64) -> bool {
        self.position(&hwnd).is_some()
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// 按 MRU 顺序遍历，最前的是最近使用的窗口。
    pub fn iter(&self) -> impl Iterator<Item = &TrackedWindow> {
        self.windows.iter()
    }

    pub fn most_recent(&self) -> Option<&TrackedWindow> {
        self.windows.first()
    }

    fn position(&self, hwnd: &i64) -> Option<usize> {
        self.windows.iter().position(|window| window.hwnd == *hwnd)
    }

    fn move_to_front(&mut self, index: usize) {
        let window = self.windows.remove(index);
        self.windows.insert(0, window);
    }
}

/// 预览区列表的固定排序键：进程名 → 类名 → 句柄，前两项忽略大小写。
///
/// 刻意不比较标题：浏览器换标签页、编辑器换文件都会改标题，按标题排会让列表跟着
/// 乱跳（D-047）。同一应用的同类窗口按句柄（即创建顺序）排，位置始终固定。
pub fn display_key(window: &TrackedWindow) -> (String, String, i64) {
    (
        window.process_name.to_lowercase(),
        window.class_name.to_lowercase(),
        window.hwnd,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(hwnd: i64, title: &str) -> TrackedWindow {
        TrackedWindow {
            hwnd,
            pid: 1000 + hwnd as u32,
            process_name: "app.exe".to_string(),
            class_name: "AppWindow".to_string(),
            title: title.to_string(),
        }
    }

    fn hwnds(registry: &WindowRegistry) -> Vec<i64> {
        registry.iter().map(|window| window.hwnd).collect()
    }

    #[test]
    fn new_window_can_be_pushed_back_or_activated() {
        let mut registry = WindowRegistry::new();
        registry.upsert(window(1, "a"), false);
        registry.upsert(window(2, "b"), true);
        assert_eq!(hwnds(&registry), vec![2, 1]);
        assert_eq!(registry.most_recent().map(|w| w.hwnd), Some(2));
    }

    #[test]
    fn activating_moves_window_to_front_and_keeps_others() {
        let mut registry = WindowRegistry::new();
        registry.upsert(window(1, "a"), true);
        registry.upsert(window(2, "b"), true);
        registry.upsert(window(3, "c"), true);
        assert_eq!(hwnds(&registry), vec![3, 2, 1]);

        assert!(registry.activate(1));
        assert_eq!(hwnds(&registry), vec![1, 3, 2]);

        assert!(registry.activate(1));
        assert_eq!(hwnds(&registry), vec![1, 3, 2], "已是最前时不变");
        assert!(!registry.activate(99), "不存在的窗口返回 false");
    }

    #[test]
    fn upsert_updates_fields_without_reordering() {
        let mut registry = WindowRegistry::new();
        registry.upsert(window(1, "a"), true);
        registry.upsert(window(2, "b"), true);
        registry.upsert(window(1, "a 改"), false);

        assert_eq!(hwnds(&registry), vec![2, 1]);
        let updated = registry.iter().find(|w| w.hwnd == 1).expect("窗口仍在");
        assert_eq!(updated.title, "a 改");
    }

    #[test]
    fn remove_returns_entry_only_once() {
        let mut registry = WindowRegistry::new();
        registry.upsert(window(1, "a"), true);

        assert_eq!(registry.remove(1).map(|w| w.hwnd), Some(1));
        assert!(registry.is_empty());
        assert_eq!(registry.remove(1), None);
        assert!(!registry.contains(1));
    }

    fn ordered(mut windows: Vec<TrackedWindow>) -> Vec<i64> {
        windows.sort_by_cached_key(display_key);
        windows.iter().map(|w| w.hwnd).collect()
    }

    #[test]
    fn display_key_sorts_by_process_name_ignoring_case() {
        let mut wechat = window(1, "chat");
        wechat.process_name = "Weixin.exe".to_string();
        let mut edge = window(2, "page");
        edge.process_name = "msedge.exe".to_string();
        let mut code = window(3, "main.rs");
        code.process_name = "Code.exe".to_string();

        assert_eq!(ordered(vec![wechat, edge, code]), vec![3, 2, 1]);
    }

    #[test]
    fn display_key_keeps_same_app_windows_in_creation_order() {
        let mut first = window(30, "tab b");
        first.class_name = "Chrome_WidgetWin_1".to_string();
        let mut second = window(12, "tab a");
        second.class_name = "Chrome_WidgetWin_1".to_string();

        // 标题变化不影响位置，句柄小的（先创建的）在前。
        assert_eq!(ordered(vec![first, second]), vec![12, 30]);
    }

    #[test]
    fn display_key_differs_by_class_name() {
        let mut main = window(5, "main");
        main.class_name = "MainWindow".to_string();
        let mut dialog = window(6, "dlg");
        dialog.class_name = "AuxWindow".to_string();

        assert_eq!(ordered(vec![main, dialog]), vec![6, 5]);
    }
}
