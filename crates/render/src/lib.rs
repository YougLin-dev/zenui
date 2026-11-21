//! 平台无关的呈现场景：把 presentation snapshot 布局成绝对逻辑几何、
//! 文本 run、命中测试和 caret/selection 几何。
//!
//! 输入是 [`PresentationSnapshot`] 与 [`Viewport`]；输出是 [`Scene`]。每次
//! 构建都做一次完整 Taffy 重排，Node ID 原样保留。场景不持有闭包、平台
//! 句柄或第二棵保留树，也不读取运行时内部。

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod error;
mod geometry;
pub mod gpu;
mod scene;
mod text;

pub use error::SceneError;
pub use geometry::{Point, Rect, Viewport};
pub use gpu::{GpuContext, GpuRenderer, RenderError, RenderPalette, RenderTargetConfig};
pub use parley::FontData;
pub use scene::{
    Decoration, DecorationKind, PositionedGlyph, ResolvedStyle, Scene, SceneElement, SceneEnv,
    TextRun, Visual,
};
pub use wgpu;
