//! 预览区窗口的运行时状态与图标缓存。

use std::collections::HashMap;
use std::sync::mpsc::Sender;

use airpane_core::snapshot::Rect;
use airpane_platform_win::icon::{AppIcon, app_icon};
use airpane_platform_win::theme::ColorScheme;
use windows::Win32::Foundation::RECT;

use super::StripAction;

/// 窗口过程需要访问的状态。
pub(super) struct StripState {
    pub(super) dpi: u32,
    /// 配置里的预览区宽度（DIP）。
    pub(super) width_dip: u32,
    /// 缩略图列表已滚动的距离（物理像素）。
    pub(super) scroll: i32,
    /// 当前悬停项的窗口句柄。用句柄而不是下标，列表重排后依然对得上。
    pub(super) hovered: Option<isize>,
    /// 是否已向系统注册过 `WM_MOUSELEAVE`，避免每次移动都重复注册。
    pub(super) tracking_leave: bool,
    /// 当前系统主题，决定这套配色（D-054）。
    pub(super) scheme: ColorScheme,
    /// 应用图标缓存，同一应用的多个窗口共用。
    pub(super) icons: IconCache,
    /// 当前正在拖拽的项（D-053）。
    pub(super) drag: Option<Drag>,
    pub(super) entries: Vec<Entry>,
    /// 工具区动作的出口。
    pub(super) actions: Sender<StripAction>,
}

/// 拖拽中的项。
#[derive(Debug, Clone, Copy)]
pub(super) struct Drag {
    pub(super) window: isize,
    /// 按下时的客户区 y 坐标，用于判定是否超过阈值。
    pub(super) start_y: i32,
    /// 超过阈值才算拖拽；否则松开时当作点击。
    pub(super) moved: bool,
    /// 插入位置：`0..=entries.len()`。
    pub(super) target: usize,
    /// 被拖的项是不是当前窗口（那种情况不该激活）。
    pub(super) active: bool,
}

pub(super) struct Entry {
    pub(super) window: isize,
    pub(super) pid: u32,
    pub(super) class_name: String,
    /// 当前在舞台区的窗口（D-051）。
    pub(super) active: bool,
    /// 应用图标句柄，生命周期由 `StripState::icons` 持有。
    pub(super) icon: Option<isize>,
    /// DWM 缩略图句柄（`HTHUMBNAIL`，在绑定里表示为 `isize`）。
    pub(super) thumb: isize,
    /// 最近一次已知的源窗口矩形，用于宽高比。
    pub(super) source: Rect,
    /// 缩略图区域（不含图标条）。
    pub(super) rect: RECT,
    /// 缩略图下方的图标条（D-048）。
    pub(super) bar: RECT,
}

/// 应用图标缓存：key 是 `(pid, 类名)`，同一个进程的不同窗口类可能挂不同图标（D-046）。
#[derive(Default)]
pub(super) struct IconCache {
    icons: HashMap<(u32, String), Option<AppIcon>>,
}

impl IconCache {
    /// 取 `(pid, 类名)` 的图标句柄，没缓存过就加载一次。
    ///
    /// 返回的句柄由缓存持有，只在对应缓存条目存在期间有效。
    pub(super) fn handle(&mut self, hwnd: isize, pid: u32, class_name: &str) -> Option<isize> {
        let key = (pid, class_name.to_string());
        self.icons
            .entry(key)
            .or_insert_with(|| app_icon(hwnd, pid))
            .as_ref()
            .map(|icon| icon.handle().0 as isize)
    }

    /// 丢掉当前列表里已经没有的条目，避免长时间运行时缓存无界增长。
    pub(super) fn retain_windows(&mut self, entries: &[Entry]) {
        self.icons.retain(|(pid, class_name), _| {
            entries
                .iter()
                .any(|entry| entry.pid == *pid && &entry.class_name == class_name)
        });
    }
}
