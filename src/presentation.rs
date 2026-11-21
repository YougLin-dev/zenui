use std::borrow::Cow;
use std::fmt;

use zenui_style::StyleDeclaration;

/// Presentation snapshot 中一个已提交元素的稳定身份。
///
/// Slot 在挂载时分配且不复用；generation 是这次元素挂载的序号。两者连同来源
/// Runtime 共同构成身份，因此重新挂载得到的不会是旧身份。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId {
    runtime: u32,
    slot: u32,
    generation: u64,
}

impl NodeId {
    pub(crate) const fn mint(runtime: u32, slot: u32, generation: u64) -> Self {
        Self {
            runtime,
            slot,
            generation,
        }
    }

    pub(crate) const fn runtime_id(self) -> u32 {
        self.runtime
    }

    pub(crate) const fn slot(self) -> u32 {
        self.slot
    }

    pub(crate) const fn generation(self) -> u64 {
        self.generation
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeId({})", self.slot)
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

/// 从已提交界面生成的一次性只读描述；不包含几何、闭包或平台句柄。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentationSnapshot {
    pub(crate) roots: Vec<SnapshotNode>,
    pub(crate) focus: Option<NodeId>,
}

impl PresentationSnapshot {
    /// 已提交的根节点，保持挂载顺序。
    pub fn roots(&self) -> &[SnapshotNode] {
        &self.roots
    }

    /// 当前焦点元素；没有可聚焦的已提交目标时为 `None`。
    pub const fn focus(&self) -> Option<NodeId> {
        self.focus
    }

    /// 先序遍历全部已提交元素。
    pub fn elements(&self) -> impl Iterator<Item = &SnapshotElement> {
        let mut stack: Vec<&SnapshotNode> = self.roots.iter().rev().collect();
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
        match self.focus {
            Some(id) => writeln!(f, "focus=#{}", id.slot)?,
            None => writeln!(f, "focus=-")?,
        }
        for node in &self.roots {
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
