//! 平台无关的呈现场景：把 presentation snapshot 布局成绝对逻辑几何、
//! 文本 run、命中测试和 caret/selection 几何。
//!
//! 输入是 [`zenui::PresentationSnapshot`]、[`Viewport`] 与宿主提供的 [`SceneEnv`]；
//! 输出是 [`Scene`]。每次构建都做一次完整 Taffy 重排，Node ID 原样保留。
//! 滚动偏移随 [`SceneEnv`] 进入，场景平移全部几何但不改变快照或 Node ID。
//! 重复构建可复用 [`SceneCache`] 的文本 layout；场景不持有闭包、平台句柄或
//! 第二棵保留树，也不读取运行时内部。

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod error;
mod geometry;
pub mod gpu;
mod scene;
mod scrollbar;
mod text;
mod text_assets;
mod view;

pub use error::SceneError;
pub use geometry::{Point, Rect, Viewport};
pub use gpu::{
    GpuContext, GpuRenderer, HighContrastColors, RenderError, RenderPalette, RenderTargetConfig,
};
pub use parley::FontData;
pub use scene::{
    Decoration, DecorationKind, PositionedGlyph, ResolvedStyle, Scene, SceneCache, SceneElement,
    SceneEnv, SceneRowBand, TextRun, Visual,
};
pub use scrollbar::Scrollbar;
pub use text_assets::{FontAsset, TextAssetError, TextAssets};
pub use view::{PresentationApply, PresentationView};
pub use wgpu;
