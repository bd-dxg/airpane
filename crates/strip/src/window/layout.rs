//! 预览区布局：缩略图与信息条矩形、滚动、DPI 换算与命中测试。

use airpane_core::config::effective_strip_width;
use airpane_platform_win::monitor::primary_work_area;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION, DWM_TNP_VISIBLE,
    DwmUpdateThumbnailProperties,
};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos,
};
use windows::core::BOOL;

use super::DEFAULT_WIDTH_DIP;
use super::state::{Entry, StripState};

/// 底部工具区的高度（DIP）。
pub(super) const TOOLBAR_HEIGHT_DIP: i32 = 40;
/// 列表左右与顶部内边距。
pub(super) const PADDING_DIP: i32 = 8;
/// 相邻两项之间的间距。
pub(super) const GAP_DIP: i32 = 6;
/// 缩略图下方图标条的高度（D-048）：不显示标题，只留一个大图标。
pub(super) const ICON_BAND_DIP: i32 = 40;
/// 图标条里应用图标的边长。32 DIP 刚好是 `ICON_BIG` / 大文件图标的原始尺寸。
pub(super) const ICON_DIP: i32 = 32;
/// 图标距图标条左边缘的距离，与缩略图左边缘对齐。
pub(super) const ICON_INSET_DIP: i32 = 8;
/// 缩略图区的最小高度。
const MIN_ITEM_DIP: i32 = 56;
/// 缩略图区的最大高度。
const MAX_ITEM_DIP: i32 = 160;
/// 源窗口尺寸不可用时使用的宽高比（16:9 的倒数）。
const FALLBACK_ASPECT: f64 = 9.0 / 16.0;

/// 重新计算窗口位置、DPI 与缩略图布局。
///
/// 安全前提：`state` 要么为空，要么指向存活的 `StripState`。
pub(super) unsafe fn relayout_window(hwnd: HWND, state: *mut StripState) {
    let work = work_area();
    // SAFETY: 只查询本窗口所在显示器的 DPI。
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let dpi = if dpi == 0 { 96 } else { dpi };
    let width_dip = if state.is_null() {
        DEFAULT_WIDTH_DIP
    } else {
        // SAFETY: 见函数文档，`state` 只在窗口存续期间使用。
        unsafe { (*state).width_dip }
    };
    let width = physical_width(width_dip, dpi, work.right - work.left);

    // SAFETY: 只移动本程序自己的窗口，并保持置顶与不抢焦点。
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            work.left,
            work.top,
            width,
            work.bottom - work.top,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(Some(hwnd), None, true);
    }

    if state.is_null() {
        return;
    }
    // SAFETY: 见函数文档。
    unsafe {
        (*state).dpi = dpi;
        apply_layout(hwnd, state);
    }
}

/// 按当前 DPI、客户区大小与滚动位置重新排列缩略图，并推送到 DWM。
///
/// 安全前提：`state` 要么为空，要么指向存活的 `StripState`。
pub(super) unsafe fn apply_layout(hwnd: HWND, state: *mut StripState) {
    if state.is_null() {
        return;
    }
    // SAFETY: 见函数文档。
    unsafe {
        let dpi = (*state).dpi;
        let width = client_width(hwnd);
        let available = client_height(hwnd) - toolbar_height(dpi);
        layout_entries(
            dpi,
            width,
            available,
            &mut (*state).scroll,
            &mut (*state).entries,
        );
        apply_thumbnails(&(*state).entries);
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

/// 按顺序纵向排列缩略图，每项是「缩略图 + 图标条」，高度按源窗口宽高比计算。
///
/// `scroll` 会被钳制在 `0..=(内容总高 - 可用高)` 内。超出工具区的项仍然会写进
/// `rect` 与 `bar`，但会被窗口客户区裁掉。悬停不改变尺寸，只改绘制（D-049）。
fn layout_entries(dpi: u32, width: i32, available: i32, scroll: &mut i32, entries: &mut [Entry]) {
    let padding = scaled(PADDING_DIP, dpi);
    let gap = scaled(GAP_DIP, dpi);
    let bar_height = scaled(ICON_BAND_DIP, dpi);
    let inner = (width - padding * 2).max(1);
    let min_height = scaled(MIN_ITEM_DIP, dpi);
    let max_height = scaled(MAX_ITEM_DIP, dpi);

    let mut heights = Vec::with_capacity(entries.len());
    let mut total = padding;
    for entry in entries.iter() {
        let aspect = if entry.source.width() > 0 {
            entry.source.height() as f64 / entry.source.width() as f64
        } else {
            FALLBACK_ASPECT
        };
        let height = ((inner as f64 * aspect) as i32).clamp(min_height, max_height);
        heights.push(height + bar_height);
        total += height + bar_height + gap;
    }

    let max_scroll = (total - available).max(0);
    *scroll = (*scroll).clamp(0, max_scroll);

    let mut top = padding - *scroll;
    for (entry, height) in entries.iter_mut().zip(heights) {
        let thumb_bottom = top + height - bar_height;
        entry.rect = RECT {
            left: padding,
            top,
            right: padding + inner,
            bottom: thumb_bottom,
        };
        entry.bar = RECT {
            left: padding,
            top: thumb_bottom,
            right: padding + inner,
            bottom: top + height,
        };
        top += height + gap;
    }
}

fn apply_thumbnails(entries: &[Entry]) {
    for entry in entries {
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY,
            rcDestination: entry.rect,
            opacity: 255,
            fVisible: BOOL::from(true),
            ..Default::default()
        };
        // SAFETY: 缩略图句柄由本窗口持有，属性结构在栈上有效。
        unsafe {
            let _ = DwmUpdateThumbnailProperties(entry.thumb, &props);
        }
    }
}

/// 命中测试：缩略图与图标条都算在项内。
pub(super) fn hit_test(entries: &[Entry], x: i32, y: i32) -> Option<&Entry> {
    entries
        .iter()
        .find(|entry| contains(&entry.rect, x, y) || contains(&entry.bar, x, y))
}

/// 工具区高度（物理像素）。
pub(super) fn toolbar_height(dpi: u32) -> i32 {
    scaled(TOOLBAR_HEIGHT_DIP, dpi)
}

/// 工具区本身占用的矩形，贴客户区底部。
pub(super) fn toolbar_rect(dpi: u32, width: i32, height: i32) -> RECT {
    let bar = toolbar_height(dpi);
    RECT {
        left: 0,
        top: height - bar,
        right: width,
        bottom: height,
    }
}

/// 工具区里两个按钮的矩形。
pub(super) fn toolbar_buttons(dpi: u32, width: i32, height: i32) -> [RECT; 2] {
    let bar = toolbar_rect(dpi, width, height);
    let padding = scaled(PADDING_DIP, dpi);
    let gap = scaled(GAP_DIP, dpi);
    let inner = (width - padding * 2 - gap).max(2) / 2;
    let top = bar.top + scaled(6, dpi);
    let bottom = bar.bottom - scaled(6, dpi);
    [
        RECT {
            left: padding,
            top,
            right: padding + inner,
            bottom,
        },
        RECT {
            left: padding + inner + gap,
            top,
            right: padding + inner + gap + inner,
            bottom,
        },
    ]
}

pub(super) fn contains(rect: &RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

pub(super) fn scaled(value: i32, dpi: u32) -> i32 {
    value * dpi as i32 / 96
}

/// 把配置里的 DIP 宽度换算成物理像素，并应用 D-012 的钳制规则。
pub(super) fn physical_width(width_dip: u32, dpi: u32, available_px: i32) -> i32 {
    let dpi = dpi.max(1) as i32;
    let available_dip = available_px * 96 / dpi;
    let effective = effective_strip_width(width_dip, available_dip);
    effective as i32 * dpi / 96
}

pub(super) fn client_width(hwnd: HWND) -> i32 {
    let mut rect = RECT::default();
    // SAFETY: 输出参数是本栈帧上的 RECT。
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    rect.right - rect.left
}

pub(super) fn client_height(hwnd: HWND) -> i32 {
    let mut rect = RECT::default();
    // SAFETY: 输出参数是本栈帧上的 RECT。
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    rect.bottom - rect.top
}

/// 主显示器工作区（扣除任务栏）。
pub(super) fn work_area() -> RECT {
    let area = primary_work_area();
    RECT {
        left: area.left,
        top: area.top,
        right: area.right,
        bottom: area.bottom,
    }
}
