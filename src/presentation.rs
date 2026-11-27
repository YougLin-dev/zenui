use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use zenui_style::{Length, StyleDeclaration};

use crate::node::{NodeRef, PresentationKey};

/// Presentation snapshot 中一个已提交元素的稳定身份。
///
/// Runtime 身份 + arena 槽位 + generation 共同构成身份；槽位复用会推进
/// generation，因此重新挂载得到的不会是旧身份。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId {
    runtime: u32,
    node: NodeRef,
}

const _: () = assert!(std::mem::size_of::<NodeId>() == 16);

impl NodeId {
    pub(crate) const fn mint(runtime: u32, node: NodeRef) -> Self {
        Self { runtime, node }
    }

    pub(crate) const fn node(self) -> NodeRef {
        self.node
    }

    /// 呈现层使用的公开 key；文本等非元素节点只有 key，没有 `NodeId`。
    pub const fn presentation_key(self) -> PresentationKey {
        PresentationKey::from_node(self.node)
    }

    pub(crate) const fn runtime_id(self) -> u32 {
        self.runtime
    }

    pub(crate) const fn slot(self) -> u32 {
        self.node.slot()
    }

    pub(crate) const fn generation(self) -> u64 {
        self.node.generation() as u64
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeId({})", self.slot())
    }
}

/// 一个 Node ID 相对当前已提交界面的状态。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeStatus {
    /// 该 ID 对应一个已提交元素。
    Live,
    /// 本 Runtime 分配过该 ID，但元素已不在已提交界面中。
    Stale,
    /// 本 Runtime 从未分配过该 ID。
    Unknown,
}

/// 应用外观；影响语义颜色调色板与 `dark:` 条件。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Appearance {
    #[default]
    Light,
    Dark,
}

/// 一棵已提交树的呈现数据根；只在 crate 内可见，对外由
/// [`PresentationSnapshot`] 发布。
///
/// `revision` 是构建快照时该树的 revision。树提交了新的有效变化会分配新
/// revision，旧根不再匹配；没有变化时重复读取共享同一个 `Arc`，旧根在最后
/// 一个引用释放后回收。
#[derive(Debug)]
pub(crate) struct SnapshotRoot {
    pub(crate) roots: Vec<SnapshotNode>,
    pub(crate) focus: Option<NodeId>,
    pub(crate) appearance: Appearance,
    pub(crate) row_bands: Vec<SnapshotRowBand>,
    pub(crate) revision: u64,
}

/// 内容等价；版本语义由 [`PresentationSnapshot`] 的相等性负责。
impl PartialEq for SnapshotRoot {
    fn eq(&self, other: &Self) -> bool {
        self.roots == other.roots
            && self.focus == other.focus
            && self.appearance == other.appearance
            && self.row_bands == other.row_bands
    }
}

impl Eq for SnapshotRoot {}

/// 从已提交界面生成的不可变呈现单位；同一 revision 的快照共享一个数据根。
///
/// 这是唯一对外发布的呈现单位；内部数据根 `SnapshotRoot` 不通过公开 API
/// 暴露。`Clone` 只增加 `Arc` 引用计数。相等性先走 `Arc::ptr_eq` 快路径，
/// 再比较 revision；只有 revision 相同而根不同时才深入比较内容。
#[derive(Clone, Debug)]
pub struct PresentationSnapshot {
    pub(crate) root: Arc<SnapshotRoot>,
}

/// 一个窗口化 `For` 的行带元数据；宿主据此把滚动偏移换算成行窗口。
///
/// `top` 是行带起点处的 spacer 元素，`first_row` 是当前已挂载的起始行，
/// `total_rows` 是完整行数。行高是已声明的外层行槽高度；几何在场景构建
/// 时才解析成逻辑点。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotRowBand {
    pub(crate) key: u64,
    pub(crate) top: NodeId,
    pub(crate) row_height: Length,
    pub(crate) first_row: usize,
    pub(crate) total_rows: usize,
}

impl SnapshotRowBand {
    /// 跨快照稳定的行带键；只对产生它的 `For` 有效。
    pub const fn key(self) -> u64 {
        self.key
    }

    /// 行带内容起点处的 spacer 元素。
    pub const fn top(self) -> NodeId {
        self.top
    }

    /// 声明的统一行槽高度。
    pub const fn row_height(self) -> Length {
        self.row_height
    }

    /// 当前已挂载的起始行。
    pub const fn first_row(self) -> usize {
        self.first_row
    }

    /// 完整行数；与当前挂载窗口无关。
    pub const fn total_rows(self) -> usize {
        self.total_rows
    }
}

impl PresentationSnapshot {
    pub(crate) fn new(
        roots: Vec<SnapshotNode>,
        focus: Option<NodeId>,
        appearance: Appearance,
        row_bands: Vec<SnapshotRowBand>,
        revision: u64,
    ) -> Self {
        Self {
            root: Arc::new(SnapshotRoot {
                roots,
                focus,
                appearance,
                row_bands,
                revision,
            }),
        }
    }

    /// 构建本快照的树 revision；树提交有效变化后推进。
    pub fn revision(&self) -> u64 {
        self.root.revision
    }

    /// 已提交的应用外观。
    pub fn appearance(&self) -> Appearance {
        self.root.appearance
    }

    /// 已提交的根节点，保持挂载顺序。
    pub fn roots(&self) -> &[SnapshotNode] {
        &self.root.roots
    }

    /// 当前焦点元素；没有可聚焦的已提交目标时为 `None`。
    pub fn focus(&self) -> Option<NodeId> {
        self.root.focus
    }

    /// 窗口化 `For` 的行带，保持挂载顺序；没有窗口化 `For` 时为空。
    pub fn row_bands(&self) -> &[SnapshotRowBand] {
        &self.root.row_bands
    }

    /// 先序遍历全部已提交元素。
    pub fn elements(&self) -> impl Iterator<Item = &SnapshotElement> {
        let mut stack: Vec<&SnapshotNode> = self.root.roots.iter().rev().collect();
        std::iter::from_fn(move || {
            while let Some(node) = stack.pop() {
                if let SnapshotNode::Element(element) = node {
                    stack.extend(element.children.iter().rev());
                    return Some(element);
                }
            }
            None
        })
    }

    /// 按 Node ID 查找已提交元素。
    pub fn node(&self, id: NodeId) -> Option<&SnapshotElement> {
        self.elements().find(|element| element.id == id)
    }

    /// 按 `aria:label` 查找已提交元素。
    pub fn find_label(&self, label: &str) -> Option<&SnapshotElement> {
        self.elements()
            .find(|element| element.label() == Some(label))
    }
}

impl PartialEq for PresentationSnapshot {
    fn eq(&self, other: &Self) -> bool {
        // 同一个已发布根的快照直接相等；内容没变时运行时复用同一个 Arc。
        if Arc::ptr_eq(&self.root, &other.root) {
            return true;
        }
        // revision 是全局分配、跨树不撞号的值：不同 revision 的已提交状态
        // 不可能等价，不必深比较。
        if self.root.revision != other.root.revision {
            return false;
        }
        // 相同 revision 但不同根只可能来自防御性构造；退化为内容比较，
        // 保持相等关系的自反与传递语义。
        self.root == other.root
    }
}

impl Eq for PresentationSnapshot {}

/// 已提交界面中的一个节点：带 Node ID 的元素或不携带身份的文本节点。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotNode {
    Element(SnapshotElement),
    Text(String),
}

impl SnapshotNode {
    pub fn as_element(&self) -> Option<&SnapshotElement> {
        match self {
            Self::Element(element) => Some(element),
            Self::Text(_) => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Element(_) => None,
            Self::Text(text) => Some(text),
        }
    }

    pub fn id(&self) -> Option<NodeId> {
        self.as_element().map(SnapshotElement::id)
    }
}

/// 一个已提交元素：稳定身份、tag、呈现属性和已提交子节点顺序。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotElement {
    pub(crate) id: NodeId,
    pub(crate) tag: &'static str,
    pub(crate) attrs: Vec<SnapshotAttr>,
    pub(crate) styles: Cow<'static, [StyleDeclaration]>,
    pub(crate) edit: Option<TextEditState>,
    pub(crate) semantics: SnapshotSemantics,
    pub(crate) children: Vec<SnapshotNode>,
}

impl SnapshotElement {
    pub const fn id(&self) -> NodeId {
        self.id
    }

    pub const fn tag(&self) -> &'static str {
        self.tag
    }

    /// 已解析的呈现属性，保持声明顺序。未解析的属性不出现；
    /// checkbox 例外，其 `checked` 始终反映已提交的本地状态。
    pub fn attrs(&self) -> &[SnapshotAttr] {
        &self.attrs
    }

    /// 类型化样式声明：class 声明保持源码顺序，inline 声明附加在其后，
    /// 由 `origin` 区分。原始 `class` 字符串不会出现在属性或这里。
    pub fn styles(&self) -> &[StyleDeclaration] {
        &self.styles
    }

    pub fn attr(&self, name: &str) -> Option<&SnapshotAttr> {
        self.attrs.iter().find(|attr| attr.name == name)
    }

    /// `value` 属性的文本值。
    pub fn text_value(&self) -> Option<&str> {
        self.attr("value").and_then(SnapshotAttr::as_text)
    }

    /// `aria:label` 属性的文本值。
    pub fn label(&self) -> Option<&str> {
        self.attr("aria:label").and_then(SnapshotAttr::as_text)
    }

    /// `disabled` 是否为真；缺省属性视为未禁用。
    pub fn disabled(&self) -> bool {
        self.attr("disabled").and_then(SnapshotAttr::as_bool) == Some(true)
    }

    /// `checked` 属性的布尔值。
    pub fn checked(&self) -> Option<bool> {
        self.attr("checked").and_then(SnapshotAttr::as_bool)
    }

    /// `aria:pressed` 是否为真；缺省属性视为未按下。
    pub fn pressed(&self) -> bool {
        self.attr("aria:pressed").and_then(SnapshotAttr::as_bool) == Some(true)
    }

    /// `checked` 或 `aria:pressed` 的布尔状态；两者都缺失时为 `None`。
    ///
    /// 复选框的 `checked` 优先于 `aria:pressed`。UIA Toggle pattern 的存在性
    /// 与切换目标都使用这一个定义。
    pub fn toggle_state(&self) -> Option<bool> {
        self.checked()
            .or_else(|| self.attr("aria:pressed").and_then(SnapshotAttr::as_bool))
    }

    /// 文本输入控件的本地编辑状态；其他元素为 `None`。
    pub const fn edit(&self) -> Option<&TextEditState> {
        self.edit.as_ref()
    }

    /// 已提交子节点，保持呈现顺序。
    pub fn children(&self) -> &[SnapshotNode] {
        &self.children
    }

    /// 直接文本子节点的顺序拼接。
    pub fn text(&self) -> String {
        let mut text = String::new();
        for child in &self.children {
            if let SnapshotNode::Text(value) = child {
                text.push_str(value);
            }
        }
        text
    }

    /// 语义投影：键盘可聚焦性与已挂载的用户动作。
    ///
    /// `focusable` 不考虑 `disabled`，也不表达可见性；未挂载、被过滤或不在
    /// 行窗口内的元素不出现在快照中。消费者把本值与 [`Self::disabled`]
    /// 组合计算实际的可聚焦性。
    pub const fn semantics(&self) -> SnapshotSemantics {
        self.semantics
    }
}

/// 一个已提交元素的语义投影：固有键盘可聚焦性与已挂载的用户动作。
///
/// 这是 UIA 需要的运行时时序投影，不包含平台 provider。`focusable` 只表示
/// 元素的种类或 focus/blur 处理器允许键盘聚焦；`disabled` 与可见性仍是独立
/// 状态，由消费者组合。新增动作扩展 [`SnapshotAction`]，不向
/// [`SnapshotElement`] 逐项添加布尔字段。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnapshotSemantics {
    focusable: bool,
    actions: SnapshotActions,
}

impl SnapshotSemantics {
    pub(crate) const fn new(focusable: bool, actions: SnapshotActions) -> Self {
        Self { focusable, actions }
    }

    /// 元素是否具有键盘可聚焦的固有语义，忽略 `disabled` 与可见性。
    pub const fn focusable(&self) -> bool {
        self.focusable
    }

    /// 已挂载的用户动作集合。
    pub const fn actions(&self) -> SnapshotActions {
        self.actions
    }

    /// 指定动作是否有已挂载的处理器。
    pub const fn has_action(&self, action: SnapshotAction) -> bool {
        self.actions.contains(action)
    }
}

/// 快照可能报告的用户动作；新增动作扩展此枚举和 [`SnapshotActions`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SnapshotAction {
    Click,
    Change,
    Input,
}

impl SnapshotAction {
    const fn bit(self) -> u8 {
        match self {
            Self::Click => 1 << 0,
            Self::Change => 1 << 1,
            Self::Input => 1 << 2,
        }
    }
}

/// 一个元素上已挂载的用户动作集合。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnapshotActions(u8);

impl SnapshotActions {
    pub(crate) fn insert(&mut self, action: SnapshotAction) {
        self.0 |= action.bit();
    }

    /// 集合是否为空。
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// 指定动作是否在集合中。
    pub const fn contains(self, action: SnapshotAction) -> bool {
        self.0 & action.bit() != 0
    }

    /// 按固定顺序遍历已挂载的动作。
    pub fn iter(self) -> impl Iterator<Item = SnapshotAction> {
        [
            SnapshotAction::Click,
            SnapshotAction::Change,
            SnapshotAction::Input,
        ]
        .into_iter()
        .filter(move |action| self.contains(*action))
    }
}

/// 文本输入控件的本地编辑状态；caret 与 selection 以字符数计。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEditState {
    pub(crate) caret: usize,
    pub(crate) anchor: usize,
    pub(crate) selection: Option<(usize, usize)>,
    pub(crate) preedit: Option<String>,
    pub(crate) composition: Option<(usize, usize)>,
}

impl TextEditState {
    /// 选择的活动端（焦点）；折叠时与 `anchor` 相同。
    pub const fn caret(&self) -> usize {
        self.caret
    }

    /// 选择的活动端（焦点）；折叠时等于 `caret`。Shift 扩展选择时另一端保持不动。
    pub const fn anchor(&self) -> usize {
        self.anchor
    }

    /// 归一化的选择区间，未选择时为 `None`；`set_caret` 折叠为 `(offset, offset)`。
    pub const fn selection(&self) -> Option<(usize, usize)> {
        self.selection
    }

    /// 组合中的 preedit 文本；未组合时为 `None`。
    pub fn preedit(&self) -> Option<&str> {
        self.preedit.as_deref()
    }

    /// 组合替换的字符区间（`start..end`，按当前值计）；未组合时为 `None`。
    /// 提交时会用 preedit 替换该区间。
    pub const fn composition(&self) -> Option<(usize, usize)> {
        self.composition
    }
}

/// 一个已解析的呈现属性。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotAttr {
    pub(crate) name: &'static str,
    pub(crate) value: SnapshotValue,
}

impl SnapshotAttr {
    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn value(&self) -> &SnapshotValue {
        &self.value
    }

    pub fn as_text(&self) -> Option<&str> {
        self.value.as_text()
    }

    pub fn as_bool(&self) -> Option<bool> {
        self.value.as_bool()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotValue {
    Text(String),
    Bool(bool),
}

impl SnapshotValue {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Bool(_) => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            Self::Text(_) => None,
        }
    }
}

/// 便于人工检查的确定性树形打印；`#n` 是挂载顺序编号，只用于阅读，
/// 不是可用于派发的 Node ID。
impl fmt::Display for PresentationSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.root.focus {
            Some(id) => writeln!(f, "focus=#{}", id.slot())?,
            None => writeln!(f, "focus=-")?,
        }
        for node in &self.root.roots {
            write_node(f, node, 0)?;
        }
        Ok(())
    }
}

fn write_node(f: &mut fmt::Formatter<'_>, node: &SnapshotNode, depth: usize) -> fmt::Result {
    for _ in 0..depth {
        f.write_str("  ")?;
    }
    match node {
        SnapshotNode::Text(text) => writeln!(f, "{text:?}"),
        SnapshotNode::Element(element) => {
            write!(f, "{}#{}", element.tag, element.id.slot())?;
            for attr in &element.attrs {
                match &attr.value {
                    SnapshotValue::Text(text) => write!(f, " {}={text:?}", attr.name)?,
                    SnapshotValue::Bool(value) => write!(f, " {}={value}", attr.name)?,
                }
            }
            if let Some(edit) = &element.edit {
                write!(f, " caret={}", edit.caret)?;
                if let Some((start, end)) = edit.selection {
                    write!(f, " selection={start}..{end}")?;
                }
                if let Some(preedit) = &edit.preedit {
                    write!(f, " preedit={preedit:?}")?;
                }
            }
            writeln!(f)?;
            for child in &element.children {
                write_node(f, child, depth + 1)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::{
        Appearance, NodeId, PresentationSnapshot, SnapshotAction, SnapshotActions, SnapshotAttr,
        SnapshotElement, SnapshotNode, SnapshotSemantics, SnapshotValue,
    };
    use crate::node::NodeRef;

    fn element(semantics: SnapshotSemantics) -> SnapshotElement {
        SnapshotElement {
            id: NodeId::mint(1, NodeRef::new(0, 1).expect("test node")),
            tag: "view",
            attrs: Vec::new(),
            styles: Cow::Borrowed(&[]),
            edit: None,
            semantics,
            children: Vec::new(),
        }
    }

    #[test]
    fn action_sets_compose_and_iterate_in_a_fixed_order() {
        let mut actions = SnapshotActions::default();
        assert!(actions.is_empty());
        actions.insert(SnapshotAction::Input);
        actions.insert(SnapshotAction::Click);
        assert!(!actions.is_empty());
        assert!(actions.contains(SnapshotAction::Click));
        assert!(actions.contains(SnapshotAction::Input));
        assert!(!actions.contains(SnapshotAction::Change));
        assert_eq!(
            actions.iter().collect::<Vec<_>>(),
            vec![SnapshotAction::Click, SnapshotAction::Input]
        );
    }

    #[test]
    fn snapshot_equality_includes_the_semantic_projection() {
        let plain = element(SnapshotSemantics::new(false, SnapshotActions::default()));
        let same = element(SnapshotSemantics::new(false, SnapshotActions::default()));
        let focusable = element(SnapshotSemantics::new(true, SnapshotActions::default()));
        let mut actions = SnapshotActions::default();
        actions.insert(SnapshotAction::Click);
        let clickable = element(SnapshotSemantics::new(true, actions));

        assert_eq!(plain, same, "identical projections stay equal");
        assert_ne!(plain, focusable, "focusability contributes to equality");
        assert_ne!(
            focusable, clickable,
            "mounted actions contribute to equality"
        );
    }

    #[test]
    fn toggle_state_prefers_checked_and_falls_back_to_pressed() {
        let mut element = element(SnapshotSemantics::default());
        assert_eq!(element.toggle_state(), None);
        element.attrs.push(SnapshotAttr {
            name: "aria:pressed",
            value: SnapshotValue::Bool(true),
        });
        assert_eq!(element.toggle_state(), Some(true));
        element.attrs.push(SnapshotAttr {
            name: "checked",
            value: SnapshotValue::Bool(false),
        });
        assert_eq!(
            element.toggle_state(),
            Some(false),
            "checked wins over aria:pressed"
        );
    }

    fn snapshot(revision: u64, text: &str) -> PresentationSnapshot {
        PresentationSnapshot::new(
            vec![SnapshotNode::Text(text.to_owned())],
            None,
            Appearance::Light,
            Vec::new(),
            revision,
        )
    }

    #[test]
    fn snapshots_with_the_same_revision_and_content_are_equal() {
        let first = snapshot(7, "same");
        assert_eq!(first, first.clone(), "clones share the published root");
        assert_eq!(
            first,
            snapshot(7, "same"),
            "equal revisions compare content"
        );
    }

    #[test]
    fn a_different_revision_is_not_equal_even_with_equal_content() {
        assert_ne!(
            snapshot(7, "same"),
            snapshot(8, "same"),
            "revision alone decides inequality before any deep comparison"
        );
    }

    #[test]
    fn equal_revisions_still_compare_content_defensively() {
        assert_ne!(
            snapshot(7, "left"),
            snapshot(7, "right"),
            "independent roots at one revision cannot be assumed equal"
        );
    }
}
