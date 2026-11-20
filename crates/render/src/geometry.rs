//! 逻辑像素下的场景几何。

use crate::SceneError;

/// 场景空间中的一个逻辑点。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// x 坐标。
    pub x: f32,
    /// y 坐标。
    pub y: f32,
}

impl Point {
    /// 原点 `(0, 0)`。
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    /// 构造一个逻辑点。
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// 一个绝对逻辑矩形；宽度和高度不为负。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    /// 左边。
    pub x: f32,
    /// 上边。
    pub y: f32,
    /// 宽度。
    pub width: f32,
    /// 高度。
    pub height: f32,
}

impl Rect {
    /// 构造一个逻辑矩形。
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// 右边（不含）。
    pub fn right(self) -> f32 {
        self.x + self.width
    }

    /// 下边（不含）。
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    /// 半开区间命中：相邻矩形共享的边只属于右侧或下侧的矩形。
    pub fn contains(self, point: Point) -> bool {
        self.width > 0.0
            && self.height > 0.0
            && point.x >= self.x
            && point.x < self.right()
            && point.y >= self.y
            && point.y < self.bottom()
    }
}

/// 逻辑 viewport 与它对应的 Window scale。
///
/// 布局始终使用逻辑像素；`scale` 只随场景交给 surface 后端，不改变几何。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    width: f32,
    height: f32,
    scale: f32,
}

impl Viewport {
    /// 拒绝非有限、负尺寸或非正 scale 的 viewport。
    pub fn new(width: f32, height: f32, scale: f32) -> Result<Self, SceneError> {
        if !width.is_finite() || width < 0.0 {
            return Err(SceneError::InvalidViewport {
                reason: "viewport width must be finite and non-negative",
            });
        }
        if !height.is_finite() || height < 0.0 {
            return Err(SceneError::InvalidViewport {
                reason: "viewport height must be finite and non-negative",
            });
        }
        if !scale.is_finite() || scale <= 0.0 {
            return Err(SceneError::InvalidViewport {
                reason: "window scale must be finite and positive",
            });
        }
        Ok(Self {
            width,
            height,
            scale,
        })
    }

    /// 逻辑宽度。
    pub const fn width(self) -> f32 {
        self.width
    }

    /// 逻辑高度。
    pub const fn height(self) -> f32 {
        self.height
    }

    /// 窗口逻辑坐标到物理像素的缩放。
    pub const fn scale(self) -> f32 {
        self.scale
    }
}
