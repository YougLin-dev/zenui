//! 宿主/渲染器共用的滚动条覆盖层几何。
//!
//! 滚动条不是呈现快照元素，也没有 Node ID：它只由 viewport、内容范围、已应用
//! 偏移与最大偏移推导。GPU 绘制与宿主命中测试消费同一份几何（ADR-0010）。

use crate::geometry::{Point, Rect, Viewport};

/// 覆盖在滚动根右侧的滚动条：固定宽度的轨道与按比例缩放的 thumb。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scrollbar {
    track: Rect,
    thumb: Rect,
}

/// 轨道到窗口边缘的间距（逻辑像素）。
const INSET: f32 = 4.0;
/// 轨道宽度（逻辑像素）。
const THICKNESS: f32 = 8.0;
/// thumb 的最小高度（逻辑像素）。
const MIN_THUMB: f32 = 24.0;
/// 轨道与 thumb 的圆角半径（逻辑像素）。
pub(crate) const RADIUS: f32 = 4.0;

impl Scrollbar {
    /// 从场景的滚动度量推导覆盖层；内容可以滚动且轨道容得下最小 thumb 时才存在。
    ///
    /// 轨道位于视口右缘内侧并覆盖内容，不预留布局空间。thumb 高度按
    /// `viewport.height / content.height` 缩放，最小 24 逻辑像素；位置按
    /// 已应用偏移在最大偏移中的比例落在剩余轨道上。轨道短于最小 thumb 的
    /// 退化视口没有滚动条。
    pub(crate) fn from_metrics(
        viewport: Viewport,
        content_extent: Rect,
        scroll_offset: i32,
        max_scroll_offset: i32,
    ) -> Option<Self> {
        if max_scroll_offset <= 0 {
            return None;
        }
        let track_height = viewport.height() - INSET * 2.0;
        if track_height < MIN_THUMB || viewport.width() < INSET + THICKNESS {
            return None;
        }
        let fraction = scroll_offset as f32 / max_scroll_offset as f32;
        let thumb_height =
            (track_height * viewport.height() / content_extent.height).max(MIN_THUMB);
        let track = Rect::new(
            viewport.width() - INSET - THICKNESS,
            INSET,
            THICKNESS,
            track_height,
        );
        let y = track.y + (track_height - thumb_height) * fraction.clamp(0.0, 1.0);
        Some(Self {
            track,
            thumb: Rect::new(track.x, y, THICKNESS, thumb_height),
        })
    }

    /// 轨道矩形；高度是视口去掉上下各 4 逻辑像素间距后的剩余空间。
    pub const fn track(self) -> Rect {
        self.track
    }

    /// thumb 矩形；高度受最小高度约束，顶部与底部对应偏移的两端。
    pub const fn thumb(self) -> Rect {
        self.thumb
    }

    /// 指针是否落在滚动条上。
    ///
    /// 宿主在普通元素命中之前调用，让覆盖层获得命中优先权；区分 thumb 与
    /// 轨道时再比较 [`Scrollbar::thumb`]。
    pub fn hit(self, point: Point) -> bool {
        self.track.contains(point)
    }
}
