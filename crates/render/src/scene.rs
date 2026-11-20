//! 从 presentation snapshot 构建平台无关的场景。

use std::collections::HashMap;

use taffy::prelude::{
    AlignItems, AvailableSpace, Display, FlexDirection, JustifyContent, Style, TaffyTree, auto,
    length, zero,
};
use taffy::{
    Baselines, LayoutInput, LayoutOutput, NodeId as TaffyNodeId, Rect as TaffyRect, ResolveOrZero,
    Size as TaffySize, compute_leaf_layout,
};
use zenui::{NodeId, PresentationSnapshot, SnapshotElement, SnapshotNode};

use crate::SceneError;
use crate::geometry::{Point, Rect, Viewport};
use crate::text::{self, CharRange, TextShaper};

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

/// 一个已提交元素的绝对逻辑几何与内建视觉。
#[derive(Clone, Debug)]
pub struct SceneElement {
    id: NodeId,
    tag: &'static str,
    rect: Rect,
    visual: Visual,
    disabled: bool,
    focused: bool,
    text: Vec<TextRun>,
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

    /// 该元素在快照中是否 disabled。
    pub const fn disabled(&self) -> bool {
        self.disabled
    }

    /// 该元素是否持有快照焦点。
    pub const fn focused(&self) -> bool {
        self.focused
    }

    /// 绘制在该元素盒内的文本 run，包含按钮标题等裸文本子节点。
    pub fn text(&self) -> &[TextRun] {
        &self.text
    }
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

/// 一次完整重排的结果；几何都是逻辑像素与绝对坐标。
#[derive(Clone, Debug)]
pub struct Scene {
    viewport: Viewport,
    elements: Vec<SceneElement>,
    decorations: Vec<Decoration>,
}

impl Scene {
    /// 完整重排一次快照。每次调用都重新构建 Taffy 树，Node ID 原样保留。
    pub fn build(snapshot: &PresentationSnapshot, viewport: Viewport) -> Result<Self, SceneError> {
        Builder::new(snapshot).build(viewport)
    }

    /// 构建该场景的 viewport；scale 未参与布局。
    pub const fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// 已提交元素，按绘制顺序（先序）。
    pub fn elements(&self) -> &[SceneElement] {
        &self.elements
    }

    /// 按 Node ID 查找场景元素。
    pub fn node(&self, id: NodeId) -> Option<&SceneElement> {
        self.elements.iter().find(|element| element.id == id)
    }

    /// 最上层命中：后绘制的元素优先，半开区间。
    pub fn hit_test(&self, point: Point) -> Option<NodeId> {
        self.elements
            .iter()
            .rev()
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
    layout: parley::Layout<()>,
    origin: Point,
}

struct Builder<'a> {
    snapshot: &'a PresentationSnapshot,
    tree: TaffyTree<TextContext>,
    kinds: HashMap<TaffyNodeId, Kind>,
}

impl<'a> Builder<'a> {
    fn new(snapshot: &'a PresentationSnapshot) -> Self {
        Self {
            snapshot,
            tree: TaffyTree::new(),
            kinds: HashMap::new(),
        }
    }

    fn build(mut self, viewport: Viewport) -> Result<Scene, SceneError> {
        let mut shaper = TextShaper::new();
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
                measure_leaf(&mut shaper, inputs, context, style)
            })
            .map_err(layout_error)?;

        let mut scene = Scene {
            viewport,
            elements: Vec::new(),
            decorations: Vec::new(),
        };
        self.emit(&mut shaper, root, Point::ZERO, &mut scene)?;
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

        let node = match intrinsic {
            Intrinsic::TextInput => {
                let (display, _) = display_text(element);
                self.tree
                    .new_leaf_with_context(
                        with_grow(input_style(), top_level),
                        TextContext { text: display },
                    )
                    .map_err(layout_error)?
            }
            Intrinsic::Checkbox => self
                .tree
                .new_leaf(with_grow(checkbox_style(), top_level))
                .map_err(layout_error)?,
            Intrinsic::Button => self.container(element, with_grow(button_style(), top_level))?,
            Intrinsic::Text => {
                self.container(element, with_grow(text_container_style(), top_level))?
            }
            Intrinsic::View | Intrinsic::Form => self.container(
                element,
                with_grow(container_style(intrinsic, element), top_level),
            )?,
        };
        self.kinds.insert(node, Kind::Element(element.id()));
        Ok(node)
    }

    fn container(
        &mut self,
        element: &SnapshotElement,
        style: Style,
    ) -> Result<TaffyNodeId, SceneError> {
        let mut children = Vec::new();
        for child in element.children() {
            match child {
                SnapshotNode::Element(element) => {
                    children.push(self.build_element(element, false)?)
                }
                SnapshotNode::Text(text) => children.push(self.build_text(text)?),
            }
        }
        self.tree
            .new_with_children(style, &children)
            .map_err(layout_error)
    }

    fn build_text(&mut self, text: &str) -> Result<TaffyNodeId, SceneError> {
        let node = self
            .tree
            .new_leaf_with_context(
                text_leaf_style(),
                TextContext {
                    text: text.to_owned(),
                },
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
                    self.emit(shaper, child, Point::new(rect.x, rect.y), scene)?;
                }
            }
            Kind::Element(id) => {
                let element = self.snapshot.node(id).ok_or_else(|| SceneError::Layout {
                    message: String::from("the snapshot lost a laid-out element"),
                })?;
                let children = self.tree.children(node).map_err(layout_error)?;
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
                    let context =
                        self.tree
                            .get_node_context(*child)
                            .ok_or_else(|| SceneError::Layout {
                                message: String::from("a text leaf has no content"),
                            })?;
                    let shaped = shaper.shape(&context.text, child_rect.width);
                    runs.extend(text::runs(&shaped, Point::new(child_rect.x, child_rect.y)));
                }

                let intrinsic = Intrinsic::from_tag(element.tag()).ok_or_else(|| {
                    SceneError::UnsupportedTag {
                        node: id,
                        tag: element.tag(),
                    }
                })?;
                if intrinsic == Intrinsic::TextInput {
                    let (content, width) = content_box(rect, layout);
                    let (display, preedit) = display_text(element);
                    let layout = shaper.shape(&display, width);
                    let view = EditView {
                        element,
                        display,
                        preedit,
                        layout,
                        origin: content,
                    };
                    runs.extend(text::runs(&view.layout, content));
                    add_edit_decorations(&view, &mut scene.decorations);
                }

                let focused = self.snapshot.focus() == Some(id);
                scene.elements.push(SceneElement {
                    id,
                    tag: element.tag(),
                    rect,
                    visual: visual(intrinsic, element),
                    disabled: element.disabled(),
                    focused,
                    text: runs,
                });

                for child in children {
                    if self.kinds.get(&child) != Some(&Kind::Text) {
                        self.emit(shaper, child, Point::new(rect.x, rect.y), scene)?;
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
    let (size, baseline) = shaper.measure(
        &context.text,
        inputs.known_dimensions,
        inputs.available_space,
    );
    let mut output = compute_leaf_layout(inputs, style, |_, _| 0.0, |_, _| size);
    let padding = style
        .padding
        .resolve_or_zero(inputs.parent_size.width, |_, _| 0.0);
    let border = style
        .border
        .resolve_or_zero(inputs.parent_size.width, |_, _| 0.0);
    output.baselines = Baselines::from_first(Some(baseline + padding.top + border.top));
    output
}

fn add_edit_decorations(view: &EditView<'_>, out: &mut Vec<Decoration>) {
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
    out.push(Decoration {
        node: view.element.id(),
        kind: DecorationKind::Caret,
        rect: text::caret(&view.layout, view.origin, &view.display, edit.caret()),
    });
}

/// 组合中的显示文本在 value 的 base range 插入 preedit；运行时让 caret
/// 落在 preedit 末尾，因此起点由 caret 反推。
fn display_text(element: &SnapshotElement) -> (String, Option<CharRange>) {
    let value = element.text_value().unwrap_or("");
    let Some(edit) = element.edit() else {
        return (value.to_owned(), None);
    };
    let Some(preedit) = edit.preedit() else {
        return (value.to_owned(), None);
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
    (display, Some(CharRange::new(start, start + preedit_chars)))
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
            pressed: element
                .attr("aria:pressed")
                .and_then(|attr| attr.as_bool())
                .unwrap_or(false),
        },
        Intrinsic::Checkbox => Visual::Checkbox {
            checked: element.checked().unwrap_or(false),
        },
        Intrinsic::TextInput => Visual::TextInput,
        Intrinsic::View | Intrinsic::Form => Visual::Container,
    }
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
