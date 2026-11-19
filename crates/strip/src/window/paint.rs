//! 预览区的 GDI 绘制：背景、底部工具区，以及每一项的图标条。
//!
//! 配色跟随系统浅色/深色主题（D-054）：换主题只换 [`Palette`]，绘制代码不变。

use core::ffi::c_void;

use airpane_platform_win::theme::ColorScheme;
use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DEFAULT_GUI_FONT, DT_CENTER, DT_SINGLELINE, DT_VCENTER,
    DeleteObject, DrawTextW, EndPaint, FillRect, GetStockObject, HBRUSH, PAINTSTRUCT, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DrawIconEx, HICON};

use super::layout::{
    GAP_DIP, ICON_BAND_DIP, ICON_DIP, ICON_INSET_DIP, client_height, client_width, drop_line,
    scaled, toolbar_buttons, toolbar_rect,
};
use super::state::StripState;

/// 活跃项强调条与拖拽指示线的厚度（DIP）。
const ACCENT_DIP: i32 = 3;
/// 工具区两个按钮的文字。M1 硬编码，M3 引入多语言资源表时替换（D-033）。
const TOOLBAR_LABELS: [&str; 2] = ["设置", "桌面"];

/// 一整套配色。`COLORREF` 是 `0x00BBGGRR`，注意蓝红顺序。
pub(super) struct Palette {
    /// 列表背景。
    background: COLORREF,
    /// 图标条背景。
    band: COLORREF,
    /// 悬停项的图标条。
    hover_band: COLORREF,
    /// 当前在舞台区的项。
    active_band: COLORREF,
    /// 底部工具区。
    toolbar: COLORREF,
    /// 工具区文字。
    text: COLORREF,
    /// 活跃强调条与拖拽指示线。
    accent: COLORREF,
}

/// 深色配色（M1 原有的一套）。
const DARK: Palette = Palette {
    background: COLORREF(0x0020_2020),
    band: COLORREF(0x002B_2B2B),
    hover_band: COLORREF(0x0034_3434),
    active_band: COLORREF(0x003C_3C3C),
    toolbar: COLORREF(0x0032_3232),
    text: COLORREF(0x00E0_E0E0),
    accent: COLORREF(0x00FF_A050),
};

/// 浅色配色：中性灰底 + 深色文字，避免在浅色主题下出现一整块黑边。
const LIGHT: Palette = Palette {
    background: COLORREF(0x00F3_F3F3),
    band: COLORREF(0x00E6_E6E6),
    hover_band: COLORREF(0x00DC_DCDC),
    active_band: COLORREF(0x00D2_D2D2),
    toolbar: COLORREF(0x00EA_EAEA),
    text: COLORREF(0x001F_1F1F),
    accent: COLORREF(0x00EB_6F1F),
};

pub(super) fn palette(scheme: ColorScheme) -> &'static Palette {
    match scheme {
        ColorScheme::Light => &LIGHT,
        ColorScheme::Dark => &DARK,
    }
}

/// 绘制背景、工具区，以及每一项的图标条与高亮。
///
/// 安全前提：`state` 要么为空，要么指向存活的 `StripState`。
pub(super) fn paint(hwnd: HWND, state: *mut StripState) {
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: `ps` 是本栈帧上的 PAINTSTRUCT，BeginPaint/EndPaint 必须成对调用；
    // `DrawTextW` 的文本缓冲区在调用期间存活，矩形也是本栈帧上的局部变量。
    unsafe {
        let hdc = BeginPaint(hwnd, &mut ps);
        // SAFETY: 见函数文档，`state` 只在窗口存续期间使用。
        let colors = palette(if state.is_null() {
            ColorScheme::Dark
        } else {
            (*state).scheme
        });
        let brush: HBRUSH = CreateSolidBrush(colors.background);
        FillRect(hdc, &ps.rcPaint, brush);

        // SAFETY: 只查询本窗口 DPI。
        let dpi = GetDpiForWindow(hwnd);
        let dpi = if dpi == 0 { 96 } else { dpi };
        let width = client_width(hwnd);
        let height = client_height(hwnd);
        let toolbar = toolbar_rect(dpi, width, height);
        let toolbar_brush: HBRUSH = CreateSolidBrush(colors.toolbar);
        FillRect(hdc, &toolbar, toolbar_brush);

        let font = GetStockObject(DEFAULT_GUI_FONT);
        let previous = SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, colors.text);

        for (mut rect, label) in toolbar_buttons(dpi, width, height)
            .into_iter()
            .zip(TOOLBAR_LABELS)
        {
            let mut text: Vec<u16> = label.encode_utf16().collect();
            DrawTextW(
                hdc,
                &mut text,
                &mut rect as *mut RECT,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
        }

        // 图标条：不显示标题，只留一个 32 DIP 的应用图标（D-048）。没有图标的
        // 项留空，不画占位图形。活跃项与悬停项只改配色，不改变布局（D-049、D-051）。
        if !state.is_null() {
            let state = &*state;
            let band_brush: HBRUSH = CreateSolidBrush(colors.band);
            let hover_brush: HBRUSH = CreateSolidBrush(colors.hover_band);
            let active_brush: HBRUSH = CreateSolidBrush(colors.active_band);
            let accent_brush: HBRUSH = CreateSolidBrush(colors.accent);
            let icon_size = scaled(ICON_DIP, dpi);
            let icon_inset = scaled(ICON_INSET_DIP, dpi);
            let accent_height = scaled(ACCENT_DIP, dpi);
            let band_height = scaled(ICON_BAND_DIP, dpi);

            for entry in &state.entries {
                let hovered = state.hovered == Some(entry.window);
                let background = if entry.active {
                    active_brush
                } else if hovered {
                    hover_brush
                } else {
                    band_brush
                };
                FillRect(hdc, &entry.bar, background);
                if entry.active {
                    // 当前窗口：图标条上沿一条强调色，切窗口时只换这一条。
                    let accent = RECT {
                        left: entry.bar.left,
                        top: entry.bar.top,
                        right: entry.bar.right,
                        bottom: entry.bar.top + accent_height,
                    };
                    FillRect(hdc, &accent, accent_brush);
                }
                if let Some(icon) = entry.icon {
                    let top = entry.bar.top + (band_height - icon_size) / 2;
                    let _ = DrawIconEx(
                        hdc,
                        entry.bar.left + icon_inset,
                        top,
                        HICON(icon as *mut c_void),
                        icon_size,
                        icon_size,
                        0,
                        None,
                        DI_NORMAL,
                    );
                }
            }

            let _ = DeleteObject(accent_brush.into());
            let _ = DeleteObject(active_brush.into());
            let _ = DeleteObject(hover_brush.into());
            let _ = DeleteObject(band_brush.into());

            // 拖拽指示线：标出松手后的落点（D-053）。
            if let Some(drag) = state.drag {
                if drag.moved {
                    let gap = scaled(GAP_DIP, dpi);
                    if let Some(line) = drop_line(&state.entries, drag.target, gap) {
                        let line_brush: HBRUSH = CreateSolidBrush(colors.accent);
                        FillRect(hdc, &line, line_brush);
                        let _ = DeleteObject(line_brush.into());
                    }
                }
            }
        }

        SelectObject(hdc, previous);
        let _ = DeleteObject(toolbar_brush.into());
        let _ = DeleteObject(brush.into());
        let _ = EndPaint(hwnd, &ps);
    }
}
