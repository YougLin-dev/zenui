//! 从 presentation snapshot 构建平台无关的场景。

use std::collections::HashMap;
use std::sync::Arc;

use taffy::prelude::{
    AlignItems, AvailableSpace, Display, FlexDirection, FlexWrap, JustifyContent, Style, TaffyTree,
    auto, length, percent, zero,
};
use taffy::{
    Baselines, LayoutInput, LayoutOutput, NodeId as TaffyNodeId, Rect as TaffyRect, ResolveOrZero,
    Size as TaffySize, compute_leaf_layout,
};
use zenui::{Appearance, NodeId, PresentationSnapshot, SnapshotElement, SnapshotNode};
use zenui_style::{Color, Condition, Property, StyleDeclaration, StyleOrigin, StyleValue};

use crate::SceneError;
use crate::geometry::{Point, Rect, Viewport};
use crate::scrollbar::Scrollbar;
use crate::text::{self, CharRange, TextShaper};
use crate::text_assets::{TextAssetError, TextAssets};

/// 场景理解的 intrinsic；其他 tag 是显式错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Intrinsic {
    View,
    Form,
    Text,
    Button,
    Checkbox,
    TextInput,
}

impl Intrinsic {
    fn from_tag(tag: &'static str) -> Option<Self> {
        match tag {
            "view" => Some(Self::View),
            "form" => Some(Self::Form),
            "text" => Some(Self::Text),
            "button" => Some(Self::Button),
            "checkbox" => Some(Self::Checkbox),
            "text-input" => Some(Self::TextInput),
            _ => None,
        }
    }

    const fn is_leaf(self) -> bool {
        matches!(self, Self::Checkbox | Self::TextInput)
    }
}

/// 语义或布局属性。class 与 style:* 不在其中：场景 seam 不理解它们，
/// 因此显式报错而不是静默丢弃。
const SUPPORTED_ATTRS: &[&str] = &[
    "value",
    "checked",
    "disabled",
    "aria:label",
    "aria:pressed",
    "role",
    "type",
    "id",
];

/// class 与 inline 声明解析后的值；未声明的属性不参与覆盖。
///
/// 颜色保留角色或字面量形式，由后端按当前调色板解析。字号与字重进入
/// 文本整形、测量与 caret 几何；不透明度与圆角只影响绘制。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedStyle {
    background: Option<Color>,
    text_color: Option<Color>,
    border_color: Option<Color>,
    border_width: Option<f32>,
    radius: f32,
    opacity: f32,
    text: text::TextStyle,
}

impl ResolvedStyle {
    /// 没有任何声明的起点：颜色与边框未设置，圆角为 0，不透明度为 1，
    /// 文本使用 14 点、400 字重。
    const fn initial() -> Self {
        Self {
            background: None,
            text_color: None,
            border_color: None,
            border_width: None,
            radius: 0.0,
            opacity: 1.0,
            text: text::TextStyle::DEFAULT,
        }
    }

    /// 背景色；`None` 表示沿用 intrinsic 的内建视觉。
    pub const fn background(self) -> Option<Color> {
        self.background
    }

    /// 文本颜色；`None` 表示使用调色板的文本色。
    pub const fn text_color(self) -> Option<Color> {
        self.text_color
    }

    /// 边框颜色；`None` 表示使用调色板的边框色。
    pub const fn border_color(self) -> Option<Color> {
        self.border_color
    }

    /// class 声明的边框宽度（逻辑点）；`None` 表示沿用 intrinsic 默认。
    pub const fn border_width(self) -> Option<f32> {
        self.border_width
    }

    /// 圆角半径（逻辑点）；只影响绘制，不裁剪子节点或命中区域。
    pub const fn radius(self) -> f32 {
        self.radius
    }

    /// 元素自身绘制的不透明度（0–1）；不作用于子节点。
    pub const fn opacity(self) -> f32 {
        self.opacity
    }

    /// 文本字号（逻辑点）。
    pub const fn font_size(self) -> f32 {
        self.text.size
    }

    /// 文本字重。
    pub const fn font_weight(self) -> u16 {
        self.text.weight
    }
}

/// 宿主拥有的呈现环境；不属于 presentation snapshot。
///
/// `Default` 表示没有指针、指针输入方式，滚动偏移为 0，文本缩放 100%：
/// hover 与 active 都不命中。滚动偏移是每个窗口 activation 持续的呈现状态，
/// 不随指针瞬态清除。文本缩放只影响文本与由字号派生的几何，不改变显示 DPI。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneEnv {
    hovered: Option<NodeId>,
    pressed: Option<NodeId>,
    focus_visible: bool,
    scroll_offset: i32,
    text_scale_percent: u16,
    high_contrast: bool,
    env_generation: u64,
}

impl Default for SceneEnv {
    fn default() -> Self {
        Self {
            hovered: None,
            pressed: None,
            focus_visible: false,
            scroll_offset: 0,
            text_scale_percent: 100,
            high_contrast: false,
            env_generation: 0,
        }
    }
}

impl SceneEnv {
    /// 命中测试的最上层元素；容器也可以被命中，只有控件会使用它。
    pub const fn with_hovered(mut self, node: Option<NodeId>) -> Self {
        self.hovered = node;
        self
    }

    /// 指针当前按住的节点。
    pub const fn with_pressed(mut self, node: Option<NodeId>) -> Self {
        self.pressed = node;
        self
    }

    /// 焦点是否来自键盘输入。
    pub const fn with_focus_visible(mut self, focus_visible: bool) -> Self {
        self.focus_visible = focus_visible;
        self
    }

    /// 宿主请求的逻辑像素滚动偏移；正值让内容向上移动。
    ///
    /// 场景构建时把它夹到 `[0, max_scroll_offset]`，并通过
    /// [`Scene::scroll_offset`] 报告实际采用的值。
    pub const fn with_scroll_offset(mut self, offset: i32) -> Self {
        self.scroll_offset = offset;
        self
    }

    /// 辅助功能文本缩放百分比；有效范围 100–225，越界值回退到 100。
    ///
    /// 缩放改变字号、行高与 `em` 解析，进而改变测量、caret/selection 几何、
    /// 命中与滚动范围；固定 `px` 长度仍是逻辑像素。
    pub const fn with_text_scale_percent(mut self, percent: u16) -> Self {
        self.text_scale_percent = match percent {
            100..=225 => percent,
            _ => 100,
        };
        self
    }

    /// 当前悬停的节点。
    pub const fn hovered(self) -> Option<NodeId> {
        self.hovered
    }

    /// 当前按住的节点。
    pub const fn pressed(self) -> Option<NodeId> {
        self.pressed
    }

    /// 焦点是否来自键盘输入。
    pub const fn focus_visible(self) -> bool {
        self.focus_visible
    }

    /// 宿主请求的滚动偏移；场景构建前可能超出内容范围。
    pub const fn scroll_offset(self) -> i32 {
        self.scroll_offset
    }

    /// 辅助功能文本缩放百分比。
    pub const fn text_scale_percent(self) -> u16 {
        self.text_scale_percent
    }

    /// 系统高对比度开关；不进入快照，宿主据此选择覆盖调色板。
    pub const fn with_high_contrast(mut self, high_contrast: bool) -> Self {
        self.high_contrast = high_contrast;
        self
    }

    /// 本窗口的系统高对比度状态。
    pub const fn high_contrast(self) -> bool {
        self.high_contrast
    }

    /// 每个窗口独立的环境代际；系统偏好、DPI、滚动或焦点 modality 变化时
    /// 由宿主递增。它不是树 revision，也不进入快照。
    pub const fn with_env_generation(mut self, generation: u64) -> Self {
        self.env_generation = generation;
        self
    }

    /// 本窗口的环境代际。
    pub const fn env_generation(self) -> u64 {
        self.env_generation
    }
}

/// 逐帧交互路径：最深命中节点 → 根。
///
/// `hover:`/`active:` 条件对路径内任意元素成立，因此整行 hover 不需要给
/// 每个子节点声明样式。路径不进入快照或响应式图；`generation` 在路径变化时
/// 由宿主递增，与快照 revision 和环境代际一起构成场景缓存键。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InteractionPaths {
    hover_path: Arc<[NodeId]>,
    press_path: Arc<[NodeId]>,
    focus_path: Arc<[NodeId]>,
    generation: u64,
}

impl InteractionPaths {
    /// 由三条“最深节点 → 根”的路径构造；`generation` 由宿主维护。
    pub fn new(
        hover_path: impl Into<Arc<[NodeId]>>,
        press_path: impl Into<Arc<[NodeId]>>,
        focus_path: impl Into<Arc<[NodeId]>>,
        generation: u64,
    ) -> Self {
        Self {
            hover_path: hover_path.into(),
            press_path: press_path.into(),
            focus_path: focus_path.into(),
            generation,
        }
    }

    /// 推进路径代际；路径内容不变时用于让缓存键失效。
    pub const fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }

    /// 最深命中节点到根的路径；空表示没有 hover。
    pub fn hover_path(&self) -> &[NodeId] {
        &self.hover_path
    }

    /// press 目标到根的路径；空表示没有 press。
    pub fn press_path(&self) -> &[NodeId] {
        &self.press_path
    }

    /// 焦点节点到根的路径；空表示没有焦点。
    pub fn focus_path(&self) -> &[NodeId] {
        &self.focus_path
    }

    /// 最深命中节点，驱动内建 hover 视觉。
    pub fn hovered(&self) -> Option<NodeId> {
        self.hover_path.first().copied()
    }

    /// press 目标，驱动内建按下视觉。
    pub fn pressed(&self) -> Option<NodeId> {
        self.press_path.first().copied()
    }

    /// 焦点节点。
    pub fn focused(&self) -> Option<NodeId> {
        self.focus_path.first().copied()
    }

    /// 路径代际；路径内容变化时由宿主递增。
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

/// 场景缓存键：快照 revision、宿主环境代际与交互路径代际。
///
/// 三者独立：外观走快照 revision，系统偏好/DPI/滚动/焦点 modality 走环境
/// 代际，hover/press/focus 路径走交互代际。快照未变时环境变化仍必须重建
/// 场景与无障碍几何。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SceneKey {
    snapshot_revision: u64,
    env_generation: u64,
    interaction_generation: u64,
}

impl SceneKey {
    /// 由构建本次场景的三个代际组成。
    pub const fn new(
        snapshot_revision: u64,
        env_generation: u64,
        interaction_generation: u64,
    ) -> Self {
        Self {
            snapshot_revision,
            env_generation,
            interaction_generation,
        }
    }

    /// 构建本次场景的已发布快照 revision。
    pub const fn snapshot_revision(self) -> u64 {
        self.snapshot_revision
    }

    /// 构建本次场景时的宿主环境代际。
    pub const fn env_generation(self) -> u64 {
        self.env_generation
    }

    /// 构建本次场景时的交互路径代际。
    pub const fn interaction_generation(self) -> u64 {
        self.interaction_generation
    }
}

/// 一个已提交元素的绝对逻辑几何与内建视觉。
#[derive(Clone, Debug)]
pub struct SceneElement {
    id: NodeId,
    tag: &'static str,
    rect: Rect,
    visual: Visual,
    resolved: ResolvedStyle,
    disabled: bool,
    hovered: bool,
    pointer_pressed: bool,
    focused: bool,
    text: Vec<TextRun>,
    caret_stops: Option<Vec<CaretStop>>,
}

/// 显示文本的一个字符边界；`value_index` 已换算回编辑器的字符偏移。
#[derive(Clone, Copy, Debug)]
struct CaretStop {
    value_index: usize,
    caret: Rect,
}

impl SceneElement {
    /// 稳定的 Node ID；重新挂载才产生新身份。
    pub const fn id(&self) -> NodeId {
        self.id
    }

    /// 已提交的 intrinsic 名称。
    pub const fn tag(&self) -> &'static str {
        self.tag
    }

    /// 绝对逻辑矩形，包含边框盒。
    pub const fn rect(&self) -> Rect {
        self.rect
    }

    /// 后端应绘制的内建视觉。
    pub const fn visual(&self) -> Visual {
        self.visual
    }

    /// class 声明解析后的绘制值。
    pub const fn resolved_style(&self) -> ResolvedStyle {
        self.resolved
    }

    /// 该元素在快照中是否 disabled。
    pub const fn disabled(&self) -> bool {
        self.disabled
    }

    /// 该元素是否持有快照焦点。
    pub const fn focused(&self) -> bool {
        self.focused
    }

    /// 该元素是否是 `SceneEnv` 里当前悬停的节点。
    pub const fn hovered(&self) -> bool {
        self.hovered
    }

    /// 该元素是否是 `SceneEnv` 里当前按住的节点。
    pub const fn pointer_pressed(&self) -> bool {
        self.pointer_pressed
    }

    /// 绘制在该元素盒内的文本 run，包含按钮标题等裸文本子节点。
    ///
    /// 元素完全在 viewport 外时为空；几何和样式仍可查询。
    pub fn text(&self) -> &[TextRun] {
        &self.text
    }

    /// 文本输入内离 `point` 最近的字符边界；其他元素返回 `None`。
    ///
    /// 组合中的 preedit 不属于 value，落在 preedit 内的点击夹到组合起点。
    pub fn caret_index(&self, point: Point) -> Option<usize> {
        let stops = self.caret_stops.as_ref()?;
        stops
            .iter()
            .min_by(|left, right| {
                caret_distance(left.caret, point).total_cmp(&caret_distance(right.caret, point))
            })
            .map(|stop| stop.value_index)
    }

    /// 值偏移处的 caret 几何；组合窗口用它对齐 preedit 起点。
    pub fn caret_at_value(&self, value_index: usize) -> Option<Rect> {
        self.caret_stops
            .as_ref()?
            .iter()
            .filter(|stop| stop.value_index == value_index)
            .map(|stop| stop.caret)
            .min_by(|left, right| left.x.total_cmp(&right.x))
    }
}

fn caret_distance(caret: Rect, point: Point) -> f32 {
    let dx = point.x - caret.x;
    let dy = if point.y < caret.y {
        caret.y - point.y
    } else if point.y > caret.bottom() {
        point.y - caret.bottom()
    } else {
        0.0
    };
    dx * dx + dy * dy
}

/// 后端为该元素绘制哪一种内建视觉。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visual {
    /// `view` 与 `form` 容器。
    Container,
    /// `text` 元素；内容在 [`SceneElement::text`]。
    Text,
    /// `button`；`pressed` 来自 `aria:pressed`。
    Button {
        /// 该按钮当前是否处于按下/选中状态。
        pressed: bool,
    },
    /// `checkbox`；盒是绝对逻辑几何中的 16×16 方块。
    Checkbox {
        /// 已提交的勾选状态。
        checked: bool,
    },
    /// `text-input`；文本、caret 与 selection 反映本地编辑状态。
    TextInput,
}

/// 一个已整形的绝对坐标 glyph run。
#[derive(Clone, Debug)]
pub struct TextRun {
    font: parley::FontData,
    font_size: f32,
    normalized_coords: Vec<i16>,
    baseline: f32,
    glyphs: Vec<PositionedGlyph>,
}

impl TextRun {
    pub(crate) fn new(
        font: parley::FontData,
        font_size: f32,
        normalized_coords: Vec<i16>,
        baseline: f32,
        glyphs: Vec<PositionedGlyph>,
    ) -> Self {
        Self {
            font,
            font_size,
            normalized_coords,
            baseline,
            glyphs,
        }
    }

    /// 该 run 使用的字体句柄。
    pub const fn font(&self) -> &parley::FontData {
        &self.font
    }

    /// 该 run 的字号（逻辑像素）。
    pub const fn font_size(&self) -> f32 {
        self.font_size
    }

    /// 可变字体的 normalized coords。
    pub fn normalized_coords(&self) -> &[i16] {
        &self.normalized_coords
    }

    /// run 的基线绝对 y 坐标。
    pub const fn baseline(&self) -> f32 {
        self.baseline
    }

    /// 绝对坐标下的已定位字形。
    pub fn glyphs(&self) -> &[PositionedGlyph] {
        &self.glyphs
    }
}

/// 绝对逻辑坐标下的一个字形。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    /// 字体内的 glyph ID。
    pub id: u32,
    /// 字形原点的绝对 x 坐标。
    pub x: f32,
    /// 字形原点的绝对 y 坐标。
    pub y: f32,
    /// 该字形的前进宽度。
    pub advance: f32,
}

/// 本地文本编辑状态的几何装饰。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecorationKind {
    /// 输入获得焦点时的 caret。
    Caret,
    /// 非折叠 selection 的高亮。
    Selection,
    /// 组合中 preedit 的下划线。
    PreeditUnderline,
}

/// caret、selection 或 preedit 下划线的一角。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decoration {
    node: NodeId,
    kind: DecorationKind,
    rect: Rect,
}

impl Decoration {
    /// 装饰所属的文本输入。
    pub const fn node(self) -> NodeId {
        self.node
    }

    /// 装饰类别。
    pub const fn kind(self) -> DecorationKind {
        self.kind
    }

    /// 装饰的绝对逻辑矩形。
    pub const fn rect(self) -> Rect {
        self.rect
    }
}

/// 跨多次场景构建复用的文本布局缓存。
///
/// 一个 UI 线程持有一个实例即可。缓存按文本、字号、字重与断行宽度存储
/// Parley layout；快照或指针瞬态变化时只重新整形真正变化的文本。
pub struct SceneCache {
    shaper: TextShaper,
}

impl SceneCache {
    /// 创建一个空的文本布局缓存。
    #[must_use]
    pub fn new() -> Self {
        Self {
            shaper: TextShaper::new(),
        }
    }

    /// 用应用提供的字体与分词语料创建文本布局缓存。
    ///
    /// 字体在构造期注册并校验；空字节、非字体容器、没有可用家族、未注册的主家族
    /// 或回退家族都返回 typed error，不会推迟到绘制。默认不启用系统回退：已配置
    /// 字体覆盖的文本不会改用系统字体。
    ///
    /// # Errors
    ///
    /// 资产不满足 [`TextAssets`] 的约定时返回 [`TextAssetError`]。
    pub fn with_text_assets(assets: TextAssets) -> Result<Self, TextAssetError> {
        Ok(Self {
            shaper: TextShaper::with_assets(assets)?,
        })
    }
}

impl Default for SceneCache {
    fn default() -> Self {
        Self::new()
    }
}

/// 一个窗口化 `For` 的行带几何；宿主据此把滚动偏移换算成挂载窗口（ADR-0011）。
///
/// `top` 是行带内容顶端的场景布局坐标（不含滚动偏移），`row_height` 是
/// 已解析的逻辑点行高。行带总高度恒为 `total_rows × row_height`，与当前
/// 挂载了哪些行无关。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneRowBand {
    key: u64,
    top: f32,
    row_height: f32,
    first_row: usize,
    total_rows: usize,
}

impl SceneRowBand {
    /// 跨快照稳定的行带键；用于 [`crate::Scene::row_bands`] 与运行时窗口更新。
    pub const fn key(self) -> u64 {
        self.key
    }

    /// 行带内容顶端的场景布局坐标；不随滚动偏移改变。
    pub const fn top(self) -> f32 {
        self.top
    }

    /// 已解析的统一行槽高度（逻辑点）。
    pub const fn row_height(self) -> f32 {
        self.row_height
    }

    /// 当前已挂载的起始行。
    pub const fn first_row(self) -> usize {
        self.first_row
    }

    /// 完整行数。
    pub const fn total_rows(self) -> usize {
        self.total_rows
    }
}

/// 一次完整重排的结果；几何都是逻辑像素与绝对坐标。
#[derive(Clone, Debug)]
pub struct Scene {
    /// 构建这份场景的缓存键；快照 revision、环境代际与交互路径代际。
    key: SceneKey,
    viewport: Viewport,
    content_extent: Rect,
    scroll_offset: i32,
    max_scroll_offset: i32,
    elements: Vec<SceneElement>,
    /// 与视口相交的元素在 `elements` 中的下标，按绘制顺序。
    ///
    /// 绘制与命中只遍历这份索引；`elements` 仍保留全部已提交元素，供
    /// Node 查询与未来的辅助功能树使用。
    visible: Vec<u32>,
    /// 窗口化 `For` 的行带几何，保持挂载顺序。
    row_bands: Vec<SceneRowBand>,
    decorations: Vec<Decoration>,
    caret_visible: bool,
}

impl Scene {
    /// 用默认环境完整重排一次快照；没有指针瞬态与交互路径。
    pub fn build(snapshot: &PresentationSnapshot, viewport: Viewport) -> Result<Self, SceneError> {
        Self::build_with(
            snapshot,
            viewport,
            &SceneEnv::default(),
            &InteractionPaths::default(),
        )
    }

    /// 用宿主提供的呈现环境与交互路径完整重排一次快照。
    ///
    /// 每次调用都重新构建 Taffy 树，Node ID 原样保留；请求的偏移会夹到内容
    /// 范围，实际值见 [`Scene::scroll_offset`]。`env.hovered/pressed` 是内建
    /// 视觉的派生值；路径为空时按单个节点处理，非空时 `hover:`/`active:`
    /// 条件对路径内任意元素成立。
    pub fn build_with(
        snapshot: &PresentationSnapshot,
        viewport: Viewport,
        env: &SceneEnv,
        paths: &InteractionPaths,
    ) -> Result<Self, SceneError> {
        let mut cache = SceneCache::new();
        Self::build_cached(snapshot, viewport, env, paths, &mut cache)
    }

    /// 与 [`Scene::build_with`] 相同，但复用调用方持有的文本布局缓存。
    ///
    /// 适合消息循环中的重复构建：未变化的文本不需要重新整形。缓存只影响
    /// 构建耗时，不改变场景几何、字形或快照语义。
    pub fn build_cached(
        snapshot: &PresentationSnapshot,
        viewport: Viewport,
        env: &SceneEnv,
        paths: &InteractionPaths,
        cache: &mut SceneCache,
    ) -> Result<Self, SceneError> {
        Builder::new(snapshot, *env, paths).build(viewport, cache)
    }

    /// 构建这份场景的缓存键：快照 revision、环境代际与交互路径代际。
    pub const fn cache_key(&self) -> SceneKey {
        self.key
    }

    /// 构建该场景的 viewport；scale 未参与布局。
    pub const fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// 滚动前的内容范围：原点是 `(0, 0)`，宽度等于 viewport 宽度，高度是
    /// 合成根按像素对齐后的高度且不小于 viewport 高度。
    pub const fn content_extent(&self) -> Rect {
        self.content_extent
    }

    /// 场景实际采用的滚动偏移；请求值已夹到内容范围。
    pub const fn scroll_offset(&self) -> i32 {
        self.scroll_offset
    }

    /// 最大可用滚动偏移：内容高度减 viewport 高度，向下取整且不小于 0。
    pub const fn max_scroll_offset(&self) -> i32 {
        self.max_scroll_offset
    }

    /// 覆盖在视口右缘的滚动条几何；内容能够滚动且 thumb 短于轨道时才存在。
    ///
    /// 由 viewport、内容范围、已应用偏移与最大偏移推导，不新增元素或快照
    /// 属性，也不改变布局与 Node ID。thumb 高度按 viewport/content 比例缩放
    /// 且最小 24 逻辑像素；轨道短于最小 thumb 或 thumb 会填满轨道的退化视口
    /// 不绘制。GPU 绘制与宿主命中测试使用同一份几何。
    pub fn scrollbar(&self) -> Option<Scrollbar> {
        Scrollbar::from_metrics(
            self.viewport,
            self.content_extent,
            self.scroll_offset,
            self.max_scroll_offset,
        )
    }

    /// 是否绘制 caret；几何在 `decorations` 中保留，供 IME 定位使用。
    pub const fn caret_visible(&self) -> bool {
        self.caret_visible
    }

    /// caret 闪烁或组合期间隐藏绘制，但不影响 caret 几何查询。
    pub const fn set_caret_visible(&mut self, visible: bool) {
        self.caret_visible = visible;
    }

    /// 已提交元素，按绘制顺序（先序），几何已按滚动偏移平移。
    ///
    /// viewport 外的元素仍在列表中并保留几何，但不生成 glyph run 与 caret
    /// stops；窗口变大、滚动或内容移动后按新 viewport 重新生成。
    pub fn elements(&self) -> &[SceneElement] {
        &self.elements
    }

    /// 按 Node ID 查找场景元素；离屏元素同样可查询。
    pub fn node(&self, id: NodeId) -> Option<&SceneElement> {
        self.elements.iter().find(|element| element.id == id)
    }

    /// 与视口相交的元素，按绘制顺序；离屏元素只存在于 [`Scene::elements`]。
    ///
    /// 索引在场景构建时一次算好，绘制与命中都不再扫描全部已提交元素。
    pub fn visible_elements(&self) -> impl Iterator<Item = &SceneElement> {
        self.visible
            .iter()
            .map(|index| &self.elements[*index as usize])
    }

    /// 窗口化 `For` 的行带几何，保持呈现顺序；没有窗口化 `For` 时为空。
    pub fn row_bands(&self) -> &[SceneRowBand] {
        &self.row_bands
    }

    /// 最上层命中：后绘制的元素优先，半开区间。
    ///
    /// `point` 是场景坐标；视口外的点始终返回 `None`，即使平移后的内容
    /// 几何覆盖该点。视口内的点只可能落在与视口相交的元素上，因此只扫描
    /// 可见索引，结果与全量反向扫描一致。
    pub fn hit_test(&self, point: Point) -> Option<NodeId> {
        let viewport = Rect::new(0.0, 0.0, self.viewport.width(), self.viewport.height());
        if !viewport.contains(point) {
            return None;
        }
        self.visible
            .iter()
            .rev()
            .map(|index| &self.elements[*index as usize])
            .find(|element| element.rect.contains(point))
            .map(SceneElement::id)
    }

    /// 按绘制顺序排列的文本编辑装饰。
    pub fn decorations(&self) -> &[Decoration] {
        &self.decorations
    }

    /// 某个文本输入当前的 caret 矩形（只有聚焦时才存在）。
    pub fn caret(&self, node: NodeId) -> Option<Rect> {
        self.decorations
            .iter()
            .find(|decoration| decoration.node == node && decoration.kind == DecorationKind::Caret)
            .map(|decoration| decoration.rect)
    }
}

struct TextContext {
    text: String,
    style: text::TextStyle,
}

impl TextContext {
    fn new(text: String, style: text::TextStyle) -> Self {
        Self { text, style }
    }
}

/// 一条声明在解析时能看到的控件状态：语义字段来自已提交的快照，
/// hover/press 来自宿主交互路径。
#[derive(Clone, Copy, Debug, Default)]
struct ElementState {
    dark: bool,
    hovered: bool,
    focus: bool,
    focus_visible: bool,
    pressed: bool,
    disabled: bool,
    checked: bool,
    aria_pressed: bool,
}

impl ElementState {
    const fn active(self, condition: Condition) -> bool {
        match condition {
            Condition::Base => true,
            Condition::Dark => self.dark,
            Condition::Hover => self.hovered,
            Condition::Focus => self.focus,
            Condition::FocusVisible => self.focus_visible,
            Condition::Active => self.pressed,
            Condition::Disabled => self.disabled,
            Condition::Checked => self.checked,
            Condition::AriaPressed => self.aria_pressed,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Root,
    Element(NodeId),
    Text,
}

/// 一个文本输入的已整形显示状态，供 run 与装饰共用。
struct EditView<'a> {
    element: &'a SnapshotElement,
    display: String,
    preedit: Option<CharRange>,
    composition: Option<(usize, usize)>,
    layout: parley::Layout<()>,
    origin: Point,
}

struct Builder<'a> {
    snapshot: &'a PresentationSnapshot,
    env: SceneEnv,
    paths: &'a InteractionPaths,
    tree: TaffyTree<TextContext>,
    kinds: HashMap<TaffyNodeId, Kind>,
    resolved: HashMap<TaffyNodeId, ResolvedStyle>,
    /// emit 按 Node ID 取快照元素；snapshot 的公开查找是线性的。
    elements: HashMap<NodeId, &'a SnapshotElement>,
}

impl<'a> Builder<'a> {
    fn new(snapshot: &'a PresentationSnapshot, env: SceneEnv, paths: &'a InteractionPaths) -> Self {
        Self {
            snapshot,
            env,
            paths,
            tree: TaffyTree::new(),
            kinds: HashMap::new(),
            resolved: HashMap::new(),
            elements: snapshot
                .elements()
                .map(|element| (element.id(), element))
                .collect(),
        }
    }

    /// hover 条件是否命中该元素：路径非空时对路径内任意元素成立。
    fn hovered_path_contains(&self, id: NodeId) -> bool {
        if self.paths.hover_path().is_empty() {
            self.env.hovered() == Some(id)
        } else {
            self.paths.hover_path().contains(&id)
        }
    }

    /// active 条件是否命中该元素：路径非空时对路径内任意元素成立。
    fn press_path_contains(&self, id: NodeId) -> bool {
        if self.paths.press_path().is_empty() {
            self.env.pressed() == Some(id)
        } else {
            self.paths.press_path().contains(&id)
        }
    }

    /// 内建 hover 视觉只落在最深命中节点。
    fn hovered_self(&self, id: NodeId) -> bool {
        if self.paths.hover_path().is_empty() {
            self.env.hovered() == Some(id)
        } else {
            self.paths.hovered() == Some(id)
        }
    }

    /// 内建按下视觉只落在 press 目标本身。
    fn pressed_self(&self, id: NodeId) -> bool {
        if self.paths.press_path().is_empty() {
            self.env.pressed() == Some(id)
        } else {
            self.paths.pressed() == Some(id)
        }
    }

    fn build(mut self, viewport: Viewport, cache: &mut SceneCache) -> Result<Scene, SceneError> {
        let shaper = &mut cache.shaper;
        let mut roots = Vec::new();
        for root in self.snapshot.roots() {
            match root {
                SnapshotNode::Element(element) => roots.push(self.build_element(element, true)?),
                SnapshotNode::Text(_) => return Err(SceneError::BareRootText),
            }
        }
        let root = self
            .tree
            .new_with_children(root_style(viewport), &roots)
            .map_err(layout_error)?;
        self.kinds.insert(root, Kind::Root);

        let available = TaffySize {
            width: AvailableSpace::Definite(viewport.width()),
            height: AvailableSpace::MaxContent,
        };
        self.tree
            .compute_layout_with_measure(root, available, |inputs, _node, context, style| {
                measure_leaf(shaper, inputs, context, style)
            })
            .map_err(layout_error)?;

        let root_size = self.tree.layout(root).map_err(layout_error)?.size;
        // Taffy 把尺寸对齐到整像素；内容范围仍报告 viewport 的精确逻辑宽度，
        // 并把高度夹到不低于 viewport，保持公开不变式。
        let content_extent = Rect::new(
            0.0,
            0.0,
            viewport.width(),
            root_size.height.max(viewport.height()),
        );
        let max_scroll_offset = (content_extent.height - viewport.height()).max(0.0) as i32;
        let scroll_offset = self.env.scroll_offset.clamp(0, max_scroll_offset);
        let mut scene = Scene {
            key: SceneKey::new(
                self.snapshot.revision(),
                self.env.env_generation(),
                self.paths.generation(),
            ),
            viewport,
            content_extent,
            scroll_offset,
            max_scroll_offset,
            elements: Vec::new(),
            visible: Vec::new(),
            row_bands: Vec::new(),
            decorations: Vec::new(),
            caret_visible: true,
        };
        let clip = Rect::new(0.0, 0.0, viewport.width(), viewport.height());
        // 平移发生在布局之后：内容坐标不变，所有 emitted 几何共享同一偏移。
        let origin = Point::new(0.0, 0.0 - scroll_offset as f32);
        self.emit(shaper, root, origin, &mut scene, clip)?;
        // 与 GPU 剔除谓词一致：视口内的点只可能命中这份索引里的元素。
        scene.visible = scene
            .elements
            .iter()
            .enumerate()
            .filter(|(_, element)| viewport_intersects(element.rect(), viewport))
            .map(|(index, _)| index as u32)
            .collect();
        for band in self.snapshot.row_bands() {
            let element = scene
                .elements
                .iter()
                .find(|element| element.id() == band.top())
                .ok_or_else(|| SceneError::Layout {
                    message: String::from("a row band lost its top spacer"),
                })?;
            scene.row_bands.push(SceneRowBand {
                key: band.key(),
                top: element.rect().y + scroll_offset as f32,
                row_height: band.row_height().resolve(element.resolved.font_size()),
                first_row: band.first_row(),
                total_rows: band.total_rows(),
            });
        }
        Ok(scene)
    }

    fn build_element(
        &mut self,
        element: &SnapshotElement,
        top_level: bool,
    ) -> Result<TaffyNodeId, SceneError> {
        let intrinsic =
            Intrinsic::from_tag(element.tag()).ok_or_else(|| SceneError::UnsupportedTag {
                node: element.id(),
                tag: element.tag(),
            })?;
        for attr in element.attrs() {
            if !SUPPORTED_ATTRS.contains(&attr.name()) {
                return Err(SceneError::UnsupportedAttribute {
                    node: element.id(),
                    name: attr.name(),
                });
            }
        }
        if intrinsic.is_leaf() && !element.children().is_empty() {
            return Err(SceneError::UnsupportedContent {
                node: element.id(),
                reason: "a leaf intrinsic cannot have children",
            });
        }

        let base = match intrinsic {
            Intrinsic::TextInput => input_style(),
            Intrinsic::Checkbox => checkbox_style(),
            Intrinsic::Button => button_style(),
            Intrinsic::Text => text_container_style(),
            Intrinsic::View | Intrinsic::Form => container_style(intrinsic, element),
        };
        let state = ElementState {
            dark: self.snapshot.appearance() == Appearance::Dark,
            hovered: self.hovered_path_contains(element.id()),
            focus: self.snapshot.focus() == Some(element.id()),
            focus_visible: self.env.focus_visible && self.snapshot.focus() == Some(element.id()),
            pressed: self.press_path_contains(element.id()),
            disabled: element.disabled(),
            checked: element.checked().unwrap_or(false),
            aria_pressed: element.pressed(),
        };
        let (style, resolved) = resolve_style(
            with_grow(base, top_level),
            element.id(),
            element.styles(),
            state,
            self.env.text_scale_percent(),
        )?;
        let node = match intrinsic {
            Intrinsic::TextInput => {
                let (display, _, _) = display_text(element);
                self.tree
                    .new_leaf_with_context(style, TextContext::new(display, resolved.text))
                    .map_err(layout_error)?
            }
            Intrinsic::Checkbox => self.tree.new_leaf(style).map_err(layout_error)?,
            Intrinsic::Button | Intrinsic::Text | Intrinsic::View | Intrinsic::Form => {
                self.container(element, style, resolved)?
            }
        };
        self.kinds.insert(node, Kind::Element(element.id()));
        self.resolved.insert(node, resolved);
        Ok(node)
    }

    fn container(
        &mut self,
        element: &SnapshotElement,
        style: Style,
        resolved: ResolvedStyle,
    ) -> Result<TaffyNodeId, SceneError> {
        let mut children = Vec::new();
        for child in element.children() {
            match child {
                SnapshotNode::Element(element) => {
                    children.push(self.build_element(element, false)?)
                }
                SnapshotNode::Text(text) => children.push(self.build_text(text.text(), resolved)?),
            }
        }
        self.tree
            .new_with_children(style, &children)
            .map_err(layout_error)
    }

    fn build_text(
        &mut self,
        text: &str,
        resolved: ResolvedStyle,
    ) -> Result<TaffyNodeId, SceneError> {
        let node = self
            .tree
            .new_leaf_with_context(
                text_leaf_style(),
                TextContext::new(text.to_owned(), resolved.text),
            )
            .map_err(layout_error)?;
        self.kinds.insert(node, Kind::Text);
        Ok(node)
    }

    fn emit(
        &self,
        shaper: &mut TextShaper,
        node: TaffyNodeId,
        origin: Point,
        scene: &mut Scene,
        clip: Rect,
    ) -> Result<(), SceneError> {
        let layout = self.tree.layout(node).map_err(layout_error)?;
        let rect = Rect::new(
            origin.x + layout.location.x,
            origin.y + layout.location.y,
            layout.size.width,
            layout.size.height,
        );
        let kind = self
            .kinds
            .get(&node)
            .copied()
            .ok_or_else(|| SceneError::Layout {
                message: String::from("a laid-out node has no scene kind"),
            })?;
        match kind {
            Kind::Root => {
                for child in self.tree.children(node).map_err(layout_error)? {
                    self.emit(shaper, child, Point::new(rect.x, rect.y), scene, clip)?;
                }
            }
            Kind::Element(id) => {
                let element =
                    self.elements
                        .get(&id)
                        .copied()
                        .ok_or_else(|| SceneError::Layout {
                            message: String::from("the snapshot lost a laid-out element"),
                        })?;
                let children = self.tree.children(node).map_err(layout_error)?;
                let resolved =
                    self.resolved
                        .get(&node)
                        .copied()
                        .ok_or_else(|| SceneError::Layout {
                            message: String::from("a laid-out element has no resolved style"),
                        })?;
                let mut runs = Vec::new();
                for child in &children {
                    if self.kinds.get(child) != Some(&Kind::Text) {
                        continue;
                    }
                    let child_layout = self.tree.layout(*child).map_err(layout_error)?;
                    let child_rect = Rect::new(
                        rect.x + child_layout.location.x,
                        rect.y + child_layout.location.y,
                        child_layout.size.width,
                        child_layout.size.height,
                    );
                    if !intersects(child_rect, clip) {
                        continue;
                    }
                    let context =
                        self.tree
                            .get_node_context(*child)
                            .ok_or_else(|| SceneError::Layout {
                                message: String::from("a text leaf has no content"),
                            })?;
                    let shaped = shaper.shape(&context.text, child_rect.width, context.style);
                    runs.extend(text::runs(&shaped, Point::new(child_rect.x, child_rect.y)));
                }

                let intrinsic = Intrinsic::from_tag(element.tag()).ok_or_else(|| {
                    SceneError::UnsupportedTag {
                        node: id,
                        tag: element.tag(),
                    }
                })?;
                let focused = self.snapshot.focus() == Some(id);
                let mut caret_stops = None;
                if intrinsic == Intrinsic::TextInput && intersects(rect, clip) {
                    let (content, width) = content_box(rect, layout);
                    let (display, preedit, composition) = display_text(element);
                    let layout = shaper.shape(&display, width, resolved.text);
                    let view = EditView {
                        element,
                        display,
                        preedit,
                        composition,
                        layout,
                        origin: content,
                    };
                    runs.extend(text::runs(&view.layout, content));
                    add_edit_decorations(&view, focused, &mut scene.decorations);
                    caret_stops = Some(
                        text::caret_stops(&view.layout, view.origin, &view.display)
                            .into_iter()
                            .map(|(display_index, caret)| CaretStop {
                                value_index: value_index(
                                    display_index,
                                    view.preedit,
                                    view.composition,
                                ),
                                caret,
                            })
                            .collect(),
                    );
                }

                scene.elements.push(SceneElement {
                    id,
                    tag: element.tag(),
                    rect,
                    visual: visual(intrinsic, element),
                    resolved,
                    disabled: element.disabled(),
                    hovered: self.hovered_self(id),
                    pointer_pressed: self.pressed_self(id),
                    focused,
                    text: runs,
                    caret_stops,
                });

                for child in children {
                    if self.kinds.get(&child) != Some(&Kind::Text) {
                        self.emit(shaper, child, Point::new(rect.x, rect.y), scene, clip)?;
                    }
                }
            }
            Kind::Text => {}
        }
        Ok(())
    }
}

fn measure_leaf(
    shaper: &mut TextShaper,
    inputs: LayoutInput,
    context: Option<&mut TextContext>,
    style: &Style,
) -> LayoutOutput {
    let Some(context) = context else {
        return compute_leaf_layout(inputs, style, |_, _| 0.0, |_, _| TaffySize::ZERO);
    };
    let padding = style
        .padding
        .resolve_or_zero(inputs.parent_size.width, |_, _| 0.0);
    let border = style
        .border
        .resolve_or_zero(inputs.parent_size.width, |_, _| 0.0);
    // Taffy 的 known dimension 是 border box；文本按 content box 断行，
    // 否则带 padding 的文本（如 text-input）会按更宽的盒子测量，
    // 却在更窄的内容盒里绘制，行数和高度都对不上。
    let inset =
        |size: Option<f32>, start: f32, end: f32| size.map(|size| (size - start - end).max(0.0));
    let known = TaffySize {
        width: inset(
            inputs.known_dimensions.width,
            padding.left + border.left,
            padding.right + border.right,
        ),
        height: inset(
            inputs.known_dimensions.height,
            padding.top + border.top,
            padding.bottom + border.bottom,
        ),
    };
    let (size, baseline) =
        shaper.measure(&context.text, known, inputs.available_space, context.style);
    let mut output = compute_leaf_layout(inputs, style, |_, _| 0.0, |_, _| size);
    output.baselines = Baselines::from_first(Some(baseline + padding.top + border.top));
    output
}

fn add_edit_decorations(view: &EditView<'_>, focused: bool, out: &mut Vec<Decoration>) {
    let Some(edit) = view.element.edit() else {
        return;
    };
    if let Some(selection) = edit.selection() {
        for rect in text::selection(
            &view.layout,
            view.origin,
            &view.display,
            CharRange::new(selection.0, selection.1),
        ) {
            out.push(Decoration {
                node: view.element.id(),
                kind: DecorationKind::Selection,
                rect,
            });
        }
    }
    if let Some(preedit) = view.preedit {
        for rect in text::selection(&view.layout, view.origin, &view.display, preedit) {
            out.push(Decoration {
                node: view.element.id(),
                kind: DecorationKind::PreeditUnderline,
                rect: Rect::new(rect.x, rect.bottom() - 1.0, rect.width, 1.0),
            });
        }
    }
    if focused {
        out.push(Decoration {
            node: view.element.id(),
            kind: DecorationKind::Caret,
            rect: text::caret(&view.layout, view.origin, &view.display, edit.caret()),
        });
    }
}

/// 把显示文本的字符偏移换算回编辑器 value 的字符偏移。
///
/// preedit 的长度与被替换的组合区间无关：显示文本是
/// `value[..start] + preedit + value[end..]`。preedit 之前的偏移不变，
/// 之后按 `end + (display - preedit.end)` 平移；preedit 内部（含空
/// preedit 的插入点）的点击夹到组合起点，preedit 末尾映射到 `end`。
fn value_index(
    display_index: usize,
    preedit: Option<CharRange>,
    composition: Option<(usize, usize)>,
) -> usize {
    let (Some(preedit), Some((start, end))) = (preedit, composition) else {
        return display_index;
    };
    if display_index < preedit.start() {
        display_index
    } else if display_index > preedit.end() {
        end + (display_index - preedit.end())
    } else if display_index == preedit.end() && preedit.end() > preedit.start() {
        end
    } else {
        start
    }
}

/// 组合中的显示文本在 value 的 base range 插入 preedit；运行时让 caret
/// 落在 preedit 末尾，因此起点由 caret 反推。
fn display_text(element: &SnapshotElement) -> (String, Option<CharRange>, Option<(usize, usize)>) {
    let value = element.text_value().unwrap_or("");
    let Some(edit) = element.edit() else {
        return (value.to_owned(), None, None);
    };
    let Some(preedit) = edit.preedit() else {
        return (value.to_owned(), None, None);
    };
    let chars = value.chars().count();
    let preedit_chars = preedit.chars().count();
    let (start, end) = edit
        .composition()
        .unwrap_or((edit.caret().saturating_sub(preedit_chars), edit.caret()));
    let start = start.min(chars);
    let end = end.max(start).min(chars);
    let mut display = String::with_capacity(value.len() + preedit.len());
    display.push_str(&value[..byte_at(value, start)]);
    display.push_str(preedit);
    display.push_str(&value[byte_at(value, end)..]);
    (
        display,
        Some(CharRange::new(start, start + preedit_chars)),
        Some((start, end)),
    )
}

fn byte_at(value: &str, char_offset: usize) -> usize {
    value
        .char_indices()
        .nth(char_offset)
        .map_or(value.len(), |(byte, _)| byte)
}

fn visual(intrinsic: Intrinsic, element: &SnapshotElement) -> Visual {
    match intrinsic {
        Intrinsic::Text => Visual::Text,
        Intrinsic::Button => Visual::Button {
            pressed: element.pressed(),
        },
        Intrinsic::Checkbox => Visual::Checkbox {
            checked: element.checked().unwrap_or(false),
        },
        Intrinsic::TextInput => Visual::TextInput,
        Intrinsic::View | Intrinsic::Form => Visual::Container,
    }
}

/// 半开区间相交；完全在 viewport 之外的文本无需整形或生成 glyph run。
/// 元素是否可能影响视口内的像素；与 GPU 绘制使用的剔除谓词一致。
///
/// 零面积元素也保留在索引里，避免丢掉带 glyph run 的退化元素。
fn viewport_intersects(rect: Rect, viewport: Viewport) -> bool {
    viewport.width() > 0.0
        && viewport.height() > 0.0
        && rect.right() > 0.0
        && rect.x < viewport.width()
        && rect.bottom() > 0.0
        && rect.y < viewport.height()
}

fn intersects(rect: Rect, clip: Rect) -> bool {
    rect.width > 0.0
        && rect.height > 0.0
        && clip.width > 0.0
        && clip.height > 0.0
        && rect.x < clip.right()
        && rect.right() > clip.x
        && rect.y < clip.bottom()
        && rect.bottom() > clip.y
}

/// 边框盒内的内容原点与可用宽度。
fn content_box(rect: Rect, layout: &taffy::Layout) -> (Point, f32) {
    let origin = Point::new(
        rect.x + layout.border.left + layout.padding.left,
        rect.y + layout.border.top + layout.padding.top,
    );
    let width = (rect.width
        - layout.border.left
        - layout.border.right
        - layout.padding.left
        - layout.padding.right)
        .max(0.0);
    (origin, width)
}

fn with_grow(mut style: Style, top_level: bool) -> Style {
    if top_level {
        style.flex_grow = 1.0;
    }
    style
}

/// 按条件优先级与源码顺序把活动声明合并到 intrinsic 默认样式上。
///
/// 高 rank 的条件胜出；相同 rank 时后出现的声明覆盖先出现的。
fn resolve_style(
    mut style: Style,
    node: NodeId,
    declarations: &[StyleDeclaration],
    state: ElementState,
    text_scale_percent: u16,
) -> Result<(Style, ResolvedStyle), SceneError> {
    let mut resolved = ResolvedStyle::initial();
    let mut width = None;
    let mut height = None;
    let mut winners: HashMap<Property, (StyleOrigin, Condition)> = HashMap::new();
    for declaration in declarations {
        let condition = declaration.condition();
        // 宏在展开期就拒绝状态变体里的布局属性；手工构造的快照必须由场景
        // 守同一规则，否则一次 hover 会改变几何。
        if condition != Condition::Base && !declaration.property().is_paint() {
            return Err(SceneError::UnsupportedStyle {
                node,
                property: declaration.property(),
                value: declaration.value(),
            });
        }
        if !state.active(condition) {
            continue;
        }
        let origin = declaration.origin();
        let beats = match winners.get(&declaration.property()) {
            None => true,
            Some((best_origin, best_condition)) => match (origin, best_origin) {
                (StyleOrigin::Inline, StyleOrigin::Class) => true,
                (StyleOrigin::Class, StyleOrigin::Inline) => false,
                _ => condition.rank() >= best_condition.rank(),
            },
        };
        if !beats {
            continue;
        }
        winners.insert(declaration.property(), (origin, condition));
        apply_declaration(
            &mut style,
            &mut resolved,
            &mut width,
            &mut height,
            node,
            *declaration,
        )?;
    }
    // 文本缩放放大最终字号；固定 px 长度保持逻辑像素，`em` 宽度与高度
    // 因此要等这里缩放之后再解析。
    resolved.text.size *= f32::from(text_scale_percent) / 100.0;
    if let Some(width) = width {
        style.size.width = length(width.resolve(resolved.text.size));
    }
    if let Some(height) = height {
        style.size.height = length(height.resolve(resolved.text.size));
    }
    Ok((style, resolved))
}

fn apply_declaration(
    style: &mut Style,
    resolved: &mut ResolvedStyle,
    width: &mut Option<zenui_style::Length>,
    height: &mut Option<zenui_style::Length>,
    node: NodeId,
    declaration: StyleDeclaration,
) -> Result<(), SceneError> {
    let (property, value) = (declaration.property(), declaration.value());
    match (property, value) {
        (Property::PaddingX, StyleValue::Points(points)) => {
            let points = points as f32;
            style.padding.left = length(points);
            style.padding.right = length(points);
        }
        (Property::PaddingY, StyleValue::Points(points)) => {
            let points = points as f32;
            style.padding.top = length(points);
            style.padding.bottom = length(points);
        }
        (Property::Gap, StyleValue::Points(points)) => {
            let points = points as f32;
            style.gap.width = length(points);
            style.gap.height = length(points);
        }
        (Property::FlexDirection, StyleValue::Row) => style.flex_direction = FlexDirection::Row,
        (Property::FlexDirection, StyleValue::Column) => {
            style.flex_direction = FlexDirection::Column;
        }
        (Property::FlexWrap, StyleValue::Wrap) => style.flex_wrap = FlexWrap::Wrap,
        (Property::AlignItems, StyleValue::CenterItems) => style.align_items = AlignItems::CENTER,
        (Property::JustifyContent, StyleValue::SpaceBetween) => {
            style.justify_content = JustifyContent::SPACE_BETWEEN;
        }
        (Property::Width, StyleValue::FullWidth) => {
            style.size.width = percent(1.0);
            *width = None;
        }
        (Property::Width, StyleValue::Length(extent)) => *width = Some(extent),
        (Property::Height, StyleValue::Length(extent)) => *height = Some(extent),
        (Property::FlexGrow, StyleValue::Grow) => {
            style.flex_grow = 1.0;
            style.flex_shrink = 1.0;
            style.flex_basis = length(0.0);
        }
        (Property::Background, StyleValue::Color(color)) => resolved.background = Some(color),
        (Property::TextColor, StyleValue::Color(color)) => resolved.text_color = Some(color),
        (Property::BorderColor, StyleValue::Color(color)) => resolved.border_color = Some(color),
        (Property::BorderWidth, StyleValue::Points(points)) => {
            resolved.border_width = Some(points as f32);
        }
        (Property::Radius, StyleValue::Points(points)) => resolved.radius = points as f32,
        (Property::Opacity, StyleValue::Opacity(percent)) => {
            resolved.opacity = f32::from(percent) / 100.0;
        }
        (Property::FontSize, StyleValue::Points(points)) => {
            resolved.text.size = points as f32;
        }
        (Property::FontWeight, StyleValue::Weight(weight)) => {
            resolved.text.weight = weight;
        }
        _ => {
            return Err(SceneError::UnsupportedStyle {
                node,
                property,
                value,
            });
        }
    }
    Ok(())
}

fn root_style(viewport: Viewport) -> Style {
    Style {
        display: Display::Flex,
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::STRETCH,
        justify_content: JustifyContent::FLEX_START,
        gap: TaffySize {
            width: zero(),
            height: length(8.0),
        },
        size: TaffySize {
            width: length(viewport.width()),
            height: auto(),
        },
        min_size: TaffySize {
            width: auto(),
            height: length(viewport.height()),
        },
        ..Style::default()
    }
}

fn container_style(intrinsic: Intrinsic, element: &SnapshotElement) -> Style {
    let role = element.attr("role").and_then(|attr| attr.as_text());
    let row =
        intrinsic == Intrinsic::Form || matches!(role, Some("group" | "toolbar" | "listitem"));
    let main = role == Some("main");
    let gap = if row || main { 8.0 } else { 4.0 };
    Style {
        display: Display::Flex,
        flex_direction: if row {
            FlexDirection::Row
        } else {
            FlexDirection::Column
        },
        align_items: if row {
            AlignItems::CENTER
        } else {
            AlignItems::STRETCH
        },
        justify_content: JustifyContent::FLEX_START,
        padding: if main {
            TaffyRect::length(16.0)
        } else {
            TaffyRect::length(0.0)
        },
        gap: TaffySize {
            width: if row { length(gap) } else { zero() },
            height: if row { zero() } else { length(gap) },
        },
        ..Style::default()
    }
}

fn text_container_style() -> Style {
    Style {
        display: Display::Flex,
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::STRETCH,
        ..Style::default()
    }
}

fn text_leaf_style() -> Style {
    Style {
        flex_shrink: 1.0,
        ..Style::default()
    }
}

fn button_style() -> Style {
    Style {
        display: Display::Flex,
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::CENTER,
        justify_content: JustifyContent::CENTER,
        padding: TaffyRect {
            left: length(12.0),
            right: length(12.0),
            top: length(4.0),
            bottom: length(4.0),
        },
        border: TaffyRect::length(1.0),
        min_size: TaffySize {
            width: auto(),
            height: length(24.0),
        },
        flex_shrink: 0.0,
        ..Style::default()
    }
}

fn checkbox_style() -> Style {
    Style {
        size: TaffySize {
            width: length(16.0),
            height: length(16.0),
        },
        flex_shrink: 0.0,
        ..Style::default()
    }
}

fn input_style() -> Style {
    Style {
        min_size: TaffySize {
            width: length(80.0),
            height: length(24.0),
        },
        padding: TaffyRect {
            left: length(6.0),
            right: length(6.0),
            top: length(4.0),
            bottom: length(4.0),
        },
        border: TaffyRect::length(1.0),
        flex_grow: 1.0,
        flex_basis: auto(),
        ..Style::default()
    }
}

fn layout_error(error: taffy::TaffyError) -> SceneError {
    SceneError::Layout {
        message: error.to_string(),
    }
}
