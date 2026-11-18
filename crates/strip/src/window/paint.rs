//! 预览区的 GDI 绘制：背景、底部工具区，以及每一项的信息条（图标 + 标题）。

use core::ffi::c_void;

use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DEFAULT_GUI_FONT, DT_CENTER, DT_SINGLELINE, DT_VCENTER,
    DeleteObject, DrawTextW, EndPaint, FillRect, GetStockObject, HBRUSH, PAINTSTRUCT, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DrawIconEx, HICON};

use super::layout::{
    ICON_BAND_DIP, ICON_DIP, ICON_INSET_DIP, client_height, client_width, scaled, toolbar_buttons,
    toolbar_rect,
};
use super::state::StripState;

/// 预览区背景色（深灰，M0 阶段的降级视觉）。
const BACKGROUND: COLORREF = COLORREF(0x0020_2020);
/// 底部工具区背景色，比列表区域略亮。
const TOOLBAR_BACKGROUND: COLORREF = COLORREF(0x0032_3232);
/// 缩略图下方图标条的背景色，在列表背景上留出一条能分辨的分隔。
const ICON_BAND_BACKGROUND: COLORREF = COLORREF(0x002B_2B2B);
/// 悬停项的图标条背景，比普通项略亮。
const HOVER_BAND_BACKGROUND: COLORREF = COLORREF(0x0038_3838);
/// 悬停高亮的强调色（浅蓝），画在图标条上沿。
const ACCENT: COLORREF = COLORREF(0x00FF_A050);
/// 悬停高亮条的厚度（DIP，D-049）。
const ACCENT_DIP: i32 = 3;
/// 工具区与图标的文字颜色。
const TEXT_COLOR: COLORREF = COLORREF(0x00E0_E0E0);
/// 工具区两个按钮的文字。M1 硬编码，M3 引入多语言资源表时替换（D-033）。
const TOOLBAR_LABELS: [&str; 2] = ["设置", "桌面"];

/// 绘制背景、工具区，以及每一项的图标条与悬停高亮。
///
/// 安全前提：`state` 要么为空，要么指向存活的 `StripState`。
pub(super) fn paint(hwnd: HWND, state: *mut StripState) {
    let mut ps = PAINTSTRUCT::default();
    // SAFETY: `ps` 是本栈帧上的 PAINTSTRUCT，BeginPaint/EndPaint 必须成对调用；
    // `DrawTextW` 的文本缓冲区在调用期间存活，矩形也是本栈帧上的局部变量。
    unsafe {
        let hdc = BeginPaint(hwnd, &mut ps);
        let brush: HBRUSH = CreateSolidBrush(BACKGROUND);
        FillRect(hdc, &ps.rcPaint, brush);

        // SAFETY: 只查询本窗口 DPI。
        let dpi = GetDpiForWindow(hwnd);
        let dpi = if dpi == 0 { 96 } else { dpi };
        let width = client_width(hwnd);
        let height = client_height(hwnd);
        let toolbar = toolbar_rect(dpi, width, height);
        let toolbar_brush: HBRUSH = CreateSolidBrush(TOOLBAR_BACKGROUND);
        FillRect(hdc, &toolbar, toolbar_brush);

        let font = GetStockObject(DEFAULT_GUI_FONT);
        let previous = SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, TEXT_COLOR);

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
        // 项留空，不画占位图形。悬停项只改配色，不改变布局（D-049）。
        if !state.is_null() {
            // SAFETY: 见函数文档，`state` 只在窗口存续期间使用。
            let state = &*state;
            let band_brush: HBRUSH = CreateSolidBrush(ICON_BAND_BACKGROUND);
            let hover_brush: HBRUSH = CreateSolidBrush(HOVER_BAND_BACKGROUND);
            let accent_brush: HBRUSH = CreateSolidBrush(ACCENT);
            let icon_size = scaled(ICON_DIP, dpi);
            let icon_inset = scaled(ICON_INSET_DIP, dpi);
            let accent_height = scaled(ACCENT_DIP, dpi);
            let band_height = scaled(ICON_BAND_DIP, dpi);

            for entry in &state.entries {
                let hovered = state.hovered == Some(entry.window);
                FillRect(
                    hdc,
                    &entry.bar,
                    if hovered { hover_brush } else { band_brush },
                );
                if hovered {
                    // 强调色只占图标条上沿一条，作为「当前悬停」的提示。
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
            let _ = DeleteObject(hover_brush.into());
            let _ = DeleteObject(band_brush.into());
        }

        SelectObject(hdc, previous);
        let _ = DeleteObject(toolbar_brush.into());
        let _ = DeleteObject(brush.into());
        let _ = EndPaint(hwnd, &ps);
    }
}
