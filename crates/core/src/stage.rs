//! Stage 模式的布局计算。
//!
//! 舞台区 = 主显示器工作区减去左侧预览区（D-019）。这里只有纯计算，DPI 换算与
//! 窗口移动由上层负责。

use crate::snapshot::Rect;

/// 当前模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// 普通桌面：不做任何布局。
    #[default]
    Normal,
    /// 台前调度：活跃窗口占舞台区，其余窗口留在原位。
    Stage,
}

/// 舞台区：工作区去掉左侧预览区。
///
/// 预览区宽度超过工作区宽度时，舞台区收窄到 0 宽，避免算出负数矩形。
pub fn stage_rect(work_area: Rect, strip_width_px: i32) -> Rect {
    let strip_width = strip_width_px.clamp(0, work_area.width().max(0));
    Rect {
        left: work_area.left + strip_width,
        top: work_area.top,
        right: work_area.right,
        bottom: work_area.bottom,
    }
}

/// 把给定尺寸居中放进舞台区（D-025：只记尺寸，位置每次重算）。
///
/// 尺寸超过舞台区时收到舞台区大小；尺寸为 0 或负数时收到 1 像素。
pub fn centered_rect(stage: Rect, width: i32, height: i32) -> Rect {
    let width = width.clamp(1, stage.width().max(1));
    let height = height.clamp(1, stage.height().max(1));
    let left = stage.left + (stage.width() - width) / 2;
    let top = stage.top + (stage.height() - height) / 2;
    Rect {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work_area() -> Rect {
        Rect {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1400,
        }
    }

    #[test]
    fn stage_starts_after_the_strip() {
        let stage = stage_rect(work_area(), 200);
        assert_eq!(stage.left, 200);
        assert_eq!(stage.right, 2560);
        assert_eq!(stage.top, 0);
        assert_eq!(stage.bottom, 1400);
        assert_eq!(stage.width(), 2360);
    }

    #[test]
    fn stage_respects_a_non_zero_work_area_origin() {
        let work_area = Rect {
            left: 100,
            top: 50,
            right: 1500,
            bottom: 1000,
        };
        let stage = stage_rect(work_area, 200);
        assert_eq!(stage.left, 300);
        assert_eq!(stage.top, 50);
    }

    #[test]
    fn oversized_strip_is_clamped_to_work_area() {
        let stage = stage_rect(work_area(), 5000);
        assert_eq!(stage.left, 2560);
        assert_eq!(stage.width(), 0);
    }

    #[test]
    fn negative_strip_width_is_treated_as_zero() {
        let stage = stage_rect(work_area(), -50);
        assert_eq!(stage.left, 0);
    }

    #[test]
    fn centered_rect_is_horizontally_and_vertically_centered() {
        let stage = stage_rect(work_area(), 200);
        let placed = centered_rect(stage, 1000, 700);
        assert_eq!(placed.width(), 1000);
        assert_eq!(placed.height(), 700);
        assert_eq!(placed.left - stage.left, (stage.width() - 1000) / 2);
        assert_eq!(placed.top - stage.top, (stage.height() - 700) / 2);
    }

    #[test]
    fn remembered_size_larger_than_stage_is_clamped() {
        let stage = stage_rect(work_area(), 200);
        let placed = centered_rect(stage, 5000, 5000);
        assert_eq!(placed, stage, "超大记忆尺寸应退回占满舞台区");
    }

    #[test]
    fn non_positive_size_becomes_one_pixel() {
        let stage = stage_rect(work_area(), 200);
        let placed = centered_rect(stage, 0, -10);
        assert_eq!(placed.width(), 1);
        assert_eq!(placed.height(), 1);
    }
}
