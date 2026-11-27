use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

use zenui_core::model::{ChangeSet, FilterId, Row, RowId, StoreId};
use zenui_core::{Counters, PanicInfo, RowSet, catch_panic};
use zenui_style::{Color, Length, Opacity, Property, StyleValue};

use crate::context::{self, ContextFault, DepKey, FieldKind, Phase, with_state};
use crate::dispatch::{DispatchError, NodeOperation};
use crate::menu::{
    CommandError, CommandEvent, CommandHandler, CommandSource, MenuCommand, MenuCommandState,
    MenuSpec, MenuStateError, MenuStateHandler,
};
use crate::node::{
    AttrListId, AuxHandles, BindingList, BindingListId, BindingPart, BindingPatchLog,
    BindingPatchMark, BindingRef, BindingTable, BindingType, BindingValue, EditStateId,
    ElementData, ElementFlags, EventListId, NO_PAYLOAD, NodeKindTag, NodeRef, PartId, Pool,
    PresentationKey, ReadSlot, SemanticsId, SlotArena, StyleListId, TagId, TreeKey,
};
use crate::presentation::{
    Appearance, NodeId, NodeStatus, PresentationDelta, PresentationSnapshot, PresentationUpdate,
    SnapshotAction, SnapshotActions, SnapshotAttr, SnapshotElement, SnapshotNode, SnapshotRowBand,
    SnapshotSemantics, SnapshotText, SnapshotValue, TextEditState,
};
use crate::session::{SessionAction, SessionCtx, SessionError, SessionPolicy};
use crate::signal::{MemoCell, SignalCell};
use crate::store::{CachedRowSet, TodoId, TodoRow, WriteError};
use crate::task::{TaskRecord, WakeCallback};
use crate::text_edit::{self, Edit, Movement};
use crate::tray::{TrayError, TraySpec};
use crate::view::{
    Attr, AttrValue, BlurEvent, ChildrenFactory, CompositionEvent, CompositionPhase, ErasedRowView,
    ErasedStore, Event, FocusCause, FocusEvent, ForDriver, ForRows, HandlerError, HandlerOutcome,
    InlineStyle, Key, KeyEvent, MountError, MountPlan, MountResult, PlanNode, ReadFault, ReadValue,
    StyleSlot, SubmitEvent,
};
use crate::window::{
    ActivationId, CatalogEntry, CatalogError, CloseError, CloseEvent, CloseHandler, CloseOutcome,
    ClosePolicy, CloseReply, CloseRequest, CloseSource, DropError, DropEvent, DropHandler,
    WindowAction, WindowCatalog, WindowCtx, WindowError, WindowInfo, WindowRecord, WindowRequest,
    WindowRole,
};

/// 路由型输入的来源；指针、键盘、菜单等共享同一套目标校验与错误通道。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventSource {
    Pointer,
    Keyboard,
    Menu,
    Tray,
    Accessibility,
    Window,
}

/// 一次 Intent 的目标；窗口目标携带打开时的 activation。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EventTarget {
    Node(NodeId),
    Window(WindowCtx),
    App,
}

impl EventTarget {
    pub const fn node(self) -> Option<NodeId> {
        match self {
            Self::Node(node) => Some(node),
            Self::Window(_) | Self::App => None,
        }
    }

    pub const fn window(self) -> Option<WindowCtx> {
        match self {
            Self::Window(window) => Some(window),
            Self::Node(_) | Self::App => None,
        }
    }
}

impl fmt::Debug for EventTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Node(node) => write!(f, "Node({node:?})"),
            Self::Window(window) => write!(f, "Window({:?})", window.key),
            Self::App => f.write_str("App"),
        }
    }
}

/// 类型化载荷；后续切片的键盘、菜单等 intent 在此扩展。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentPayload {
    Click,
    Activate,
    /// 指针按下；宿主已算好 text-input 的字符偏移。
    Press {
        caret: Option<usize>,
    },
    KeyDown {
        key: Key,
        modifiers: Modifiers,
    },
}

/// 键盘修饰键快照；指针点击当前只带默认值。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    shift: bool,
    control: bool,
    alt: bool,
    meta: bool,
}

impl Modifiers {
    pub const fn new(shift: bool, control: bool, alt: bool, meta: bool) -> Self {
        Self {
            shift,
            control,
            alt,
            meta,
        }
    }

    pub const fn shift(self) -> bool {
        self.shift
    }

    pub const fn control(self) -> bool {
        self.control
    }

    pub const fn alt(self) -> bool {
        self.alt
    }

    pub const fn meta(self) -> bool {
        self.meta
    }
}

/// 一个路由型用户意图。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Intent {
    source: EventSource,
    target: EventTarget,
    payload: IntentPayload,
    modifiers: Modifiers,
}

impl Intent {
    pub const fn new(
        source: EventSource,
        target: EventTarget,
        payload: IntentPayload,
        modifiers: Modifiers,
    ) -> Self {
        Self {
            source,
            target,
            payload,
            modifiers,
        }
    }

    /// 指针点击一个元素；路由时使用 closest-wins。
    pub const fn click(target: NodeId) -> Self {
        Self::new(
            EventSource::Pointer,
            EventTarget::Node(target),
            IntentPayload::Click,
            Modifiers::new(false, false, false, false),
        )
    }

    /// 激活一个窗口；携带的 activation 在派发前校验。
    pub const fn activate_window(window: WindowCtx) -> Self {
        Self::new(
            EventSource::Window,
            EventTarget::Window(window),
            IntentPayload::Activate,
            Modifiers::new(false, false, false, false),
        )
    }

    pub const fn activate_app() -> Self {
        Self::new(
            EventSource::Window,
            EventTarget::App,
            IntentPayload::Activate,
            Modifiers::new(false, false, false, false),
        )
    }

    /// 指针按下；`caret` 是宿主从场景算出的字符偏移，runtime 不依赖 render。
    pub const fn press(target: NodeId, caret: Option<usize>) -> Self {
        Self::new(
            EventSource::Pointer,
            EventTarget::Node(target),
            IntentPayload::Press { caret },
            Modifiers::new(false, false, false, false),
        )
    }

    /// 一个节点 keydown；target 通常是焦点节点。
    pub const fn keydown(target: NodeId, key: Key, modifiers: Modifiers) -> Self {
        Self::new(
            EventSource::Keyboard,
            EventTarget::Node(target),
            IntentPayload::KeyDown { key, modifiers },
            modifiers,
        )
    }

    /// 没有节点焦点时的窗口根 keydown。
    pub const fn keydown_window(window: WindowCtx, key: Key, modifiers: Modifiers) -> Self {
        Self::new(
            EventSource::Keyboard,
            EventTarget::Window(window),
            IntentPayload::KeyDown { key, modifiers },
            modifiers,
        )
    }

    pub const fn source(self) -> EventSource {
        self.source
    }

    pub const fn target(self) -> EventTarget {
        self.target
    }

    pub const fn payload(self) -> IntentPayload {
        self.payload
    }

    pub const fn modifiers(self) -> Modifiers {
        self.modifiers
    }
}

/// 一次 turn 中可能改变显露目标的三类位置变化。
///
/// 宿主在 turn 结束后据此置位待决显露；显式滚动仍会清除该请求。标志描述的是
/// 变化本身，不表示目标当前是否在视口内。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RevealFlags {
    focus_changed: bool,
    caret_changed: bool,
    composition_changed: bool,
}

impl RevealFlags {
    /// 焦点节点发生了变化。
    pub const fn focus_changed(self) -> bool {
        self.focus_changed
    }

    /// 聚焦输入的 caret 或 selection 发生了变化。
    pub const fn caret_changed(self) -> bool {
        self.caret_changed
    }

    /// 组合的 preedit 发生了变化。
    pub const fn composition_changed(self) -> bool {
        self.composition_changed
    }

    /// 任一位置变化需要宿主执行显露。
    pub const fn needs_reveal(self) -> bool {
        self.focus_changed || self.caret_changed || self.composition_changed
    }

    fn merge(&mut self, other: Self) {
        self.focus_changed |= other.focus_changed;
        self.caret_changed |= other.caret_changed;
        self.composition_changed |= other.composition_changed;
    }
}

/// 一棵树中与显露有关的焦点、caret/selection 与 preedit 指纹。
///
/// turn 开始与结束时各取一次；两次之间的差异就是该 turn 的 [`RevealFlags`]。
#[derive(Clone, PartialEq, Eq)]
struct RevealProbe {
    focused: Option<NodeId>,
    caret: usize,
    anchor: usize,
    preedit: Option<String>,
}

impl RevealProbe {
    fn diff(before: &Self, after: &Self) -> RevealFlags {
        let focus_changed = before.focused != after.focused;
        // 焦点换了输入时 caret 数值没有可比性，只报告焦点变化。选择区间由
        // caret/anchor 派生，不单独比较存储编码：折叠选择的 `None` 与
        // `Some((c, c))` 表示同一位置。
        let caret_changed =
            !focus_changed && (before.caret != after.caret || before.anchor != after.anchor);
        RevealFlags {
            focus_changed,
            caret_changed,
            composition_changed: before.preedit != after.preedit,
        }
    }
}

/// turn 结束时把初始指纹与当前树的差异写进报告。
fn finish_reveal(report: &mut TurnReport, tree: Option<TreeSlot>, before: Option<&RevealProbe>) {
    if let (Some(tree), Some(before)) = (tree, before) {
        let after = with_state(|state| state.reveal_probe(tree));
        report.set_reveal(RevealProbe::diff(before, &after));
    }
}

/// 一次 turn 的 flow 结果。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TurnOutcome {
    consumed: bool,
    prevent_default: bool,
}

impl TurnOutcome {
    pub const fn consumed(self) -> bool {
        self.consumed
    }

    pub const fn prevent_default(self) -> bool {
        self.prevent_default
    }
}

/// turn 内出现的用户错误；写入已保留，错误不静默丢弃。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TurnError {
    HandlerFailed(HandlerError),
    HandlerPanicked(PanicInfo),
}

impl fmt::Display for TurnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HandlerFailed(error) => write!(f, "handler failed: {error}"),
            Self::HandlerPanicked(panic) => write!(f, "handler panicked: {panic}"),
        }
    }
}

impl std::error::Error for TurnError {}

/// 一次用户输入 turn 的报告；宿主据此决定是否执行原生默认动作。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TurnReport {
    outcome: TurnOutcome,
    errors: Vec<TurnError>,
    reveal: RevealFlags,
    changed: bool,
}

impl TurnReport {
    pub const fn outcome(&self) -> TurnOutcome {
        self.outcome
    }

    pub fn errors(&self) -> &[TurnError] {
        &self.errors
    }

    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// 本 turn 中焦点、caret/selection 与组合的变化；宿主据此申请显露。
    pub const fn reveal(&self) -> RevealFlags {
        self.reveal
    }

    pub const fn changed(&self) -> bool {
        self.changed
    }

    fn set_reveal(&mut self, reveal: RevealFlags) {
        self.reveal = reveal;
    }

    fn push_error(&mut self, error: TurnError) {
        self.errors.push(error);
    }

    fn set_outcome(&mut self, consumed: bool, prevent_default: bool) {
        self.outcome = TurnOutcome {
            consumed,
            prevent_default,
        };
    }

    fn mark_changed(&mut self) {
        self.changed = true;
    }

    /// 合并一个嵌套默认动作的报告：错误、flow、显露标志与 changed 都要回到外层 turn。
    fn absorb(&mut self, other: Self) {
        self.errors.extend(other.errors);
        self.changed |= other.changed;
        self.reveal.merge(other.reveal);
        self.outcome.consumed |= other.outcome.consumed;
        self.outcome.prevent_default |= other.outcome.prevent_default;
    }
}

const HIDDEN_TARGET: &str = "target window is hidden";
const SUSPENDED_TARGET: &str = "target window input is suspended";

pub(crate) enum Resource {
    Signal(u32),
    Memo(u32),
    Event(usize),
    /// 随 owner 释放的 arena 节点；保证文本/Show/For 等没有元素槽的节点也能回收。
    Node(NodeRef),
}

/// 窗口根 keydown 处理器；调用时从 state 克隆 Rc，避免持有 State 借用。
type WindowKeyDownHandler = Rc<RefCell<Box<dyn FnMut(&mut KeyEvent) -> HandlerOutcome>>>;

pub(crate) struct OwnerRecord {
    parent: Option<u32>,
    children: Vec<u32>,
    resources: Vec<Resource>,
    /// 本 owner 直接创建的挂载元素槽位；析构时清空并归还。
    elements: Vec<usize>,
    /// 本 owner 直接创建的组件槽位；析构时清空并归还。
    components: Vec<usize>,
    disposed: bool,
}

/// 统一 UI 节点 arena：节点身份、元素 payload、tag 注册表与辅助列表。
pub(crate) struct NodeArena {
    slots: SlotArena,
    next_tree: u32,
    tags: Vec<&'static str>,
    tag_ids: BTreeMap<&'static str, TagId>,
    /// 与元素槽位一一对应；NodeSlot.payload 就是这里的下标。
    elements: Vec<Option<ElementData>>,
    texts: Pool<TextData>,
    shows: Pool<ShowData>,
    forces: Pool<ForData>,
    aux: AuxArenas,
    /// 每棵树一份绑定槽表；树关闭后随最后一个节点释放而丢弃。
    binding_tables: BTreeMap<TreeKey, BindingTable>,
}

impl NodeArena {
    fn new() -> Self {
        Self {
            slots: SlotArena::new(),
            next_tree: 1,
            tags: Vec::new(),
            tag_ids: BTreeMap::new(),
            elements: Vec::new(),
            texts: Pool::new(),
            shows: Pool::new(),
            forces: Pool::new(),
            aux: AuxArenas::new(),
            binding_tables: BTreeMap::new(),
        }
    }

    fn binding_table(&self, tree: TreeKey) -> Option<&BindingTable> {
        self.binding_tables.get(&tree)
    }

    fn binding_table_mut(&mut self, tree: TreeKey) -> &mut BindingTable {
        self.binding_tables
            .entry(tree)
            .or_insert_with(BindingTable::new)
    }

    /// 树的全部节点释放后丢弃绑定表；重新打开同一个树槽位会建立空表。
    fn drop_binding_table(&mut self, tree: TreeKey) {
        if let Some(table) = self.binding_tables.remove(&tree) {
            debug_assert_eq!(
                table.live(),
                0,
                "a retired tree releases every binding slot"
            );
        }
    }

    fn tree_of(&self, node: NodeRef) -> Option<TreeKey> {
        self.slots.get(node).ok().map(|slot| slot.tree)
    }

    fn binding_list_mut(&mut self, id: BindingListId) -> &mut BindingList {
        self.aux
            .binding_lists
            .get_mut(id.raw())
            .expect("element binding-list handles stay live until the element is retired")
    }

    /// 释放一个直接引用；绑定表已随树丢弃时是静默无操作。
    fn release_binding(&mut self, tree: TreeKey, reference: BindingRef) {
        if let Some(table) = self.binding_tables.get_mut(&tree) {
            let released = table.release(reference);
            debug_assert!(released, "a mounted binding slot is live");
        }
    }

    /// 释放元素的整张绑定列表和其中每个槽，列表本身由 `AuxArenas` 归还。
    fn release_binding_list(&mut self, tree: TreeKey, id: BindingListId) {
        let Some(list) = self.aux.binding_lists.get(id.raw()) else {
            return;
        };
        if let Some(table) = self.binding_tables.get_mut(&tree) {
            for (_, reference) in list.iter() {
                let released = table.release(reference);
                debug_assert!(released, "a mounted element binding slot is live");
            }
        }
    }

    /// 分配一棵树的稳定键；耗尽时是 fatal 开发期错误。
    fn allocate_tree(&mut self) -> TreeKey {
        let key = TreeKey::from_raw(self.next_tree);
        self.next_tree = self
            .next_tree
            .checked_add(1)
            .expect("tree key space exhausted");
        key
    }

    fn insert(
        &mut self,
        kind: NodeKindTag,
        payload: u32,
        tree: TreeKey,
        parent: Option<NodeRef>,
    ) -> Result<NodeRef, crate::node::NodeError> {
        self.slots.insert(kind, payload, tree, parent)
    }

    fn get(&self, node: NodeRef) -> Result<&crate::node::NodeSlot, crate::node::NodeError> {
        self.slots.get(node)
    }

    fn require_kind(
        &self,
        node: NodeRef,
        kind: NodeKindTag,
    ) -> Result<&crate::node::NodeSlot, crate::node::NodeError> {
        self.slots.require_kind(node, kind)
    }

    fn set_payload(&mut self, node: NodeRef, payload: u32) -> Result<(), crate::node::NodeError> {
        self.slots.set_payload(node, payload)
    }

    /// 释放节点身份、它的 payload、绑定槽与辅助列表；重复释放无操作。
    fn release(&mut self, node: NodeRef) {
        if let Ok(slot) = self.slots.get(node) {
            let kind = slot.kind;
            let payload = slot.payload;
            let tree = slot.tree;
            match kind {
                Some(NodeKindTag::Element) => {
                    if let Some(data) = self
                        .elements
                        .get_mut(payload as usize)
                        .and_then(Option::take)
                    {
                        self.release_binding_list(tree, data.aux.bindings);
                        self.aux.release(data.aux);
                    }
                }
                Some(NodeKindTag::Text) => {
                    if let Some(data) = self.texts.release(payload) {
                        self.release_binding(data.tree, data.binding);
                    }
                }
                Some(NodeKindTag::Show) => {
                    if let Some(data) = self.shows.release(payload) {
                        self.release_binding(tree, data.when);
                    }
                }
                Some(NodeKindTag::For) => {
                    if let Some(data) = self.forces.release(payload)
                        && let MountedForRows::Read(binding) = data.rows
                    {
                        self.release_binding(tree, binding);
                    }
                }
                _ => {}
            }
        }
        self.slots.release(node);
    }

    fn reserve_text(&mut self, data: TextData) -> u32 {
        self.texts.reserve(data)
    }

    fn text(&self, payload: u32) -> &TextData {
        self.texts
            .get(payload)
            .expect("a live text node has a text payload")
    }

    fn reserve_show(&mut self, data: ShowData) -> u32 {
        self.shows.reserve(data)
    }

    fn show(&self, payload: u32) -> &ShowData {
        self.shows
            .get(payload)
            .expect("a live Show node has a Show payload")
    }

    fn take_show(&mut self, payload: u32) -> ShowData {
        self.shows
            .take(payload)
            .expect("a live Show node has a Show payload")
    }

    fn put_show(&mut self, payload: u32, data: ShowData) {
        self.shows.put(payload, data);
    }

    fn reserve_for(&mut self, data: ForData) -> u32 {
        self.forces.reserve(data)
    }

    fn for_node(&self, payload: u32) -> &ForData {
        self.forces
            .get(payload)
            .expect("a live For node has a For payload")
    }

    fn take_for(&mut self, payload: u32) -> ForData {
        self.forces
            .take(payload)
            .expect("a live For node has a For payload")
    }

    fn put_for(&mut self, payload: u32, data: ForData) {
        self.forces.put(payload, data);
    }

    fn slot_exists(&self, node: NodeRef) -> bool {
        self.slots.slot_exists(node)
    }

    fn live(&self) -> usize {
        self.slots.live()
    }

    fn storage_len(&self) -> usize {
        self.slots.storage_len()
    }

    fn intern_tag(&mut self, tag: &'static str) -> TagId {
        if let Some(id) = self.tag_ids.get(tag) {
            return *id;
        }
        let id = TagId::from_index(self.tags.len());
        self.tags.push(tag);
        self.tag_ids.insert(tag, id);
        id
    }

    fn tag(&self, id: TagId) -> &'static str {
        self.tags
            .get(id.raw() as usize)
            .copied()
            .expect("tag ids come from this registry")
    }

    fn place_element_data(&mut self, slot: usize, data: ElementData) {
        if slot == self.elements.len() {
            self.elements.push(Some(data));
        } else {
            debug_assert!(slot < self.elements.len(), "element slots are pinned");
            self.elements[slot] = Some(data);
        }
    }

    fn element_data(&self, slot: usize) -> Option<&ElementData> {
        self.elements.get(slot).and_then(Option::as_ref)
    }

    fn reserve_element_aux(&mut self, seed: AuxSeed) -> AuxHandles {
        self.aux.reserve(seed)
    }

    fn attrs(&self, id: AttrListId) -> &[MountedAttr] {
        self.aux.attrs.get(id.raw()).map_or(&[], Vec::as_slice)
    }

    fn attrs_mut(&mut self, id: AttrListId) -> &mut Vec<MountedAttr> {
        self.aux
            .attrs
            .get_mut(id.raw())
            .expect("element attr handles stay live until the element is retired")
    }

    fn class_styles(&self, id: StyleListId) -> &'static [zenui_style::StyleDeclaration] {
        self.aux.class_styles.get(id.raw()).copied().unwrap_or(&[])
    }

    fn inline_styles(&self, id: StyleListId) -> &[MountedInlineStyle] {
        self.aux
            .inline_styles
            .get(id.raw())
            .map_or(&[], Vec::as_slice)
    }

    fn inline_styles_mut(&mut self, id: StyleListId) -> &mut Vec<MountedInlineStyle> {
        self.aux
            .inline_styles
            .get_mut(id.raw())
            .expect("element inline-style handles stay live until the element is retired")
    }

    fn events(&self, id: EventListId) -> &[usize] {
        self.aux.events.get(id.raw()).map_or(&[], Vec::as_slice)
    }

    fn edit(&self, id: EditStateId) -> &LocalState {
        self.aux
            .edits
            .get(id.raw())
            .expect("element edit handles stay live")
    }

    fn edit_mut(&mut self, id: EditStateId) -> &mut LocalState {
        self.aux
            .edits
            .get_mut(id.raw())
            .expect("element edit handles stay live")
    }

    fn semantics(&self, id: SemanticsId) -> SnapshotSemantics {
        self.aux
            .semantics
            .get(id.raw())
            .copied()
            .unwrap_or_default()
    }

    /// 当前存活的辅助句柄总数；测试用它验证元素卸载时的释放。
    #[cfg(test)]
    fn aux_live(&self) -> usize {
        self.aux.live()
    }
}

/// 文本节点的 payload：绑定槽引用；已提交文本只在 `BindingSlot.current`。
struct TextData {
    binding: BindingRef,
    /// 所属树；绑定表按树索引。
    tree: TreeKey,
    /// 节点身份；补丁按 `(tree, node, part)` 去重。
    node: NodeRef,
}

/// 元素挂载时一次性写入辅助 arena 的初始列表。
struct AuxSeed {
    attrs: Vec<MountedAttr>,
    class_styles: &'static [zenui_style::StyleDeclaration],
    inline_styles: Vec<MountedInlineStyle>,
    events: Vec<usize>,
    edit: LocalState,
    semantics: SnapshotSemantics,
}

struct AuxArenas {
    attrs: Pool<Vec<MountedAttr>>,
    class_styles: Pool<&'static [zenui_style::StyleDeclaration]>,
    inline_styles: Pool<Vec<MountedInlineStyle>>,
    events: Pool<Vec<usize>>,
    edits: Pool<LocalState>,
    binding_lists: Pool<BindingList>,
    semantics: Pool<SnapshotSemantics>,
}

impl AuxArenas {
    const fn new() -> Self {
        Self {
            attrs: Pool::new(),
            class_styles: Pool::new(),
            inline_styles: Pool::new(),
            events: Pool::new(),
            edits: Pool::new(),
            binding_lists: Pool::new(),
            semantics: Pool::new(),
        }
    }

    fn reserve(&mut self, seed: AuxSeed) -> AuxHandles {
        AuxHandles {
            attrs: AttrListId::from_raw(self.attrs.reserve(seed.attrs)),
            class_styles: StyleListId::from_raw(self.class_styles.reserve(seed.class_styles)),
            inline_styles: StyleListId::from_raw(self.inline_styles.reserve(seed.inline_styles)),
            events: EventListId::from_raw(self.events.reserve(seed.events)),
            edit: EditStateId::from_raw(self.edits.reserve(seed.edit)),
            bindings: BindingListId::from_raw(self.binding_lists.reserve(BindingList::default())),
            semantics: SemanticsId::from_raw(self.semantics.reserve(seed.semantics)),
        }
    }

    fn release(&mut self, handles: AuxHandles) {
        self.attrs.release(handles.attrs.raw());
        self.class_styles.release(handles.class_styles.raw());
        self.inline_styles.release(handles.inline_styles.raw());
        self.events.release(handles.events.raw());
        self.edits.release(handles.edit.raw());
        self.binding_lists.release(handles.bindings.raw());
        self.semantics.release(handles.semantics.raw());
    }

    fn live(&self) -> usize {
        self.attrs.live()
            + self.class_styles.live()
            + self.inline_styles.live()
            + self.events.live()
            + self.edits.live()
            + self.binding_lists.live()
            + self.semantics.live()
    }
}

impl fmt::Debug for NodeArena {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeArena")
            .field("live", &self.live())
            .field("storage", &self.storage_len())
            .field("aux_live", &self.aux.live())
            .finish()
    }
}

static NEXT_RUNTIME: AtomicU32 = AtomicU32::new(1);

/// 一棵已提交树的身份：headless 根，或目录中的窗口键。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum TreeSlot {
    Root,
    Window(&'static str),
}

/// 一棵已提交界面，以及它拥有的 Owner、焦点与结算前序。
struct MountedTree {
    nodes: Vec<MountedNode>,
    owner: u32,
    focused: Option<usize>,
    /// 焦点元素的 tag；元素卸载后仍用于同 tag 焦点接任。
    focused_tag: Option<&'static str>,
    settle_before: Option<Vec<usize>>,
    /// 只在这棵树出现有效呈现变化时取新值；不同树互不影响。
    revision: u64,
    /// 最近一次构建的呈现快照；revision 仍匹配时重复读取共享同一个 `Arc`
    /// 根。隐藏树在 flush 推进 revision 后保留旧根，快照构建延迟到第一次
    /// 读取（show 或任何呈现读取入口）。
    snapshot: RefCell<Option<PresentationSnapshot>>,
    /// 上次成功发布的快照；`presentation_update` 据此计算累计 delta。
    /// 隐藏窗口在 hide 时清空，重新显示后的首次发布因此是完整根。
    published: RefCell<Option<PresentationSnapshot>>,
}

/// 一个窗口化 `For` 的挂载窗口：从 `first` 行开始挂载 `count` 行。
///
/// 请求按行带身份保存；运行时把区间夹到当前行数，并允许聚焦行在窗口外
/// 一行以内时把窗口扩大一行。`count` 为零表示该行带当前不挂载任何行。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowWindow {
    first: usize,
    count: usize,
}

impl RowWindow {
    /// 从 `first` 行开始挂载 `count` 行。
    pub const fn new(first: usize, count: usize) -> Self {
        Self { first, count }
    }

    /// 请求的起始行。
    pub const fn first(self) -> usize {
        self.first
    }

    /// 请求挂载的行数。
    pub const fn count(self) -> usize {
        self.count
    }
}

/// 一次批量行窗口结构 turn 的请求与提交记录。
#[derive(Default)]
struct RowWindowBatch {
    /// 本批改变了请求的行带。
    bands: BTreeSet<u64>,
    /// 已经被对应 `For` 消费并提交的行带；失败时这些行带需要一次修复
    /// 刷新才能把已提交的行搬回旧窗口。
    applied: BTreeSet<u64>,
}

/// 没有宿主窗口时的默认挂载行数；ADR-0011 要求它远大于常见视口行数。
const DEFAULT_WINDOW_ROWS: usize = 32;

/// 一个 Runtime 的应用会话状态。
#[derive(Default)]
struct SessionState {
    /// 未声明时使用 `QuitOnLastClose`。
    policy: SessionPolicy,
    declared: bool,
    /// 显式退出已开始；之后的窗口打开与会话动作被拒绝。
    quitting: bool,
    /// UI 处理期排队、在窗口请求之后结算的会话动作。
    requests: Vec<SessionAction>,
}

/// 自上次 flush 以来所有 Store 提交的累计净变化。
///
/// 以日志形式保存每次提交：flush 开始时记下长度，成功结束后只丢弃本次
/// flush 已经消费的前缀。结构/工厂执行中产生的新提交留给下一次 flush 的
/// step 0（与重新执行结构遍历相比，这是不丢删除行的最小正确行为）。
#[derive(Default)]
struct PendingChanges {
    log: Vec<(StoreId, ChangeSet)>,
}

impl PendingChanges {
    fn record(&mut self, store: StoreId, changes: &ChangeSet) {
        if changes.is_empty() {
            return;
        }
        self.log.push((store, changes.clone()));
    }

    fn removed(&self) -> impl Iterator<Item = RowId> + '_ {
        self.log
            .iter()
            .flat_map(|(_, changes)| changes.removed.iter().copied())
    }

    /// 成员真正迁移过的 Store。
    fn changed_stores(&self) -> impl Iterator<Item = StoreId> + '_ {
        self.log
            .iter()
            .filter(|(_, changes)| !changes.membership.is_empty())
            .map(|(store, _)| *store)
    }

    fn mark(&self) -> usize {
        self.log.len()
    }

    /// 丢弃本次 flush 开始时已经存在的提交；之后追加的提交保留。
    fn drain_consumed(&mut self, mark: usize) {
        self.log.drain(..mark.min(self.log.len()));
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.log.len()
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.log.is_empty()
    }
}

/// 把 Store 的提交累计到当前 runtime 的 `PendingChanges`。
///
/// 在 `For` 计划挂载时安装（`ForPlan::attach`）。观察者按 runtime 身份
/// 注册，多个 runtime 可以同时观察同一个 Store；观察者只持弱引用，runtime
/// 释放后返回 `false`，Store 会把它移除。
pub(crate) fn observe_store_commits<R: Row>(store: &zenui_core::model::Store<R>) {
    let Some(state) = context::current_state() else {
        return;
    };
    let key = state.borrow().id;
    let pending = Rc::downgrade(&state.borrow().pending);
    store.set_commit_observer(
        key,
        Some(Rc::new(move |store, changes| {
            let Some(pending) = pending.upgrade() else {
                return false;
            };
            pending.borrow_mut().record(store, changes);
            true
        })),
    );
}

pub(crate) struct State {
    pub(crate) id: u32,
    pub(crate) read_only: u32,
    /// 正在挂载或刷新的树；元素挂载时记录自己的树，查找为 O(1)。
    building_tree: Option<TreeSlot>,
    /// 统一 UI 节点身份；旧 payload 存储仍由 elements/components/trees 承担。
    nodes: NodeArena,
    /// runtime 树槽位到 arena 树键的映射。
    arena_trees: BTreeMap<TreeSlot, TreeKey>,
    /// 反向映射：从节点槽位的 TreeKey 解析回 runtime 树槽位。
    arena_tree_slots: BTreeMap<TreeKey, TreeSlot>,
    /// 树 revision 的唯一分配器；每个树只在自身变化时取新值。
    next_tree_revision: u64,
    /// 自上次刷新以来出现可见变化的树；树在刷新期间被临时取出，
    /// 因此不能把标记存放在 `MountedTree` 内。
    dirty_trees: BTreeSet<TreeSlot>,
    /// 自上次公开 `Runtime::refresh` 报告以来 revision 推进的树。
    changed_trees: BTreeSet<TreeSlot>,
    /// 本次刷新观察到的读取失败；绑定保留最后值，诊断按值去重。
    pub(crate) read_faults: Vec<ReadFault>,
    /// 本 turn 已提交的绑定补丁；按 `(tree, node, part)` 去重。
    binding_patches: BindingPatchLog,
    /// 自上次 flush 以来的 Store 提交；观察者持弱引用，避免在写事务内
    /// 再次借用 State。
    pending: Rc<RefCell<PendingChanges>>,
    /// 刷新轮次；同一 binding 每轮最多求值一次，焦点结算会开启新一轮。
    refresh_round: u64,
    /// 脏依赖代数；值绑定不直接读它，相等写入不推进。
    pub(crate) write_epochs: BTreeMap<DepKey, u64>,
    /// 字段写入尝试代数；相等写入同样推进，文本编辑器据此判断处理器
    /// 是否处理过这次编辑。
    pub(crate) field_writes: BTreeMap<(TodoId, FieldKind), u64>,
    pub(crate) write_error: Option<WriteError>,
    /// Todo 是通用行模型的第一个实例化；行身份与业务键都由它维护。
    pub(crate) todo_store: zenui_core::model::Store<TodoRow>,
    /// 业务键种子；只在事务成功提交后推进，失败的事务不消耗键。
    pub(crate) next_todo_key: u64,
    /// 旧 `RowSet` 投影缓存，按 membership/order revision 失效。
    pub(crate) rowset_cache: [Option<CachedRowSet>; 3],
    pub(crate) store_counters: Counters,
    pub(crate) phase: Phase,
    pub(crate) fault: Option<ContextFault>,
    pub(crate) appearance: Appearance,
    pub(crate) signals: Vec<SignalCell>,
    pub(crate) memos: Vec<MemoCell>,
    pub(crate) reverse: BTreeMap<DepKey, BTreeSet<u32>>,
    pub(crate) computing: Vec<(u32, Vec<DepKey>)>,
    pub(crate) once_reads: u64,
    owners: BTreeMap<u32, OwnerRecord>,
    next_owner: u32,
    pub(crate) current_owner: Option<u32>,
    events: Vec<Option<Event>>,
    elements: Vec<MountedElement>,
    components: Vec<MountedComponent>,
    /// 卸载后保留 NodeId tombstone 的槽位；新挂载复用并换新 generation。
    free_elements: Vec<usize>,
    free_components: Vec<usize>,
    free_events: Vec<usize>,
    trees: BTreeMap<TreeSlot, MountedTree>,
    app_setup: bool,
    window_factory_ran: bool,
    catalog: BTreeMap<&'static str, CatalogEntry>,
    primary: Option<&'static str>,
    catalog_declared: bool,
    focused_window: Option<&'static str>,
    windows: BTreeMap<&'static str, WindowRecord>,
    /// runtime 全局窗口槽分配器；槽位单调递增、不复用。
    next_activation_slot: u32,
    /// UI 处理期排队、在事件/交付结算后执行的窗口生命周期请求。
    window_requests: Vec<WindowRequest>,
    /// 每个 activation 在 factory 中注册的关闭处理器，随窗口关闭释放。
    close_handlers: BTreeMap<&'static str, CloseHandler>,
    /// 每个 activation 在 factory 中注册的文件拖放处理器，随窗口关闭释放。
    drop_handlers: BTreeMap<&'static str, DropHandler>,
    /// 每个 activation 的窗口根 keydown fallback。
    keydown_handlers: BTreeMap<&'static str, WindowKeyDownHandler>,
    /// 命令 handler 内排队关闭请求时继承的关闭来源。
    active_close_source: Option<CloseSource>,
    /// 每个 activation 在 factory 中注册的命令处理器，随窗口关闭释放。
    command_handlers: BTreeMap<&'static str, BTreeMap<&'static str, CommandHandler>>,
    /// 应用级命令处理器；不随窗口关闭释放，运行时启动后拒绝注册。
    app_command_handlers: BTreeMap<&'static str, CommandHandler>,
    /// 应用级托盘声明；由宿主在启动时读取。
    tray: Option<TraySpec>,
    /// 应用会话的最后窗口策略与显式退出状态。
    session: SessionState,
    /// 每个 activation 的只读菜单状态回调，随窗口关闭释放。
    menu_state_handlers: BTreeMap<&'static str, MenuStateHandler>,
    /// 按父窗口键索引的待决关闭 epoch；存在即表示父窗口输入被挂起。
    close_epochs: BTreeMap<&'static str, CloseEpoch>,
    /// UI 处理期排队的 Allow/Deny 回复，在事件/交付结算后执行。
    close_replies: Vec<CloseReplyRequest>,
    next_close_epoch: u64,
    /// 窗口打开/关闭/隐藏/显示时递增；原生宿主据此跳过无变化的扫描。
    window_revision: u64,
    /// 已构建的 presentation snapshot 次数；宿主与基准用它验证滚动不重建快照。
    presentation_builds: Cell<u64>,
    /// 宿主为窗口化 For 声明的行窗口，按行带键索引。
    row_windows: BTreeMap<u64, RowWindow>,
    /// 每个 For owner 拥有的行带键；owner 释放时一并丢弃请求。
    owner_bands: BTreeMap<u32, Vec<u64>>,
    next_row_band: u64,
    /// 正在刷新的树在刷新前的焦点元素，供窗口化 For 的行 grace 使用。
    refresh_focus: Option<usize>,
    /// 本帧窗口化 For 卸载了焦点所在行；刷新后清除焦点而不是结算到邻居。
    window_focus_dropped: Option<usize>,
    /// 正在进行的批量行窗口结构 turn；只有该 turn 内的刷新会记录哪些
    /// 行带已经消费并提交了请求。
    row_window_batch: Option<RowWindowBatch>,
    pub(crate) tasks: BTreeMap<u32, TaskRecord>,
    pub(crate) next_task: u32,
    /// 已排队、等待宿主弹出的文件选择请求；任务本身在 `tasks` 中。
    pub(crate) pending_file_dialogs: Vec<crate::file_dialog::PendingFileDialog>,
    pub(crate) wake_callback: Arc<RwLock<Option<WakeCallback>>>,
    setup_counts: BTreeMap<&'static str, u64>,
    next_instance: u64,
    unmounts: u64,
    /// 文本编辑谱系归约的有界记录；测试据此刻画分类顺序，不是公开 API。
    text_trace: VecDeque<TextTrace>,
}

/// 拟议结构的回滚标记：补丁日志位置 + 当前树的 dirty 标记。
///
/// 被拒绝的 Show 分支或 For 行不可见，因此它们产生的补丁和“树已变化”
/// 标记都必须回到拟议之前的状态，revision 不能因未提交的结构推进。
#[derive(Clone, Copy)]
struct SpeculationMark {
    patches: BindingPatchMark,
    tree: Option<TreeSlot>,
    was_dirty: bool,
}

/// 旧焦点离开可见集合后的一次结算：旧目标（可选 blur）与接任目标。
struct FocusSettlement {
    old: usize,
    /// 旧目标仍在挂载树中（例如刚被禁用）时，需要向它派发一次 blur。
    blur_old: bool,
    target: Option<usize>,
}

/// 一次待决的 Confirm 关闭；`reply` 保证同一 epoch 只有一个有效回复。
struct CloseEpoch {
    id: u64,
    parent: WindowCtx,
    confirmation: &'static str,
    /// 挂起前父窗口的焦点；Deny 时尝试恢复，节点已消失则走正常结算。
    focus: Option<NodeId>,
    reply: Option<CloseReply>,
}

/// 已排队、尚未执行的关闭回复。
struct CloseReplyRequest {
    request: CloseRequest,
    reply: CloseReply,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemInfo {
    /// 行数据来源 Store。
    pub store: StoreId,
    /// 行身份。
    pub row: RowId,
    /// 挂载时捕获的 Todo 业务键；第二个模型或非 Todo Store 的行返回 `None`。
    /// 行被删除但旧映射仍保留时，这个键也不变。
    pub todo: Option<TodoId>,
    pub instance: u64,
    pub owner: u32,
}

/// 当前焦点目标的公开身份；`label` 为空表示只有遍历顺序和标签名可用。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FocusInfo {
    pub tag: &'static str,
    pub label: Option<String>,
}

/// 自上次公开 [`Runtime::refresh`] 报告以来的有效呈现变化。
///
/// 只统计会改变 presentation snapshot 的变化；空 click、相等写入与纯
/// runtime 计数器变化都不在这里出现。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RefreshReport {
    /// 任一树推进了 revision。
    pub changed: bool,
    /// headless 根树推进了 revision。
    pub root_changed: bool,
    /// revision 推进的窗口键，按字典序排列。
    pub changed_windows: Vec<&'static str>,
    /// 本次刷新遇到的 typed 读取失败；值绑定保留最后已提交值。
    faults: Vec<ReadFault>,
}

impl RefreshReport {
    /// 是否有任何有效呈现变化。
    pub const fn changed(&self) -> bool {
        self.changed
    }

    /// revision 推进的窗口键。
    pub fn changed_windows(&self) -> &[&'static str] {
        &self.changed_windows
    }

    /// 本次刷新遇到的 typed 读取失败，按值去重。
    pub fn faults(&self) -> &[ReadFault] {
        &self.faults
    }
}

/// 记录一个绑定来源的 memo 槽位；其他来源没有 memo 依赖。
fn collect_slot_memo(slot: Option<ReadSlot>, live: &mut BTreeSet<u32>) {
    if let Some(ReadSlot::Memo(source)) = slot {
        live.insert(source.id);
    }
}

/// 原生控件与显式 focus/blur 处理器决定元素的固有可聚焦性。
fn inherent_focusable(state: &State, tag: &str, events: &[usize]) -> bool {
    matches!(tag, "button" | "text-input" | "checkbox")
        || events.iter().any(|id| {
            state
                .events
                .get(*id)
                .and_then(Option::as_ref)
                .is_some_and(|event| {
                    matches!(event_kind(event), EventKind::Focus | EventKind::Blur)
                })
        })
}

/// 已挂载的用户动作集合；焦点/组合等运行时事件不在此投影里。
fn actions_for_events(state: &State, events: &[usize]) -> SnapshotActions {
    let mut actions = SnapshotActions::default();
    for id in events {
        let Some(event) = state.events.get(*id).and_then(Option::as_ref) else {
            continue;
        };
        match event_kind(event) {
            EventKind::Click => actions.insert(SnapshotAction::Click),
            EventKind::Change => actions.insert(SnapshotAction::Change),
            EventKind::Input => actions.insert(SnapshotAction::Input),
            _ => {}
        }
    }
    actions
}

impl State {
    /// 已挂载绑定与结构触发直接引用的 memo，加上它们的 memo 依赖闭包。
    ///
    /// liveness 从每棵树的绑定槽出发；被卸载分支的槽已经归还绑定表，因此不会
    /// 把只被处理器或废弃节点读取的派生值提前结算。`quarantined` 行上的绑定
    /// 在本轮视为 non-live：它们的子树不再作为种子，只服务这些行的 memo
    /// （包括上游依赖）因此不会进入 worklist，边与订阅保留到结构提交。
    pub(crate) fn live_memo_set(&self, quarantined: &BTreeSet<RowId>) -> BTreeSet<u32> {
        let mut live = BTreeSet::new();
        for (slot, tree) in &self.trees {
            if let Some(tree_key) = self.arena_trees.get(slot) {
                self.collect_node_memos(*tree_key, &tree.nodes, quarantined, &mut live);
            }
        }
        let mut stack: Vec<u32> = live.iter().copied().collect();
        while let Some(memo) = stack.pop() {
            for key in &self.memos[memo as usize].deps {
                if let DepKey::Memo(dep) = key
                    && live.insert(*dep)
                {
                    stack.push(*dep);
                }
            }
        }
        live
    }

    /// flush step 0：本帧需要 quarantine 的行。
    ///
    /// 已删除行来自累计的 `ChangeSet.removed`；成员迁移只带 filter 身份，
    /// 因此对成员变化的 Store 上的所有已挂载行做保守隔离，结构提交确认后
    /// 保留的行自然恢复（它们的 memo 在值阶段按需求值）。
    fn quarantined_rows(&self) -> BTreeSet<RowId> {
        let pending = self.pending.borrow();
        let mut rows: BTreeSet<RowId> = pending.removed().collect();
        let changed_stores: BTreeSet<StoreId> = pending.changed_stores().collect();
        drop(pending);
        if changed_stores.is_empty() {
            return rows;
        }
        for tree in self.trees.values() {
            self.walk_for_rows(&tree.nodes, &mut |store, row| {
                if changed_stores.contains(&store) {
                    rows.insert(row);
                }
            });
        }
        rows
    }

    /// 遍历所有已挂载的 For 行：`(行 Store, RowId)`；嵌套 For 一并遍历。
    fn walk_for_rows(&self, nodes: &[MountedNode], visit: &mut impl FnMut(StoreId, RowId)) {
        for node in nodes {
            match node {
                MountedNode::Text(_) => {}
                MountedNode::Element(id) => {
                    self.walk_for_rows(&self.elements[*id].children, visit);
                }
                MountedNode::Component(id) => {
                    self.walk_for_rows(&self.components[*id].children, visit);
                }
                MountedNode::Show(payload) => {
                    if let Some(branch) = &self.nodes.show(*payload).active {
                        self.walk_for_rows(&branch.children, visit);
                    }
                }
                MountedNode::For(payload) => {
                    let data = self.nodes.for_node(*payload);
                    for (row, item) in &data.items {
                        visit(data.store, *row);
                        self.walk_for_rows(&item.children, visit);
                    }
                }
            }
        }
    }

    fn collect_node_memos(
        &self,
        tree: TreeKey,
        nodes: &[MountedNode],
        quarantined: &BTreeSet<RowId>,
        live: &mut BTreeSet<u32>,
    ) {
        let table = self.nodes.binding_table(tree);
        for node in nodes {
            match node {
                MountedNode::Text(payload) => {
                    let data = self.nodes.text(*payload);
                    collect_slot_memo(table.and_then(|table| table.read_slot(data.binding)), live);
                }
                MountedNode::Element(id) => {
                    if let Some(table) = table {
                        for attr in self.element_attrs(*id) {
                            collect_slot_memo(table.read_slot(attr.binding), live);
                        }
                        for style in self.element_inline_styles(*id) {
                            collect_slot_memo(table.read_slot(style.binding), live);
                        }
                    }
                    self.collect_node_memos(tree, &self.elements[*id].children, quarantined, live);
                }
                MountedNode::Component(id) => {
                    self.collect_node_memos(
                        tree,
                        &self.components[*id].children,
                        quarantined,
                        live,
                    );
                }
                MountedNode::Show(payload) => {
                    let show = self.nodes.show(*payload);
                    collect_slot_memo(table.and_then(|table| table.read_slot(show.when)), live);
                    if let Some(branch) = &show.active {
                        self.collect_node_memos(tree, &branch.children, quarantined, live);
                    }
                }
                MountedNode::For(payload) => {
                    let data = self.nodes.for_node(*payload);
                    if let MountedForRows::Read(binding) = data.rows {
                        collect_slot_memo(table.and_then(|table| table.read_slot(binding)), live);
                    }
                    // quarantined 行上的绑定本轮视为 non-live：整棵行子树都不
                    // 作为 liveness 种子，只有外部消费者能把这些 memo 拉回。
                    for (row, item) in &data.items {
                        if quarantined.contains(row) {
                            continue;
                        }
                        self.collect_node_memos(tree, &item.children, quarantined, live);
                    }
                }
            }
        }
    }

    fn new() -> Self {
        let mut owners = BTreeMap::new();
        owners.insert(
            0,
            OwnerRecord {
                parent: None,
                children: Vec::new(),
                resources: Vec::new(),
                elements: Vec::new(),
                components: Vec::new(),
                disposed: false,
            },
        );
        Self {
            id: NEXT_RUNTIME.fetch_add(1, Ordering::Relaxed),
            read_only: 0,
            building_tree: None,
            nodes: NodeArena::new(),
            arena_trees: BTreeMap::new(),
            arena_tree_slots: BTreeMap::new(),
            next_tree_revision: 1,
            dirty_trees: BTreeSet::new(),
            changed_trees: BTreeSet::new(),
            read_faults: Vec::new(),
            binding_patches: BindingPatchLog::default(),
            pending: Rc::new(RefCell::new(PendingChanges::default())),
            refresh_round: 0,
            write_epochs: BTreeMap::new(),
            field_writes: BTreeMap::new(),
            write_error: None,
            todo_store: zenui_core::model::Store::new(crate::store::todo_schema()),
            next_todo_key: 1,
            rowset_cache: [None, None, None],
            store_counters: Counters::default(),
            phase: Phase::Idle,
            fault: None,
            appearance: Appearance::Light,
            signals: Vec::new(),
            memos: Vec::new(),
            reverse: BTreeMap::new(),
            computing: Vec::new(),
            once_reads: 0,
            owners,
            next_owner: 1,
            current_owner: None,
            events: Vec::new(),
            elements: Vec::new(),
            components: Vec::new(),
            free_elements: Vec::new(),
            free_components: Vec::new(),
            free_events: Vec::new(),
            trees: BTreeMap::new(),
            app_setup: false,
            window_factory_ran: false,
            catalog: BTreeMap::new(),
            primary: None,
            catalog_declared: false,
            focused_window: None,
            windows: BTreeMap::new(),
            next_activation_slot: 1,
            window_requests: Vec::new(),
            close_handlers: BTreeMap::new(),
            drop_handlers: BTreeMap::new(),
            keydown_handlers: BTreeMap::new(),
            active_close_source: None,
            command_handlers: BTreeMap::new(),
            app_command_handlers: BTreeMap::new(),
            tray: None,
            session: SessionState::default(),
            menu_state_handlers: BTreeMap::new(),
            close_epochs: BTreeMap::new(),
            close_replies: Vec::new(),
            next_close_epoch: 1,
            window_revision: 0,
            presentation_builds: Cell::new(0),
            row_windows: BTreeMap::new(),
            owner_bands: BTreeMap::new(),
            next_row_band: 1,
            refresh_focus: None,
            window_focus_dropped: None,
            row_window_batch: None,
            tasks: BTreeMap::new(),
            next_task: 1,
            pending_file_dialogs: Vec::new(),
            wake_callback: Arc::new(RwLock::new(None)),
            setup_counts: BTreeMap::new(),
            next_instance: 1,
            unmounts: 0,
            text_trace: VecDeque::new(),
        }
    }

    /// 追加一条文本编辑谱系事件；超出测试序列所需的长度后丢弃最旧记录。
    fn push_text_trace(&mut self, event: TextTrace) {
        self.text_trace.push_back(event);
        while self.text_trace.len() > TEXT_TRACE_EVENTS {
            self.text_trace.pop_front();
        }
    }

    /// 把编辑器文本推进到派发值并记录这次本地编辑。
    fn dispatch_text_edit(&mut self, element: usize, value: &str, epoch: Option<u64>) {
        let trace = {
            let LocalState::TextInput(editor) = self.element_local_mut(element) else {
                unreachable!("dispatch requires a mounted text input");
            };
            editor.dispatch(value, epoch)
        };
        self.push_text_trace(trace);
        self.mark_element_changed(element);
    }

    /// 为元素槽位分配 arena 节点；槽位复用换 generation，旧 NodeId 解析失败。
    fn mint_element_node(&mut self, slot: usize, owner: u32, parent: Option<NodeRef>) -> NodeId {
        let node = self.register_node(NodeKindTag::Element, owner, parent);
        self.nodes
            .set_payload(node, u32::try_from(slot).expect("node identity exhausted"))
            .expect("the node was just inserted");
        NodeId::mint(self.id, node)
    }

    /// 预留一个元素槽位并登记到 owner；`place_element` 完成挂载。
    fn reserve_element(&mut self, owner: u32) -> usize {
        let slot = self.free_elements.pop().unwrap_or(self.elements.len());
        if let Some(record) = self.owners.get_mut(&owner) {
            record.elements.push(slot);
        }
        slot
    }

    fn place_element(&mut self, slot: usize, element: MountedElement) {
        if slot == self.elements.len() {
            self.elements.push(element);
        } else {
            debug_assert!(slot < self.elements.len(), "element slots are pinned");
            self.elements[slot] = element;
        }
    }

    /// 预留一个组件槽位并登记到 owner；组件 id 就是槽位。
    fn reserve_component(&mut self, owner: u32) -> usize {
        let slot = self.free_components.pop().unwrap_or(self.components.len());
        if let Some(record) = self.owners.get_mut(&owner) {
            record.components.push(slot);
        }
        slot
    }

    fn place_component(&mut self, slot: usize, component: MountedComponent) {
        if slot == self.components.len() {
            self.components.push(component);
        } else {
            debug_assert!(slot < self.components.len(), "component slots are pinned");
            self.components[slot] = component;
        }
    }

    fn reserve_event(&mut self, event: Event) -> usize {
        let slot = self.free_events.pop().unwrap_or(self.events.len());
        if slot == self.events.len() {
            self.events.push(Some(event));
        } else {
            self.events[slot] = Some(event);
        }
        slot
    }

    /// 卸载元素：只留 NodeId 供 stale 判定；payload 已随节点释放。
    fn retire_element(&mut self, slot: usize) {
        let Some(element) = self.elements.get_mut(slot) else {
            return;
        };
        let id = element.id;
        *element = MountedElement {
            id,
            children: Vec::new(),
        };
        self.free_elements.push(slot);
    }

    fn retire_component(&mut self, slot: usize) {
        let Some(component) = self.components.get_mut(slot) else {
            return;
        };
        component.owner = 0;
        component.children = Vec::new();
        self.free_components.push(slot);
    }

    fn tree(&self, key: TreeSlot) -> Option<&MountedTree> {
        self.trees.get(&key)
    }

    fn tree_mut(&mut self, key: TreeSlot) -> Option<&mut MountedTree> {
        self.trees.get_mut(&key)
    }

    /// 取走指定窗口的树供刷新使用。
    fn take_tree(&mut self, key: TreeSlot) -> Option<MountedTree> {
        self.trees.remove(&key)
    }

    fn put_tree(&mut self, key: TreeSlot, tree: MountedTree) {
        self.trees.insert(key, tree);
    }

    /// 当前接收输入的窗口；没有焦点窗口时退到目录中的第一个可见窗口或 headless 根。
    fn active_tree_key(&self) -> Option<TreeSlot> {
        if let Some(key) = self.focused_window
            && self.tree_is_visible(TreeSlot::Window(key))
        {
            return Some(TreeSlot::Window(key));
        }
        self.windows
            .keys()
            .copied()
            .find(|key| self.window_is_visible(key))
            .map(TreeSlot::Window)
            .or_else(|| {
                self.trees
                    .contains_key(&TreeSlot::Root)
                    .then_some(TreeSlot::Root)
            })
    }

    fn window_ctx(&self, key: &'static str) -> Option<WindowCtx> {
        let record = self.windows.get(key)?;
        let live = self.window_is_live(key, record.activation, record.owner)
            && self.trees.contains_key(&TreeSlot::Window(key));
        live.then(|| WindowCtx::new(self.id, key, record.activation, record.owner, record.id))
    }

    pub(crate) fn window_is_visible(&self, key: &'static str) -> bool {
        self.windows
            .get(key)
            .is_some_and(|record| record.open && record.visible)
            && self.trees.contains_key(&TreeSlot::Window(key))
    }

    fn tree_is_visible(&self, key: TreeSlot) -> bool {
        match key {
            TreeSlot::Root => self.trees.contains_key(&key),
            TreeSlot::Window(key) => self.window_is_visible(key),
        }
    }

    fn first_visible_window(&self) -> Option<&'static str> {
        self.windows
            .keys()
            .copied()
            .find(|key| self.window_is_visible(key))
    }

    fn hide_window(&mut self, key: &'static str) {
        let Some(record) = self.windows.get_mut(key) else {
            return;
        };
        if record.visible {
            record.visible = false;
            self.bump_window_revision();
            self.mark_tree_changed(TreeSlot::Window(key));
            self.commit_tree_revision(TreeSlot::Window(key));
            // 隐藏窗口不再发布呈现：清空发布基线，重新显示后的首次发布
            // 必须是完整根，不能用隐藏期间的旧 revision 猜 delta。
            if let Some(tree) = self.trees.get(&TreeSlot::Window(key)) {
                *tree.published.borrow_mut() = None;
            }
        }
    }

    fn show_window(&mut self, key: &'static str) {
        let Some(record) = self.windows.get_mut(key) else {
            return;
        };
        if !record.visible {
            record.visible = true;
            self.bump_window_revision();
            self.mark_tree_changed(TreeSlot::Window(key));
            self.commit_tree_revision(TreeSlot::Window(key));
        }
    }

    fn bump_window_revision(&mut self) {
        self.window_revision = self.window_revision.wrapping_add(1);
    }

    /// 标记一棵树的已提交呈现需要推进 revision。
    ///
    /// 刷新期间树暂时不在 `trees` 中，标记仍必须生效，因此使用独立集合。
    fn mark_tree_changed(&mut self, key: TreeSlot) {
        self.dirty_trees.insert(key);
    }

    /// 标记元素所属的树；元素已释放或尚未挂载时是静默无操作。
    fn mark_element_changed(&mut self, element: usize) {
        if let Some(key) = self.tree_for_element(element) {
            self.mark_tree_changed(key);
        }
    }

    /// 标记当前正在挂载/刷新的树；没有树上下文时无操作。
    fn mark_building_tree_changed(&mut self) {
        if let Some(key) = self.building_tree {
            self.mark_tree_changed(key);
        }
    }

    /// 开启一个新的刷新轮次；同一 binding 每轮最多求值一次。
    fn bump_refresh_round(&mut self) {
        self.refresh_round = self.refresh_round.wrapping_add(1);
    }

    /// 记录拟议结构的起点；被拒绝时用 [`State::rollback_speculation`] 还原。
    fn mark_speculation(&self) -> SpeculationMark {
        let tree = self.building_tree;
        SpeculationMark {
            patches: self.binding_patches.mark(),
            tree,
            was_dirty: tree.is_some_and(|key| self.dirty_trees.contains(&key)),
        }
    }

    /// 丢弃拟议结构记录的补丁；没有其他变化时同时撤销它的 dirty 标记。
    fn rollback_speculation(&mut self, mark: SpeculationMark) {
        self.binding_patches.rollback(mark.patches);
        if let Some(tree) = mark.tree
            && !mark.was_dirty
        {
            self.dirty_trees.remove(&tree);
        }
    }

    /// 释放节点并丢弃它在本 turn 的补丁；日志只保留仍挂载的节点。
    fn retire_node(&mut self, node: NodeRef) {
        if let Some(tree) = self.nodes.tree_of(node) {
            self.binding_patches.remove_node(tree, node);
        }
        self.nodes.release(node);
    }

    /// 应用级变化（例如 appearance）影响所有已提交树。
    fn mark_all_trees_changed(&mut self) {
        let keys: Vec<TreeSlot> = self.trees.keys().copied().collect();
        for key in keys {
            self.mark_tree_changed(key);
        }
    }

    /// 分配一个全局唯一的树 revision；只用于让不同树/重新打开的窗口不撞号。
    fn allocate_tree_revision(&mut self) -> u64 {
        let revision = self.next_tree_revision;
        self.next_tree_revision = self.next_tree_revision.wrapping_add(1).max(1);
        revision
    }

    /// 在刷新末提交本树的有效变化；返回是否推进了 revision。
    fn commit_tree_revision(&mut self, key: TreeSlot) -> bool {
        if !self.dirty_trees.remove(&key) {
            return false;
        }
        if !self.trees.contains_key(&key) {
            return false;
        }
        let revision = self.allocate_tree_revision();
        if let Some(tree) = self.trees.get_mut(&key) {
            tree.revision = revision;
        }
        self.changed_trees.insert(key);
        true
    }

    fn tree_for_element(&self, element: usize) -> Option<TreeSlot> {
        let node = self.elements.get(element)?.id.node();
        let slot = self.nodes.get(node).ok()?;
        self.arena_tree_slots.get(&slot.tree).copied()
    }

    fn element_data(&self, element: usize) -> &ElementData {
        let payload = self
            .element_payload(element)
            .expect("a mounted element has a live arena payload");
        self.nodes
            .element_data(payload)
            .expect("a mounted element has live element data")
    }

    fn element_payload(&self, element: usize) -> Option<usize> {
        let node = self.elements.get(element)?.id.node();
        let slot = self.nodes.get(node).ok()?;
        Some(slot.payload as usize)
    }

    fn element_tag(&self, element: usize) -> &'static str {
        self.element_tag_opt(element)
            .expect("a mounted element has a tag")
    }

    fn element_tag_opt(&self, element: usize) -> Option<&'static str> {
        let payload = self.element_payload(element)?;
        self.nodes
            .element_data(payload)
            .map(|data| self.nodes.tag(data.tag))
    }

    fn element_owner(&self, element: usize) -> u32 {
        self.element_data(element).owner
    }

    fn element_aux(&self, element: usize) -> AuxHandles {
        self.element_data(element).aux
    }

    fn element_aux_opt(&self, element: usize) -> Option<AuxHandles> {
        let node = self.elements.get(element)?.id.node();
        let slot = self.nodes.get(node).ok()?;
        self.nodes
            .element_data(slot.payload as usize)
            .map(|data| data.aux)
    }

    fn element_attrs(&self, element: usize) -> &[MountedAttr] {
        self.nodes.attrs(self.element_aux(element).attrs)
    }

    fn element_attrs_mut(&mut self, element: usize) -> &mut Vec<MountedAttr> {
        let aux = self.element_aux(element);
        self.nodes.attrs_mut(aux.attrs)
    }

    fn element_class_styles(&self, element: usize) -> &'static [zenui_style::StyleDeclaration] {
        self.nodes
            .class_styles(self.element_aux(element).class_styles)
    }

    fn element_inline_styles(&self, element: usize) -> &[MountedInlineStyle] {
        self.nodes
            .inline_styles(self.element_aux(element).inline_styles)
    }

    fn element_inline_styles_mut(&mut self, element: usize) -> &mut Vec<MountedInlineStyle> {
        let aux = self.element_aux(element);
        self.nodes.inline_styles_mut(aux.inline_styles)
    }

    fn element_events(&self, element: usize) -> &[usize] {
        self.nodes.events(self.element_aux(element).events)
    }

    fn element_local(&self, element: usize) -> &LocalState {
        self.nodes.edit(self.element_aux(element).edit)
    }

    fn element_local_mut(&mut self, element: usize) -> &mut LocalState {
        let aux = self.element_aux(element);
        self.nodes.edit_mut(aux.edit)
    }

    fn element_semantics(&self, element: usize) -> SnapshotSemantics {
        self.nodes.semantics(self.element_aux(element).semantics)
    }

    fn element_label_text(&self, element: usize) -> Option<&str> {
        self.element_attr_text(element, "aria:label")
    }

    /// 已提交的文本属性；`type` 等标记只读取当前值。
    fn element_attr_text(&self, element: usize, name: &str) -> Option<&str> {
        let attr = self
            .element_attrs(element)
            .iter()
            .find(|attr| attr.name == name)?;
        self.element_binding_table(element)?
            .text_current(attr.binding)
    }

    /// 元素所属树的绑定表；元素已释放时返回 `None`。
    fn element_binding_table(&self, element: usize) -> Option<&BindingTable> {
        let node = self.elements.get(element)?.id.node();
        self.nodes.binding_table(self.nodes.tree_of(node)?)
    }

    /// 元素所属的 arena 树键。
    fn element_tree(&self, element: usize) -> Option<TreeKey> {
        let node = self.elements.get(element)?.id.node();
        self.nodes.tree_of(node)
    }

    /// 已提交文本；首次求值失败、没有最后值时呈现为空串。
    fn text_value(&self, payload: u32) -> String {
        let data = self.nodes.text(payload);
        self.nodes
            .binding_table(data.tree)
            .and_then(|table| table.slot::<String>(data.binding))
            .and_then(|slot| slot.current.clone())
            .unwrap_or_default()
    }

    /// 当前挂载/刷新树的绑定表键；没有树上下文时使用根树。
    fn building_tree_key(&mut self) -> TreeKey {
        let slot = self.building_tree.unwrap_or(TreeSlot::Root);
        self.arena_tree(slot)
    }

    /// 记录一次已提交的绑定变化；补丁按 `(tree, node, part)` 去重。
    fn record_binding_patch<T: BindingValue>(
        &mut self,
        tree: TreeKey,
        node: NodeRef,
        part: BindingPart,
        old: Option<T>,
        new: Option<T>,
        fault: Option<ReadFault>,
    ) {
        self.binding_patches
            .record(tree, node, part, old, new, fault);
    }

    fn has_event(&self, element: usize, kind: EventKind) -> bool {
        self.element_events(element).iter().any(|id| {
            self.events
                .get(*id)
                .and_then(Option::as_ref)
                .is_some_and(|event| event_kind(event) == kind)
        })
    }

    /// `type="submit"` 的按钮由 click/Enter 默认提交最近 form。
    fn is_submit_button(&self, element: usize) -> bool {
        self.element_tag(element) == "button"
            && self.element_attr_text(element, "type") == Some("submit")
    }

    /// 元素自己是否挂载了普通 click handler。
    fn has_click_handler(&self, element: usize) -> bool {
        self.element_events(element).iter().any(|id| {
            self.events
                .get(*id)
                .and_then(Option::as_ref)
                .is_some_and(|event| event_kind(event) == EventKind::Click)
        })
    }

    /// 原生交互控件即使没有 handler 也阻断 closest-wins 的祖先穿透。
    fn is_interactive_control(&self, element: usize) -> bool {
        matches!(
            self.element_tag(element),
            "button" | "checkbox" | "text-input"
        )
    }

    /// 从命中元素沿父链找最近的可处理 click；交互控件无 handler 时停止。
    fn closest_click_target(&self, hit: NodeId) -> Result<Option<usize>, DispatchError> {
        let mut node = hit.node();
        loop {
            let slot = self
                .nodes
                .get(node)
                .map_err(|_| DispatchError::StaleNode(hit))?;
            if slot.kind == Some(NodeKindTag::Element) {
                let payload = slot.payload as usize;
                let live = self
                    .elements
                    .get(payload)
                    .is_some_and(|element| element.id.node() == node);
                if !live {
                    return Err(DispatchError::StaleNode(hit));
                }
                if self.has_click_handler(payload) || self.is_submit_button(payload) {
                    return Ok(Some(payload));
                }
                if self.is_interactive_control(payload) {
                    return Ok(None);
                }
            }
            let Some(parent) = slot.parent else {
                return Ok(None);
            };
            node = parent;
        }
    }

    /// 从根到命中元素的元素路径；结构节点透明。
    fn element_path(&self, hit: usize) -> Vec<usize> {
        let mut path = Vec::new();
        let mut node = self.elements[hit].id.node();
        while let Ok(slot) = self.nodes.get(node) {
            if slot.kind == Some(NodeKindTag::Element) {
                let payload = slot.payload as usize;
                if self
                    .elements
                    .get(payload)
                    .is_some_and(|element| element.id.node() == node)
                {
                    path.push(payload);
                }
            }
            let Some(parent) = slot.parent else {
                break;
            };
            node = parent;
        }
        path.reverse();
        path
    }

    fn ancestor_form(&self, element: usize) -> Option<usize> {
        let mut node = self.elements.get(element)?.id.node();
        loop {
            let slot = self.nodes.get(node).ok()?;
            if slot.kind == Some(NodeKindTag::Element) {
                let payload = slot.payload as usize;
                if self.elements.get(payload).is_some_and(|element| {
                    element.id.node() == node && self.element_tag(payload) == "form"
                }) {
                    return Some(payload);
                }
            }
            node = slot.parent?;
        }
    }

    /// 文本输入的 L4 caret/selection 默认；返回是否改变本地编辑状态。
    fn move_text_caret(&mut self, element: usize, key: Key, shift: bool) -> bool {
        let LocalState::TextInput(editor) = self.element_local_mut(element) else {
            return false;
        };
        let length = editor.value.chars().count();
        let caret = editor.caret.min(length);
        let anchor = editor.anchor.min(length);
        let selection = editor
            .selection
            .filter(|(start, end)| start != end)
            .map(|(start, end)| (start.min(length), end.min(length)));
        let (caret, anchor, selection) = match key {
            Key::ArrowLeft => match (shift, selection) {
                (false, Some((start, _))) => (start, start, None),
                _ => {
                    let next = caret.saturating_sub(1);
                    if shift {
                        (next, anchor, Some((anchor.min(next), anchor.max(next))))
                    } else {
                        (next, next, None)
                    }
                }
            },
            Key::ArrowRight => match (shift, selection) {
                (false, Some((_, end))) => (end, end, None),
                _ => {
                    let next = (caret + 1).min(length);
                    if shift {
                        (next, anchor, Some((anchor.min(next), anchor.max(next))))
                    } else {
                        (next, next, None)
                    }
                }
            },
            Key::Home => {
                if shift {
                    (0, anchor, Some((0, anchor)))
                } else {
                    (0, 0, None)
                }
            }
            Key::End => {
                if shift {
                    (length, anchor, Some((anchor.min(length), length)))
                } else {
                    (length, length, None)
                }
            }
            _ => return false,
        };
        let changed =
            editor.caret != caret || editor.anchor != anchor || editor.selection != selection;
        editor.caret = caret;
        editor.anchor = anchor;
        editor.selection = selection;
        changed
    }

    /// 窗口/activation 目标仍可接收输入；不做 nearest-target 回退。
    fn intent_window_usable(&self, window: WindowCtx) -> bool {
        window.runtime == self.id
            && self.window_live_in_tree(window)
            && self.window_is_visible(window.key)
            && !self.close_epochs.contains_key(window.key)
    }

    fn element_is_disabled(&self, element: usize) -> bool {
        let Some(attr) = self
            .element_attrs(element)
            .iter()
            .find(|attr| attr.name == "disabled")
        else {
            return false;
        };
        self.element_binding_table(element)
            .is_some_and(|table| table.bool_current(attr.binding) == Some(true))
    }

    /// `value` 绑定的写入代数；静态值与缺失绑定返回 `None`。
    fn element_value_epoch(&self, element: usize) -> Option<u64> {
        let attr = self
            .element_attrs(element)
            .iter()
            .find(|attr| attr.name == "value")?;
        let table = self.element_binding_table(element)?;
        if !table.is_text(attr.binding) {
            return None;
        }
        let source = table.read_slot(attr.binding)?;
        source.epoch(self)
    }

    /// 取得或分配一棵树的 arena 键；同一个 runtime 树槽位在存续期间保持稳定。
    fn arena_tree(&mut self, slot: TreeSlot) -> TreeKey {
        if let Some(key) = self.arena_trees.get(&slot) {
            return *key;
        }
        let key = self.nodes.allocate_tree();
        self.arena_trees.insert(slot, key);
        self.arena_tree_slots.insert(key, slot);
        key
    }

    /// 在 arena 中登记一个已挂载节点，并把回收责任交给指定 owner。
    fn register_node(&mut self, kind: NodeKindTag, owner: u32, parent: Option<NodeRef>) -> NodeRef {
        let tree_key = self.arena_tree(self.building_tree.unwrap_or(TreeSlot::Root));
        let node = self
            .nodes
            .insert(kind, NO_PAYLOAD, tree_key, parent)
            .expect("a mounted parent node is live");
        if let Some(record) = self.owners.get_mut(&owner) {
            record.resources.push(Resource::Node(node));
        }
        node
    }

    /// 指定树里是否还有这个行带的行窗口化 For；过期行带由宿主忽略。
    fn tree_has_band(&self, key: TreeSlot, band: u64) -> bool {
        self.tree(key)
            .is_some_and(|tree| contains_band(self, &tree.nodes, band))
    }

    /// 一棵树里全部可寻址窗口的行带键；一次遍历服务整批校验。
    fn tree_band_keys(&self, key: TreeSlot) -> BTreeSet<u64> {
        let mut bands = BTreeSet::new();
        if let Some(tree) = self.tree(key) {
            collect_band_keys(self, &tree.nodes, &mut bands);
        }
        bands
    }

    /// 行带所属的 `For` payload；行带已随 owner 释放时返回 `None`。
    fn for_payload_for_band(&self, key: TreeSlot, band: u64) -> Option<u32> {
        self.tree(key)
            .and_then(|tree| find_for_payload(self, &tree.nodes, band))
    }

    fn element_is_visible(&self, element: usize) -> bool {
        self.tree_for_element(element)
            .is_some_and(|key| self.tree_is_visible(key))
    }

    /// 窗口仍存活且其树属于同一 Owner。
    /// 第一个窗口 factory 或根树已经启动；应用级命令注册在此之前关闭。
    pub(crate) fn runtime_started(&self) -> bool {
        self.window_factory_ran || !self.trees.is_empty()
    }

    fn window_live_in_tree(&self, window: WindowCtx) -> bool {
        self.trees
            .get(&TreeSlot::Window(window.key))
            .is_some_and(|tree| {
                tree.owner == window.owner
                    && self.window_is_live(window.key, window.activation, window.owner)
            })
    }

    /// 当前焦点目标；焦点属于树，而不是整个应用。
    fn focused_in(&self, key: TreeSlot) -> Option<usize> {
        self.tree(key).and_then(|tree| tree.focused)
    }

    /// 树在显露相关状态上的指纹：焦点节点与聚焦输入的 caret/selection/preedit。
    fn reveal_probe(&self, key: TreeSlot) -> RevealProbe {
        let focused = self.focused_in(key);
        let node = focused.map(|element| self.elements[element].id);
        let Some(editor) = focused.and_then(|element| match self.element_local(element) {
            LocalState::TextInput(editor) => Some(editor),
            LocalState::None | LocalState::Checkbox { .. } => None,
        }) else {
            return RevealProbe {
                focused: node,
                caret: 0,
                anchor: 0,
                preedit: None,
            };
        };
        RevealProbe {
            focused: node,
            caret: editor.caret,
            anchor: editor.anchor,
            preedit: editor
                .composition
                .as_ref()
                .map(|composition| composition.preedit.clone()),
        }
    }

    fn set_focused_in(&mut self, key: TreeSlot, focused: Option<usize>) {
        let tag = focused.and_then(|element| self.element_tag_opt(element));
        let changed = self.tree_mut(key).is_some_and(|tree| {
            if tree.focused == focused {
                return false;
            }
            tree.focused = focused;
            tree.focused_tag = tag;
            true
        });
        if !changed {
            return;
        }
        self.mark_tree_changed(key);
        // 焦点变化结束所有输入的连续输入 run。
        for element in 0..self.elements.len() {
            if self.element_aux_opt(element).is_some()
                && let LocalState::TextInput(editor) = self.element_local_mut(element)
            {
                editor.history.break_run();
            }
        }
    }

    /// 焦点窗口中的呈现焦点：仍在已提交树中且可聚焦。
    fn presented_focus(&self) -> Option<usize> {
        self.active_tree_key()
            .and_then(|key| self.presented_focus_in(key))
    }

    fn presented_focus_in(&self, key: TreeSlot) -> Option<usize> {
        self.focused_in(key)
            .filter(|element| self.element_is_mounted(*element) && self.is_focusable(*element))
    }

    fn snapshot(&self, key: TreeSlot) -> Option<PresentationSnapshot> {
        let tree = self.tree(key)?;
        // revision 未推进时已构建的根仍然精确；重复读取只克隆 `Arc`。
        if let Some(cached) = tree.snapshot.borrow().as_ref()
            && cached.revision() == tree.revision
        {
            return Some(cached.clone());
        }
        self.presentation_builds
            .set(self.presentation_builds.get().wrapping_add(1));
        let mut roots = Vec::new();
        collect_snapshot(self, &tree.nodes, &mut roots);
        let mut row_bands = Vec::new();
        collect_row_bands(self, &tree.nodes, &mut row_bands);
        let focus = self
            .presented_focus_in(key)
            .map(|element| self.elements[element].id);
        let snapshot =
            PresentationSnapshot::new(roots, focus, self.appearance, row_bands, tree.revision);
        *tree.snapshot.borrow_mut() = Some(snapshot.clone());
        Some(snapshot)
    }

    /// 发布当前快照及相对上次成功发布的累计 delta。
    ///
    /// 发布基线按树保存；隐藏窗口在 hide 时清空，重新显示后的首次发布是
    /// 完整根（`delta == None`）。读取快照本身不发布，也不推进基线。
    fn presentation_update(&self, key: TreeSlot) -> Option<PresentationUpdate> {
        let tree = self.tree(key)?;
        let snapshot = self.snapshot(key)?;
        let previous = tree.published.borrow().clone();
        let delta = previous
            .as_ref()
            .and_then(|previous| PresentationDelta::between(previous, &snapshot));
        *tree.published.borrow_mut() = Some(snapshot.clone());
        Some(PresentationUpdate::new(snapshot, delta))
    }

    pub(crate) fn attach_to_owner(&mut self, resource: Resource) {
        if let Some(owner) = self.current_owner
            && let Some(record) = self.owners.get_mut(&owner)
        {
            record.resources.push(resource);
        }
    }

    pub(crate) fn create_owner(&mut self, parent: Option<u32>) -> u32 {
        let id = self.next_owner;
        self.next_owner += 1;
        self.owners.insert(
            id,
            OwnerRecord {
                parent,
                children: Vec::new(),
                resources: Vec::new(),
                elements: Vec::new(),
                components: Vec::new(),
                disposed: false,
            },
        );
        if let Some(parent) = parent
            && let Some(record) = self.owners.get_mut(&parent)
        {
            record.children.push(id);
        }
        id
    }

    /// 先释放本 Owner 的订阅/信号/Memo/事件，再递归释放子 Owner；
    /// 这样已卸载分支的旧回调不会命中新一代界面。
    pub(crate) fn dispose_owner(&mut self, id: u32) {
        let Some(record) = self.owners.get_mut(&id) else {
            return;
        };
        if record.disposed {
            return;
        }
        record.disposed = true;
        let children = std::mem::take(&mut record.children);
        let resources = std::mem::take(&mut record.resources);
        let elements = std::mem::take(&mut record.elements);
        let components = std::mem::take(&mut record.components);
        let parent = record.parent;

        // 随 owner 释放的行带请求不会再有宿主读取；同时丢弃，避免窗口化
        // For 反复挂载/卸载时累积失效请求。
        if let Some(bands) = self.owner_bands.remove(&id) {
            for band in bands {
                self.row_windows.remove(&band);
            }
        }
        self.release_resources(resources);
        for child in children {
            self.dispose_owner(child);
        }
        // 槽位只保留 NodeId tombstone：旧 NodeId 仍是 Stale，新挂载换
        // generation 后复用槽位，挂载次数不再决定常驻内存。
        for slot in elements {
            self.retire_element(slot);
        }
        for slot in components {
            self.retire_component(slot);
        }
        if id != 0 {
            if let Some(parent) = parent
                && let Some(record) = self.owners.get_mut(&parent)
            {
                record.children.retain(|child| *child != id);
            }
            self.owners.remove(&id);
        }
    }

    fn release_resources(&mut self, resources: Vec<Resource>) {
        for resource in resources {
            match resource {
                Resource::Signal(signal) => {
                    if let Some(cell) = self.signals.get_mut(signal as usize) {
                        cell.disposed = true;
                    }
                }
                Resource::Memo(memo) => {
                    if let Some(cell) = self.memos.get_mut(memo as usize) {
                        cell.disposed = true;
                        cell.compute = None;
                        cell.value = None;
                        cell.dirty = true;
                        cell.poisoned = None;
                    }
                }
                Resource::Event(event) => {
                    if let Some(cell) = self.events.get_mut(event) {
                        *cell = None;
                        self.free_events.push(event);
                    }
                }
                Resource::Node(node) => self.retire_node(node),
            }
        }
    }

    fn release_root_resources(&mut self) {
        let resources = self
            .owners
            .get_mut(&0)
            .map(|record| std::mem::take(&mut record.resources))
            .unwrap_or_default();
        self.release_resources(resources);
        // 失败的根挂载把绑定表一并丢弃；已提交的根树保留自己的表。
        if !self.trees.contains_key(&TreeSlot::Root)
            && let Some(tree) = self.arena_trees.get(&TreeSlot::Root).copied()
        {
            self.nodes.drop_binding_table(tree);
        }
    }

    /// 树的所有节点释放后丢弃它的绑定表；重新打开窗口会建立新表。
    fn drop_tree_binding_table(&mut self, slot: TreeSlot) {
        if let Some(tree) = self.arena_trees.get(&slot).copied() {
            self.nodes.drop_binding_table(tree);
        }
    }

    /// 读取外观；memo 计算期间记为依赖。
    pub(crate) fn read_appearance(&mut self) -> Appearance {
        if self.phase == Phase::Memo {
            self.note_dep(DepKey::Appearance);
        }
        self.appearance
    }

    /// 更新外观；变化时只失效依赖外观的 memo。
    pub(crate) fn set_appearance(&mut self, appearance: Appearance) {
        if self.appearance == appearance {
            return;
        }
        self.appearance = appearance;
        self.mark_all_trees_changed();
        self.bump(DepKey::Appearance);
    }

    pub(crate) fn window_is_live(&self, key: &'static str, activation: u32, owner: u32) -> bool {
        let open = self.windows.get(key).is_some_and(|record| {
            record.open && record.activation == activation && record.owner == owner
        });
        open && self
            .owners
            .get(&owner)
            .is_some_and(|record| !record.disposed)
    }

    pub(crate) fn owner_is_within(&self, owner: u32, ancestor: u32) -> bool {
        let mut current = Some(owner);
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            current = self.owners.get(&id).and_then(|record| record.parent);
        }
        false
    }

    pub(crate) fn begin_window(&mut self, key: &'static str) -> (ActivationId, u32) {
        self.window_factory_ran = true;
        // 旧 activation 的处理器不能进入新 factory；正常关闭已经清理过。
        self.drop_handlers.remove(key);
        self.keydown_handlers.remove(key);
        self.command_handlers.remove(key);
        self.menu_state_handlers.remove(key);
        let generation = self
            .windows
            .get(key)
            .map_or(1, |record| record.activation + 1);
        // 槽位是 runtime 全局的：同一次目录打开得到不同令牌，重新打开换新槽。
        let slot = self.next_activation_slot;
        self.next_activation_slot = self.next_activation_slot.saturating_add(1).max(1);
        let id = ActivationId::new(slot, generation);
        let owner = self.create_owner(Some(0));
        self.windows.insert(
            key,
            WindowRecord {
                activation: generation,
                id,
                owner,
                open: true,
                visible: true,
            },
        );
        self.bump_window_revision();
        (id, owner)
    }

    pub(crate) fn mark_window_closed(&mut self, key: &'static str) {
        self.close_handlers.remove(key);
        self.drop_handlers.remove(key);
        self.keydown_handlers.remove(key);
        self.command_handlers.remove(key);
        self.menu_state_handlers.remove(key);
        let Some(record) = self.windows.get_mut(key) else {
            return;
        };
        if record.open || record.visible {
            record.open = false;
            record.visible = false;
            self.bump_window_revision();
        }
    }

    /// 记录 UI 处理期的窗口请求；只允许来自仍存活窗口的 Event/Delivery 处理器。
    /// 当前命令 handler 内发起的关闭来源；普通 UI 回调是程序化关闭。
    pub(crate) fn close_request_source(&self) -> Option<CloseSource> {
        self.active_close_source
    }

    pub(crate) fn queue_window_request(
        &mut self,
        request: WindowRequest,
    ) -> Result<(), WindowError> {
        if !matches!(self.phase, Phase::Event | Phase::Delivery) {
            return Err(WindowError::State(
                "window requests are only available during event or delivery handlers",
            ));
        }
        if !self.window_live_in_tree(request.from) {
            return Err(WindowError::Stale);
        }
        if let WindowAction::Show(key) = request.action
            && !self.catalog.contains_key(key)
        {
            return Err(WindowError::UnknownWindow(key));
        }
        self.window_requests.push(request);
        Ok(())
    }

    /// 会话动作与窗口请求共用 Event/Delivery 阶段门，但只按目录键打开窗口。
    pub(crate) fn queue_session_request(
        &mut self,
        action: SessionAction,
    ) -> Result<(), SessionError> {
        if !matches!(self.phase, Phase::Event | Phase::Delivery) {
            return Err(SessionError::State(
                "session requests are only available during event or delivery handlers",
            ));
        }
        if self.session.quitting {
            return Err(SessionError::Ended);
        }
        if let SessionAction::Show(key) = action
            && !self.catalog.contains_key(key)
        {
            return Err(SessionError::UnknownWindow(key));
        }
        self.session.requests.push(action);
        Ok(())
    }

    /// factory 注册入口共用的阶段、owner 与 activation 校验。
    fn ensure_factory_registration(
        &self,
        window: WindowCtx,
        message: &'static str,
    ) -> Result<(), WindowError> {
        if self.phase != Phase::Setup || self.current_owner != Some(window.owner) {
            return Err(WindowError::State(message));
        }
        if !self.window_is_live(window.key, window.activation, window.owner) {
            return Err(WindowError::Stale);
        }
        Ok(())
    }

    /// 在窗口 factory 中登记关闭处理器；必须是该 activation 自己的 Setup。
    pub(crate) fn register_close_handler(
        &mut self,
        window: WindowCtx,
        handler: CloseHandler,
    ) -> Result<(), WindowError> {
        self.ensure_factory_registration(
            window,
            "close handlers must be registered during the window factory",
        )?;
        if self.close_handlers.contains_key(window.key) {
            return Err(WindowError::State(
                "a close handler is already registered for this activation",
            ));
        }
        self.close_handlers.insert(window.key, handler);
        Ok(())
    }

    /// 在窗口 factory 中登记文件拖放处理器；必须是该 activation 自己的 Setup。
    pub(crate) fn register_drop_handler(
        &mut self,
        window: WindowCtx,
        handler: DropHandler,
    ) -> Result<(), WindowError> {
        self.ensure_factory_registration(
            window,
            "drop handlers must be registered during the window factory",
        )?;
        if self.drop_handlers.contains_key(window.key) {
            return Err(WindowError::State(
                "a drop handler is already registered for this activation",
            ));
        }
        self.drop_handlers.insert(window.key, handler);
        Ok(())
    }

    /// 在窗口 factory 中登记窗口根 keydown fallback；每个 activation 一次。
    pub(crate) fn register_keydown_handler(
        &mut self,
        window: WindowCtx,
        handler: WindowKeyDownHandler,
    ) -> Result<(), WindowError> {
        self.ensure_factory_registration(
            window,
            "keydown handlers must be registered during the window factory",
        )?;
        if self.keydown_handlers.contains_key(window.key) {
            return Err(WindowError::State(
                "a keydown handler is already registered for this activation",
            ));
        }
        self.keydown_handlers.insert(window.key, handler);
        Ok(())
    }

    /// 在窗口 factory 中登记命令处理器；只接受该 activation 自己的 Setup。
    pub(crate) fn register_command_handler(
        &mut self,
        window: WindowCtx,
        id: &'static str,
        handler: CommandHandler,
    ) -> Result<(), WindowError> {
        self.ensure_factory_registration(
            window,
            "command handlers must be registered during the window factory",
        )?;
        if !self.menu_declares(window.key, id) {
            return Err(WindowError::Command(CommandError::UnknownCommand(id)));
        }
        let handlers = self.command_handlers.entry(window.key).or_default();
        if handlers.contains_key(id) {
            return Err(WindowError::Command(CommandError::AlreadyRegistered(id)));
        }
        handlers.insert(id, handler);
        Ok(())
    }

    /// 在窗口 factory 中登记只读菜单状态回调；每个 activation 一次。
    pub(crate) fn register_menu_state_handler(
        &mut self,
        window: WindowCtx,
        handler: MenuStateHandler,
    ) -> Result<(), WindowError> {
        self.ensure_factory_registration(
            window,
            "menu state handlers must be registered during the window factory",
        )?;
        if self.menu_state_handlers.contains_key(window.key) {
            return Err(WindowError::State(
                "a menu state handler is already registered for this activation",
            ));
        }
        self.menu_state_handlers.insert(window.key, handler);
        Ok(())
    }

    /// 菜单声明但既没有窗口处理器也没有应用级处理器的第一个命令 id。
    ///
    /// 打开窗口前必须为空。应用级处理器可以被多个窗口的菜单共享，因此声明
    /// 项不要求在 factory 里重复注册。
    pub(crate) fn missing_command_handler(&self, key: &'static str) -> Option<&'static str> {
        let entry = self.catalog.get(key)?;
        let handlers = self.command_handlers.get(key);
        entry
            .menu
            .iter()
            .chain(entry.context_menu.iter())
            .flat_map(MenuSpec::command_ids)
            .find(|id| {
                !self.app_command_handlers.contains_key(id)
                    && handlers.is_none_or(|handlers| !handlers.contains_key(id))
            })
    }

    /// 窗口的菜单栏或上下文菜单是否声明了这个命令 id。
    pub(crate) fn menu_declares(&self, key: &'static str, id: &str) -> bool {
        self.catalog.get(key).is_some_and(|entry| {
            entry
                .menu
                .iter()
                .chain(entry.context_menu.iter())
                .any(|menu| menu.contains_command(id))
        })
    }

    /// 当前 activation 注册的命令处理器。
    pub(crate) fn command_handler(&self, key: &'static str, id: &str) -> Option<CommandHandler> {
        self.command_handlers
            .get(key)
            .and_then(|handlers| handlers.get(id))
            .cloned()
    }

    /// 应用级命令处理器；窗口关闭不影响它。
    pub(crate) fn app_command_handler(&self, id: &str) -> Option<CommandHandler> {
        self.app_command_handlers.get(id).cloned()
    }

    /// 确认窗口读取父窗口当前 epoch 的请求；普通窗口或没有待决请求时为 `None`。
    pub(crate) fn close_request_for(
        &self,
        window: WindowCtx,
    ) -> Result<Option<CloseRequest>, WindowError> {
        if !self.window_is_live(window.key, window.activation, window.owner) {
            return Err(WindowError::Stale);
        }
        let Some(entry) = self.catalog.get(window.key) else {
            return Err(WindowError::UnknownWindow(window.key));
        };
        let WindowRole::CloseConfirmation { parent } = entry.spec.role() else {
            return Ok(None);
        };
        let Some(epoch) = self.close_epochs.get(parent) else {
            return Ok(None);
        };
        if epoch.confirmation != window.key
            || !self.window_is_live(
                epoch.parent.key,
                epoch.parent.activation,
                epoch.parent.owner,
            )
        {
            return Ok(None);
        }
        Ok(Some(CloseRequest::new(self.id, epoch.parent, epoch.id)))
    }

    /// 记录一次回复；第一个有效回复赢得 epoch，其余返回 `Stale`。
    pub(crate) fn queue_close_reply(
        &mut self,
        request: CloseRequest,
        reply: CloseReply,
    ) -> Result<(), WindowError> {
        if !matches!(self.phase, Phase::Event | Phase::Delivery) {
            return Err(WindowError::State(
                "close replies are only available during event or delivery handlers",
            ));
        }
        let epoch = self.close_epochs.get(request.parent.key);
        let valid = epoch.is_some_and(|epoch| {
            epoch.id == request.epoch
                && epoch.parent == request.parent
                && epoch.reply.is_none()
                && self.window_is_live(
                    request.parent.key,
                    request.parent.activation,
                    request.parent.owner,
                )
        });
        if !valid {
            return Err(WindowError::Stale);
        }
        let Some(epoch) = self.close_epochs.get_mut(request.parent.key) else {
            return Err(WindowError::Stale);
        };
        epoch.reply = Some(reply);
        self.close_replies
            .push(CloseReplyRequest { request, reply });
        Ok(())
    }

    /// 目录中绑定到 `parent` 的确认窗口；目录校验保证至多一个。
    pub(crate) fn confirmation_window(&self, parent: &'static str) -> Option<&'static str> {
        self.catalog.iter().find_map(|(key, entry)| {
            (entry.spec.confirmation_parent() == Some(parent)).then_some(*key)
        })
    }

    /// 作为某个待决 epoch 确认窗口的父窗口 identity。
    pub(crate) fn pending_parent_for_confirmation(&self, key: &'static str) -> Option<WindowCtx> {
        self.close_epochs
            .values()
            .find(|epoch| epoch.confirmation == key)
            .map(|epoch| epoch.parent)
    }

    /// 可见且没有待决关闭的窗口才接收输入。
    pub(crate) fn window_accepts_input(&self, key: &'static str) -> bool {
        self.window_is_visible(key) && !self.close_epochs.contains_key(key)
    }

    fn element_accepts_input(&self, element: usize) -> bool {
        self.tree_for_element(element).is_some_and(|key| match key {
            TreeSlot::Root => true,
            TreeSlot::Window(key) => self.window_accepts_input(key),
        })
    }

    /// 正常关闭只取消窗口所有权交付；app 所有权任务按声明继续存活。
    pub(crate) fn cancel_window(&mut self, key: &'static str, activation: u32) {
        self.cancel_window_tasks(key, activation);
        self.mark_window_closed(key);
    }

    /// 失败的挂载还取消在该 setup 子树中启动的任务，包括 app 所有权任务。
    pub(crate) fn abort_window(&mut self, key: &'static str, activation: u32, owner: u32) {
        self.cancel_window_tasks(key, activation);
        self.cancel_tasks_started_under(owner);
        self.mark_window_closed(key);
    }

    pub(crate) fn has_tree(&mut self) -> bool {
        !self.trees.is_empty()
    }

    fn context_error(fault: ContextFault) -> MountError {
        MountError::Context {
            operation: fault.operation,
            phase: fault.phase,
        }
    }

    fn find_element(&self, target: &FindTarget) -> Option<usize> {
        let active = self.active_tree_key();
        if let Some(tree) = active.and_then(|key| self.trees.get(&key))
            && let Some(element) = find_element(self, &tree.nodes, target)
        {
            return Some(element);
        }
        self.trees
            .iter()
            .filter(|(key, _)| Some(**key) != active)
            .find_map(|(_, tree)| find_element(self, &tree.nodes, target))
    }

    fn put_event(&mut self, id: usize, event: Event) {
        // 派发期间元素可能已被卸载并复用槽位；只有槽位仍为空时才放回，
        // 否则交给新挂载的处理器。
        if self.events.get(id).is_some_and(Option::is_none) {
            self.events[id] = Some(event);
        }
    }

    fn update_local(&mut self, element: usize, value: LocalUpdate) {
        match (self.element_local_mut(element), value) {
            (LocalState::TextInput(editor), LocalUpdate::Input(value)) => {
                let length = value.chars().count();
                editor.value = value;
                editor.caret = length;
                editor.anchor = length;
                editor.selection = None;
                // 整体替换是外部编辑：丢弃在途派发，等待绑定确认新值。
                editor.lineage.discard_pending();
            }
            (LocalState::Checkbox { checked }, LocalUpdate::Change(value)) => {
                *checked = value;
            }
            _ => return,
        }
        self.mark_element_changed(element);
    }

    fn text_input(&self, target: &FindTarget) -> Option<usize> {
        let element = self.find_element(target)?;
        match self.element_local(element) {
            LocalState::TextInput(editor) => Some(editor.caret),
            _ => None,
        }
    }

    /// 只校验控件类型：组合中的取消必须对已禁用的输入仍然可用。
    fn require_text_input(
        &self,
        node: NodeId,
        element: usize,
        operation: NodeOperation,
    ) -> Result<(), DispatchError> {
        if matches!(self.element_local(element), LocalState::TextInput(_)) {
            Ok(())
        } else {
            Err(DispatchError::WrongKind { node, operation })
        }
    }

    /// 文本输入且未禁用；不检查组合状态。
    fn require_enabled_text_input(
        &self,
        node: NodeId,
        element: usize,
        operation: NodeOperation,
    ) -> Result<(), DispatchError> {
        self.require_text_input(node, element, operation)?;
        if self.element_is_disabled(element) {
            return Err(DispatchError::Rejected {
                node,
                operation,
                reason: "text input is disabled",
            });
        }
        Ok(())
    }

    /// 可编辑的文本输入：文本输入、未禁用、未在组合中。
    fn require_editable_text_input(
        &self,
        node: NodeId,
        element: usize,
        operation: NodeOperation,
    ) -> Result<(), DispatchError> {
        self.require_enabled_text_input(node, element, operation)?;
        if self.element_local(element).is_composing() {
            return Err(DispatchError::Rejected {
                node,
                operation,
                reason: "text input is composing",
            });
        }
        Ok(())
    }

    /// 组合中的文本输入；操作必须是组合终态或更新。
    fn require_composing_text_input(
        &self,
        node: NodeId,
        element: usize,
        operation: NodeOperation,
    ) -> Result<(), DispatchError> {
        if self.element_local(element).is_composing() {
            Ok(())
        } else {
            Err(DispatchError::Rejected {
                node,
                operation,
                reason: "text input is not composing",
            })
        }
    }

    /// 把一个已分配过的 Node ID 解析回旧的元素存储槽位；不检查可见性。
    ///
    /// `NodeId` 的 arena 槽位与元素存储下标是两套空间，任何需要元素数据的
    /// 路径都必须经过这里，不能把 `slot()` 当下标使用。
    fn element_slot(&self, node: NodeId) -> Option<usize> {
        if node.runtime_id() != self.id {
            return None;
        }
        let slot = self
            .nodes
            .require_kind(node.node(), NodeKindTag::Element)
            .ok()?;
        let element = slot.payload as usize;
        self.elements
            .get(element)
            .is_some_and(|element| element.id == node)
            .then_some(element)
    }

    /// 判定一个 Node ID 相对已提交界面的状态。
    fn node_status(&self, node: NodeId) -> NodeStatus {
        if node.runtime_id() != self.id || !self.nodes.slot_exists(node.node()) {
            return NodeStatus::Unknown;
        }
        let Some(element) = self.element_slot(node) else {
            return NodeStatus::Stale;
        };
        if self.element_is_mounted(element) {
            NodeStatus::Live
        } else {
            NodeStatus::Stale
        }
    }

    /// 解析一个存活的 Node ID；未知、已卸载与隐藏窗口分别报错，绝不猜测替代目标。
    fn element_for_node(&self, node: NodeId) -> Result<usize, DispatchError> {
        self.element_for_node_with(node, false)
    }

    /// 与 [`State::element_for_node`] 相同；`allow_suspended` 仅供取消 IME 组合使用，
    /// 否则用户无法从模态关闭提示中取消父窗口里已经开始的组合。
    fn element_for_node_with(
        &self,
        node: NodeId,
        allow_suspended: bool,
    ) -> Result<usize, DispatchError> {
        if node.runtime_id() != self.id {
            return Err(DispatchError::UnknownNode(node));
        }
        if !self.nodes.slot_exists(node.node()) {
            return Err(DispatchError::UnknownNode(node));
        }
        let Some(element) = self.element_slot(node) else {
            return Err(DispatchError::StaleNode(node));
        };
        match self.tree_for_element(element) {
            Some(key) if self.tree_is_visible(key) => {
                if allow_suspended || self.element_accepts_input(element) {
                    Ok(element)
                } else {
                    Err(DispatchError::Suspended(node))
                }
            }
            Some(_) => Err(DispatchError::Hidden(node)),
            None => Err(DispatchError::StaleNode(node)),
        }
    }

    fn set_text_input_caret(&mut self, target: &FindTarget, offset: usize) -> bool {
        let Some(element) = self.find_element(target) else {
            return false;
        };
        if !self.element_accepts_input(element) {
            return false;
        }
        let LocalState::TextInput(editor) = self.element_local_mut(element) else {
            return false;
        };
        if editor.is_composing() {
            return false;
        }
        let changed = editor.caret != offset
            || editor.anchor != offset
            || editor.selection != Some((offset, offset));
        editor.caret = offset;
        editor.anchor = offset;
        editor.selection = Some((offset, offset));
        if changed {
            self.mark_element_changed(element);
            if let Some(key) = self.tree_for_element(element) {
                self.commit_tree_revision(key);
            }
        }
        true
    }

    /// 原生控件与显式挂接 focus/blur 的元素参与遍历；禁用元素不参与接任或 Tab 顺序。
    fn is_focusable(&self, element: usize) -> bool {
        !self.element_is_disabled(element) && self.element_is_inherently_focusable(element)
    }

    /// 元素本身的键盘可聚焦语义；`disabled` 与可见性由调用方另行组合。
    fn element_is_inherently_focusable(&self, element: usize) -> bool {
        inherent_focusable(
            self,
            self.element_tag(element),
            self.element_events(element),
        )
    }

    /// Tab/方向键共用的确定性前序：控件先于自己的子树，Show 只看活动分支，For 按业务顺序。
    fn focus_order(&self, key: TreeSlot) -> Vec<usize> {
        let mut order = Vec::new();
        if let Some(tree) = self.tree(key) {
            collect_focusable(self, &tree.nodes, &mut order);
        }
        order
    }

    /// 结算快照包含旧一代的全部挂载元素；某个控件当时禁用、本帧重新启用时仍可接任。
    fn element_order(&self, key: TreeSlot) -> Vec<usize> {
        let mut order = Vec::new();
        if let Some(tree) = self.tree(key) {
            collect_elements(self, &tree.nodes, &mut order);
        }
        order
    }

    fn element_is_mounted(&self, element: usize) -> bool {
        self.tree_for_element(element).is_some()
    }

    /// 旧焦点不在新的可聚焦集合时，下一次结算才产生接任目标。
    ///
    /// 只在旧一代的挂载顺序里选：同 tag 的下一控件、同 tag 的上一控件，
    /// 再退到任意下一控件、任意上一控件；本帧新挂载的控件不会因返场抢焦点，
    /// 当时禁用、本帧重新启用的旧控件仍然可以接任。
    fn focus_settlement(&mut self, key: TreeSlot) -> Option<FocusSettlement> {
        let old = self.focused_in(key)?;
        let after = self.focus_order(key);
        if after.contains(&old) {
            return None;
        }
        let (before, tag) = {
            let tree = self.tree_mut(key)?;
            (
                tree.settle_before.take().unwrap_or_default(),
                tree.focused_tag,
            )
        };
        let target = focus_replacement(self, &before, &after, old, tag);
        Some(FocusSettlement {
            old,
            blur_old: self.element_is_mounted(old),
            target,
        })
    }

    fn take_element_event(&mut self, element: usize, kind: EventKind) -> Option<(usize, Event)> {
        let event = self.element_events(element).iter().copied().find(|id| {
            self.events
                .get(*id)
                .and_then(Option::as_ref)
                .is_some_and(|event| event_kind(event) == kind)
        })?;
        Some((event, self.events.get_mut(event)?.take()?))
    }
}

pub enum MountedNode {
    Text(u32),
    Element(usize),
    Component(usize),
    Show(u32),
    For(u32),
}

pub struct MountedElement {
    id: NodeId,
    children: Vec<MountedNode>,
}

struct MountedInlineStyle {
    property: zenui_style::Property,
    binding: BindingRef,
}

pub struct MountedAttr {
    name: &'static str,
    binding: BindingRef,
}

/// 已提交属性值的比较用投影；值本身只在 `BindingSlot.current`。
#[derive(Clone, PartialEq, Eq)]
enum AttrCurrent {
    Text(String),
    Bool(bool),
}

/// 一个属性绑定的求值计划；已提交值只用于比较，写回仍以槽位为唯一权威。
struct AttrPlan {
    name: &'static str,
    binding: BindingRef,
    source: ReadSlot,
    value_type: BindingType,
    committed: Option<AttrCurrent>,
}

impl AttrPlan {
    fn describe(table: &BindingTable, name: &'static str, binding: BindingRef) -> Self {
        let source = table
            .read_slot(binding)
            .expect("a mounted attribute binding is live");
        let value_type = table
            .kind(binding)
            .expect("a mounted attribute binding has a value type");
        let committed = match value_type {
            BindingType::Text => table
                .text_current(binding)
                .map(|value| AttrCurrent::Text(value.to_owned())),
            BindingType::Bool => table.bool_current(binding).map(AttrCurrent::Bool),
            _ => unreachable!("an attribute binding is text or bool"),
        };
        Self {
            name,
            binding,
            source,
            value_type,
            committed,
        }
    }

    /// 静态/once 槽返回 `None`：值已在挂载时提交，永不重算。
    fn evaluate(&self) -> Option<ReadValue<AttrCurrent>> {
        if self.source.is_static() {
            return None;
        }
        Some(match self.value_type {
            BindingType::Text => match self.source.evaluate::<String>() {
                ReadValue::Value(value) => ReadValue::Value(AttrCurrent::Text(value)),
                ReadValue::Fault(fault) => ReadValue::Fault(fault),
            },
            BindingType::Bool => match self.source.evaluate::<bool>() {
                ReadValue::Value(value) => ReadValue::Value(AttrCurrent::Bool(value)),
                ReadValue::Fault(fault) => ReadValue::Fault(fault),
            },
            _ => unreachable!("an attribute binding is text or bool"),
        })
    }

    fn mark_fault(&self, state: &mut State, tree: TreeKey, fault: ReadFault) {
        match self.value_type {
            BindingType::Text => {
                if let Some(slot) = state
                    .nodes
                    .binding_table_mut(tree)
                    .slot_mut::<String>(self.binding)
                {
                    slot.fault = Some(fault);
                }
            }
            BindingType::Bool => {
                if let Some(slot) = state
                    .nodes
                    .binding_table_mut(tree)
                    .slot_mut::<bool>(self.binding)
                {
                    slot.fault = Some(fault);
                }
            }
            _ => unreachable!("an attribute binding is text or bool"),
        }
    }

    /// 写回求值结果；返回可见值是否变化。失败从不走到这里。
    fn commit_value(
        &self,
        state: &mut State,
        tree: TreeKey,
        node: NodeRef,
        next: AttrCurrent,
    ) -> bool {
        let part = BindingPart::Attr(self.name);
        match (self.committed.clone(), next) {
            (Some(AttrCurrent::Text(old)), AttrCurrent::Text(value)) => {
                let changed = old != value;
                if let Some(slot) = state
                    .nodes
                    .binding_table_mut(tree)
                    .slot_mut::<String>(self.binding)
                {
                    slot.fault = None;
                    if changed {
                        slot.current = Some(value.clone());
                    }
                }
                if changed {
                    state.record_binding_patch(tree, node, part, Some(old), Some(value), None);
                }
                changed
            }
            (None, AttrCurrent::Text(value)) => {
                if let Some(slot) = state
                    .nodes
                    .binding_table_mut(tree)
                    .slot_mut::<String>(self.binding)
                {
                    slot.fault = None;
                    slot.current = Some(value.clone());
                }
                state.record_binding_patch::<String>(tree, node, part, None, Some(value), None);
                true
            }
            (Some(AttrCurrent::Bool(old)), AttrCurrent::Bool(value)) => {
                let changed = old != value;
                if let Some(slot) = state
                    .nodes
                    .binding_table_mut(tree)
                    .slot_mut::<bool>(self.binding)
                {
                    slot.fault = None;
                    if changed {
                        slot.current = Some(value);
                    }
                }
                if changed {
                    state.record_binding_patch(tree, node, part, Some(old), Some(value), None);
                }
                changed
            }
            (None, AttrCurrent::Bool(value)) => {
                if let Some(slot) = state
                    .nodes
                    .binding_table_mut(tree)
                    .slot_mut::<bool>(self.binding)
                {
                    slot.fault = None;
                    slot.current = Some(value);
                }
                state.record_binding_patch::<bool>(tree, node, part, None, Some(value), None);
                true
            }
            _ => unreachable!("an attribute binding keeps one value type"),
        }
    }
}

/// 一条 inline style 绑定的求值计划。
struct StylePlan {
    property: Property,
    binding: BindingRef,
    source: ReadSlot,
    value_type: BindingType,
    committed: Option<StyleValue>,
}

impl StylePlan {
    fn describe(table: &BindingTable, property: Property, binding: BindingRef) -> Self {
        let source = table
            .read_slot(binding)
            .expect("a mounted style binding is live");
        let value_type = table
            .kind(binding)
            .expect("a mounted style binding has a value type");
        let committed = table.inline_style(binding);
        Self {
            property,
            binding,
            source,
            value_type,
            committed,
        }
    }

    fn evaluate(&self) -> Option<ReadValue<StyleValue>> {
        if self.source.is_static() {
            return None;
        }
        Some(match self.value_type {
            BindingType::Color => match self.source.evaluate::<Color>() {
                ReadValue::Value(value) => ReadValue::Value(StyleValue::Color(value)),
                ReadValue::Fault(fault) => ReadValue::Fault(fault),
            },
            BindingType::Length => match self.source.evaluate::<Length>() {
                ReadValue::Value(value) => ReadValue::Value(StyleValue::Length(value)),
                ReadValue::Fault(fault) => ReadValue::Fault(fault),
            },
            BindingType::Opacity => match self.source.evaluate::<Opacity>() {
                ReadValue::Value(value) => ReadValue::Value(StyleValue::Opacity(value.value())),
                ReadValue::Fault(fault) => ReadValue::Fault(fault),
            },
            _ => unreachable!("a style binding is color, length or opacity"),
        })
    }

    fn mark_fault(&self, state: &mut State, tree: TreeKey, fault: ReadFault) {
        match self.value_type {
            BindingType::Color => {
                if let Some(slot) = state
                    .nodes
                    .binding_table_mut(tree)
                    .slot_mut::<Color>(self.binding)
                {
                    slot.fault = Some(fault);
                }
            }
            BindingType::Length => {
                if let Some(slot) = state
                    .nodes
                    .binding_table_mut(tree)
                    .slot_mut::<Length>(self.binding)
                {
                    slot.fault = Some(fault);
                }
            }
            BindingType::Opacity => {
                if let Some(slot) = state
                    .nodes
                    .binding_table_mut(tree)
                    .slot_mut::<Opacity>(self.binding)
                {
                    slot.fault = Some(fault);
                }
            }
            _ => unreachable!("a style binding is color, length or opacity"),
        }
    }

    /// 写回求值结果；返回可见样式是否变化。
    fn commit_value(
        &self,
        state: &mut State,
        tree: TreeKey,
        node: NodeRef,
        next: StyleValue,
    ) -> bool {
        let part = BindingPart::Style(self.property);
        match next {
            StyleValue::Color(value) => {
                let old = match self.committed {
                    Some(StyleValue::Color(old)) => Some(old),
                    _ => None,
                };
                let changed = old != Some(value);
                if let Some(slot) = state
                    .nodes
                    .binding_table_mut(tree)
                    .slot_mut::<Color>(self.binding)
                {
                    slot.fault = None;
                    slot.current = Some(value);
                }
                if changed {
                    state.record_binding_patch(tree, node, part, old, Some(value), None);
                }
                changed
            }
            StyleValue::Length(value) => {
                let old = match self.committed {
                    Some(StyleValue::Length(old)) => Some(old),
                    _ => None,
                };
                let changed = old != Some(value);
                if let Some(slot) = state
                    .nodes
                    .binding_table_mut(tree)
                    .slot_mut::<Length>(self.binding)
                {
                    slot.fault = None;
                    slot.current = Some(value);
                }
                if changed {
                    state.record_binding_patch(tree, node, part, old, Some(value), None);
                }
                changed
            }
            StyleValue::Opacity(value) => {
                let old = match self.committed {
                    Some(StyleValue::Opacity(old)) => Some(old),
                    _ => None,
                };
                let changed = old != Some(value);
                if let Some(slot) = state
                    .nodes
                    .binding_table_mut(tree)
                    .slot_mut::<Opacity>(self.binding)
                {
                    slot.fault = None;
                    slot.current = Some(Opacity::percent(value));
                }
                if changed {
                    state.record_binding_patch(
                        tree,
                        node,
                        part,
                        old.map(Opacity::percent),
                        Some(Opacity::percent(value)),
                        None,
                    );
                }
                changed
            }
            _ => unreachable!("a style binding keeps one value type"),
        }
    }
}

enum LocalState {
    None,
    /// Box 让每个元素槽位保持小尺寸，只有文本输入承担编辑器状态的分配。
    TextInput(Box<TextEditing>),
    Checkbox {
        checked: bool,
    },
}

impl LocalState {
    fn is_composing(&self) -> bool {
        matches!(self, Self::TextInput(editor) if editor.is_composing())
    }
}

/// 一个已挂载文本输入的本地编辑状态。
///
/// `value` 是编辑器当前文本：本地编辑先写这里，模型结算或外部替换再更新它。
/// 渲染与辅助功能读取的是这个值，因此模型滞后时不会把刚输入的文字回滚。
struct TextEditing {
    value: String,
    caret: usize,
    /// 选择的活动端（焦点）；`caret` 与它相同表示折叠。
    anchor: usize,
    selection: Option<(usize, usize)>,
    composition: Option<Composition>,
    /// 这个输入自己的撤销/重做历史；随元素卸载释放。
    history: TextHistory,
    lineage: EditLineage,
}

impl TextEditing {
    fn new(origin: u64) -> Self {
        Self {
            value: String::new(),
            caret: 0,
            anchor: 0,
            selection: None,
            composition: None,
            history: TextHistory::default(),
            lineage: EditLineage::new(origin),
        }
    }

    fn is_composing(&self) -> bool {
        self.composition.is_some()
    }

    /// 记一次本地派发并把编辑器文本推进到派发值；处理器随后运行。
    fn dispatch(&mut self, value: &str, epoch: Option<u64>) -> TextTrace {
        self.adopt(value);
        let lineage = &mut self.lineage;
        lineage.revision = lineage.revision.wrapping_add(1);
        lineage.in_flight.push_back(DispatchedEdit {
            revision: lineage.revision,
            digest: text_digest(value),
        });
        while lineage.in_flight.len() > TEXT_IN_FLIGHT {
            lineage.in_flight.pop_front();
        }
        lineage.turn_epoch = Some(epoch);
        TextTrace::LocalEdit {
            origin: lineage.origin,
            revision: lineage.revision,
        }
    }

    /// 归约刷新看到的已提交值；返回需要写入 trace 的分类事件。
    ///
    /// 模型是值的权威，但模型可能滞后于本地编辑。分类依据是编辑谱系而不是
    /// 单纯的“值变了”：同一命令内处理器写过的绑定按结算处理（包括归一化、
    /// 截断或写回原值）；晚到的旧 revision 结算被忽略；其余变化是外部替换。
    fn observe(&mut self, committed: &str, epoch: Option<u64>) -> Option<TextTrace> {
        let origin = self.lineage.origin;
        if self.lineage.observed.is_none() {
            // 挂载后的第一次刷新：以绑定值初始化编辑器，没有可分类的编辑。
            self.adopt(committed);
            self.lineage.observed = Some(committed.to_owned());
            return None;
        }
        let observed_changed = self.lineage.observed.as_deref() != Some(committed);
        let turn = self.lineage.turn_epoch.take();
        // 只有高于结算水位的派发才是当前编辑；更早的记录只用于辨认晚到值。
        let pending = self
            .lineage
            .in_flight
            .back()
            .filter(|edit| edit.revision > self.lineage.settled)
            .map(|edit| edit.revision);
        let digest = text_digest(committed);
        let stale = self
            .lineage
            .in_flight
            .iter()
            .rev()
            .find(|edit| Some(edit.revision) != pending && edit.digest == digest)
            .map(|edit| edit.revision);
        let event = match pending {
            None => {
                if self.value == committed {
                    None
                } else if let Some(stale) = stale {
                    // 已结算 revision 的晚到或重复值：本地文本与历史保持不变。
                    Some(TextTrace::StaleSettlement {
                        origin,
                        current: self.lineage.revision,
                        stale,
                    })
                } else {
                    Some(self.external_reset(committed))
                }
            }
            Some(revision) => {
                let matched = self
                    .lineage
                    .in_flight
                    .back()
                    .is_some_and(|edit| edit.revision == revision && edit.digest == digest);
                if matched {
                    let normalized = self.settle_committed(committed, revision);
                    Some(TextTrace::Settlement {
                        origin,
                        revision,
                        normalized,
                    })
                } else if let Some(stale) = stale {
                    // 旧 revision 的结算晚到：保留最新本地文本与历史。
                    Some(TextTrace::StaleSettlement {
                        origin,
                        current: revision,
                        stale,
                    })
                } else if let Some(base) = turn {
                    let written = match (base, epoch) {
                        (None, _) | (Some(_), None) => true,
                        (Some(base), Some(current)) => base != current,
                    };
                    // 同一命令内的刷新：处理器已经处理过这次编辑，写入值
                    // 即使等于旧值或被归一化，也仍是这次编辑的结算。
                    if written || observed_changed {
                        let normalized = self.settle_committed(committed, revision);
                        Some(TextTrace::Settlement {
                            origin,
                            revision,
                            normalized,
                        })
                    } else {
                        None
                    }
                } else if !observed_changed {
                    // 模型仍停在编辑前的值：处理器把写入推迟到了之后的命令。
                    None
                } else {
                    Some(self.external_reset(committed))
                }
            }
        };
        self.note_observed(committed);
        event
    }

    /// 结算当前派发：采用提交值、夹取偏移并推进结算水位。
    fn settle_committed(&mut self, committed: &str, revision: u64) -> bool {
        let normalized = self.value != committed;
        self.settle_value(committed);
        self.lineage.settled = self.lineage.settled.max(revision);
        self.lineage.in_flight.pop_back();
        normalized
    }

    /// 外部替换：丢弃在途派发、清空历史、采用模型值并确定 selection 结果。
    ///
    /// 组合中到达的替换取消组合：preedit 被丢弃且不派发 `on:input`，caret
    /// 落到新值末尾。非组合替换按下述规则夹取：空值归零并清空选择；非空值
    /// 夹取 caret/anchor，完整落在新值内的 selection 保留，越界的收拢到末尾。
    fn external_reset(&mut self, committed: &str) -> TextTrace {
        let origin = self.lineage.origin;
        let revision = self.lineage.revision;
        self.history.clear();
        self.lineage.in_flight.clear();
        self.lineage.settled = self.lineage.revision;
        self.adopt(committed);
        let length = self.value.chars().count();
        if self.composition.take().is_some() {
            self.caret = length;
            self.anchor = length;
            self.selection = None;
            return TextTrace::CompositionCancelledByReset { origin, revision };
        }
        if length == 0 {
            self.caret = 0;
            self.anchor = 0;
            self.selection = None;
        } else if self.selection.is_some_and(|(_, end)| end > length) {
            self.caret = length;
            self.anchor = length;
            self.selection = None;
        } else {
            self.caret = self.caret.min(length);
            self.anchor = self.anchor.min(length);
        }
        TextTrace::ExternalReset { origin, revision }
    }

    /// 采用模型值并把 caret/anchor/selection 夹回文本范围。
    fn settle_value(&mut self, value: &str) {
        self.adopt(value);
        self.clamp_offsets();
    }

    fn adopt(&mut self, value: &str) {
        if self.value != value {
            self.value.clear();
            self.value.push_str(value);
        }
    }

    fn note_observed(&mut self, committed: &str) {
        if self.lineage.observed.as_deref() != Some(committed) {
            self.lineage.observed = Some(committed.to_owned());
        }
    }

    fn clamp_offsets(&mut self) {
        let length = self.value.chars().count();
        self.caret = self.caret.min(length);
        self.anchor = self.anchor.min(length);
        if let Some((start, end)) = self.selection {
            self.selection = Some(text_edit::normalized_range(start, end, length));
        }
    }

    /// 元素卸载时释放编辑器文本、历史与谱系观察值。
    fn reset(&mut self) {
        self.value = String::new();
        self.history.clear();
        self.lineage.discard_pending();
        self.lineage.observed = None;
    }
}

/// 一个文本输入的编辑谱系。
///
/// `origin` 取本次挂载的 Node ID generation：重挂载必然得到新的 origin，
/// 旧挂载的派发也只可能落在旧 Node ID 上。`revision` 只随本地编辑推进，
/// `in_flight` 保留尚未结算的派发摘要，使模型滞后时刷新不会回滚本地文本。
struct EditLineage {
    origin: u64,
    revision: u64,
    /// 已结算的最高 revision。更早的派发即使仍在 `in_flight` 里保留摘要供
    /// 晚到结算匹配，也不再是“当前编辑”，不能被当作待结算的目标。
    settled: u64,
    /// 按 revision 递增的未结算派发；容量有界，更早的记录用于识别晚到的结算。
    in_flight: VecDeque<DispatchedEdit>,
    /// 最近一次刷新看到的已提交值；`None` 表示还未绑定初值。
    observed: Option<String>,
    /// 本次命令的收尾刷新尚未发生。外层 `None` 表示不在命令内；内层 `None`
    /// 表示值绑定不可写（静态值或缺失），此时任何提交值都按已结算处理。
    turn_epoch: Option<Option<u64>>,
}

impl EditLineage {
    fn new(origin: u64) -> Self {
        Self {
            origin,
            revision: 0,
            settled: 0,
            in_flight: VecDeque::new(),
            observed: None,
            turn_epoch: None,
        }
    }

    /// 丢弃在途派发与本次命令的结算窗口；外部整体替换或卸载使用。
    fn discard_pending(&mut self) {
        self.in_flight.clear();
        self.turn_epoch = None;
    }

    /// 派发命令在收尾前失败：这次编辑不再等待结算。
    fn abort_dispatch(&mut self) {
        self.turn_epoch = None;
        self.in_flight.pop_back();
    }
}

/// 一次已派发、尚未被模型结算的本地编辑。
struct DispatchedEdit {
    revision: u64,
    /// 派发值的摘要。保留摘要而不是整段文本，让证据窗口覆盖更多编辑；
    /// 模型值是普通 `String`，识别晚到结算只需要相等性。
    digest: u64,
}

/// 编辑值的稳定摘要；同一进程内的相等比较足够，不用于持久化或跨进程。
fn text_digest(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// 文本编辑谱系归约的有界内部记录；测试据此刻画分类顺序，不属于公开 API。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextTrace {
    /// 一次本地编辑派发。
    LocalEdit { origin: u64, revision: u64 },
    /// 当前编辑被模型结算；`normalized` 表示提交值不同于派发值。
    Settlement {
        origin: u64,
        revision: u64,
        normalized: bool,
    },
    /// 旧编辑的结算晚到，被忽略。
    StaleSettlement {
        origin: u64,
        current: u64,
        stale: u64,
    },
    /// 非本地编辑产生的值；编辑器采用它并清空历史。
    ExternalReset { origin: u64, revision: u64 },
    /// 外部替换在组合中到达：preedit 被丢弃，编辑器采用新值。
    CompositionCancelledByReset { origin: u64, revision: u64 },
    /// 文本输入挂载建立新的编辑 origin；初次挂载同样计入。
    Remount { origin: u64 },
}

/// preedit 只存在于编辑器本地；`base_*` 记录组合开始时的已提交状态，取消时恢复。
struct Composition {
    base_caret: usize,
    base_anchor: usize,
    base_selection: Option<(usize, usize)>,
    preedit: String,
}

impl Composition {
    /// 组合替换的字符区间；没有范围选择时折叠在组合开始的 caret。
    fn base_range(&self) -> (usize, usize) {
        match self.base_selection {
            Some((start, end)) => (start.min(end), start.max(end)),
            None => (self.base_caret, self.base_caret),
        }
    }
}

/// 一次可撤销编辑前后的完整本地状态；`value` 是编辑器当时的文本，
/// 在模型滞后时可能还未被结算。
#[derive(Clone, PartialEq, Eq, Debug)]
struct TextSnapshot {
    value: String,
    caret: usize,
    anchor: usize,
    selection: Option<(usize, usize)>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct TextEntry {
    before: TextSnapshot,
    after: TextSnapshot,
}

impl TextEntry {
    fn bytes(&self) -> usize {
        self.before.value.len() + self.after.value.len()
    }
}

/// 一次已提交编辑是否属于可合并的连续输入。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EditKind {
    /// 单字符插入；相邻同类插入可以合并为一个撤销步。
    Typing(char),
    /// 多字符插入、整体替换、删除或组合提交；结束当前输入 run。
    Boundary,
}

/// 当前打开的连续输入 run；空白附在前一个词上，下一个非空白字符开启新步。
#[derive(Clone, Copy, Debug)]
struct InsertRun {
    scalars: usize,
    last: char,
}

const HISTORY_ENTRIES: usize = 100;
const HISTORY_BYTES: usize = 256 * 1024;

/// 每个文本输入保留的未结算派发摘要数；更早的晚到结算按外部替换处理。
const TEXT_IN_FLIGHT: usize = 256;

/// 编辑谱系 trace 的事件上限；测试只依赖最近一次序列。
const TEXT_TRACE_EVENTS: usize = 64;
/// 一个连续输入 run 最多合并的单字符数；到界后下一次插入开启新步。
const INSERT_RUN_SCALARS: usize = 64;

/// 每个文本输入自己的有界编辑历史。
///
/// 历史只保存已提交值的快照；撤销和重做都通过一次 `on:input` 写回模型，因此
/// 应用始终是值的权威。结算与外部替换的区分在 [`EditLineage`]；这里只保存
/// 撤销/重做栈和连续输入 run。
#[derive(Default)]
struct TextHistory {
    undo: VecDeque<TextEntry>,
    redo: VecDeque<TextEntry>,
    undo_bytes: usize,
    redo_bytes: usize,
    run: Option<InsertRun>,
}

impl TextHistory {
    fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.undo_bytes = 0;
        self.redo_bytes = 0;
        self.run = None;
    }

    /// 结束当前连续输入；下一次单字符插入开启新的撤销步。
    fn break_run(&mut self) {
        self.run = None;
    }

    /// 记录一次已提交编辑；提交前后值相同则不记录，也不清空 redo。
    ///
    /// 相邻单字符插入在同一个 run 内合并：保留最早的 `before`，只推进
    /// `after`。空白并入前一个 run，下一个非空白字符、其它编辑种类、
    /// 移动、焦点变化或撤销/重做都会结束 run。
    fn record(&mut self, before: TextSnapshot, after: TextSnapshot, kind: EditKind) {
        if before.value == after.value {
            return;
        }
        self.redo.clear();
        self.redo_bytes = 0;
        let EditKind::Typing(scalar) = kind else {
            self.run = None;
            self.push_undo(TextEntry { before, after });
            return;
        };
        if self.can_extend_run(scalar, &before) {
            let entry = self.undo.back_mut().expect("an open run has an entry");
            self.undo_bytes = self.undo_bytes.saturating_sub(entry.bytes());
            entry.after = after;
            self.undo_bytes += entry.bytes();
            let run = self.run.as_mut().expect("an open run has state");
            run.scalars += 1;
            run.last = scalar;
            trim_history(&mut self.undo, &mut self.undo_bytes);
            if self.undo.is_empty() {
                self.run = None;
            }
            return;
        }
        self.run = Some(InsertRun {
            scalars: 1,
            last: scalar,
        });
        self.push_undo(TextEntry { before, after });
    }

    fn can_extend_run(&self, scalar: char, before: &TextSnapshot) -> bool {
        let Some(run) = self.run else {
            return false;
        };
        if run.scalars >= INSERT_RUN_SCALARS {
            return false;
        }
        // 空白附在前一个词上；下一个词的首字符开启新步。
        if run.last.is_whitespace() && !scalar.is_whitespace() {
            return false;
        }
        // run 必须仍指向栈顶，且这次插入紧接在它记录的状态之后。
        self.undo.back().is_some_and(|entry| entry.after == *before)
    }

    fn push_undo(&mut self, entry: TextEntry) {
        self.undo_bytes += entry.bytes();
        self.undo.push_back(entry);
        trim_history(&mut self.undo, &mut self.undo_bytes);
    }

    fn push_redo(&mut self, entry: TextEntry) {
        self.redo_bytes += entry.bytes();
        self.redo.push_back(entry);
        trim_history(&mut self.redo, &mut self.redo_bytes);
    }

    /// 取走 undo 栈顶，并同步扣减它占用的字节预算。
    fn pop_undo(&mut self) -> Option<TextEntry> {
        let entry = self.undo.pop_back()?;
        self.undo_bytes = self.undo_bytes.saturating_sub(entry.bytes());
        if self.undo.is_empty() {
            self.undo_bytes = 0;
        }
        Some(entry)
    }

    /// 取走 redo 栈顶，并同步扣减它占用的字节预算。
    fn pop_redo(&mut self) -> Option<TextEntry> {
        let entry = self.redo.pop_back()?;
        self.redo_bytes = self.redo_bytes.saturating_sub(entry.bytes());
        if self.redo.is_empty() {
            self.redo_bytes = 0;
        }
        Some(entry)
    }
}

/// 淘汰最旧条目，直到条目数与快照文本字节数都回到上限内。
fn trim_history(stack: &mut VecDeque<TextEntry>, bytes: &mut usize) {
    while stack.len() > HISTORY_ENTRIES || (*bytes > HISTORY_BYTES && !stack.is_empty()) {
        if let Some(entry) = stack.pop_front() {
            *bytes -= entry.bytes();
        }
    }
}

fn text_snapshot(local: &LocalState) -> Option<TextSnapshot> {
    let LocalState::TextInput(editor) = local else {
        return None;
    };
    Some(TextSnapshot {
        value: editor.value.clone(),
        caret: editor.caret,
        anchor: editor.anchor,
        selection: editor.selection,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HistoryDirection {
    Undo,
    Redo,
}

enum LocalUpdate {
    Input(String),
    Change(bool),
}

pub struct MountedComponent {
    owner: u32,
    children: Vec<MountedNode>,
}

struct ShowData {
    owner: u32,
    node: NodeRef,
    when: BindingRef,
    then: ChildrenFactory,
    fallback: Option<ChildrenFactory>,
    active: Option<MountedBranch>,
    /// 最近一次求值 `when` 的刷新轮次；同一轮不重复求值。
    structure_round: u64,
    /// 活动分支的值最近被预刷新的轮次；同一轮不再做一次值遍历。
    values_round: u64,
}

pub struct MountedBranch {
    which: bool,
    owner: u32,
    children: Vec<MountedNode>,
}

/// 已挂载 For 的行集来源：绑定槽，或跟随 Store 投影。
pub(crate) enum MountedForRows {
    Read(BindingRef),
    Projection(FilterId),
}

struct ForData {
    owner: u32,
    node: NodeRef,
    /// 行数据来源 Store；用于行集归属校验。
    store: StoreId,
    rows: MountedForRows,
    /// 唯一持有 `R`/`K` 的驱动；键映射、快照与行 factory 都经它。
    driver: Rc<dyn ForDriver>,
    /// 窗口化 For 的统一行高；`None` 挂载全部行（ADR-0010）。
    row_height: Option<Length>,
    /// 跨快照稳定的行带键；宿主按它声明行窗口。
    band: u64,
    total_rows: usize,
    first_row: usize,
    /// 最近一次提交的有效行窗口跨度；静态行集的已删除行不在 `order`
    /// 里，因此不能用 `order.len()` 还原宿主请求的窗口。
    window_count: usize,
    /// 窗口化 For 的上下 spacer 元素槽位；保持内容总高度不变。
    spacers: Option<(usize, usize)>,
    /// 最近一次提交的完整投影；完整行顺序的唯一来源。
    committed_rows: Option<RowSet>,
    items: BTreeMap<RowId, MountedItem>,
    /// 当前已挂载窗口的行身份，按呈现顺序；窗口外的行不在其中。
    order: Vec<RowId>,
    /// 最近一次求值 `each` 的刷新轮次；同一轮不重复求值。
    structure_round: u64,
}

pub struct MountedItem {
    owner: u32,
    /// 已提交投影中的下标；焦点 grace 在投影未变时 O(1) 取用。
    row_index: usize,
    /// 挂载时捕获的 Todo 业务键；第二个模型为 None。行删除后保持稳定。
    todo: Option<TodoId>,
    instance: u64,
    children: Vec<MountedNode>,
}

/// `row_height × rows` 保持单位不变；定点乘法饱和，极端行数不会回绕。
fn rows_height(row_height: Length, rows: usize) -> Length {
    let factor = i64::try_from(rows).unwrap_or(i64::MAX);
    let fixed = i64::from(row_height.fixed())
        .saturating_mul(factor)
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    Length::from_fixed(fixed as i32, row_height.unit())
}

/// 为窗口化 For 创建上下两个零高 spacer；高度由每次刷新按窗口更新。
///
/// spacer 是运行时自有的合成绑定：挂载时写入静态 `role` 与高度，之后高度由
/// [`set_spacer_height`] 作为结构几何更新直接写回槽位。
fn ensure_spacers(state: &mut State, owner: u32, parent: Option<NodeRef>) -> (usize, usize) {
    let tree = state.building_tree_key();
    let mut make = || {
        let slot = state.reserve_element(owner);
        let id = state.mint_element_node(slot, owner, parent);
        let (role_part, role, height_part, height) = {
            let table = state.nodes.binding_table_mut(tree);
            (
                table.part_id(BindingPart::Attr("role")),
                table.insert::<String>(ReadSlot::Static, Some(String::from("spacer")), None),
                table.part_id(BindingPart::Style(Property::Height)),
                table.insert::<Length>(ReadSlot::Static, Some(Length::px(0.0)), None),
            )
        };
        let aux = state.nodes.reserve_element_aux(AuxSeed {
            attrs: vec![MountedAttr {
                name: "role",
                binding: role,
            }],
            class_styles: &[],
            inline_styles: vec![MountedInlineStyle {
                property: Property::Height,
                binding: height,
            }],
            events: Vec::new(),
            edit: LocalState::None,
            semantics: SnapshotSemantics::new(false, SnapshotActions::default()),
        });
        {
            let list = state.nodes.binding_list_mut(aux.bindings);
            debug_assert!(list.insert(role_part, role).is_none());
            debug_assert!(list.insert(height_part, height).is_none());
        }
        let tag = state.nodes.intern_tag("view");
        state.nodes.place_element_data(
            slot,
            ElementData {
                tag,
                flags: ElementFlags::NONE,
                owner,
                aux,
            },
        );
        state.place_element(
            slot,
            MountedElement {
                id,
                children: Vec::new(),
            },
        );
        slot
    };
    (make(), make())
}

/// 更新 spacer 的已提交高度；数量和单位一起保留，em 在场景里按最终字号解析。
fn set_spacer_height(state: &mut State, spacer: usize, rows: usize, row_height: Length) {
    let height = rows_height(row_height, rows);
    let Some(binding) = state
        .element_inline_styles(spacer)
        .iter()
        .find(|style| style.property == Property::Height)
        .map(|style| style.binding)
    else {
        return;
    };
    let Some(tree) = state.element_tree(spacer) else {
        return;
    };
    let (changed, old) = {
        let Some(slot) = state
            .nodes
            .binding_table_mut(tree)
            .slot_mut::<Length>(binding)
        else {
            return;
        };
        let changed = slot.current != Some(height);
        let old = slot.current;
        slot.current = Some(height);
        slot.fault = None;
        (changed, old)
    };
    if changed {
        let node = state.elements[spacer].id.node();
        state.record_binding_patch(
            tree,
            node,
            BindingPart::Style(Property::Height),
            old,
            Some(height),
            None,
        );
        state.mark_element_changed(spacer);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EventKind {
    Click,
    Input,
    Change,
    Submit,
    KeyDown,
    KeyDownCapture,
    KeyDownBubble,
    Focus,
    Blur,
    CompositionStart,
    CompositionUpdate,
    CompositionEnd,
    CompositionCancel,
}

fn event_kind(event: &Event) -> EventKind {
    match event {
        Event::Click(_) => EventKind::Click,
        Event::Input(_) => EventKind::Input,
        Event::Change(_) => EventKind::Change,
        Event::Submit(_) => EventKind::Submit,
        Event::KeyDown(_) => EventKind::KeyDown,
        Event::KeyDownCapture(_) => EventKind::KeyDownCapture,
        Event::KeyDownBubble(_) => EventKind::KeyDownBubble,
        Event::Focus(_) => EventKind::Focus,
        Event::Blur(_) => EventKind::Blur,
        Event::CompositionStart(_) => EventKind::CompositionStart,
        Event::CompositionUpdate(_) => EventKind::CompositionUpdate,
        Event::CompositionEnd(_) => EventKind::CompositionEnd,
        Event::CompositionCancel(_) => EventKind::CompositionCancel,
    }
}

enum FindTarget {
    Label(String),
    Text(String),
}

/// factory 在挂载期间创建的资源归 slot/分支自己的 Owner，而不是上一个组件。
///
/// 恢复通过 RAII guard 完成：factory panic 时 `current_owner` 也回到调用前的值。
fn with_owner<R>(owner: u32, f: impl FnOnce() -> R) -> R {
    let _guard = OwnerGuard::enter(owner);
    f()
}

struct OwnerGuard {
    previous: Option<u32>,
}

impl OwnerGuard {
    fn enter(owner: u32) -> Self {
        let previous = with_state(|state| {
            let previous = state.current_owner;
            state.current_owner = Some(owner);
            previous
        });
        Self { previous }
    }
}

impl Drop for OwnerGuard {
    fn drop(&mut self) {
        with_state(|state| state.current_owner = self.previous);
    }
}

/// 进入一个阶段并在退出（包括 unwind）时恢复之前的阶段。
struct PhaseGuard {
    previous: Phase,
}

impl PhaseGuard {
    fn enter(phase: Phase) -> Self {
        let previous = with_state(|state| {
            let previous = state.phase;
            state.phase = phase;
            previous
        });
        Self { previous }
    }
}

impl Drop for PhaseGuard {
    fn drop(&mut self) {
        with_state(|state| state.phase = self.previous);
    }
}

/// 挂载/刷新期间指定元素归属的树；Drop 恢复调用前的上下文。
struct TreeContextGuard {
    previous: Option<TreeSlot>,
}

impl TreeContextGuard {
    fn enter(key: TreeSlot) -> Self {
        let previous = with_state(|state| {
            let previous = state.building_tree;
            state.building_tree = Some(key);
            previous
        });
        Self { previous }
    }
}

impl Drop for TreeContextGuard {
    fn drop(&mut self) {
        with_state(|state| state.building_tree = self.previous);
    }
}

fn with_tree_context<R>(key: TreeSlot, f: impl FnOnce() -> R) -> R {
    let _guard = TreeContextGuard::enter(key);
    f()
}

/// 刷新期间临时借出一棵树；无论正常返回还是 unwind 都把树放回。
struct TreeRefreshGuard {
    key: TreeSlot,
    tree: Option<MountedTree>,
    previous_phase: Phase,
    previous_tree: Option<TreeSlot>,
}

impl TreeRefreshGuard {
    fn new(
        key: TreeSlot,
        tree: MountedTree,
        previous_phase: Phase,
        previous_tree: Option<TreeSlot>,
    ) -> Self {
        Self {
            key,
            tree: Some(tree),
            previous_phase,
            previous_tree,
        }
    }

    fn nodes(&mut self) -> &mut Vec<MountedNode> {
        &mut self
            .tree
            .as_mut()
            .expect("tree refresh guard owns the tree")
            .nodes
    }

    fn restore(mut self) {
        if let Some(tree) = self.tree.take() {
            with_state(|state| {
                state.put_tree(self.key, tree);
                state.building_tree = self.previous_tree;
            });
        }
    }
}

impl Drop for TreeRefreshGuard {
    fn drop(&mut self) {
        if let Some(tree) = self.tree.take() {
            with_state(|state| {
                state.put_tree(self.key, tree);
                state.building_tree = self.previous_tree;
                state.refresh_focus = None;
                state.window_focus_dropped = None;
                state.phase = self.previous_phase;
            });
        }
    }
}

/// 求值元素属性期间临时借出 attrs 与 inline styles；unwind 时原样放回。
struct ElementScratchGuard {
    id: usize,
    aux: AuxHandles,
    attrs: Option<Vec<MountedAttr>>,
    styles: Option<Vec<MountedInlineStyle>>,
}

impl ElementScratchGuard {
    fn take(id: usize) -> Self {
        let (aux, attrs, styles) = with_state(|state| {
            let aux = state.element_aux(id);
            let attrs = std::mem::take(state.nodes.attrs_mut(aux.attrs));
            let styles = std::mem::take(state.nodes.inline_styles_mut(aux.inline_styles));
            (aux, attrs, styles)
        });
        Self {
            id,
            aux,
            attrs: Some(attrs),
            styles: Some(styles),
        }
    }

    fn attrs(&self) -> &[MountedAttr] {
        self.attrs.as_deref().expect("element attrs are taken")
    }

    fn styles(&self) -> &[MountedInlineStyle] {
        self.styles.as_deref().expect("element styles are taken")
    }

    fn take_attrs(&mut self) -> Vec<MountedAttr> {
        self.attrs.take().expect("element attrs are taken")
    }

    fn take_styles(&mut self) -> Vec<MountedInlineStyle> {
        self.styles.take().expect("element styles are taken")
    }
}

impl Drop for ElementScratchGuard {
    fn drop(&mut self) {
        if self.attrs.is_none() && self.styles.is_none() {
            return;
        }
        with_state(|state| {
            if state.element_aux_opt(self.id) != Some(self.aux) {
                return;
            }
            if let Some(attrs) = self.attrs.take() {
                *state.nodes.attrs_mut(self.aux.attrs) = attrs;
            }
            if let Some(styles) = self.styles.take() {
                *state.nodes.inline_styles_mut(self.aux.inline_styles) = styles;
            }
        });
    }
}

/// 刷新期间临时借出 Show payload；返回或 unwind 时放回同一个槽位。
struct ShowPayloadGuard {
    payload: u32,
    data: Option<ShowData>,
}

impl ShowPayloadGuard {
    fn data_mut(&mut self) -> &mut ShowData {
        self.data.as_mut().expect("the Show payload is taken")
    }
}

impl Drop for ShowPayloadGuard {
    fn drop(&mut self) {
        if let Some(data) = self.data.take() {
            with_state(|state| state.nodes.put_show(self.payload, data));
        }
    }
}

/// 刷新期间临时借出 For payload；返回或 unwind 时放回同一个槽位。
struct ForPayloadGuard {
    payload: u32,
    data: Option<ForData>,
}

impl ForPayloadGuard {
    fn data_mut(&mut self) -> &mut ForData {
        self.data.as_mut().expect("the For payload is taken")
    }
}

impl Drop for ForPayloadGuard {
    fn drop(&mut self) {
        if let Some(data) = self.data.take() {
            with_state(|state| state.nodes.put_for(self.payload, data));
        }
    }
}

#[derive(Clone, Copy)]
enum ChildrenOwner {
    Element(usize),
    Component(usize),
}

/// 刷新子节点期间临时借出 children；unwind 时放回，旧结构不会丢。
struct ChildrenGuard {
    owner: ChildrenOwner,
    children: Option<Vec<MountedNode>>,
}

impl ChildrenGuard {
    fn take(owner: ChildrenOwner) -> Self {
        let children = with_state(|state| match owner {
            ChildrenOwner::Element(id) => std::mem::take(&mut state.elements[id].children),
            ChildrenOwner::Component(id) => std::mem::take(&mut state.components[id].children),
        });
        Self {
            owner,
            children: Some(children),
        }
    }

    fn children_mut(&mut self) -> &mut Vec<MountedNode> {
        self.children.as_mut().expect("children are taken")
    }

    fn restore(mut self) {
        if let Some(children) = self.children.take() {
            with_state(|state| match self.owner {
                ChildrenOwner::Element(id) => state.elements[id].children = children,
                ChildrenOwner::Component(id) => state.components[id].children = children,
            });
        }
    }
}

impl Drop for ChildrenGuard {
    fn drop(&mut self) {
        if let Some(children) = self.children.take() {
            with_state(|state| match self.owner {
                ChildrenOwner::Element(id) => {
                    if let Some(element) = state.elements.get_mut(id) {
                        element.children = children;
                    }
                }
                ChildrenOwner::Component(id) => {
                    if let Some(component) = state.components.get_mut(id) {
                        component.children = children;
                    }
                }
            });
        }
    }
}

/// 事件槽的 `take → catch → put`：handler panic 时槽位也必须回到运行时。
fn invoke_event<R>(
    event_id: usize,
    mut event: Event,
    invoke: impl FnOnce(&mut Event) -> R,
) -> Result<R, PanicInfo> {
    let outcome = catch_panic(|| invoke(&mut event));
    with_state(|state| state.put_event(event_id, event));
    outcome
}

/// click 槽返回 `HandlerOutcome`；event 槽在 panic 后也必须放回。
fn invoke_click_event(event_id: usize, event: Event) -> Result<HandlerOutcome, PanicInfo> {
    invoke_event(event_id, event, |event| match event {
        Event::Click(handler) => handler(),
        _ => HandlerOutcome::Ok,
    })
}

/// 把一次 handler 结果记入 turn 报告；返回是否发生了 panic。
fn record_handler_outcome(
    outcome: Result<HandlerOutcome, PanicInfo>,
    report: &mut TurnReport,
) -> bool {
    match outcome {
        Ok(HandlerOutcome::Ok) => false,
        Ok(HandlerOutcome::Err(error)) => {
            report.push_error(TurnError::HandlerFailed(error));
            false
        }
        Err(panic) => {
            report.push_error(TurnError::HandlerPanicked(panic));
            true
        }
    }
}

fn find_element(state: &State, nodes: &[MountedNode], target: &FindTarget) -> Option<usize> {
    for node in nodes {
        match node {
            MountedNode::Text(_) => {}
            MountedNode::Element(id) => {
                let element = &state.elements[*id];
                let matches = match target {
                    FindTarget::Label(label) => state
                        .element_label_text(*id)
                        .is_some_and(|value| value == label),
                    FindTarget::Text(text) => element.children.iter().any(|child| {
                        matches!(
                            child,
                            MountedNode::Text(node)
                                if state.text_value(*node) == *text
                        )
                    }),
                };
                if matches {
                    return Some(*id);
                }
                if let Some(found) = find_element(state, &element.children, target) {
                    return Some(found);
                }
            }
            MountedNode::Component(id) => {
                if let Some(found) = find_element(state, &state.components[*id].children, target) {
                    return Some(found);
                }
            }
            MountedNode::Show(payload) => {
                let show = state.nodes.show(*payload);
                if let Some(branch) = &show.active
                    && let Some(found) = find_element(state, &branch.children, target)
                {
                    return Some(found);
                }
            }
            MountedNode::For(payload) => {
                let for_node = state.nodes.for_node(*payload);
                for key in &for_node.order {
                    if let Some(item) = for_node.items.get(key)
                        && let Some(found) = find_element(state, &item.children, target)
                    {
                        return Some(found);
                    }
                }
            }
        }
    }
    None
}

/// 应用运行时：声明窗口目录、挂载界面、驱动任务并生成呈现快照。
///
/// 一个应用通常创建一个 `Runtime`，交给 Windows 宿主或 headless 测试使用。
/// Signal、Memo 与 Store 句柄归属于应用 owner；窗口关闭不会释放它们。
pub struct Runtime {
    pub(crate) state: Rc<RefCell<State>>,
}

impl Runtime {
    /// 创建一个没有窗口、没有已挂载界面的运行时。
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(State::new())),
        }
    }

    /// 在任何窗口 factory 运行前运行一次应用级 setup，返回值由窗口 factory 捕获。
    ///
    /// setup 中的 Signal、Memo 与 Store 归属于应用 owner；窗口关闭不会释放它们。
    /// setup 只成功运行一次；失败时不保留本次创建的资源，可以重试。
    /// 应用级 setup 之后不能再调用 [`Runtime::mount`]。
    pub fn setup<R>(&mut self, setup: impl FnOnce() -> R) -> Result<R, MountError> {
        {
            let state = self.state.borrow();
            if state.app_setup {
                return Err(MountError::Setup("application setup already ran"));
            }
            if state.runtime_started() {
                return Err(MountError::Setup(
                    "application setup must run before any window factory",
                ));
            }
        }
        let state = self.state.clone();
        context::install(&state, || {
            with_state(|state| state.fault = None);
            let outcome = with_owner(0, || {
                let _phase = PhaseGuard::enter(Phase::Setup);
                catch_panic(setup)
            });
            let value = match outcome {
                Ok(value) => value,
                Err(panic) => {
                    with_state(State::release_root_resources);
                    return Err(MountError::Panic(panic));
                }
            };
            if let Some(fault) = with_state(|state| state.fault.take()) {
                with_state(State::release_root_resources);
                return Err(State::context_error(fault));
            }
            with_state(|state| state.app_setup = true);
            Ok(value)
        })
    }

    pub fn mount(
        &mut self,
        root: impl FnOnce() -> MountResult<MountPlan> + 'static,
    ) -> Result<(), MountError> {
        if self.state.borrow().app_setup {
            return Err(MountError::Setup(
                "cannot mount a headless root after application setup",
            ));
        }
        if !self.state.borrow().trees.is_empty() {
            return Err(MountError::Component("root is already mounted"));
        }
        let state = self.state.clone();
        let result = context::install(&state, || {
            // 失败的挂载把本 turn 已经记录的结构/值补丁一并回滚，不能把
            // 已释放节点的补丁留给后续消费者。
            let patch_mark = with_state(|state| state.binding_patches.mark());
            with_state(|state| {
                state.fault = None;
                state.write_error = None;
            });
            let plan = match with_owner(0, || catch_panic(root)) {
                Ok(Ok(plan)) => plan,
                Ok(Err(error)) => {
                    with_state(|state| {
                        state.binding_patches.rollback(patch_mark);
                        state.release_root_resources();
                    });
                    return Err(error);
                }
                Err(panic) => {
                    with_state(|state| {
                        state.binding_patches.rollback(patch_mark);
                        state.release_root_resources();
                    });
                    return Err(MountError::Panic(panic));
                }
            };
            let mut tree =
                match with_tree_context(TreeSlot::Root, || self.mount_plan(plan, 0, None)) {
                    Ok(tree) => tree,
                    Err(error) => {
                        with_state(|state| {
                            state.binding_patches.rollback(patch_mark);
                            state.release_root_resources();
                        });
                        return Err(error);
                    }
                };
            if let Some(fault) = with_state(|state| state.fault.take()) {
                self.release_nodes(&mut tree);
                with_state(|state| {
                    state.binding_patches.rollback(patch_mark);
                    // 根 factory 的 signal/memo 挂在 owner 0 上；失败的挂载
                    // 必须释放它们，同时丢弃未提交的根绑定表。
                    state.release_root_resources();
                });
                return Err(State::context_error(fault));
            }
            with_state(|state| {
                state.put_tree(
                    TreeSlot::Root,
                    MountedTree {
                        nodes: tree,
                        owner: 0,
                        focused: None,
                        focused_tag: None,
                        settle_before: None,
                        revision: 0,
                        snapshot: RefCell::new(None),
                        published: RefCell::new(None),
                    },
                );
                state.mark_tree_changed(TreeSlot::Root);
                state.phase = Phase::Idle;
            });
            if let Err(error) = self.refresh_internal() {
                // 首次刷新失败：根界面并未成功挂载，撤销这次提交，释放
                // 它的节点、绑定表和 owner 0 资源，允许调用方重新 mount。
                let nodes =
                    with_state(|state| state.trees.remove(&TreeSlot::Root).map(|tree| tree.nodes));
                if let Some(mut nodes) = nodes {
                    self.release_nodes(&mut nodes);
                }
                with_state(|state| {
                    state.binding_patches.rollback(patch_mark);
                    state.release_root_resources();
                    state.dirty_trees.remove(&TreeSlot::Root);
                    state.changed_trees.remove(&TreeSlot::Root);
                    state.phase = Phase::Idle;
                });
                return Err(error);
            }
            Ok(())
        });
        if result.is_err() {
            // 失败的挂载不能把工厂/结算记录的旁路错误留给下一个 turn。
            let mut state = self.state.borrow_mut();
            state.fault = None;
            state.write_error = None;
        }
        result
    }

    /// 失败的计划先撤销交付，再释放已创建的界面与 Owner。
    fn abort_open(
        &mut self,
        key: &'static str,
        activation: u32,
        owner: u32,
        tree: Option<Vec<MountedNode>>,
    ) {
        with_state(|state| {
            state.abort_window(key, activation, owner);
            // 被放弃的 activation 不能把 factory/memo 的错误旁路留给后续 turn。
            state.fault = None;
            state.write_error = None;
        });
        if let Some(mut nodes) = tree {
            self.release_nodes(&mut nodes);
        }
        with_state(|state| {
            state.dispose_owner(owner);
            state.drop_tree_binding_table(TreeSlot::Window(key));
        });
    }

    /// 声明静态窗口目录；声明本身不运行 factory，也不挂载界面。
    ///
    /// 目录必须先于任何窗口 factory，并且只声明一次。重复键在
    /// [`WindowCatalog::window`] 立即报错；缺失或多个 primary 在这里报错。
    pub fn declare_windows(
        &mut self,
        declare: impl FnOnce(&mut WindowCatalog) -> Result<(), CatalogError>,
    ) -> Result<(), CatalogError> {
        {
            let state = self.state.borrow();
            if state.catalog_declared {
                return Err(CatalogError::AlreadyDeclared);
            }
            if state.runtime_started() {
                return Err(CatalogError::DeclaredAfterWindow);
            }
        }
        let mut catalog = WindowCatalog::new();
        declare(&mut catalog)?;
        let (entries, primary) = catalog.finish()?;
        let mut state = self.state.borrow_mut();
        state.catalog = entries;
        state.primary = Some(primary);
        state.catalog_declared = true;
        Ok(())
    }

    /// 目录声明的 primary 键；尚未声明目录时为 `None`。
    pub fn primary_key(&self) -> Option<&'static str> {
        self.state.borrow().primary
    }

    /// 窗口打开、关闭、隐藏或显示时递增的修订号；无变化时宿主可跳过扫描。
    pub fn window_revision(&self) -> u64 {
        self.state.borrow().window_revision
    }

    /// 当前仍存活的窗口及目录描述，按目录键排序。
    ///
    /// 原生宿主用它增量同步 HWND；headless 调用方通常直接按键操作。
    pub fn windows(&self) -> Vec<WindowInfo> {
        let state = self.state.borrow();
        state
            .windows
            .keys()
            .filter_map(|key| {
                let window = state.window_ctx(key)?;
                let spec = state.catalog.get(key)?.spec;
                Some(WindowInfo {
                    window,
                    spec,
                    visible: state.window_is_visible(key),
                })
            })
            .collect()
    }

    /// 窗口目录声明的静态菜单；没有菜单、窗口已关闭或属于另一个 Runtime 时为 `None`。
    ///
    /// 原生宿主在创建 HWND 时读取这份声明；菜单命令仍经运行时命令路径派发。
    pub fn window_menu(&self, window: WindowCtx) -> Option<MenuSpec> {
        if !self.window_alive(window) {
            return None;
        }
        let state = self.state.borrow();
        state.catalog.get(window.key)?.menu.clone()
    }

    /// 窗口声明的窗口级上下文菜单；没有声明、窗口已关闭或属于另一个 Runtime
    /// 时为 `None`。菜单在打开时由宿主构建，命令仍经运行时命令路径派发。
    pub fn window_context_menu(&self, window: WindowCtx) -> Option<MenuSpec> {
        if !self.window_alive(window) {
            return None;
        }
        let state = self.state.borrow();
        state.catalog.get(window.key)?.context_menu.clone()
    }

    /// 在只读作用域内求值窗口的菜单状态回调。
    ///
    /// 没有注册回调时返回 `Ok(None)`，宿主使用命令的声明默认状态。回调内的
    /// Signal 写与 Store 事务被拒绝并作为 typed error 返回，模型保持已提交值；
    /// 回调 panic 被 contain。宿主在 `WM_INITMENU`/`WM_INITMENUPOPUP` 与命令
    /// 派发前调用。
    pub fn menu_state(
        &self,
        window: WindowCtx,
        command: &MenuCommand,
    ) -> Result<Option<MenuCommandState>, MenuStateError> {
        let state = self.state.clone();
        if window.runtime != state.borrow().id {
            return Err(MenuStateError::WrongRuntime);
        }
        let handler = {
            let state = state.borrow();
            if !state.window_is_live(window.key, window.activation, window.owner) {
                return Err(MenuStateError::Stale);
            }
            state.menu_state_handlers.get(window.key).cloned()
        };
        let Some(handler) = handler else {
            return Ok(None);
        };
        context::install(&state, || {
            let previous_phase = with_state(|state| {
                state.fault = None;
                state.write_error = None;
                state.read_only += 1;
                state.phase
            });
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(command)));
            with_state(|state| {
                state.phase = previous_phase;
                state.read_only = state.read_only.saturating_sub(1);
            });
            let value = match outcome {
                Ok(value) => value,
                Err(_) => {
                    // 展开前记录的部分 fault 不属于任何调用方，丢弃后再报告 panic。
                    with_state(|state| {
                        state.fault = None;
                        state.write_error = None;
                    });
                    return Err(MenuStateError::HandlerPanicked);
                }
            };
            if let Some(fault) = with_state(|state| state.fault.take()) {
                return Err(MenuStateError::Context {
                    operation: fault.operation,
                    phase: fault.phase,
                });
            }
            if let Some(error) = with_state(|state| state.write_error.take()) {
                return Err(MenuStateError::Write(error));
            }
            Ok(Some(value))
        })
    }

    /// 显示目录中的窗口：未打开时运行 factory 产生新 activation；已打开时只聚焦，
    /// 隐藏的窗口恢复可见且不重跑 factory。
    ///
    /// factory 失败仍消耗这一代 activation；下一次成功打开使用新一代。
    /// 挂载或首帧刷新失败会释放新窗口并让它保持关闭。
    /// primary 窗口由 Host 首次显示；headless 调用方按目录键选择。
    /// 显式退出之后返回 [`WindowError::State`]，不再打开窗口。
    pub fn show(&mut self, key: &'static str) -> Result<WindowCtx, WindowError> {
        if self.state.borrow().session.quitting {
            return Err(WindowError::State("the application session has ended"));
        }
        let factory = {
            let state = self.state.borrow();
            let entry = state
                .catalog
                .get(key)
                .ok_or(WindowError::UnknownWindow(key))?;
            Rc::clone(&entry.factory)
        };
        if self.state.borrow().trees.contains_key(&TreeSlot::Root) {
            return Err(WindowError::State(
                "cannot show a window while a headless root is mounted",
            ));
        }
        let live = self.state.borrow().window_ctx(key);
        if let Some(window) = live {
            let mut state = self.state.borrow_mut();
            state.show_window(key);
            state.focused_window = Some(key);
            drop(state);
            return Ok(window);
        }
        self.open_window_at(key, move |window| factory(window))
    }

    /// 该 activation 是否在 factory 中注册了 `on_drop` 处理器。
    ///
    /// 原生宿主只在窗口可见且这里返回 `true` 时注册 OLE drop target；已关闭、
    /// 过期或外部 Runtime 的窗口返回 `false`。
    pub fn window_has_drop_handler(&self, window: WindowCtx) -> bool {
        let state = self.state.borrow();
        window.runtime == state.id
            && state.window_live_in_tree(window)
            && state.drop_handlers.contains_key(window.key)
    }

    /// 隐藏窗口：保留 activation、树、焦点与本地编辑状态，只停止输入与呈现。
    ///
    /// 参与待决关闭的窗口先按 Deny 结算：隐藏父窗口会恢复其输入与焦点后再隐藏；
    /// 隐藏确认窗口等于关闭确认窗口，父窗口回到可编辑状态。
    pub fn hide(&mut self, window: WindowCtx) -> Result<(), WindowError> {
        let state = self.state.clone();
        if window.runtime != state.borrow().id {
            return Err(WindowError::WrongRuntime);
        }
        context::install(&state, || {
            if !with_state(|state| state.window_live_in_tree(window)) {
                return Err(WindowError::Stale);
            }
            // 隐藏任何参与待决关闭的窗口都先按 Deny 结算，绝不隐式丢弃编辑。
            if let Some(parent) =
                with_state(|state| state.pending_parent_for_confirmation(window.key))
            {
                self.deny_close(parent)?;
                if !with_state(|state| state.window_live_in_tree(window)) {
                    return Ok(());
                }
            }
            if with_state(|state| state.close_epochs.contains_key(window.key)) {
                self.deny_close(window)?;
            }
            let mut state = state.borrow_mut();
            state.hide_window(window.key);
            if state.focused_window == Some(window.key) {
                state.focused_window = state.first_visible_window();
            }
            drop(state);
            Ok(())
        })
    }

    /// 向窗口投递一次已接受的文件拖放。
    ///
    /// 路径按给定顺序交给该 activation 在 factory 中注册的 `on_drop` 处理器；
    /// 处理器在 `Phase::Event` 中运行一次，可以写 Store/Signal、排队窗口请求与
    /// 启动任务。重复路径与 Unicode 原样保留，不做去重或规范化。
    ///
    /// 外部 Runtime 的窗口、已关闭/过期的 activation、隐藏窗口、等待关闭确认的
    /// 窗口、空路径与没有处理器的窗口都在进入事件阶段前拒绝。
    pub fn drop_files(&mut self, window: WindowCtx, paths: Vec<PathBuf>) -> Result<(), DropError> {
        let state = self.state.clone();
        if window.runtime != state.borrow().id {
            return Err(DropError::WrongRuntime);
        }
        context::install(&state, || {
            if !with_state(|state| state.window_live_in_tree(window)) {
                return Err(DropError::Stale);
            }
            if paths.is_empty() {
                return Err(DropError::Empty);
            }
            if !with_state(|state| state.window_is_visible(window.key)) {
                return Err(DropError::Hidden);
            }
            if with_state(|state| {
                state.close_epochs.contains_key(window.key)
                    || state.pending_parent_for_confirmation(window.key).is_some()
            }) {
                return Err(DropError::Suspended);
            }
            let Some(handler) = with_state(|state| state.drop_handlers.get(window.key).cloned())
            else {
                return Err(DropError::NoHandler);
            };
            let previous = with_state(|state| {
                let previous = state.phase;
                state.phase = Phase::Event;
                state.fault = None;
                state.write_error = None;
                previous
            });
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                handler(DropEvent::new(window, paths));
            }));
            with_state(|state| state.phase = previous);
            if outcome.is_err() {
                return Err(DropError::HandlerPanicked);
            }
            self.finish_command().map_err(DropError::Command)
        })
    }

    /// 打开一个 headless 窗口 activation；factory 每次打开都会重新执行。
    ///
    /// 这是 stage-one 的单窗口兼容入口，声明窗口目录后不再可用；
    /// 多窗口使用 [`Runtime::declare_windows`] 与 [`Runtime::show`]。
    /// factory 失败仍消耗 activation，下一次打开使用新一代。
    pub fn open_window(
        &mut self,
        key: &'static str,
        factory: impl Fn(WindowCtx) -> MountResult<MountPlan> + 'static,
    ) -> Result<WindowCtx, WindowError> {
        {
            let state = self.state.borrow();
            if state.session.quitting {
                return Err(WindowError::State("the application session has ended"));
            }
            if state.catalog_declared {
                return Err(WindowError::State(
                    "open_window cannot be used after a catalog is declared",
                ));
            }
            if !state.trees.is_empty() {
                return Err(WindowError::Unsupported(
                    "a second headless window while one is mounted",
                ));
            }
        }
        self.open_window_at(key, factory)
    }

    fn open_window_at(
        &mut self,
        key: &'static str,
        factory: impl Fn(WindowCtx) -> MountResult<MountPlan> + 'static,
    ) -> Result<WindowCtx, WindowError> {
        let state = self.state.clone();
        context::install(&state, || {
            // 失败的窗口挂载回滚本次挂载记录的补丁，不把已释放节点的
            // 记录留给后续消费者。
            let patch_mark = with_state(|state| state.binding_patches.mark());
            with_state(|state| {
                state.fault = None;
                state.write_error = None;
            });
            let (activation_id, owner) = with_state(|state| state.begin_window(key));
            let activation = activation_id.generation();
            let window = WindowCtx::new(state.borrow().id, key, activation, owner, activation_id);
            let plan = with_owner(owner, || {
                let _phase = PhaseGuard::enter(Phase::Setup);
                catch_panic(|| factory(window))
            });
            let plan = match plan {
                Ok(Ok(plan)) => plan,
                Ok(Err(error)) => {
                    with_state(|state| state.binding_patches.rollback(patch_mark));
                    self.abort_open(key, activation, owner, None);
                    return Err(WindowError::Mount(error));
                }
                Err(panic) => {
                    with_state(|state| state.binding_patches.rollback(patch_mark));
                    self.abort_open(key, activation, owner, None);
                    return Err(WindowError::Mount(MountError::Panic(panic)));
                }
            };
            if let Some(id) = with_state(|state| state.missing_command_handler(key)) {
                with_state(|state| state.binding_patches.rollback(patch_mark));
                self.abort_open(key, activation, owner, None);
                return Err(WindowError::Command(CommandError::MissingHandler(id)));
            }
            let tree = match with_tree_context(TreeSlot::Window(key), || {
                self.mount_plan(plan, owner, None)
            }) {
                Ok(tree) => tree,
                Err(error) => {
                    with_state(|state| state.binding_patches.rollback(patch_mark));
                    self.abort_open(key, activation, owner, None);
                    return Err(WindowError::Mount(error));
                }
            };
            if let Some(fault) = with_state(|state| state.fault.take()) {
                with_state(|state| state.binding_patches.rollback(patch_mark));
                self.abort_open(key, activation, owner, Some(tree));
                return Err(WindowError::Mount(MountError::Context {
                    operation: fault.operation,
                    phase: fault.phase,
                }));
            }
            with_state(|state| {
                state.put_tree(
                    TreeSlot::Window(key),
                    MountedTree {
                        nodes: tree,
                        owner,
                        focused: None,
                        focused_tag: None,
                        settle_before: None,
                        revision: 0,
                        snapshot: RefCell::new(None),
                        published: RefCell::new(None),
                    },
                );
                state.mark_tree_changed(TreeSlot::Window(key));
                state.phase = Phase::Idle;
            });
            if let Err(error) = self.refresh_internal() {
                let nodes = with_state(|state| {
                    state
                        .trees
                        .remove(&TreeSlot::Window(key))
                        .map(|tree| tree.nodes)
                });
                self.abort_open(key, activation, owner, nodes);
                return Err(WindowError::Mount(error));
            }
            with_state(|state| state.focused_window = Some(key));
            Ok(window)
        })
    }

    /// 关闭 activation：释放窗口 UI、取消窗口所有权交付；应用级模型保留。
    ///
    /// 参数是待决关闭的确认窗口时按 Deny 结算；参数是有待决 epoch 的父窗口时，
    /// 确认窗口与父窗口作为一组释放，不留悬挂提示。
    pub fn close_window(&mut self, window: WindowCtx) -> Result<(), WindowError> {
        let state = self.state.clone();
        if window.runtime != state.borrow().id {
            return Err(WindowError::WrongRuntime);
        }
        context::install(&state, || {
            if !with_state(|state| state.window_live_in_tree(window)) {
                return Err(WindowError::Stale);
            }
            if let Some(parent) =
                with_state(|state| state.pending_parent_for_confirmation(window.key))
            {
                return self.deny_close(parent);
            }
            if with_state(|state| state.close_epochs.contains_key(window.key)) {
                return self.release_close_group(window.key);
            }
            self.close_window_now(window)
        })
    }

    /// 走目录声明的关闭策略；程序化关闭的来源是 [`CloseSource::Programmatic`]。
    pub fn request_close(&mut self, window: WindowCtx) -> Result<(), WindowError> {
        self.request_close_from(window, CloseSource::Programmatic)
    }

    /// 走目录声明的关闭策略：Immediate 直接释放；Confirm 建立或复用 epoch。
    ///
    /// 隐藏的 Confirm 窗口先恢复可见，再走关闭协议，避免不可见的确认提示；
    /// 确认窗口在待决 epoch 中的关闭请求等价于 Deny。
    pub fn request_close_from(
        &mut self,
        window: WindowCtx,
        source: CloseSource,
    ) -> Result<(), WindowError> {
        let state = self.state.clone();
        if window.runtime != state.borrow().id {
            return Err(WindowError::WrongRuntime);
        }
        context::install(&state, || {
            if !with_state(|state| state.window_live_in_tree(window)) {
                return Err(WindowError::Stale);
            }
            if let Some(parent) =
                with_state(|state| state.pending_parent_for_confirmation(window.key))
            {
                return self.deny_close(parent);
            }
            let Some(policy) = with_state(|state| {
                state
                    .catalog
                    .get(window.key)
                    .map(|entry| entry.spec.policy())
            }) else {
                return Err(WindowError::UnknownWindow(window.key));
            };
            if policy == ClosePolicy::Confirm {
                if !with_state(|state| state.window_is_visible(window.key)) {
                    self.show(window.key)?;
                }
                if let Some(confirmation) = with_state(|state| {
                    state
                        .close_epochs
                        .get(window.key)
                        .map(|epoch| epoch.confirmation)
                }) {
                    self.show(confirmation)?;
                    return Ok(());
                }
                return self.begin_confirm_close(window, source);
            }
            self.close_window_now(window)
        })
    }

    /// 确认窗口的工厂读取父窗口当前关闭请求。
    pub fn confirmation_request(
        &self,
        window: WindowCtx,
    ) -> Result<Option<CloseRequest>, WindowError> {
        let state = self.state.borrow();
        if window.runtime != state.id {
            return Err(WindowError::WrongRuntime);
        }
        state.close_request_for(window)
    }

    /// 窗口当前是否接收输入；待决关闭的父窗口在 Deny/Allow 前保持挂起。
    pub fn window_accepts_input(&self, window: WindowCtx) -> bool {
        let state = self.state.borrow();
        window.runtime == state.id
            && state.window_is_live(window.key, window.activation, window.owner)
            && state.window_accepts_input(window.key)
    }

    /// 注册应用级命令处理器；必须在运行时打开窗口或挂载根树之前调用。
    ///
    /// 应用级处理器不依赖 activation，窗口全部关闭后仍可派发。托盘与共享
    /// 动作使用 [`Runtime::dispatch_app_command`]；窗口目标派发只在窗口没有
    /// 注册同一 id 时回退到这里。重复 id 与启动后注册都返回 typed error，
    /// 不覆盖已有处理器。
    pub fn on_app_command(
        &mut self,
        id: &'static str,
        handler: impl Fn(CommandEvent) + 'static,
    ) -> Result<(), CommandError> {
        let mut state = self.state.borrow_mut();
        if state.runtime_started() {
            return Err(CommandError::RegistrationClosed);
        }
        if state.app_command_handlers.contains_key(id) {
            return Err(CommandError::AlreadyRegistered(id));
        }
        state.app_command_handlers.insert(id, Rc::new(handler));
        Ok(())
    }

    /// 声明应用托盘；每个 Runtime 一次，必须在运行时启动前调用。
    ///
    /// 菜单命令由应用级处理器承接；宿主启动时用
    /// [`Runtime::missing_tray_handler`] 校验每个命令都有处理器。tooltip、
    /// 重复命令 id、快捷键与菜单上限在这里校验，失败不保留声明。
    pub fn declare_tray(&mut self, tray: TraySpec) -> Result<(), TrayError> {
        let mut state = self.state.borrow_mut();
        if state.tray.is_some() {
            return Err(TrayError::AlreadyDeclared);
        }
        if state.runtime_started() {
            return Err(TrayError::RegistrationClosed);
        }
        tray.validate()?;
        state.tray = Some(tray);
        Ok(())
    }

    /// 应用托盘声明；没有声明时为 `None`。宿主在启动时读取。
    pub fn tray(&self) -> Option<TraySpec> {
        self.state.borrow().tray.clone()
    }

    /// 托盘菜单声明但还没有应用级处理器的第一个命令 id。
    ///
    /// 宿主在创建图标前调用；没有托盘声明或全部命令都有处理器时返回
    /// `None`。
    pub fn missing_tray_handler(&self) -> Option<&'static str> {
        let state = self.state.borrow();
        state
            .tray
            .as_ref()?
            .first_missing_handler(|id| state.app_command_handler(id).is_some())
    }

    /// 声明最后一个窗口关闭后的会话策略；每个 Runtime 一次。
    ///
    /// 必须在首个窗口打开或根树挂载之前调用。未声明时使用
    /// [`SessionPolicy::QuitOnLastClose`]；`StayResident` 的原生宿主要求同时
    /// 声明托盘，否则 `zenui-windows` 的 `Host::open` 拒绝启动。
    pub fn declare_session(&mut self, policy: SessionPolicy) -> Result<(), SessionError> {
        let mut state = self.state.borrow_mut();
        if state.session.declared {
            return Err(SessionError::AlreadyDeclared);
        }
        if state.runtime_started() {
            return Err(SessionError::RegistrationClosed);
        }
        state.session.policy = policy;
        state.session.declared = true;
        Ok(())
    }

    /// 当前应用会话策略；未声明时为 [`SessionPolicy::QuitOnLastClose`]。
    pub fn session_policy(&self) -> SessionPolicy {
        self.state.borrow().session.policy
    }

    /// 应用会话的 Copy handle；可在命令处理器中捕获并排队 show/quit。
    pub fn session(&self) -> SessionCtx {
        SessionCtx::new(self.state.borrow().id)
    }

    /// 应用会话是否仍在运行；显式退出后为 `false`。
    ///
    /// 关闭最后一个窗口是否结束宿主会话由 [`Runtime::session_policy`] 决定；
    /// Runtime 本身仍可由嵌入方重新打开窗口，除非已经显式退出。
    pub fn session_alive(&self) -> bool {
        !self.state.borrow().session.quitting
    }

    /// 通过窗口的静态菜单派发一个命令。
    ///
    /// 菜单栏、快捷键、上下文菜单与辅助技术共用这条路径。处理器在
    /// `Phase::Event` 中恰好运行一次，可以写 Store/Signals 并排队窗口请求；
    /// 窗口已关闭、隐藏、等待关闭确认或属于另一个 Runtime 时返回 typed
    /// error。处理器 panic 被 contain，不向调用方展开。
    ///
    /// 窗口没有为 `id` 注册处理器时回退到应用级处理器，事件暴露目标窗口。
    /// `CommandSource::Tray` 从不运行窗口处理器，只查找应用级处理器。
    pub fn dispatch_command(
        &mut self,
        window: WindowCtx,
        id: &'static str,
        source: CommandSource,
    ) -> Result<(), CommandError> {
        self.dispatch_command_on(window, id, source, None)
    }

    /// 与 [`Runtime::dispatch_command`] 相同，并携带可选的上下文目标节点。
    ///
    /// `target` 必须是这个窗口里仍然存活的 Node ID；菜单栏与快捷键通常不传
    /// 目标，上下文菜单与辅助动作用它表达被操作的元素。
    pub fn dispatch_command_on(
        &mut self,
        window: WindowCtx,
        id: &'static str,
        source: CommandSource,
        target: Option<NodeId>,
    ) -> Result<(), CommandError> {
        let state = self.state.clone();
        if window.runtime != state.borrow().id {
            return Err(CommandError::WrongRuntime);
        }
        context::install(&state, || {
            if !with_state(|state| state.window_live_in_tree(window)) {
                return Err(CommandError::Stale);
            }
            let window_handler = with_state(|state| state.command_handler(window.key, id));
            let app_handler = with_state(|state| state.app_command_handler(id));
            // 托盘动作不归任何 activation 所有；窗口来源先用自己的 activation
            // 处理器，只有它没有注册同一 id 时才回退到应用级表。
            let (handler, from_app) = if source == CommandSource::Tray {
                (app_handler.ok_or(CommandError::MissingHandler(id))?, true)
            } else if let Some(handler) = window_handler {
                (handler, false)
            } else if let Some(handler) = app_handler {
                (handler, true)
            } else if with_state(|state| state.menu_declares(window.key, id)) {
                return Err(CommandError::MissingHandler(id));
            } else {
                return Err(CommandError::UnknownCommand(id));
            };
            if source != CommandSource::Tray {
                if !with_state(|state| state.window_is_visible(window.key)) {
                    return Err(CommandError::Hidden);
                }
                if with_state(|state| state.close_epochs.contains_key(window.key)) {
                    return Err(CommandError::Suspended);
                }
            }
            if let Some(target) = target {
                let valid = with_state(|state| {
                    state.node_status(target) == NodeStatus::Live
                        && state
                            .element_slot(target)
                            .and_then(|element| state.tree_for_element(element))
                            == Some(TreeSlot::Window(window.key))
                });
                if !valid {
                    return Err(CommandError::InvalidTarget(target));
                }
            }

            let event = if from_app {
                CommandEvent::new(source, target).with_window(window)
            } else {
                CommandEvent::new(source, target)
            };
            self.run_command_handler(id, handler, event)
        })
    }

    /// 派发应用级命令；没有窗口时也能运行。
    ///
    /// 只查找应用级处理器，从不回退到窗口处理器。`target` 必须是存活的
    /// activation：另一个 Runtime 返回 [`CommandError::WrongRuntime`]，关闭或
    /// 被新一代替换的 activation 返回 [`CommandError::Stale`]，不运行处理器。
    /// 隐藏与等待关闭确认的窗口仍是合法目标，托盘可借此恢复或关闭窗口。
    ///
    /// 目标窗口通过 [`CommandEvent::window`] 暴露；处理器可以排队
    /// show/hide/close 请求，由 [`Runtime::apply_window_requests`] 结算。
    pub fn dispatch_app_command(
        &mut self,
        id: &'static str,
        source: CommandSource,
        target: Option<WindowCtx>,
    ) -> Result<(), CommandError> {
        let state = self.state.clone();
        if target.is_some_and(|target| target.runtime != state.borrow().id) {
            return Err(CommandError::WrongRuntime);
        }
        context::install(&state, || {
            if let Some(target) = target
                && !with_state(|state| {
                    state.window_is_live(target.key, target.activation, target.owner)
                })
            {
                return Err(CommandError::Stale);
            }
            let handler = with_state(|state| state.app_command_handler(id))
                .ok_or(CommandError::MissingHandler(id))?;
            self.run_command_handler(id, handler, CommandEvent::for_app(source, target))
        })
    }

    /// 在 Event 阶段运行一个处理器并提交结果；panic 被 contain。
    fn run_command_handler(
        &mut self,
        id: &'static str,
        handler: CommandHandler,
        event: CommandEvent,
    ) -> Result<(), CommandError> {
        let previous = with_state(|state| {
            let previous = (state.phase, state.active_close_source);
            state.phase = Phase::Event;
            state.active_close_source = Some(CloseSource::Command);
            state.fault = None;
            state.write_error = None;
            previous
        });
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(event)));
        with_state(|state| {
            state.phase = previous.0;
            state.active_close_source = previous.1;
        });
        if outcome.is_err() {
            return Err(CommandError::HandlerPanicked(id));
        }
        if self.tree_keys().is_empty() {
            return self
                .finish_command_without_ui()
                .map_err(CommandError::Mount);
        }
        self.finish_command().map_err(CommandError::Mount)
    }

    /// 结算 UI 处理器排队的窗口请求与关闭回复，返回已应用的数量。
    ///
    /// 原生宿主在每条输入消息与每次任务交付后自动调用；headless 驱动在
    /// 派发事件后调用。来源窗口在结算前关闭时请求被静默丢弃；其余错误
    /// 继续应用后续动作，再返回第一个错误。关闭处理器可能在结算中要求
    /// 显示确认窗口，因此 Confirm 的 request_close 内部也会结算这些 show 请求。
    pub fn apply_window_requests(&mut self) -> Result<usize, WindowError> {
        let replies = std::mem::take(&mut self.state.borrow_mut().close_replies);
        let mut applied = 0;
        let mut first_error = None;
        for reply in replies {
            match self.apply_close_reply(reply) {
                Ok(()) => applied += 1,
                Err(WindowError::Stale) => {}
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        match self.settle_show_requests() {
            Ok(requests) => applied += requests,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(applied),
        }
    }

    /// 结算 UI 处理器排队的会话动作，返回已应用的数量。
    ///
    /// 原生宿主在窗口请求之后调用；headless 驱动在同一 turn 的
    /// [`Runtime::apply_window_requests`] 之后调用。`Show` 按目录键重新打开
    /// 已关闭窗口；`Quit` 关闭全部 activation 并拒绝之后的窗口打开，排在同
    /// 一批次 `Quit` 之后的动作随之丢弃。显式退出不运行逐窗口的关闭确认。
    ///
    /// `Quit` 之前的动作按排队顺序应用；单个动作失败时继续应用后续动作，
    /// 再返回第一个错误。
    pub fn apply_session_requests(&mut self) -> Result<usize, SessionError> {
        let requests = std::mem::take(&mut self.state.borrow_mut().session.requests);
        let mut applied = 0;
        let mut first_error = None;
        for action in requests {
            if !self.session_alive() {
                // quit 已在应用过程中结束会话；同一批次的后续动作不再有目标。
                break;
            }
            match action {
                SessionAction::Show(key) => {
                    if let Err(error) = self.show(key) {
                        first_error.get_or_insert(SessionError::Window(error));
                    } else {
                        applied += 1;
                    }
                }
                SessionAction::Quit => {
                    match self.quit() {
                        Ok(()) => applied += 1,
                        Err(error) => {
                            first_error.get_or_insert(error);
                        }
                    }
                    break;
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(applied),
        }
    }

    /// 显式结束应用会话：关闭全部窗口并拒绝之后的窗口打开。
    ///
    /// 这是不可撤销的会话动作；应用需要确认时应在调用前完成。待决关闭 epoch
    /// 与关闭确认窗口随所属窗口一起释放，不调用新的关闭处理器。窗口请求与
    /// 尚未结算的会话动作被丢弃，模型与外观保留在 Runtime 中。
    pub fn quit(&mut self) -> Result<(), SessionError> {
        let state = self.state.clone();
        context::install(&state, || {
            if with_state(|state| state.session.quitting) {
                return Ok(());
            }
            with_state(|state| state.session.quitting = true);
            {
                let mut state = self.state.borrow_mut();
                state.window_requests.clear();
                state.session.requests.clear();
            }
            // 目录窗口与 stage-one 的 open_window 都登记在同一个激活表里；
            // quit 不能只关闭目录条目，否则未声明的 activation 会在退出后保留。
            let contexts: Vec<WindowCtx> = {
                let state = self.state.borrow();
                state
                    .windows
                    .keys()
                    .filter_map(|key| state.window_ctx(key))
                    .collect()
            };
            let mut first_error = None;
            for window in contexts {
                match self.close_window(window) {
                    Ok(()) | Err(WindowError::Stale) => {}
                    Err(error) => {
                        first_error.get_or_insert(SessionError::Window(error));
                    }
                }
            }
            match first_error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        })
    }

    fn apply_close_reply(&mut self, reply: CloseReplyRequest) -> Result<(), WindowError> {
        let valid = {
            let state = self.state.borrow();
            state
                .close_epochs
                .get(reply.request.parent.key)
                .is_some_and(|epoch| {
                    epoch.id == reply.request.epoch
                        && epoch.parent == reply.request.parent
                        && epoch.reply == Some(reply.reply)
                })
        };
        if !valid {
            return Err(WindowError::Stale);
        }
        match reply.reply {
            CloseReply::Allow => self.release_close_group(reply.request.parent.key),
            CloseReply::Deny => self.deny_close(reply.request.parent),
        }
    }

    fn settle_show_requests(&mut self) -> Result<usize, WindowError> {
        let requests = std::mem::take(&mut self.state.borrow_mut().window_requests);
        let mut applied = 0;
        let mut first_error = None;
        for request in requests {
            match self.apply_window_request(request) {
                Ok(()) => applied += 1,
                Err(WindowError::Stale) => {}
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(applied),
        }
    }

    fn apply_window_request(&mut self, request: WindowRequest) -> Result<(), WindowError> {
        let source_live = self.state.borrow().window_live_in_tree(request.from);
        if !source_live {
            return Err(WindowError::Stale);
        }
        match request.action {
            WindowAction::Show(key) => self.show(key).map(|_| ()),
            WindowAction::Hide => self.hide(request.from),
            WindowAction::Close => self.request_close_from(
                request.from,
                request.close_source.unwrap_or(CloseSource::Programmatic),
            ),
        }
    }

    /// 创建 pending epoch、挂起父窗口输入并调用注册的 close handler。
    ///
    /// handler 通过 `request_show` 显示目录声明的确认窗口；这里立即结算这些
    /// show 请求并验证确认窗口可见，失败时绝不回退为 Allow。
    fn begin_confirm_close(
        &mut self,
        window: WindowCtx,
        source: CloseSource,
    ) -> Result<(), WindowError> {
        let state = self.state.clone();
        context::install(&state, || {
            let handler = self
                .state
                .borrow()
                .close_handlers
                .get(window.key)
                .cloned()
                .ok_or(WindowError::Close(CloseError::MissingHandler))?;
            let confirmation = self
                .state
                .borrow()
                .confirmation_window(window.key)
                .ok_or(WindowError::Close(CloseError::ConfirmationNotDeclared))?;
            let stale = self.state.borrow().window_ctx(confirmation);
            if let Some(stale) = stale {
                self.close_window(stale)?;
            }

            let request = {
                let mut state = self.state.borrow_mut();
                let epoch = state.next_close_epoch;
                state.next_close_epoch += 1;
                let focus = state
                    .focused_in(TreeSlot::Window(window.key))
                    .map(|element| state.elements[element].id);
                state.close_epochs.insert(
                    window.key,
                    CloseEpoch {
                        id: epoch,
                        parent: window,
                        confirmation,
                        focus,
                        reply: None,
                    },
                );
                CloseRequest::new(state.id, window, epoch)
            };
            let queued_before = self.state.borrow().window_requests.len();
            let outcome = self.invoke_close_handler(
                window,
                &handler,
                CloseEvent::new(request, source),
                queued_before,
            )?;

            // 显式 `request.allow()/deny()` 优先于返回值；关闭 turn 内排队的
            // 确认窗口 show 随决定一起失效。
            if let Some(reply) = self.take_handler_reply(window, request, confirmation) {
                return match reply {
                    CloseReply::Allow => self.release_close_group(window.key),
                    CloseReply::Deny => self.deny_close(window),
                };
            }

            match outcome {
                CloseOutcome::Allow => {
                    self.discard_confirmation_requests(confirmation);
                    self.release_close_group(window.key)
                }
                CloseOutcome::Deny => {
                    self.discard_confirmation_requests(confirmation);
                    self.deny_close(window)
                }
                CloseOutcome::Pending => {
                    let settled = self.settle_show_requests();
                    let shown = with_state(|state| state.window_is_visible(confirmation));
                    if let Err(error) = settled {
                        self.abort_close_epoch(window.key, queued_before);
                        self.close_visible_confirmation(confirmation);
                        return Err(error);
                    }
                    if !shown {
                        // Pending 但没有可见确认窗口：fail closed，按 Deny 结算。
                        self.deny_close(window)?;
                        return Err(WindowError::Close(CloseError::ConfirmationNotShown));
                    }
                    Ok(())
                }
            }
        })
    }

    /// 丢弃 handler 为确认窗口排队的 show；已经显式回复时不再打开它。
    fn discard_confirmation_requests(&mut self, confirmation: &'static str) {
        with_state(|state| {
            state.window_requests.retain(
                |queued| !matches!(queued.action, WindowAction::Show(key) if key == confirmation),
            );
        });
    }

    /// 在 Event 阶段调用 close handler；panic 时撤销 epoch 并报告失败。
    fn invoke_close_handler(
        &mut self,
        window: WindowCtx,
        handler: &CloseHandler,
        event: CloseEvent,
        queued_before: usize,
    ) -> Result<CloseOutcome, WindowError> {
        let previous_phase = with_state(|state| {
            let previous = state.phase;
            state.phase = Phase::Event;
            previous
        });
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(event)));
        with_state(|state| state.phase = previous_phase);
        match outcome {
            Ok(outcome) => Ok(outcome),
            Err(_) => {
                self.abort_close_epoch(window.key, queued_before);
                Err(WindowError::Close(CloseError::HandlerPanicked))
            }
        }
    }

    /// 取走处理器在调用期间排队的立即回复，并丢弃它要求显示的陈旧确认窗口。
    fn take_handler_reply(
        &mut self,
        window: WindowCtx,
        request: CloseRequest,
        confirmation: &'static str,
    ) -> Option<CloseReply> {
        let reply = with_state(|state| {
            state
                .close_epochs
                .get(window.key)
                .and_then(|epoch| epoch.reply)
        })?;
        with_state(|state| {
            state.close_replies.retain(|queued| {
                queued.request.epoch != request.epoch || queued.request.parent != window
            });
            state.window_requests.retain(|queued| {
                !matches!(
                    queued.action,
                    WindowAction::Show(key) if key == confirmation
                )
            });
        });
        Some(reply)
    }

    fn abort_close_epoch(&mut self, parent: &'static str, queued_before: usize) {
        with_state(|state| {
            state.close_epochs.remove(parent);
            state.window_requests.truncate(queued_before);
        });
    }

    fn close_visible_confirmation(&mut self, confirmation: &'static str) {
        let ctx = self.state.borrow().window_ctx(confirmation);
        if let Some(ctx) = ctx {
            let _ = self.close_window_now(ctx);
        }
    }

    /// Allow：确认窗口与父窗口作为一组释放。
    fn release_close_group(&mut self, parent: &'static str) -> Result<(), WindowError> {
        let state = self.state.clone();
        context::install(&state, || {
            let epoch = { self.state.borrow_mut().close_epochs.remove(parent) };
            let Some(epoch) = epoch else {
                return Err(WindowError::Stale);
            };
            let mut first_error = None;
            let confirmation = self.state.borrow().window_ctx(epoch.confirmation);
            if let Some(confirmation) = confirmation
                && let Err(error) = self.close_window_now(confirmation)
                && error != WindowError::Stale
            {
                first_error = Some(error);
            }
            let parent_ctx = self.state.borrow().window_ctx(parent);
            if let Some(parent_ctx) = parent_ctx {
                self.close_window_now(parent_ctx)?;
            }
            match first_error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        })
    }

    /// Deny：释放确认窗口、恢复父窗口输入，并尽量恢复挂起前的焦点。
    fn deny_close(&mut self, parent: WindowCtx) -> Result<(), WindowError> {
        let state = self.state.clone();
        context::install(&state, || {
            let epoch = { self.state.borrow_mut().close_epochs.remove(parent.key) };
            let Some(epoch) = epoch else {
                return Err(WindowError::Stale);
            };
            let mut first_error = None;
            let confirmation = self.state.borrow().window_ctx(epoch.confirmation);
            if let Some(confirmation) = confirmation
                && let Err(error) = self.close_window_now(confirmation)
                && error != WindowError::Stale
            {
                first_error = Some(error);
            }
            let still_live = self.state.borrow().window_ctx(parent.key).is_some();
            if still_live {
                with_state(|state| state.focused_window = Some(parent.key));
                if let Some(focus) = epoch.focus {
                    let target = with_state(|state| {
                        let element = state.element_slot(focus)?;
                        (state.element_is_mounted(element) && state.is_focusable(element))
                            .then_some(element)
                    });
                    if let Some(target) = target {
                        self.move_focus(TreeSlot::Window(parent.key), Some(target))?;
                    }
                }
            }
            match first_error {
                Some(error) => Err(error),
                None => Ok(()),
            }
        })
    }

    /// 无条件释放一个 activation；调用方负责先结算 close epoch。
    fn close_window_now(&mut self, window: WindowCtx) -> Result<(), WindowError> {
        let state = self.state.clone();
        if window.runtime != state.borrow().id {
            return Err(WindowError::WrongRuntime);
        }
        context::install(&state, || {
            if !with_state(|state| state.window_live_in_tree(window)) {
                return Err(WindowError::Stale);
            }
            let tree = with_state(|state| state.trees.remove(&TreeSlot::Window(window.key)));
            let Some(MountedTree { mut nodes, .. }) = tree else {
                return Err(WindowError::Stale);
            };
            with_state(|state| {
                state.cancel_window(window.key, window.activation);
            });
            self.release_nodes(&mut nodes);
            with_state(|state| {
                state.dispose_owner(window.owner);
                state.drop_tree_binding_table(TreeSlot::Window(window.key));
                if state.focused_window == Some(window.key) {
                    state.focused_window = state.first_visible_window();
                }
            });
            Ok(())
        })
    }

    pub fn window_alive(&self, window: WindowCtx) -> bool {
        let state = self.state.borrow();
        window.runtime == state.id
            && state.window_is_live(window.key, window.activation, window.owner)
    }

    /// 窗口仍然存活且处于可见状态。
    pub fn window_visible(&self, window: WindowCtx) -> bool {
        let state = self.state.borrow();
        window.runtime == state.id
            && state.window_is_live(window.key, window.activation, window.owner)
            && state.window_is_visible(window.key)
    }

    /// 批量声明窗口化 `For` 的挂载窗口，一个原子结构 turn。
    ///
    /// `bands` 的每个键来自快照的 [`SnapshotRowBand::key`]，窗口目标由宿主从
    /// 行带几何和滚动偏移换算。批量先整体校验：任一行带不属于该窗口的已
    /// 提交树、不是声明了 `row-height` 的行带（[`WindowError::Stale`]），或
    /// 同一个行带出现两次（[`WindowError::DuplicateRowBand`]）时，整批请求
    /// 都不应用。
    ///
    /// 返回报告的 [`TurnReport::changed`] 为真当且仅当本 turn 推进了窗口树
    /// 的呈现版本：有行挂载/卸载、spacer 高度变化、焦点被清除或值绑定变化。
    /// 空批次和与当前请求完全相同的批次不刷新，返回 `changed == false`。
    ///
    /// 结构 turn 里任一行工厂失败时，本批改变过的请求全部回滚到调用前的
    /// 窗口；已经提交过窗口的行带逐个再刷新一次，把行搬回旧窗口。旧行本身
    /// 也挂不回去（同一个工厂故障持续）时，该行带的请求改为描述实际提交的
    /// 窗口，下一次普通刷新不会把行搬到宿主没有请求过的地方。错误以
    /// [`WindowError::Mount`] 返回，树保持最后一次可提交的结构，宿主可以
    /// 重试同一个批次。
    pub fn set_row_windows(
        &mut self,
        window: WindowCtx,
        bands: &[(u64, RowWindow)],
    ) -> Result<TurnReport, WindowError> {
        let state = self.state.clone();
        if window.runtime != state.borrow().id {
            return Err(WindowError::WrongRuntime);
        }
        context::install(&state, || {
            if !with_state(|state| state.window_live_in_tree(window)) {
                return Err(WindowError::Stale);
            }
            let key = TreeSlot::Window(window.key);
            // 1) 预校验全部行带：任何一条失败都不应用任何请求。活行带集合
            // 每次 turn 只遍历树一次，批量大小不影响校验成本。
            let live = with_state(|state| state.tree_band_keys(key));
            let mut seen = BTreeSet::new();
            for (band, _) in bands {
                if !seen.insert(*band) {
                    return Err(WindowError::DuplicateRowBand);
                }
                if !live.contains(band) {
                    return Err(WindowError::Stale);
                }
            }
            // 2) 只记录与当前请求不同的行带；请求本身不改 revision，只有
            // 刷新真的改变了呈现（行挂载/卸载、spacer 高度、值绑定）才由
            // 刷新路径标记 dirty。
            let mut changed_bands = BTreeSet::new();
            let mut previous = Vec::new();
            for (band, request) in bands {
                let current = with_state(|state| state.row_windows.get(band).copied());
                if current == Some(*request) {
                    continue;
                }
                changed_bands.insert(*band);
                previous.push((*band, current));
                with_state(|state| {
                    state.row_windows.insert(*band, *request);
                });
            }
            if changed_bands.is_empty() {
                return Ok(TurnReport::default());
            }
            // 3) 一个求值轮次消费整批请求。批标记只在这段调用内存活：
            // refresh_for 据此记录哪些行带已经提交，调用结束即清零。
            with_state(|state| {
                state.bump_refresh_round();
                state.row_window_batch = Some(RowWindowBatch {
                    bands: changed_bands,
                    applied: BTreeSet::new(),
                });
            });
            let result = self.refresh_tree_once(key);
            let batch = with_state(|state| state.row_window_batch.take());
            let batch = batch.unwrap_or_default();
            match result {
                Ok(changed) => Ok(TurnReport {
                    changed,
                    ..TurnReport::default()
                }),
                Err(error) => {
                    // 原子回滚：恢复本批改变过的全部请求。行带已随 owner
                    // 释放时请求一并丢弃（它的 For 已不在树里），不能把
                    // 请求复活成没有归属的映射。
                    with_state(|state| {
                        for (band, previous) in previous {
                            if !state.tree_has_band(key, band) {
                                state.row_windows.remove(&band);
                                continue;
                            }
                            match previous {
                                Some(request) => {
                                    state.row_windows.insert(band, request);
                                }
                                None => {
                                    state.row_windows.remove(&band);
                                }
                            }
                        }
                    });
                    if !batch.applied.is_empty() {
                        // 失败前已有行带提交了新窗口：逐个把已提交的行带
                        // 刷回恢复后的旧请求。只刷新这些 For，不重跑整棵树，
                        // 避免再次撞上失败行或无关兄弟节点的错误。修复仍是
                        // 结构阶段：新行必须挂到本窗口的树，而不是根树。
                        let (previous_phase, previous_tree) = with_state(|state| {
                            let previous = (state.phase, state.building_tree);
                            state.refresh_focus = None;
                            state.phase = Phase::Binding;
                            state.building_tree = Some(key);
                            previous
                        });
                        for band in batch.applied {
                            // 每个行带一个新求值轮次：另一个行带的修复刷新
                            // 可能已经通过嵌套结构消费过这个 For，并把它的
                            // `structure_round` 记成当前轮；不换轮次会让本轮
                            // 自己的修复被跳过，请求与已提交窗口就此分叉。
                            with_state(State::bump_refresh_round);
                            let Some(payload) =
                                with_state(|state| state.for_payload_for_band(key, band))
                            else {
                                continue;
                            };
                            // 修复会求值用户结构（Show/嵌套 For 的触发）；
                            // 与完整刷新一样包在 panic 边界里，毒化 memo 等
                            // 用户代码不能穿过公开入口。修复失败按“旧结构
                            // 不可表示”处理。
                            let repaired = catch_panic(std::panic::AssertUnwindSafe(|| {
                                self.refresh_for(payload)
                            }));
                            if matches!(repaired, Ok(Ok(()))) {
                                continue;
                            }
                            // 旧行也挂不回去时，旧结构已经不可表示。让请求
                            // 映射描述实际提交的窗口，避免下一次普通刷新把
                            // 行悄悄搬到宿主没请求过的地方；原始错误仍然
                            // 返回给调用方。窗口跨度取最近提交的有效 count，
                            // 不能取 `order.len()`：静态行集里已删除的行不
                            // 挂载，但仍在窗口跨度内。
                            let committed = with_state(|state| {
                                let payload = state.for_payload_for_band(key, band)?;
                                let for_node = state.nodes.for_node(payload);
                                Some((for_node.first_row, for_node.window_count))
                            });
                            if let Some((first, count)) = committed {
                                with_state(|state| {
                                    state.row_windows.insert(band, RowWindow::new(first, count));
                                });
                            }
                        }
                        // 修复重挂的行没有已提交的绑定值；补一次值阶段，
                        // 动态文本与属性不会在失败帧后呈现为空。
                        let _ = self.refresh_values_once(key);
                        with_state(|state| {
                            state.commit_tree_revision(key);
                            state.phase = previous_phase;
                            state.building_tree = previous_tree;
                            // 修复是失败 turn 的恢复步骤；它产生的读失败与
                            // 焦点丢弃不留给下一次普通刷新重复报告。
                            state.fault = None;
                            state.window_focus_dropped = None;
                        });
                    }
                    Err(WindowError::Mount(error))
                }
            }
        })
    }

    /// [`Runtime::set_row_windows`] 的单带便捷形式；语义相同。
    ///
    /// `band` 来自快照的 [`SnapshotRowBand::key`]，`first`/`count` 是从行带
    /// 几何和滚动偏移换算出的目标区间。
    pub fn set_row_window(
        &mut self,
        window: WindowCtx,
        band: u64,
        first: usize,
        count: usize,
    ) -> Result<TurnReport, WindowError> {
        self.set_row_windows(window, &[(band, RowWindow::new(first, count))])
    }

    /// 刷新全部已提交树，并返回自上次报告以来的有效呈现变化。
    ///
    /// 报告是累计的：命令内部的刷新不会丢弃变化，调用方可以在一个 turn
    /// 结束后读取它，看到本 turn 推进过 revision 的树。
    pub fn refresh(&mut self) -> Result<RefreshReport, MountError> {
        self.refresh_internal()?;
        Ok(self.take_refresh_report())
    }

    pub(crate) fn refresh_internal(&mut self) -> Result<(), MountError> {
        if self.tree_keys().is_empty() {
            return Err(MountError::Component("no committed interface is mounted"));
        }
        let consumed = {
            let mut state = self.state.borrow_mut();
            state.read_faults.clear();
            // 上一个 turn 的补丁已经交付；新 turn 从空日志开始。
            let _ = state.binding_patches.take();
            state.pending.borrow().mark()
        };
        self.settle_quarantined_memos();
        self.take_flush_faults()?;
        if let Err(error) = self.refresh_all_once() {
            // 结构提交失败时旧行保留；memo worklist 只在本轮开始时运行，
            // flush 在错误处结束，因此本帧后续不会再结算这些行的 memo。
            // 已完成的窗口保留本帧结果；不能让焦点身份停留在已被移除或禁用的旧目标上。
            self.clear_invalid_focus();
            return Err(error);
        }
        self.settle_focus()?;
        // step 6：flush 完整结束后才丢弃本次消费的提交。提前返回的错误帧
        // 保留 PendingChanges；flush 执行中（结构/工厂）产生的提交也保留到
        // 下一次 flush 的 step 0，下一次 flush 仍能看到这些删除行。
        self.state
            .borrow()
            .pending
            .borrow_mut()
            .drain_consumed(consumed);
        Ok(())
    }

    /// flush step 0 + 1：按累计提交计算 quarantine，再结算可达的脏 memo。
    fn settle_quarantined_memos(&mut self) {
        let quarantined = self.state.borrow().quarantined_rows();
        self.settle_memos(&quarantined);
    }

    /// settle 在 Phase::Memo 中运行；非法写入要像刷新期写入一样冒泡为
    /// Context 错误，而不是被随后每棵树的 fault 重置吞掉。
    fn take_flush_faults(&mut self) -> Result<(), MountError> {
        let fault = self.state.borrow_mut().fault.take();
        if let Some(fault) = fault {
            self.clear_invalid_focus();
            return Err(State::context_error(fault));
        }
        let write_error = self.state.borrow_mut().write_error.take();
        if let Some(error) = write_error {
            self.clear_invalid_focus();
            return Err(MountError::Write(error));
        }
        Ok(())
    }

    /// 取走累计的树 revision 变化，组装公开报告。
    fn take_refresh_report(&mut self) -> RefreshReport {
        let mut state = self.state.borrow_mut();
        let keys = std::mem::take(&mut state.changed_trees);
        let faults = std::mem::take(&mut state.read_faults);
        let mut report = RefreshReport {
            faults,
            ..RefreshReport::default()
        };
        for key in keys {
            if !state.trees.contains_key(&key) {
                continue;
            }
            match key {
                TreeSlot::Root => report.root_changed = true,
                TreeSlot::Window(key) => report.changed_windows.push(key),
            }
        }
        report.changed = report.root_changed || !report.changed_windows.is_empty();
        report
    }

    fn tree_keys(&self) -> Vec<TreeSlot> {
        self.state.borrow().trees.keys().copied().collect()
    }

    /// flush step 1：只结算仍被已挂载消费者可达的脏 memo。
    ///
    /// 一个 memo 重算相等时不会传播，因此下游不会被拉进 worklist；只被事件
    /// 处理器读取的 memo 保持惰性，也不会有挂载期副作用。`quarantined` 行上的
    /// 绑定不作为 liveness 种子：只服务这些行的 memo 本轮跳过，边保留到结构
    /// 提交确认卸载。
    fn settle_memos(&mut self, quarantined: &BTreeSet<RowId>) {
        let state = self.state.clone();
        context::install(&state, || {
            let live = with_state(|state| state.live_memo_set(quarantined));
            let mut settled = BTreeSet::new();
            while let Some(id) = with_state(|state| state.next_dirty_live_memo(&live)) {
                // 无环图里每个 memo 一拍最多结算一次：依赖都在读取时先结算，
                // 之后的源写入在本帧被禁止。再次被选中说明动态依赖成环。
                if !settled.insert(id) {
                    with_state(|state| state.record_read_fault(ReadFault::GraphCycle));
                    break;
                }
                let (compute, previous) = with_state(|state| state.begin_memo(id));
                match catch_panic(|| compute.call()) {
                    Ok(value) => {
                        with_state(|state| state.finish_memo(id, value, previous, compute));
                    }
                    Err(panic) => {
                        with_state(|state| state.abort_memo(id, panic, previous));
                    }
                }
            }
        });
    }

    fn refresh_all_once(&mut self) -> Result<(), MountError> {
        // 每轮开始新的求值轮次；同一 binding 每轮最多求值一次。
        // 这里不在已安装的上下文里，直接借 state。
        self.state.borrow_mut().bump_refresh_round();
        // 一棵树的错误只影响它自己：后续树仍然刷新，错误在最后统一报告。
        let mut first_error = None;
        for key in self.tree_keys() {
            if let Err(error) = self.refresh_tree_once(key)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// 刷新一棵树；返回本次是否推进了这棵树的 revision。
    fn refresh_tree_once(&mut self, key: TreeSlot) -> Result<bool, MountError> {
        let state = self.state.clone();
        context::install(&state, || {
            let (taken, previous_phase, previous_tree) = with_state(|state| {
                state.fault = None;
                let previous_phase = state.phase;
                let previous_tree = state.building_tree;
                state.phase = Phase::Binding;
                state.building_tree = Some(key);
                let focused = state.focused_in(key);
                // 窗口化 For 在刷新里据此判断聚焦行是否仍在挂载窗口内。
                state.refresh_focus = focused;
                let settle_before = focused.is_some().then(|| state.element_order(key));
                if let Some(entry) = state.tree_mut(key) {
                    entry.settle_before = settle_before;
                }
                (state.take_tree(key), previous_phase, previous_tree)
            });
            let Some(tree) = taken else {
                with_state(|state| {
                    state.refresh_focus = None;
                    state.building_tree = previous_tree;
                });
                return Ok(false);
            };
            let mut guard = TreeRefreshGuard::new(key, tree, previous_phase, previous_tree);
            let result = catch_panic(std::panic::AssertUnwindSafe(|| {
                self.refresh_nodes(guard.nodes())
            }));
            guard.restore();
            // 刷新中的 panic（例如毒化且没有最后值的 memo 被文本槽读取）
            // 不能穿过公开入口；树已经放回，旧结构仍可继续使用。
            let result = match result {
                Ok(result) => result,
                Err(panic) => Err(MountError::Panic(panic)),
            };
            let fault = with_state(|state| {
                state.refresh_focus = None;
                // 焦点行被滚动窗口卸载时直接清除焦点；它不能结算到相邻行，
                // 那会把 Tab 的语义变成一次隐式焦点跳转（ADR-0011）。
                if let Some(dropped) = state.window_focus_dropped.take()
                    && state.focused_in(key) == Some(dropped)
                    && !state.element_is_mounted(dropped)
                {
                    state.set_focused_in(key, None);
                }
                state.phase = previous_phase;
                state.fault
            });
            // revision 在焦点清理之后提交，聚焦行被窗口卸载也算本树变化。
            let changed = with_state(|state| state.commit_tree_revision(key));
            result?;
            if let Some(fault) = fault {
                return Err(State::context_error(fault));
            }
            Ok(changed)
        })
    }

    /// 只求值一棵树的值绑定，不重跑结构触发。
    ///
    /// 失败批量 turn 的修复刷新重挂了旧行；新行的绑定槽没有已提交值，
    /// 必须补一次值阶段，否则动态文本/属性会呈现为空，直到下一次完整
    /// 刷新。结构已经稳定，这里只走值阶段，并沿用与完整刷新相同的
    /// panic 边界与树保护。
    fn refresh_values_once(&mut self, key: TreeSlot) -> Result<(), MountError> {
        let state = self.state.clone();
        context::install(&state, || {
            let (taken, previous_phase, previous_tree) = with_state(|state| {
                let previous_phase = state.phase;
                let previous_tree = state.building_tree;
                state.phase = Phase::Binding;
                state.building_tree = Some(key);
                state.refresh_focus = None;
                (state.take_tree(key), previous_phase, previous_tree)
            });
            let Some(tree) = taken else {
                with_state(|state| {
                    state.phase = previous_phase;
                    state.building_tree = previous_tree;
                });
                return Ok(());
            };
            let mut guard = TreeRefreshGuard::new(key, tree, previous_phase, previous_tree);
            let result = catch_panic(std::panic::AssertUnwindSafe(|| {
                self.refresh_value_nodes(guard.nodes())
            }));
            guard.restore();
            with_state(|state| {
                state.phase = previous_phase;
                state.refresh_focus = None;
            });
            match result {
                Ok(result) => result,
                Err(panic) => Err(MountError::Panic(panic)),
            }
        })
    }

    fn clear_invalid_focus(&mut self) {
        let mut state = self.state.borrow_mut();
        for key in state.trees.keys().copied().collect::<Vec<_>>() {
            let focused = state.focused_in(key);
            if focused.is_some_and(|id| !state.element_is_mounted(id) || !state.is_focusable(id)) {
                state.set_focused_in(key, None);
            }
        }
    }

    /// 结构变化后的焦点结算只做一次批量决定；handler 造成的进一步变化在下一轮结算。
    ///
    /// blur/focus 处理器可能提交 Store；每一轮在结构刷新之前重新执行
    /// step 0/1，让焦点结算产生的删除行先进入 quarantine 再求值 memo。
    fn settle_focus(&mut self) -> Result<(), MountError> {
        const MAX_SETTLEMENTS: usize = 8;

        let state = self.state.clone();
        context::install(&state, || {
            for _ in 0..MAX_SETTLEMENTS {
                if !self.settle_round(true)? {
                    return Ok(());
                }
                self.settle_quarantined_memos();
                self.take_flush_faults()?;
                self.refresh_all_once()?;
            }
            // 第 8 轮恰好收敛时焦点仍然有效；只有确实还有待结算的目标才做兜底清理。
            if self.settle_round(false)? {
                self.settle_quarantined_memos();
                self.take_flush_faults()?;
                self.refresh_all_once()?;
            }
            Ok(())
        })
    }

    /// 结算每个有焦点的窗口一轮；返回是否还有待结算的目标。
    fn settle_round(&mut self, keep_target: bool) -> Result<bool, MountError> {
        let mut settled = false;
        for key in self.tree_keys() {
            if let Some(settlement) = with_state(|state| state.focus_settlement(key)) {
                settled = true;
                self.apply_focus_settlement(key, settlement, keep_target)?;
            }
        }
        Ok(settled)
    }

    /// 应用一次焦点结算；`keep_target` 为 false 时只清理旧目标（第 8 轮兜底）。
    fn apply_focus_settlement(
        &mut self,
        key: TreeSlot,
        settlement: FocusSettlement,
        keep_target: bool,
    ) -> Result<(), MountError> {
        with_state(|state| {
            state.phase = Phase::Event;
            state.fault = None;
            state.write_error = None;
        });
        let mut panic = None;
        if settlement.blur_old {
            with_state(|state| state.set_focused_in(key, None));
            if let Err(info) = self.fire_blur(settlement.old, FocusCause::Settlement) {
                panic = Some(info);
            }
        }
        match (keep_target, settlement.target) {
            (true, Some(target)) => {
                with_state(|state| state.set_focused_in(key, Some(target)));
                if panic.is_none()
                    && let Err(info) = self.fire_focus(target, FocusCause::Settlement)
                {
                    panic = Some(info);
                }
            }
            _ => with_state(|state| state.set_focused_in(key, None)),
        }
        with_state(|state| state.phase = Phase::Idle);
        if let Some(panic) = panic {
            let _ = self.finish_command();
            return Err(MountError::Panic(panic));
        }
        if let Some(fault) = with_state(|state| state.fault.take()) {
            return Err(State::context_error(fault));
        }
        if let Some(error) = with_state(|state| state.write_error.take()) {
            return Err(MountError::Write(error));
        }
        Ok(())
    }

    fn mount_plan(
        &mut self,
        plan: MountPlan,
        parent: u32,
        parent_node: Option<NodeRef>,
    ) -> Result<Vec<MountedNode>, MountError> {
        let mut nodes = Vec::with_capacity(plan.nodes.len());
        for node in plan.nodes {
            let result = match node {
                PlanNode::Children(children) => {
                    match with_owner(parent, || catch_panic(|| children.mount())) {
                        Ok(Ok(plan)) => self.mount_plan(plan, parent, parent_node),
                        Ok(Err(error)) => Err(error),
                        Err(panic) => Err(MountError::Panic(panic)),
                    }
                }
                PlanNode::Factory(factory) => {
                    match with_owner(parent, || catch_panic(|| factory.mount())) {
                        Ok(Ok(plan)) => self.mount_plan(plan, parent, parent_node),
                        Ok(Err(error)) => Err(error),
                        Err(panic) => Err(MountError::Panic(panic)),
                    }
                }
                node => self
                    .mount_node(node, parent, parent_node)
                    .map(|node| vec![node]),
            };
            match result {
                Ok(mut mounted) => nodes.append(&mut mounted),
                Err(error) => {
                    self.release_nodes(&mut nodes);
                    return Err(error);
                }
            }
        }
        Ok(nodes)
    }

    /// 挂载期求值 Show/For 触发：memo 的 panic 属于挂载失败，不能穿过
    /// `Runtime::mount`/窗口挂载等公开入口；调用方负责释放 owner。
    fn mount_refresh(
        &mut self,
        refresh: impl FnOnce(&mut Self) -> Result<(), MountError>,
    ) -> Result<(), MountError> {
        match catch_panic(std::panic::AssertUnwindSafe(|| refresh(self))) {
            Ok(result) => result,
            Err(panic) => Err(MountError::Panic(panic)),
        }
    }

    fn mount_node(
        &mut self,
        node: PlanNode,
        parent: u32,
        parent_node: Option<NodeRef>,
    ) -> Result<MountedNode, MountError> {
        match node {
            PlanNode::Text(slot) => {
                let payload = with_state(|state| {
                    let tree = state.building_tree_key();
                    let node = state.register_node(NodeKindTag::Text, parent, parent_node);
                    let binding = {
                        let table = state.nodes.binding_table_mut(tree);
                        slot.into_binding(table)
                    };
                    let payload = state.nodes.reserve_text(TextData {
                        binding,
                        tree,
                        node,
                    });
                    state
                        .nodes
                        .set_payload(node, payload)
                        .expect("the text node is live");
                    payload
                });
                Ok(MountedNode::Text(payload))
            }
            PlanNode::Children(_) | PlanNode::Factory(_) => {
                unreachable!("slots are spliced by mount_plan")
            }
            PlanNode::Element(plan) => {
                let owner = with_state(|state| state.create_owner(Some(parent)));
                let element_node = with_state(|state| {
                    state.register_node(NodeKindTag::Element, owner, parent_node)
                });
                let children =
                    match self.mount_plan(MountPlan::new(plan.children), owner, Some(element_node))
                    {
                        Ok(children) => children,
                        Err(error) => {
                            with_state(|state| state.dispose_owner(owner));
                            return Err(error);
                        }
                    };
                let tag = plan.tag;
                let id = with_state(|state| {
                    let tree = state.building_tree_key();
                    let mut attrs: Vec<MountedAttr> = Vec::new();
                    let mut events = Vec::new();
                    let mut bindings: Vec<(PartId, BindingRef)> = Vec::new();
                    for attr in plan.attrs {
                        match attr {
                            Attr::Value { name, value } => {
                                let (part, binding) = {
                                    let table = state.nodes.binding_table_mut(tree);
                                    let part = table.part_id(BindingPart::Attr(name));
                                    let binding = match value {
                                        AttrValue::Text(read) => read.into_binding(table),
                                        AttrValue::Bool(read) => read.into_binding(table),
                                    };
                                    (part, binding)
                                };
                                // 同名属性以最后一个声明为准；先归还被替换的
                                // 槽，保证元素分配的每个槽都在它的绑定列表里。
                                if let Some(index) = attrs.iter().position(|attr| attr.name == name)
                                {
                                    let replaced = attrs.remove(index);
                                    bindings.retain(|(item, _)| *item != part);
                                    state.nodes.release_binding(tree, replaced.binding);
                                }
                                attrs.push(MountedAttr { name, binding });
                                bindings.push((part, binding));
                            }
                            Attr::Event(event) => {
                                let id = state.reserve_event(event);
                                events.push(id);
                                if let Some(record) = state.owners.get_mut(&owner) {
                                    record.resources.push(Resource::Event(id));
                                }
                            }
                        }
                    }
                    let slot = state.reserve_element(owner);
                    let id = NodeId::mint(state.id, element_node);
                    state
                        .nodes
                        .set_payload(
                            element_node,
                            u32::try_from(slot).expect("node identity exhausted"),
                        )
                        .expect("the element node is live");
                    // 每次挂载（包括同 key 重挂载）都建立新的编辑 origin；
                    // 旧挂载的 Node ID 是 StaleNode，旧派发无法命中新编辑器。
                    let local = match tag {
                        "text-input" => {
                            let origin = id.generation();
                            state.push_text_trace(TextTrace::Remount { origin });
                            LocalState::TextInput(Box::new(TextEditing::new(origin)))
                        }
                        "checkbox" => LocalState::Checkbox { checked: false },
                        _ => LocalState::None,
                    };
                    let mut inline_styles: Vec<MountedInlineStyle> =
                        Vec::with_capacity(plan.inline_styles.len());
                    for InlineStyle { property, slot } in plan.inline_styles {
                        let (part, binding) = {
                            let table = state.nodes.binding_table_mut(tree);
                            let part = table.part_id(BindingPart::Style(property));
                            let binding = match slot {
                                StyleSlot::Color(read) => read.into_binding(table),
                                StyleSlot::Length(read) => read.into_binding(table),
                                StyleSlot::Opacity(read) => read.into_binding(table),
                            };
                            (part, binding)
                        };
                        // 同一 property 重复声明时最后一个生效，被替换的槽立即归还。
                        if let Some(index) = inline_styles
                            .iter()
                            .position(|style| style.property == property)
                        {
                            let replaced: MountedInlineStyle = inline_styles.remove(index);
                            bindings.retain(|(item, _)| *item != part);
                            state.nodes.release_binding(tree, replaced.binding);
                        }
                        bindings.push((part, binding));
                        inline_styles.push(MountedInlineStyle { property, binding });
                    }
                    let semantics = SnapshotSemantics::new(
                        inherent_focusable(state, tag, &events),
                        actions_for_events(state, &events),
                    );
                    let aux = state.nodes.reserve_element_aux(AuxSeed {
                        attrs,
                        class_styles: plan.class_styles,
                        inline_styles,
                        events,
                        edit: local,
                        semantics,
                    });
                    {
                        let mut replaced = Vec::new();
                        {
                            let list = state.nodes.binding_list_mut(aux.bindings);
                            for (part, binding) in bindings {
                                if let Some(old) = list.insert(part, binding) {
                                    replaced.push(old);
                                }
                            }
                        }
                        // 防御：任何替换都必须归还旧槽，不能留在表里。
                        for old in replaced {
                            state.nodes.release_binding(tree, old);
                        }
                    }
                    let tag_id = state.nodes.intern_tag(tag);
                    state.nodes.place_element_data(
                        slot,
                        ElementData {
                            tag: tag_id,
                            flags: ElementFlags::NONE,
                            owner,
                            aux,
                        },
                    );
                    state.place_element(slot, MountedElement { id, children });
                    slot
                });
                Ok(MountedNode::Element(id))
            }
            PlanNode::Component(plan) => {
                let label = plan.label;
                let owner = with_state(|state| state.create_owner(Some(parent)));
                let component_node = with_state(|state| {
                    state.register_node(NodeKindTag::Component, owner, parent_node)
                });
                let mounted = with_owner(owner, || {
                    let _phase = PhaseGuard::enter(Phase::Setup);
                    catch_panic(plan.mount)
                });
                let fault = with_state(|state| state.fault.take());
                let plan = match mounted {
                    Ok(Ok(plan)) => plan,
                    Ok(Err(error)) => {
                        with_state(|state| state.dispose_owner(owner));
                        return Err(error);
                    }
                    Err(panic) => {
                        with_state(|state| state.dispose_owner(owner));
                        return Err(MountError::Panic(panic));
                    }
                };
                if let Some(fault) = fault {
                    with_state(|state| state.dispose_owner(owner));
                    return Err(State::context_error(fault));
                }
                let children = match self.mount_plan(plan, owner, Some(component_node)) {
                    Ok(children) => children,
                    Err(error) => {
                        with_state(|state| state.dispose_owner(owner));
                        return Err(error);
                    }
                };
                let id = with_state(|state| {
                    *state.setup_counts.entry(label).or_default() += 1;
                    let slot = state.reserve_component(owner);
                    state
                        .nodes
                        .set_payload(
                            component_node,
                            u32::try_from(slot).expect("component identity exhausted"),
                        )
                        .expect("the component node is live");
                    state.place_component(slot, MountedComponent { owner, children });
                    slot
                });
                Ok(MountedNode::Component(id))
            }
            PlanNode::Show(plan) => {
                let owner = with_state(|state| state.create_owner(Some(parent)));
                let show_node =
                    with_state(|state| state.register_node(NodeKindTag::Show, owner, parent_node));
                let payload = with_state(|state| {
                    let tree = state.building_tree_key();
                    let when = {
                        let table = state.nodes.binding_table_mut(tree);
                        plan.when.into_binding(table)
                    };
                    let payload = state.nodes.reserve_show(ShowData {
                        owner,
                        node: show_node,
                        when,
                        then: plan.then,
                        fallback: plan.fallback,
                        active: None,
                        structure_round: u64::MAX,
                        values_round: u64::MAX,
                    });
                    state
                        .nodes
                        .set_payload(show_node, payload)
                        .expect("the Show node is live");
                    payload
                });
                let speculation = with_state(|state| state.mark_speculation());
                if let Err(error) = self.mount_refresh(|state| state.refresh_show(payload)) {
                    with_state(|state| {
                        state.rollback_speculation(speculation);
                        state.dispose_owner(owner);
                    });
                    return Err(error);
                }
                Ok(MountedNode::Show(payload))
            }
            PlanNode::For(plan) => {
                let owner = with_state(|state| state.create_owner(Some(parent)));
                let for_node =
                    with_state(|state| state.register_node(NodeKindTag::For, owner, parent_node));
                let band = with_state(|state| {
                    let band = state.next_row_band;
                    state.next_row_band += 1;
                    state.owner_bands.entry(owner).or_default().push(band);
                    band
                });
                let crate::view::ForPlan {
                    store,
                    rows,
                    driver,
                    row_height,
                    attach,
                } = plan;
                // 挂载期安装提交观察者：构造 `ForSource` 可以早于 runtime。
                attach();
                let payload = with_state(|state| {
                    let tree = state.building_tree_key();
                    let rows = match rows {
                        ForRows::Read(read) => {
                            let table = state.nodes.binding_table_mut(tree);
                            MountedForRows::Read(read.into_binding(table))
                        }
                        ForRows::Projection(filter) => MountedForRows::Projection(filter),
                    };
                    let payload = state.nodes.reserve_for(ForData {
                        owner,
                        node: for_node,
                        store,
                        rows,
                        driver,
                        row_height,
                        band,
                        total_rows: 0,
                        first_row: 0,
                        window_count: 0,
                        spacers: None,
                        committed_rows: None,
                        items: BTreeMap::new(),
                        order: Vec::new(),
                        structure_round: u64::MAX,
                    });
                    state
                        .nodes
                        .set_payload(for_node, payload)
                        .expect("the For node is live");
                    payload
                });
                let speculation = with_state(|state| state.mark_speculation());
                if let Err(error) = self.mount_refresh(|state| state.refresh_for(payload)) {
                    with_state(|state| {
                        state.rollback_speculation(speculation);
                        state.dispose_owner(owner);
                    });
                    return Err(error);
                }
                Ok(MountedNode::For(payload))
            }
        }
    }

    fn release_nodes(&mut self, nodes: &mut Vec<MountedNode>) {
        with_state(|state| clear_history_in_nodes(state, nodes));
        let nodes = std::mem::take(nodes);
        for node in nodes {
            let owner = match node {
                // 文本节点没有自己的 owner，登记在父 owner 下；根挂载失败时
                // 父 owner 不会被释放，所以这里直接归还节点身份与绑定槽。
                MountedNode::Text(payload) => {
                    with_state(|state| {
                        let node = state.nodes.text(payload).node;
                        state.retire_node(node);
                    });
                    None
                }
                MountedNode::Element(id) => with_state(|state| Some(state.element_owner(id))),
                MountedNode::Component(id) => with_state(|state| Some(state.components[id].owner)),
                MountedNode::Show(payload) => {
                    with_state(|state| Some(state.nodes.show(payload).owner))
                }
                MountedNode::For(payload) => {
                    with_state(|state| Some(state.nodes.for_node(payload).owner))
                }
            };
            if let Some(owner) = owner {
                with_state(|state| {
                    state.dispose_owner(owner);
                    state.unmounts += 1;
                });
            }
        }
    }

    /// 一个 turn 的绑定刷新：结构触发先求值并提交，然后才求值文本/属性/样式。
    ///
    /// 这保证结构补丁先于值补丁；同一 `(tree, node, part)` 的补丁在
    /// `BindingPatchLog` 里去重一次。
    fn refresh_nodes(&mut self, nodes: &mut [MountedNode]) -> Result<(), MountError> {
        // 结构触发失败不回滚其他节点的值绑定：值阶段仍然执行，错误在最后
        // 报告。这样同一帧里已经可提交的属性（例如 disabled）不会丢失。
        let structure = self.refresh_structure_nodes(nodes);
        let values = self.refresh_value_nodes(nodes);
        structure.and(values)
    }

    /// 求值一个节点列表的结构触发。一个失败的分支/行只影响它自己：
    /// 后续兄弟仍在本轮求值，错误在遍历结束后统一报告。
    fn refresh_structure_nodes(&mut self, nodes: &mut [MountedNode]) -> Result<(), MountError> {
        let mut first_error = None;
        for node in nodes {
            let result = match node {
                MountedNode::Text(_) => Ok(()),
                MountedNode::Element(id) => {
                    let mut guard = ChildrenGuard::take(ChildrenOwner::Element(*id));
                    let result = self.refresh_structure_nodes(guard.children_mut());
                    guard.restore();
                    result
                }
                MountedNode::Component(id) => {
                    let mut guard = ChildrenGuard::take(ChildrenOwner::Component(*id));
                    let result = self.refresh_structure_nodes(guard.children_mut());
                    guard.restore();
                    result
                }
                MountedNode::Show(payload) => self.refresh_show(*payload),
                MountedNode::For(payload) => self.refresh_for(*payload),
            };
            if let Err(error) = result
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn refresh_value_nodes(&mut self, nodes: &mut [MountedNode]) -> Result<(), MountError> {
        for node in nodes {
            match node {
                MountedNode::Text(payload) => self.refresh_text(*payload)?,
                MountedNode::Element(id) => {
                    self.refresh_element(*id);
                    let mut guard = ChildrenGuard::take(ChildrenOwner::Element(*id));
                    let result = self.refresh_value_nodes(guard.children_mut());
                    guard.restore();
                    result?;
                }
                MountedNode::Component(id) => {
                    let mut guard = ChildrenGuard::take(ChildrenOwner::Component(*id));
                    let result = self.refresh_value_nodes(guard.children_mut());
                    guard.restore();
                    result?;
                }
                MountedNode::Show(payload) => self.refresh_show_values(*payload)?,
                MountedNode::For(payload) => self.refresh_for_values(*payload)?,
            }
        }
        Ok(())
    }

    /// 求值一个文本槽；`Static`/`Once` 在挂载时已提交，不再重算。
    fn refresh_text(&mut self, payload: u32) -> Result<(), MountError> {
        let (binding, source, tree, node) = with_state(|state| {
            let data = state.nodes.text(payload);
            let table = state
                .nodes
                .binding_table(data.tree)
                .expect("a mounted text node has a binding table");
            let slot = table
                .slot::<String>(data.binding)
                .expect("a mounted text binding slot is live");
            (data.binding, slot.source, data.tree, data.node)
        });
        if source.is_static() {
            return Ok(());
        }
        let next = source.evaluate::<String>();
        with_state(|state| {
            let mut value_patch = None;
            let mut fault = None;
            {
                let table = state.nodes.binding_table_mut(tree);
                let Some(slot) = table.slot_mut::<String>(binding) else {
                    return;
                };
                match next {
                    ReadValue::Value(value) => {
                        let old = slot.current.clone();
                        if old.as_ref() != Some(&value) {
                            slot.current = Some(value.clone());
                            value_patch = Some((old, value));
                        }
                        slot.fault = None;
                    }
                    ReadValue::Fault(next_fault) => {
                        // 读取失败保留最后已提交文本；没有最后值时呈现为空串。
                        slot.fault = Some(next_fault.clone());
                        fault = Some(next_fault);
                    }
                }
            }
            if let Some((old, value)) = value_patch {
                state.record_binding_patch(tree, node, BindingPart::Text, old, Some(value), None);
                state.mark_building_tree_changed();
            }
            if let Some(fault) = fault {
                state.record_read_fault(fault);
            }
        });
        Ok(())
    }

    /// 求值 Show 活动分支里的值绑定；结构阶段已经决定分支。
    fn refresh_show_values(&mut self, payload: u32) -> Result<(), MountError> {
        let data = with_state(|state| state.nodes.take_show(payload));
        let mut guard = ShowPayloadGuard {
            payload,
            data: Some(data),
        };
        let round = with_state(|state| state.refresh_round);
        let show = guard.data_mut();
        // 结构阶段刚预刷新过这个分支时，值阶段不再把同一批 binding 求值一次。
        if show.values_round == round {
            return Ok(());
        }
        if let Some(branch) = &mut show.active {
            self.refresh_value_nodes(&mut branch.children)?;
        }
        show.values_round = round;
        Ok(())
    }

    /// 求值已挂载行的值绑定；结构阶段已经完成行的挂载/卸载。
    fn refresh_for_values(&mut self, payload: u32) -> Result<(), MountError> {
        let data = with_state(|state| state.nodes.take_for(payload));
        let mut guard = ForPayloadGuard {
            payload,
            data: Some(data),
        };
        let data = guard.data_mut();
        for key in &data.order {
            if let Some(item) = data.items.get_mut(key) {
                self.refresh_value_nodes(&mut item.children)?;
            }
        }
        Ok(())
    }

    fn refresh_element(&mut self, id: usize) {
        // 属性求值会读取用户 memo；panic 时 guard 把旧 attrs/styles 放回元素，
        // 已提交界面因此保持完整。
        let mut scratch = ElementScratchGuard::take(id);
        let Some(tree) = with_state(|state| state.element_tree(id)) else {
            return;
        };
        let node = with_state(|state| state.elements[id].id.node());

        // 先收集槽来源与已提交值；求值用户闭包期间不持有 State 借用。
        // 没有绑定（也没有 attribute/style）的树没有绑定表，直接跳过求值。
        let attr_plans: Vec<AttrPlan> = with_state(|state| {
            let Some(table) = state.nodes.binding_table(tree) else {
                return Vec::new();
            };
            scratch
                .attrs()
                .iter()
                .map(|attr| AttrPlan::describe(table, attr.name, attr.binding))
                .collect()
        });
        let attr_values: Vec<Option<ReadValue<AttrCurrent>>> =
            attr_plans.iter().map(AttrPlan::evaluate).collect();
        let style_plans: Vec<StylePlan> = with_state(|state| {
            let Some(table) = state.nodes.binding_table(tree) else {
                return Vec::new();
            };
            scratch
                .styles()
                .iter()
                .map(|style| StylePlan::describe(table, style.property, style.binding))
                .collect()
        });
        let style_values: Vec<Option<ReadValue<StyleValue>>> =
            style_plans.iter().map(StylePlan::evaluate).collect();

        let mut attrs_changed = false;
        let mut styles_changed = false;
        let mut faults = Vec::new();
        with_state(|state| {
            for (plan, next) in attr_plans.iter().zip(attr_values) {
                match next {
                    // 静态/once 值在挂载时已写入，永不重算。
                    None => {}
                    Some(ReadValue::Value(next)) => {
                        attrs_changed |= plan.commit_value(state, tree, node, next);
                    }
                    Some(ReadValue::Fault(fault)) => {
                        // 失败不替换 current：属性保持最后已提交值，首次失败则缺席。
                        plan.mark_fault(state, tree, fault.clone());
                        faults.push(fault);
                    }
                }
            }
            for (plan, next) in style_plans.iter().zip(style_values) {
                match next {
                    None => {}
                    Some(ReadValue::Value(next)) => {
                        styles_changed |= plan.commit_value(state, tree, node, next);
                    }
                    Some(ReadValue::Fault(fault)) => {
                        plan.mark_fault(state, tree, fault.clone());
                        faults.push(fault);
                    }
                }
            }
            for fault in faults {
                state.record_read_fault(fault);
            }
        });

        // 属性列表在 scratch 里，已提交值仍从绑定槽读取；文本编辑器据此
        // 辨认模型写入。
        let value_binding = scratch
            .attrs()
            .iter()
            .find(|attr| attr.name == "value")
            .map(|attr| attr.binding);
        let (committed, epoch) = match value_binding {
            Some(binding) => with_state(|state| {
                let Some(table) = state.element_binding_table(id) else {
                    return (String::new(), None);
                };
                let committed = table.text_current(binding).unwrap_or_default().to_owned();
                // 只有文本 `value` 绑定有编辑器写入代数；bool 绑定不参与结算。
                let epoch = if table.is_text(binding) {
                    table.read_slot(binding).and_then(|slot| slot.epoch(state))
                } else {
                    None
                };
                (committed, epoch)
            }),
            None => (String::new(), None),
        };
        with_state(|state| {
            let attrs = scratch.take_attrs();
            let styles = scratch.take_styles();
            let mut local_changed = false;
            // 已提交的 `checked` 绑定先读出来，避免同时借用本地编辑状态。
            let bound = attrs
                .iter()
                .find(|attr| attr.name == "checked")
                .and_then(|attr| {
                    state
                        .element_binding_table(id)
                        .and_then(|table| table.bool_current(attr.binding))
                });
            let event = {
                if let LocalState::Checkbox { checked } = state.element_local_mut(id)
                    && let Some(value) = bound
                    && *checked != value
                {
                    *checked = value;
                    local_changed = true;
                }
                *state.element_attrs_mut(id) = attrs;
                *state.element_inline_styles_mut(id) = styles;
                match state.element_local_mut(id) {
                    LocalState::TextInput(editor) => editor.observe(&committed, epoch),
                    _ => None,
                }
            };
            if let Some(event) = event {
                // stale settlement 与未归一化的 settlement 不改变屏幕上的文本。
                local_changed |= matches!(
                    event,
                    TextTrace::Settlement {
                        normalized: true,
                        ..
                    } | TextTrace::ExternalReset { .. }
                        | TextTrace::CompositionCancelledByReset { .. }
                );
                state.push_text_trace(event);
            }
            if attrs_changed || styles_changed || local_changed {
                state.mark_element_changed(id);
            }
        });
    }

    fn refresh_show(&mut self, payload: u32) -> Result<(), MountError> {
        let data = with_state(|state| state.nodes.take_show(payload));
        let mut guard = ShowPayloadGuard {
            payload,
            data: Some(data),
        };
        let show = guard.data_mut();
        let round = with_state(|state| state.refresh_round);
        // 同一触发每轮最多求值一次；本轮挂载的行/分支不会在尾随结构遍历里重跑。
        if show.structure_round == round {
            return Ok(());
        }
        show.structure_round = round;
        let node = show.node;
        let binding = show.when;
        let tree = with_state(|state| state.nodes.tree_of(node))
            .expect("a mounted Show node belongs to a tree");
        let (source, committed) = with_state(|state| {
            let table = state
                .nodes
                .binding_table(tree)
                .expect("a mounted Show node has a binding table");
            let slot = table
                .slot::<bool>(binding)
                .expect("a mounted Show binding slot is live");
            (slot.source, slot.current)
        });
        // `Static`/`Once` 在挂载时提交；动态源失败时保持当前分支。
        let active = if source.is_static() {
            let Some(active) = committed else {
                return Ok(());
            };
            active
        } else {
            match source.evaluate::<bool>() {
                ReadValue::Value(value) => {
                    let changed = committed != Some(value);
                    with_state(|state| {
                        if let Some(slot) = state
                            .nodes
                            .binding_table_mut(tree)
                            .slot_mut::<bool>(binding)
                        {
                            slot.fault = None;
                            if changed {
                                slot.current = Some(value);
                            }
                        }
                        if changed {
                            state.record_binding_patch(
                                tree,
                                node,
                                BindingPart::ShowWhen,
                                committed,
                                Some(value),
                                None,
                            );
                        }
                    });
                    value
                }
                ReadValue::Fault(fault) => {
                    // 读取失败不伪造 false：保持当前分支，并报告 typed fault。
                    // 分支里的结构触发仍要在这一帧继续向下求值。
                    with_state(|state| {
                        if let Some(slot) = state
                            .nodes
                            .binding_table_mut(tree)
                            .slot_mut::<bool>(binding)
                        {
                            slot.fault = Some(fault.clone());
                        }
                        state.record_read_fault(fault);
                    });
                    if let Some(branch) = &mut show.active {
                        self.refresh_structure_nodes(&mut branch.children)?;
                    }
                    return Ok(());
                }
            }
        };
        let wanted = if active {
            Some(true)
        } else if show.fallback.is_some() {
            Some(false)
        } else {
            None
        };
        let branch_changed = show.active.as_ref().map(|branch| branch.which) != wanted;
        if !branch_changed {
            // 分支保持挂载时，嵌套的 Show/For 触发仍属于结构阶段。
            if let Some(branch) = &mut show.active {
                self.refresh_structure_nodes(&mut branch.children)?;
            }
            return Ok(());
        }

        // 拟议分支的预挂载/预刷新会写补丁；被拒绝的分支必须把补丁和
        // dirty 标记回滚到拟议之前，快照与 revision 都保持原样。
        let speculation = with_state(|state| state.mark_speculation());
        let replacement = match wanted {
            Some(which) => {
                let owner = with_state(|state| state.create_owner(Some(show.owner)));
                let factory = if which {
                    &show.then
                } else {
                    show.fallback.as_ref().expect("fallback is present")
                };
                let plan = match with_owner(owner, || catch_panic(|| factory.mount())) {
                    Ok(Ok(plan)) => plan,
                    Ok(Err(error)) => {
                        with_state(|state| {
                            state.dispose_owner(owner);
                            state.rollback_speculation(speculation);
                        });
                        return self.reject_show_branch(show, error);
                    }
                    Err(panic) => {
                        with_state(|state| {
                            state.dispose_owner(owner);
                            state.rollback_speculation(speculation);
                        });
                        return self.reject_show_branch(show, MountError::Panic(panic));
                    }
                };
                let children = match self.mount_plan(plan, owner, Some(show.node)) {
                    Ok(children) => children,
                    Err(error) => {
                        with_state(|state| {
                            state.dispose_owner(owner);
                            state.rollback_speculation(speculation);
                        });
                        return self.reject_show_branch(show, error);
                    }
                };
                Some(MountedBranch {
                    which,
                    owner,
                    children,
                })
            }
            None => None,
        };

        // 先刷新拟议分支；失败时释放它并保留旧分支，结构提交是最后一步。
        let mut replacement = replacement;
        if let Some(branch) = &mut replacement {
            let result = catch_panic(std::panic::AssertUnwindSafe(|| {
                self.refresh_nodes(&mut branch.children)
            }));
            let failure = match result {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error),
                Err(panic) => Some(MountError::Panic(panic)),
            };
            if let Some(error) = failure {
                with_state(|state| {
                    clear_history_in_nodes(state, &branch.children);
                    state.dispose_owner(branch.owner);
                    state.rollback_speculation(speculation);
                    state.unmounts += 1;
                });
                return self.reject_show_branch(show, error);
            }
        }

        if let Some(branch) = &show.active {
            with_state(|state| clear_history_in_nodes(state, &branch.children));
        }
        if let Some(branch) = show.active.take() {
            with_state(|state| {
                state.dispose_owner(branch.owner);
                state.unmounts += 1;
            });
        }
        // 拟议分支的预刷新已经提交了分支里的值；值阶段不再重跑这一轮。
        if replacement.is_some() {
            show.values_round = round;
        }
        show.active = replacement;
        if branch_changed {
            with_state(State::mark_building_tree_changed);
        }
        Ok(())
    }

    /// 拟议分支被拒绝后的收尾：保留的旧分支仍是已提交结构，它里面的
    /// Show/For 触发仍要在这一帧继续求值；拟议分支的错误保持为第一个
    /// 错误，保留分支的错误留到下一轮重新报告。
    fn reject_show_branch(
        &mut self,
        show: &mut ShowData,
        error: MountError,
    ) -> Result<(), MountError> {
        if let Some(branch) = &mut show.active {
            let _ = self.refresh_structure_nodes(&mut branch.children);
        }
        Err(error)
    }

    fn refresh_for(&mut self, payload: u32) -> Result<(), MountError> {
        let data = with_state(|state| state.nodes.take_for(payload));
        let mut guard = ForPayloadGuard {
            payload,
            data: Some(data),
        };
        let for_node = guard.data_mut();
        let round = with_state(|state| state.refresh_round);
        // 同一触发每轮最多求值一次；本轮挂载的行不会在尾随结构遍历里重跑。
        if for_node.structure_round == round {
            return Ok(());
        }
        for_node.structure_round = round;
        let node = for_node.node;
        let tree = with_state(|state| state.nodes.tree_of(node))
            .expect("a mounted For node belongs to a tree");
        // 1) 求值行集来源。源失败时保留最后一次提交的行；首次失败视为空列表。
        let mut source_error = None;
        let rows: Option<RowSet> = match for_node.rows {
            MountedForRows::Read(binding) => {
                let (source, committed) = with_state(|state| {
                    let table = state
                        .nodes
                        .binding_table(tree)
                        .expect("a mounted For node has a binding table");
                    let slot = table
                        .slot::<RowSet>(binding)
                        .expect("a mounted For binding slot is live");
                    (slot.source, slot.current.clone())
                });
                if source.is_static() {
                    committed
                } else {
                    match source.evaluate::<RowSet>() {
                        ReadValue::Value(next) => {
                            let changed = committed.as_ref() != Some(&next);
                            with_state(|state| {
                                if let Some(slot) = state
                                    .nodes
                                    .binding_table_mut(tree)
                                    .slot_mut::<RowSet>(binding)
                                {
                                    slot.fault = None;
                                    if changed {
                                        slot.current = Some(next.clone());
                                    }
                                }
                                if changed {
                                    state.record_binding_patch(
                                        tree,
                                        node,
                                        BindingPart::ForEach,
                                        committed.clone(),
                                        Some(next.clone()),
                                        None,
                                    );
                                }
                            });
                            Some(next)
                        }
                        ReadValue::Fault(fault) => {
                            with_state(|state| {
                                if let Some(slot) = state
                                    .nodes
                                    .binding_table_mut(tree)
                                    .slot_mut::<RowSet>(binding)
                                {
                                    slot.fault = Some(fault.clone());
                                }
                                state.record_read_fault(fault);
                            });
                            // 槽位可能已写入一个被拒绝的拟议投影；源失败时回到
                            // 最后一次真正提交的行集，而不是那个拟议值。
                            for_node.committed_rows.clone()
                        }
                    }
                }
            }
            MountedForRows::Projection(filter) => match for_node.driver.projection_rowset(filter) {
                Ok(next) => Some(next),
                Err(error) => {
                    source_error = Some(MountError::Store(error));
                    for_node.committed_rows.clone()
                }
            },
        };
        if let Some(rows) = &rows
            && rows.store() != for_node.store
        {
            let error = MountError::WrongStore {
                expected: for_node.store,
                found: rows.store(),
            };
            let _ = self.refresh_committed_for_rows(for_node);
            return Err(error);
        }
        if let Some(error) = source_error {
            let _ = self.refresh_committed_for_rows(for_node);
            return Err(error);
        }
        // 2) 投影变化时用驱动快照重建键映射：重复键在任何行挂载之前拒绝。
        // 同一份投影（含等价的独立 Arc）不重复运行用户 key 选择器。
        let projection_changed = match (&rows, &for_node.committed_rows) {
            (Some(next), Some(committed)) => next != committed,
            (Some(_), None) | (None, Some(_)) => true,
            (None, None) => false,
        };
        let driver = Rc::clone(&for_node.driver);
        let mut projection_failure = None;
        if projection_changed && let Some(next) = &rows {
            let mut buckets: BTreeMap<u64, Vec<RowId>> = BTreeMap::new();
            for id in next.ids().iter().copied() {
                // key 选择器是用户代码；panic 时不改写已提交的行映射。
                let hash = match catch_panic(|| driver.key_hash(id)) {
                    Ok(Some(hash)) => hash,
                    // 已删除的行不再挂载；遗留身份的完整策略属于行身份工作。
                    Ok(None) => continue,
                    Err(panic) => {
                        projection_failure = Some(MountError::Panic(panic));
                        break;
                    }
                };
                let mut panic = None;
                let duplicate = buckets.get(&hash).is_some_and(|bucket| {
                    bucket
                        .iter()
                        .any(|other| match catch_panic(|| driver.key_eq(*other, id)) {
                            Ok(equal) => equal,
                            Err(caught) => {
                                panic = Some(caught);
                                true
                            }
                        })
                });
                if let Some(panic) = panic {
                    projection_failure = Some(MountError::Panic(panic));
                    break;
                }
                if duplicate {
                    projection_failure = Some(MountError::DuplicateKey);
                    break;
                }
                buckets.entry(hash).or_default().push(id);
            }
        }
        if let Some(error) = projection_failure {
            // 拟议行集被拒绝：已提交的行映射、窗口与 spacer 保持原样。
            // 保留行不是拟议结构，它们里面的 Show/For 触发仍要在这一帧
            // 继续求值；拒绝本身的错误保持为第一个错误。
            let _ = self.refresh_committed_for_rows(for_node);
            return Err(error);
        }
        // total 来自行集快照本身；静态快照里已删除的行不挂载，但仍计入声明范围，
        // 窗口因此保留一个空槽而不是报错或回退。
        let total = rows.as_ref().map_or(0, RowSet::len);
        let previous_order = for_node.order.clone();
        let previous_first = for_node.first_row;
        let previous_total = for_node.total_rows;
        let had_spacers = for_node.spacers.is_some();

        let mut first = 0;
        let mut count = total;
        if for_node.row_height.is_some() {
            if for_node.spacers.is_none() {
                let spacers =
                    with_state(|state| ensure_spacers(state, for_node.owner, Some(for_node.node)));
                for_node.spacers = Some(spacers);
            }
            let requested = with_state(|state| state.row_windows.get(&for_node.band).copied())
                .unwrap_or(RowWindow {
                    first: 0,
                    count: DEFAULT_WINDOW_ROWS,
                });
            first = requested.first.min(total);
            count = requested.count.min(total - first);
            // 聚焦行在窗口外一行以内时扩大窗口，保留它的节点、caret 与组合；
            // 更远则由宿主在应用新窗口前完成 blur 与组合取消。只在已挂载行
            // 里查找焦点所属行，因此刷新成本与总行数无关。
            let focused = with_state(|state| {
                state.refresh_focus.and_then(|focused| {
                    for_node
                        .items
                        .iter()
                        .find(|(_, item)| contains_element(state, &item.children, focused))
                        .map(|(id, _)| *id)
                })
            });
            if let Some(row) = focused {
                // 投影变化时按新顺序定位；未变化时用已提交行下标，保持 O(窗口)。
                let index = if projection_changed {
                    rows.as_ref()
                        .and_then(|rows| rows.ids().iter().position(|id| *id == row))
                } else {
                    for_node.items.get(&row).map(|item| item.row_index)
                };
                if let Some(index) = index {
                    if index + 1 == first {
                        first = index;
                        count += 1;
                    } else if index == first + count {
                        count += 1;
                    }
                    count = count.min(total - first);
                    if index < first || index >= first + count {
                        with_state(|state| state.window_focus_dropped = state.refresh_focus);
                    }
                }
            }
        }

        let mut order = Vec::with_capacity(count);
        let mut fresh = Vec::new();
        let mut row_indices = Vec::new();
        // 预挂载的 fresh 行会写入补丁和 dirty 标记；行挂载失败并释放时
        // 一起回滚，未提交的行不影响 revision。
        let speculation = with_state(|state| state.mark_speculation());
        let mut mount_failure = None;
        if let Some(rows) = &rows {
            for index in first..first + count {
                let id = rows.ids()[index];
                // 静态快照可能早于删除：已删除的行不再挂载，也不进入已提交顺序，
                // 已挂载的旧项随普通卸载路径移除。遗留身份的完整策略属于行身份工作。
                if !driver.row_live(id) {
                    continue;
                }
                order.push(id);
                if for_node.items.contains_key(&id) {
                    row_indices.push((id, index));
                    continue;
                }
                let owner = with_state(|state| state.create_owner(Some(for_node.owner)));
                let snapshot = match catch_panic(|| driver.row_snapshot(id)) {
                    Ok(Some(snapshot)) => snapshot,
                    Ok(None) => {
                        with_state(|state| state.dispose_owner(owner));
                        order.pop();
                        continue;
                    }
                    Err(panic) => {
                        with_state(|state| state.dispose_owner(owner));
                        mount_failure = Some(MountError::Panic(panic));
                        break;
                    }
                };
                let todo = with_state(|state| {
                    if for_node.store == state.todo_store.id() {
                        state.todo_store.key_of(id)
                    } else {
                        None
                    }
                });
                let erased = ErasedRowView {
                    id,
                    store: ErasedStore {
                        store: for_node.store,
                    },
                    row: snapshot,
                };
                let plan = match with_owner(owner, || catch_panic(|| driver.mount_row(erased))) {
                    Ok(Ok(plan)) => plan,
                    Ok(Err(error)) => {
                        with_state(|state| state.dispose_owner(owner));
                        mount_failure = Some(MountError::FactoryFailed(Box::new(error)));
                        break;
                    }
                    Err(panic) => {
                        with_state(|state| state.dispose_owner(owner));
                        mount_failure = Some(MountError::FactoryPanicked(panic));
                        break;
                    }
                };
                let children = match self.mount_plan(plan, owner, Some(for_node.node)) {
                    Ok(children) => children,
                    Err(error) => {
                        with_state(|state| state.dispose_owner(owner));
                        mount_failure = Some(error);
                        break;
                    }
                };
                let instance = with_state(|state| {
                    let instance = state.next_instance;
                    state.next_instance += 1;
                    instance
                });
                fresh.push((
                    id,
                    MountedItem {
                        owner,
                        row_index: index,
                        todo,
                        instance,
                        children,
                    },
                ));
            }
        }
        if let Some(error) = mount_failure {
            // 拟议窗口没有提交：丢弃预挂载行和它们的补丁/dirty 标记。
            with_state(|state| state.rollback_speculation(speculation));
            self.release_items(&mut fresh);
            // 保留行不是拟议结构：它们里面的结构触发仍要在这一帧继续求值。
            let _ = self.refresh_committed_for_rows(for_node);
            return Err(error);
        }

        let keep: BTreeSet<RowId> = order.iter().copied().collect();
        let removed: Vec<RowId> = for_node
            .items
            .keys()
            .filter(|id| !keep.contains(id))
            .copied()
            .collect();
        for id in removed {
            if let Some(item) = for_node.items.remove(&id) {
                with_state(|state| clear_history_in_nodes(state, &item.children));
                with_state(|state| {
                    state.dispose_owner(item.owner);
                    state.unmounts += 1;
                });
            }
        }
        for (id, item) in fresh {
            for_node.items.insert(id, item);
        }
        for (id, index) in row_indices {
            if let Some(item) = for_node.items.get_mut(&id) {
                item.row_index = index;
            }
        }
        for_node.order = order;
        // 投影只在窗口挂载成功后提交：失败帧不会留下新顺序与旧行下标的分叉。
        for_node.committed_rows = rows;
        let structural_changed = previous_order != for_node.order
            || previous_first != first
            || previous_total != total
            || (!had_spacers && for_node.spacers.is_some());
        if structural_changed {
            with_state(State::mark_building_tree_changed);
        }
        // 行内嵌套的 Show/For 触发仍属于结构阶段；值绑定由值阶段求值，
        // 保证结构补丁先于值补丁。一行失败不影响后续行，错误在最后报告。
        let first_error = self.refresh_committed_for_rows(for_node);
        if let (Some(row_height), Some((top, bottom))) = (for_node.row_height, for_node.spacers) {
            let mounted = for_node.order.len().min(total - first);
            with_state(|state| {
                set_spacer_height(state, top, first, row_height);
                set_spacer_height(state, bottom, total - first - mounted, row_height);
            });
        }
        for_node.total_rows = total;
        for_node.first_row = first;
        for_node.window_count = count;
        if for_node.row_height.is_some() {
            // 本批请求的窗口已经消费并提交；失败的 turn 据此决定是否需要
            // 修复刷新。只有批量 turn 内的刷新才记录，普通刷新不留下标记。
            let band = for_node.band;
            with_state(|state| {
                if let Some(batch) = &mut state.row_window_batch
                    && batch.bands.contains(&band)
                {
                    batch.applied.insert(band);
                }
            });
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// 继续求值已提交行里的嵌套结构触发，返回第一个错误。
    ///
    /// 拟议窗口被拒绝时，已提交的行不是拟议结构：它们不能随拟议一起冻结，
    /// 否则失败行仍在行集里的每一帧都会跳过这些行里的 Show/For 触发。
    fn refresh_committed_for_rows(&mut self, for_node: &mut ForData) -> Option<MountError> {
        let mut first_error = None;
        for key in &for_node.order {
            if let Some(item) = for_node.items.get_mut(key)
                && let Err(error) = self.refresh_structure_nodes(&mut item.children)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error
    }

    fn release_items(&mut self, items: &mut Vec<(RowId, MountedItem)>) {
        for (_, item) in items.drain(..) {
            with_state(|state| {
                state.dispose_owner(item.owner);
                state.unmounts += 1;
            });
        }
    }

    /// 指针点击：解析到元素后走 closest-wins 路由，并返回 turn 报告。
    pub fn click_text(&mut self, text: &str) -> Result<TurnReport, MountError> {
        self.dispatch_label(
            FindTarget::Text(text.to_owned()),
            "click target not found",
            "click target not found",
            Self::activate_node,
        )
    }

    /// 指针点击：解析到元素后走 closest-wins 路由，并返回 turn 报告。
    pub fn click_label(&mut self, label: &str) -> Result<TurnReport, MountError> {
        self.dispatch_label(
            FindTarget::Label(label.to_owned()),
            "click target not found",
            "click target not found",
            Self::activate_node,
        )
    }

    /// 已解析目标的直接 click：只运行该元素自己的 handler。
    pub fn click_node(&mut self, node: NodeId) -> Result<TurnReport, DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            let tree = with_state(|state| state.tree_for_element(element));
            let reveal_before = tree.map(|tree| with_state(|state| state.reveal_probe(tree)));
            let (event_id, event) =
                self.take_node_event(node, element, EventKind::Click, NodeOperation::Click)?;
            let outcome = invoke_click_event(event_id, event);
            with_state(|state| state.phase = Phase::Idle);
            let mut report = TurnReport::default();
            match outcome {
                Ok(HandlerOutcome::Ok) => {}
                Ok(HandlerOutcome::Err(error)) => {
                    report.push_error(TurnError::HandlerFailed(error));
                }
                Err(panic) => report.push_error(TurnError::HandlerPanicked(panic)),
            }
            self.finish_command().map_err(DispatchError::Command)?;
            finish_reveal(&mut report, tree, reveal_before.as_ref());
            Ok(report)
        })
    }

    /// 路由后的激活：先运行最近 click handler，再执行 `type=submit` 表单默认。
    fn activate_element(&mut self, element: usize) -> Result<TurnReport, DispatchError> {
        let node = with_state(|state| state.elements[element].id);
        let tree = with_state(|state| state.tree_for_element(element));
        let before = tree.map(|tree| with_state(|state| state.reveal_probe(tree)));
        let has_click = with_state(|state| state.has_click_handler(element));
        let mut report = if has_click {
            self.click_node(node)?
        } else {
            TurnReport::default()
        };
        if report.is_ok()
            && let Some(element) = with_state(|state| state.element_for_node(node).ok())
        {
            self.apply_click_default(element, &mut report)?;
        }
        finish_reveal(&mut report, tree, before.as_ref());
        Ok(report)
    }

    /// `type=submit` 按钮的 click 默认：提交最近 form 一次。
    fn apply_click_default(
        &mut self,
        element: usize,
        report: &mut TurnReport,
    ) -> Result<(), DispatchError> {
        if !with_state(|state| state.is_submit_button(element)) {
            return Ok(());
        }
        let Some(form) = with_state(|state| state.ancestor_form(element)) else {
            return Ok(());
        };
        if !with_state(|state| state.has_event(form, EventKind::Submit)) {
            return Ok(());
        }
        let form = with_state(|state| state.elements[form].id);
        let submitted = self.submit_node(form)?;
        report.absorb(submitted);
        Ok(())
    }

    /// 路由一个 typed Intent；窗口/activation 目标先校验再询问处理器。
    pub fn dispatch_intent(&mut self, intent: Intent) -> Result<TurnReport, DispatchError> {
        match intent.payload() {
            IntentPayload::KeyDown { key, modifiers } => {
                return self.dispatch_keydown_intent(intent.target(), key, modifiers);
            }
            IntentPayload::Press { caret } => {
                return self.dispatch_press_intent(intent.target(), caret);
            }
            IntentPayload::Click | IntentPayload::Activate => {}
        }
        match intent.target() {
            EventTarget::Node(node) => {
                let state = self.state.clone();
                context::install(&state, || {
                    with_state(|state| state.element_for_node(node))?;
                    let target = with_state(|state| state.closest_click_target(node))?;
                    match target {
                        Some(element) => self.activate_element(element),
                        None => Ok(TurnReport::default()),
                    }
                })
            }
            EventTarget::Window(window) => {
                if !self.state.borrow().intent_window_usable(window) {
                    return Err(DispatchError::TargetClosed);
                }
                Ok(TurnReport::default())
            }
            EventTarget::App => Ok(TurnReport::default()),
        }
    }

    /// 路由原语：等价于派发一个指针 Click Intent。
    pub fn activate_node(&mut self, node: NodeId) -> Result<TurnReport, DispatchError> {
        self.dispatch_intent(Intent::click(node))
    }

    /// Press turn：最近可聚焦祖先获得焦点，text-input 按宿主偏移放置 caret。
    ///
    /// 禁用目标（含禁用祖先）不改变焦点；隐藏或挂起窗口由目标校验拒绝。
    fn dispatch_press_intent(
        &mut self,
        target: EventTarget,
        caret: Option<usize>,
    ) -> Result<TurnReport, DispatchError> {
        let EventTarget::Node(node) = target else {
            return Ok(TurnReport::default());
        };
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            let path = with_state(|state| state.element_path(element));
            if path
                .iter()
                .any(|element| with_state(|state| state.element_is_disabled(*element)))
            {
                return Ok(TurnReport::default());
            }
            let focusable = path
                .iter()
                .rev()
                .copied()
                .find(|element| with_state(|state| state.is_focusable(*element)));
            let focus_node =
                focusable.map(|element| with_state(|state| state.elements[element].id));
            let tree = with_state(|state| state.tree_for_element(element))
                .ok_or(DispatchError::StaleNode(node))?;
            let before = with_state(|state| state.reveal_probe(tree));
            let mut report = TurnReport::default();
            if with_state(|state| state.focused_in(tree)) != focusable {
                self.move_focus(tree, focusable)
                    .map_err(DispatchError::Command)?;
                report.mark_changed();
            }
            // focus handler 可能写入并重挂载；caret 只落在 flush 后仍然聚焦、
            // 仍是同一个 text-input 的目标上。
            if let (Some(focus_node), Some(caret)) = (focus_node, caret)
                && let Some(focusable) = with_state(|state| state.element_for_node(focus_node).ok())
                && with_state(|state| state.focused_in(tree)) == Some(focusable)
                && with_state(|state| state.element_tag(focusable)) == "text-input"
            {
                let length = with_state(|state| text_length(state.element_local(focusable)));
                let offset = caret.min(length);
                let changed = with_state(|state| {
                    let LocalState::TextInput(editor) = state.element_local_mut(focusable) else {
                        return false;
                    };
                    if editor.is_composing() {
                        return false;
                    }
                    let changed = editor.caret != offset
                        || editor.anchor != offset
                        || editor.selection != Some((offset, offset));
                    editor.caret = offset;
                    editor.anchor = offset;
                    editor.selection = Some((offset, offset));
                    editor.history.break_run();
                    if changed {
                        state.mark_element_changed(focusable);
                    }
                    changed
                });
                if changed {
                    if let Some(key) = with_state(|state| state.tree_for_element(focusable)) {
                        with_state(|state| state.commit_tree_revision(key));
                    }
                    report.mark_changed();
                }
            }
            self.finish_command().map_err(DispatchError::Command)?;
            finish_reveal(&mut report, Some(tree), Some(&before));
            Ok(report)
        })
    }

    /// 一个 keydown turn：capture → closest 普通 handler → bubble → L4 默认。
    fn dispatch_keydown_intent(
        &mut self,
        target: EventTarget,
        key: Key,
        modifiers: Modifiers,
    ) -> Result<TurnReport, DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let tree = match target {
                EventTarget::Node(node) => with_state(|state| {
                    state
                        .element_for_node(node)
                        .ok()
                        .and_then(|element| state.tree_for_element(element))
                }),
                EventTarget::Window(window) => Some(TreeSlot::Window(window.key)),
                EventTarget::App => None,
            };
            let before = tree.map(|tree| with_state(|state| state.reveal_probe(tree)));
            let mut report = match target {
                EventTarget::Node(node) => {
                    let element = with_state(|state| state.element_for_node(node))?;
                    if with_state(|state| state.element_local(element).is_composing()) {
                        self.keydown_node(node, key)?
                    } else {
                        let window = with_state(|state| {
                            state.tree_for_element(element).and_then(|tree| match tree {
                                TreeSlot::Window(key) => state.window_ctx(key),
                                TreeSlot::Root => None,
                            })
                        });
                        self.route_keydown(Some(element), window, key, modifiers)?
                    }
                }
                EventTarget::Window(window) => {
                    if !with_state(|state| state.intent_window_usable(window)) {
                        return Err(DispatchError::TargetClosed);
                    }
                    let focused =
                        with_state(|state| state.presented_focus_in(TreeSlot::Window(window.key)));
                    self.route_keydown(focused, Some(window), key, modifiers)?
                }
                EventTarget::App => TurnReport::default(),
            };
            finish_reveal(&mut report, tree, before.as_ref());
            Ok(report)
        })
    }

    fn route_keydown(
        &mut self,
        hit: Option<usize>,
        window: Option<WindowCtx>,
        key: Key,
        modifiers: Modifiers,
    ) -> Result<TurnReport, DispatchError> {
        let path = hit
            .map(|element| with_state(|state| state.element_path(element)))
            .unwrap_or_default();
        self.enter_event_phase();
        let mut key_event = KeyEvent::new(key);
        let mut report = TurnReport::default();
        let mut stopped = false;

        for element in &path {
            if let Some(stop) = self.run_key_event(
                *element,
                EventKind::KeyDownCapture,
                &mut key_event,
                &mut report,
            )? && stop
            {
                stopped = true;
                break;
            }
        }
        let mut ran_normal = false;
        if !stopped {
            for element in path.iter().rev() {
                if let Some(stop) =
                    self.run_key_event(*element, EventKind::KeyDown, &mut key_event, &mut report)?
                {
                    ran_normal = true;
                    stopped = stop;
                    break;
                }
            }
        }
        if !stopped
            && !ran_normal
            && let Some(window) = window
            && let Some(stop) = self.run_window_key_event(window, &mut key_event, &mut report)?
        {
            stopped = stop;
        }
        if !stopped {
            for element in path.iter().rev().skip(1) {
                if let Some(stop) = self.run_key_event(
                    *element,
                    EventKind::KeyDownBubble,
                    &mut key_event,
                    &mut report,
                )? && stop
                {
                    stopped = true;
                    break;
                }
            }
        }
        if !stopped && !key_event.is_consumed() && !key_event.prevents_default() {
            self.apply_key_default(hit, window, key, modifiers, &mut report)?;
        }
        report.set_outcome(key_event.is_consumed(), key_event.prevents_default());
        with_state(|state| state.phase = Phase::Idle);
        self.finish_command().map_err(DispatchError::Command)?;
        Ok(report)
    }

    /// 运行一个 keydown 槽；None 表示该阶段没有处理器。
    fn run_key_event(
        &mut self,
        element: usize,
        kind: EventKind,
        key_event: &mut KeyEvent,
        report: &mut TurnReport,
    ) -> Result<Option<bool>, DispatchError> {
        let Some((event_id, event)) = with_state(|state| state.take_element_event(element, kind))
        else {
            return Ok(None);
        };
        let result = invoke_event(event_id, event, |event| match event {
            Event::KeyDown(handler)
            | Event::KeyDownCapture(handler)
            | Event::KeyDownBubble(handler) => handler(key_event),
            _ => HandlerOutcome::Ok,
        });
        let mut stop = false;
        match result {
            Ok(HandlerOutcome::Ok) => {}
            Ok(HandlerOutcome::Err(error)) => {
                report.push_error(TurnError::HandlerFailed(error));
                stop = true;
            }
            Err(panic) => {
                report.push_error(TurnError::HandlerPanicked(panic));
                stop = true;
            }
        }
        Ok(Some(stop || key_event.is_consumed()))
    }

    fn run_window_key_event(
        &mut self,
        window: WindowCtx,
        key_event: &mut KeyEvent,
        report: &mut TurnReport,
    ) -> Result<Option<bool>, DispatchError> {
        let handler = self
            .state
            .borrow()
            .keydown_handlers
            .get(window.key)
            .cloned();
        let Some(handler) = handler else {
            return Ok(None);
        };
        let outcome = catch_panic(|| {
            let mut handler = handler.borrow_mut();
            handler(key_event)
        });
        let mut stop = false;
        match outcome {
            Ok(HandlerOutcome::Ok) => {}
            Ok(HandlerOutcome::Err(error)) => {
                report.push_error(TurnError::HandlerFailed(error));
                stop = true;
            }
            Err(panic) => {
                report.push_error(TurnError::HandlerPanicked(panic));
                stop = true;
            }
        }
        Ok(Some(stop || key_event.is_consumed()))
    }

    /// L4 控件默认：只在没有 consume/prevent_default 且没有 handler 错误时运行。
    fn apply_key_default(
        &mut self,
        hit: Option<usize>,
        window: Option<WindowCtx>,
        key: Key,
        modifiers: Modifiers,
        report: &mut TurnReport,
    ) -> Result<(), DispatchError> {
        match key {
            Key::Tab => {
                let Some(window) = window else {
                    return Ok(());
                };
                let result = if modifiers.shift() {
                    self.focus_previous_in(window)
                } else {
                    self.focus_next_in(window)
                };
                if result.is_ok() {
                    report.mark_changed();
                }
                result.map_err(|_| DispatchError::TargetClosed)?;
            }
            Key::Enter | Key::Space => {
                let Some(element) = hit else {
                    return Ok(());
                };
                let id = with_state(|state| state.elements[element].id);
                let tag = with_state(|state| state.element_tag(element));
                match (key, tag) {
                    (Key::Enter | Key::Space, "button") => {
                        if with_state(|state| {
                            state.has_click_handler(element) || state.is_submit_button(element)
                        }) {
                            let activated = self.activate_element(element)?;
                            report.absorb(activated);
                            report.mark_changed();
                        }
                    }
                    (Key::Space, "checkbox") => {
                        let next = with_state(|state| match state.element_local(element) {
                            LocalState::Checkbox { checked } => Some(!checked),
                            _ => None,
                        });
                        if let Some(next) = next {
                            let has_change = with_state(|state| {
                                state.element_events(element).iter().any(|event| {
                                    state
                                        .events
                                        .get(*event)
                                        .and_then(Option::as_ref)
                                        .is_some_and(|event| event_kind(event) == EventKind::Change)
                                })
                            });
                            if has_change {
                                let change = self.change_node(id, next)?;
                                report.absorb(change);
                            } else {
                                with_state(|state| {
                                    state.update_local(element, LocalUpdate::Change(next))
                                });
                            }
                            report.mark_changed();
                        }
                    }
                    (Key::Enter, "text-input") => {
                        let form = with_state(|state| state.ancestor_form(element));
                        if let Some(form) = form {
                            let form_id = with_state(|state| state.elements[form].id);
                            let has_submit =
                                with_state(|state| state.has_event(form, EventKind::Submit));
                            if has_submit {
                                let submitted = self.submit_node(form_id)?;
                                report.absorb(submitted);
                                report.mark_changed();
                            }
                        }
                    }
                    _ => {}
                }
            }
            Key::Backspace | Key::Delete => {
                let Some(element) = hit else {
                    return Ok(());
                };
                if with_state(|state| state.element_tag(element)) != "text-input"
                    || with_state(|state| state.element_is_disabled(element))
                {
                    return Ok(());
                }
                let id = with_state(|state| state.elements[element].id);
                if key == Key::Backspace {
                    self.backspace_node(id)?;
                } else {
                    self.delete_forward_node(id)?;
                }
                report.mark_changed();
            }
            Key::ArrowLeft
            | Key::ArrowRight
            | Key::Home
            | Key::End
            | Key::PageUp
            | Key::PageDown => {
                let Some(element) = hit else {
                    return Ok(());
                };
                if with_state(|state| state.element_tag(element)) != "text-input"
                    || with_state(|state| state.element_is_disabled(element))
                {
                    return Ok(());
                }
                let changed =
                    with_state(|state| state.move_text_caret(element, key, modifiers.shift()));
                if changed {
                    with_state(|state| state.mark_element_changed(element));
                    report.mark_changed();
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// 先取事件再进入 Event 阶段：被拒绝的派发不会把运行时留在 Event 阶段。
    fn take_node_event(
        &mut self,
        node: NodeId,
        element: usize,
        kind: EventKind,
        operation: NodeOperation,
    ) -> Result<(usize, Event), DispatchError> {
        let Some(taken) = with_state(|state| state.take_element_event(element, kind)) else {
            return Err(DispatchError::WrongKind { node, operation });
        };
        with_state(|state| {
            state.phase = Phase::Event;
            state.fault = None;
            state.write_error = None;
        });
        Ok(taken)
    }

    /// label 只是查找助手：解析到 Node ID 后与原生派发共用同一条路径。
    fn dispatch_label<T>(
        &mut self,
        target: FindTarget,
        not_found: &'static str,
        wrong_kind: &'static str,
        dispatch: impl FnOnce(&mut Self, NodeId) -> Result<T, DispatchError>,
    ) -> Result<T, MountError> {
        let Some(node) = self.label_node(&target) else {
            return Err(MountError::Component(not_found));
        };
        dispatch(self, node).map_err(|error| match error {
            DispatchError::UnknownNode(_) | DispatchError::StaleNode(_) => {
                MountError::Component(not_found)
            }
            DispatchError::Hidden(_) => MountError::Component(HIDDEN_TARGET),
            DispatchError::Suspended(_) => MountError::Component(SUSPENDED_TARGET),
            DispatchError::WrongKind { .. } => MountError::Component(wrong_kind),
            DispatchError::Rejected { reason, .. } => MountError::Component(reason),
            DispatchError::Command(error) => error,
            DispatchError::Panic(panic) => MountError::Panic(panic),
            DispatchError::TargetClosed => MountError::Component("target is closed"),
        })
    }

    fn label_node(&self, target: &FindTarget) -> Option<NodeId> {
        let state = self.state.borrow();
        let element = state.find_element(target)?;
        Some(state.elements[element].id)
    }

    /// label 助手的整体值替换；视为外部替换并清空该输入的撤销/重做历史。
    ///
    /// 它仍然派发一次 `on:input`，应用模型保持值的权威；handler 错误与 panic
    /// 进入返回的 turn 报告。
    pub fn input(&mut self, label: &str, value: &str) -> Result<TurnReport, MountError> {
        let target = FindTarget::Label(label.to_owned());
        let state = self.state.clone();
        context::install(&state, || {
            let Some(element) = with_state(|state| state.find_element(&target)) else {
                return Err(MountError::Component("input target not found"));
            };
            if !with_state(|state| state.element_is_visible(element)) {
                return Err(MountError::Component(HIDDEN_TARGET));
            }
            if !with_state(|state| state.element_accepts_input(element)) {
                return Err(MountError::Component(SUSPENDED_TARGET));
            }
            if with_state(|state| state.element_local(element).is_composing()) {
                return Err(MountError::Component("text input is composing"));
            }
            let Some((event_id, event)) =
                with_state(|state| state.take_element_event(element, EventKind::Input))
            else {
                return Err(MountError::Component("input target not found"));
            };
            with_state(|state| {
                state.phase = Phase::Event;
                state.fault = None;
                state.write_error = None;
                if let LocalState::TextInput(editor) = state.element_local_mut(element) {
                    editor.history.clear();
                }
                state.update_local(element, LocalUpdate::Input(value.to_owned()));
            });
            let outcome = invoke_event(event_id, event, |event| match event {
                Event::Input(handler) => handler(value.to_owned()),
                _ => HandlerOutcome::Ok,
            });
            let mut report = TurnReport::default();
            let panicked = record_handler_outcome(outcome, &mut report);
            with_state(|state| state.phase = Phase::Idle);
            if panicked {
                let _ = self.finish_command();
                return Ok(report);
            }
            self.finish_command()?;
            report.mark_changed();
            Ok(report)
        })
    }

    /// 在 caret/selection 处插入文本；一次编辑恰好派发一次 `on:input`。
    ///
    /// 连续的单字符插入合并为一个有界撤销步（ADR-0014）；多字符插入结束当前步。
    pub fn insert_text_node(
        &mut self,
        node: NodeId,
        text: &str,
    ) -> Result<TurnReport, DispatchError> {
        self.type_text_node(node, text)
    }

    /// 宿主可打印字符默认：执行一次插入编辑与一次 `on:input`，返回 turn 报告。
    ///
    /// 不形成 `Input` intent；headless 与 Win32 宿主共用这条路径。handler 的
    /// `Err` 或 panic 进入报告：`Err` 保留已提交的编辑，panic 撤销尚未结算的
    /// 本地编辑，并保持事件槽可复用。
    pub fn type_text_node(
        &mut self,
        node: NodeId,
        text: &str,
    ) -> Result<TurnReport, DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            self.apply_text_edit(node, element, NodeOperation::InsertText, Edit::Insert(text))
        })
    }

    /// 整体替换文本输入的已提交值；一次编辑、一次 `on:input`，caret 到末尾。
    ///
    /// 与 `input` 的外部替换语义不同，这次替换保留既有撤销/重做历史，并作为
    /// 一条新的撤销记录；新值与当前值相同时是静默无操作。组合中、禁用或
    /// 非文本输入的目标被拒绝。
    pub fn replace_text_node(
        &mut self,
        node: NodeId,
        value: &str,
    ) -> Result<TurnReport, DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            self.apply_text_edit(node, element, NodeOperation::SetValue, Edit::Replace(value))
        })
    }

    /// 删除 selection，或 caret 前一个字符；没有可删除内容时是不派发事件的无操作。
    pub fn backspace_node(&mut self, node: NodeId) -> Result<TurnReport, DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            self.apply_text_edit(node, element, NodeOperation::Backspace, Edit::Backspace)
        })
    }

    /// 删除 selection，或 caret 后一个字符；没有可删除内容时是不派发事件的无操作。
    pub fn delete_forward_node(&mut self, node: NodeId) -> Result<TurnReport, DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            self.apply_text_edit(
                node,
                element,
                NodeOperation::DeleteForward,
                Edit::DeleteForward,
            )
        })
    }

    /// 设置折叠 caret；偏移按字符夹取到当前值，不派发事件。
    pub fn set_caret_node(
        &mut self,
        node: NodeId,
        offset: usize,
    ) -> Result<TurnReport, DispatchError> {
        self.place_selection(node, NodeOperation::SetCaret, Movement::Caret(offset))
    }

    /// 设置 selection；起点终点按字符夹取并归一化，不派发事件。
    pub fn set_selection_node(
        &mut self,
        node: NodeId,
        start: usize,
        end: usize,
    ) -> Result<TurnReport, DispatchError> {
        self.place_selection(
            node,
            NodeOperation::SetSelection,
            Movement::Selection(start, end),
        )
    }

    /// 选中全部文本；caret 停在末尾，不派发事件。
    pub fn select_all_node(&mut self, node: NodeId) -> Result<TurnReport, DispatchError> {
        self.place_selection(node, NodeOperation::SelectAll, Movement::SelectAll)
    }

    fn place_selection(
        &mut self,
        node: NodeId,
        operation: NodeOperation,
        movement: Movement,
    ) -> Result<TurnReport, DispatchError> {
        let mut state = self.state.borrow_mut();
        let element = state.element_for_node(node)?;
        state.require_editable_text_input(node, element, operation)?;
        let key = state.tree_for_element(element);
        let reveal_before = key.map(|key| state.reveal_probe(key));
        let length = text_length(state.element_local(element));
        let (anchor, focus) = match movement {
            Movement::Caret(offset) => {
                let offset = offset.min(length);
                (offset, offset)
            }
            Movement::Selection(anchor, focus) => (anchor.min(length), focus.min(length)),
            Movement::SelectAll => (0, length),
        };
        set_local_selection(state.element_local_mut(element), anchor, focus);
        if let LocalState::TextInput(editor) = state.element_local_mut(element) {
            editor.history.break_run();
        }
        state.mark_element_changed(element);
        // 直接 selection 原语不经过 refresh；提交 revision 让宿主看到编辑状态变化。
        if let Some(key) = key {
            state.commit_tree_revision(key);
        }
        let mut report = TurnReport::default();
        if let (Some(key), Some(before)) = (key, reveal_before) {
            let after = state.reveal_probe(key);
            report.set_reveal(RevealProbe::diff(&before, &after));
        }
        Ok(report)
    }

    fn apply_text_edit(
        &mut self,
        node: NodeId,
        element: usize,
        operation: NodeOperation,
        edit: Edit<'_>,
    ) -> Result<TurnReport, DispatchError> {
        with_state(|state| state.require_editable_text_input(node, element, operation))?;
        let tree = with_state(|state| state.tree_for_element(element));
        let reveal_before = tree.map(|tree| with_state(|state| state.reveal_probe(tree)));
        let (before, caret, selection) = with_state(|state| {
            let LocalState::TextInput(editor) = state.element_local(element) else {
                unreachable!("require_editable_text_input validated the input");
            };
            let before = TextSnapshot {
                value: editor.value.clone(),
                caret: editor.caret,
                anchor: editor.anchor,
                selection: editor.selection,
            };
            (before, editor.caret, editor.selection)
        });
        let epoch = with_state(|state| state.element_value_epoch(element));
        let kind = match edit {
            Edit::Insert(text) => {
                let mut scalars = text.chars();
                match (scalars.next(), scalars.next()) {
                    (Some(scalar), None) => EditKind::Typing(scalar),
                    _ => EditKind::Boundary,
                }
            }
            Edit::Replace(_) | Edit::Backspace | Edit::DeleteForward => EditKind::Boundary,
        };
        let Some(outcome) = text_edit::apply(&before.value, caret, selection, edit) else {
            let mut report = TurnReport::default();
            finish_reveal(&mut report, tree, reveal_before.as_ref());
            return Ok(report);
        };
        let text_edit::EditOutcome {
            value: new_value,
            caret: next_caret,
        } = outcome;
        let (event_id, event) = self.take_node_event(node, element, EventKind::Input, operation)?;
        with_state(|state| {
            if let LocalState::TextInput(editor) = state.element_local_mut(element) {
                editor.caret = next_caret;
                editor.anchor = next_caret;
                editor.selection = None;
            }
            state.dispatch_text_edit(element, &new_value, epoch);
        });
        // 事件槽与 Event 阶段必须在 handler panic 时也恢复；panic 撤销这次
        // 尚未结算的本地编辑，handler `Err` 则保留已提交的编辑。
        let outcome = invoke_event(event_id, event, |event| match event {
            Event::Input(handler) => handler(new_value),
            _ => HandlerOutcome::Ok,
        });
        let mut report = TurnReport::default();
        let panicked = record_handler_outcome(outcome, &mut report);
        with_state(|state| state.phase = Phase::Idle);
        if panicked {
            with_state(|state| abort_text_dispatch(state, element));
            let _ = self.finish_command();
            finish_reveal(&mut report, tree, reveal_before.as_ref());
            return Ok(report);
        }
        if let Err(error) = self.finish_command() {
            with_state(|state| abort_text_dispatch(state, element));
            return Err(DispatchError::Command(error));
        }
        with_state(|state| clamp_local_edit(state, element));
        self.record_text_edit(element, before, kind);
        report.mark_changed();
        finish_reveal(&mut report, tree, reveal_before.as_ref());
        Ok(report)
    }

    /// 把一次编辑前后的编辑器状态记入该输入自己的历史。
    ///
    /// 只记录仍然挂载的元素；同一命令内结算的归一化值已经写回编辑器，
    /// 因此条目以提交后的文本为准；提交前后值相同的编辑不产生条目。
    fn record_text_edit(&mut self, element: usize, before: TextSnapshot, kind: EditKind) {
        with_state(|state| {
            if !state.element_is_mounted(element) {
                return;
            }
            let Some(after) = text_snapshot(state.element_local(element)) else {
                return;
            };
            let LocalState::TextInput(editor) = state.element_local_mut(element) else {
                return;
            };
            editor.history.record(before, after, kind);
        });
    }

    /// 撤销该输入最近一个撤销步；空栈是静默无操作。
    ///
    /// 连续单字符插入可能已合并为一个撤销步。撤销通过一次 `on:input` 写回该步
    /// 开始前的值，应用模型仍是值的权威。精确恢复 caret/anchor/selection 只对
    /// 原样提交值的处理器成立。
    pub fn undo_node(&mut self, node: NodeId) -> Result<TurnReport, DispatchError> {
        self.apply_history(node, NodeOperation::Undo, HistoryDirection::Undo)
    }

    /// 重做该输入最近一个被撤销的撤销步；空栈是静默无操作。
    ///
    /// 重做同样通过一次 `on:input` 写回，恢复的是该步提交后的状态；合并 run
    /// 的重做仍是一步。
    pub fn redo_node(&mut self, node: NodeId) -> Result<TurnReport, DispatchError> {
        self.apply_history(node, NodeOperation::Redo, HistoryDirection::Redo)
    }

    fn apply_history(
        &mut self,
        node: NodeId,
        operation: NodeOperation,
        direction: HistoryDirection,
    ) -> Result<TurnReport, DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            with_state(|state| state.require_editable_text_input(node, element, operation))?;
            let tree = with_state(|state| state.tree_for_element(element));
            let reveal_before = tree.map(|tree| with_state(|state| state.reveal_probe(tree)));
            // 撤销/重做后继续输入必须开启新的 run，不能并入更早的条目。
            with_state(|state| {
                let LocalState::TextInput(editor) = state.element_local_mut(element) else {
                    unreachable!("require_editable_text_input validated the input");
                };
                editor.history.break_run();
            });
            let entry = with_state(|state| {
                let LocalState::TextInput(editor) = state.element_local(element) else {
                    unreachable!("require_editable_text_input validated the input");
                };
                match direction {
                    HistoryDirection::Undo => editor.history.undo.back(),
                    HistoryDirection::Redo => editor.history.redo.back(),
                }
                .cloned()
            });
            let Some(entry) = entry else {
                let mut report = TurnReport::default();
                finish_reveal(&mut report, tree, reveal_before.as_ref());
                return Ok(report);
            };
            let target = match direction {
                HistoryDirection::Undo => entry.before.clone(),
                HistoryDirection::Redo => entry.after.clone(),
            };
            let (event_id, event) =
                self.take_node_event(node, element, EventKind::Input, operation)?;
            let epoch = with_state(|state| state.element_value_epoch(element));
            let length = target.value.chars().count();
            let (caret, anchor, selection) = (
                target.caret.min(length),
                target.anchor.min(length),
                target
                    .selection
                    .map(|(start, end)| text_edit::normalized_range(start, end, length)),
            );
            with_state(|state| {
                if let LocalState::TextInput(editor) = state.element_local_mut(element) {
                    editor.caret = caret;
                    editor.anchor = anchor;
                    editor.selection = selection;
                }
                state.dispatch_text_edit(element, &target.value, epoch);
            });
            let panic = invoke_event(event_id, event, |event| {
                if let Event::Input(handler) = event {
                    handler(target.value);
                }
            })
            .err();
            with_state(|state| state.phase = Phase::Idle);
            if panic.is_some() {
                with_state(|state| abort_text_dispatch(state, element));
            }
            if panic.is_none()
                && let Err(error) = self.finish_command()
            {
                with_state(|state| abort_text_dispatch(state, element));
                return Err(DispatchError::Command(error));
            }
            if let Some(panic) = panic {
                let _ = self.finish_command();
                return Err(DispatchError::Panic(panic));
            }
            with_state(|state| {
                clamp_local_edit(state, element);
                if !state.element_is_mounted(element) {
                    return;
                }
                let LocalState::TextInput(editor) = state.element_local_mut(element) else {
                    return;
                };
                match direction {
                    HistoryDirection::Undo => {
                        if editor.history.pop_undo().is_some() {
                            editor.history.push_redo(entry);
                        }
                    }
                    HistoryDirection::Redo => {
                        if editor.history.pop_redo().is_some() {
                            editor.history.push_undo(entry);
                        }
                    }
                }
            });
            let mut report = TurnReport::default();
            report.mark_changed();
            finish_reveal(&mut report, tree, reveal_before.as_ref());
            Ok(report)
        })
    }

    /// 开始一次本地 composition；preedit 只进入编辑器，不写入模型。
    pub fn compose_start(&mut self, label: &str) -> Result<(), MountError> {
        self.dispatch_label(
            FindTarget::Label(label.to_owned()),
            "composition target not found",
            "composition target is not a text input",
            Self::compose_start_node,
        )
    }

    /// 按 Node ID 开始组合；目标必须是文本输入且未在组合中。
    pub fn compose_start_node(&mut self, node: NodeId) -> Result<(), DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            with_state(|state| {
                state.require_enabled_text_input(node, element, NodeOperation::CompositionStart)
            })?;
            let (caret, selection) = with_state(|state| {
                let LocalState::TextInput(editor) = state.element_local_mut(element) else {
                    unreachable!("require_enabled_text_input validated the input");
                };
                if editor.is_composing() {
                    return Err(DispatchError::Rejected {
                        node,
                        operation: NodeOperation::CompositionStart,
                        reason: "text input is already composing",
                    });
                }
                editor.composition = Some(Composition {
                    base_caret: editor.caret,
                    base_anchor: editor.anchor,
                    base_selection: editor.selection,
                    preedit: String::new(),
                });
                editor.history.break_run();
                Ok((editor.caret, editor.selection))
            })?;
            with_state(|state| state.mark_element_changed(element));
            self.enter_event_phase();
            let panic = self
                .dispatch_element_event(element, EventKind::CompositionStart, |event| {
                    if let Event::CompositionStart(handler) = event {
                        handler(CompositionEvent::new(
                            CompositionPhase::Start,
                            String::new(),
                            caret,
                            selection,
                        ));
                    }
                })
                .err();
            self.finish_event(panic)
        })
    }

    /// 更新编辑器本地的 preedit；caret 跟随 preedit 末尾，模型保持不变。
    pub fn compose_update(&mut self, label: &str, preedit: &str) -> Result<(), MountError> {
        self.dispatch_label(
            FindTarget::Label(label.to_owned()),
            "composition target not found",
            "composition target is not a text input",
            move |runtime, node| runtime.compose_update_node(node, preedit),
        )
    }

    /// 按 Node ID 更新 preedit；目标必须已在组合中。
    pub fn compose_update_node(
        &mut self,
        node: NodeId,
        preedit: &str,
    ) -> Result<(), DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            with_state(|state| {
                state.require_enabled_text_input(node, element, NodeOperation::CompositionUpdate)
            })?;
            let (caret, selection) = with_state(|state| {
                let LocalState::TextInput(editor) = state.element_local_mut(element) else {
                    unreachable!("require_enabled_text_input validated the input");
                };
                let Some(composition) = editor.composition.as_mut() else {
                    return Err(DispatchError::Rejected {
                        node,
                        operation: NodeOperation::CompositionUpdate,
                        reason: "text input is not composing",
                    });
                };
                composition.preedit = preedit.to_owned();
                let caret = composition.base_range().0 + preedit.chars().count();
                editor.caret = caret;
                editor.anchor = caret;
                editor.selection = None;
                Ok((caret, editor.selection))
            })?;
            with_state(|state| state.mark_element_changed(element));
            self.enter_event_phase();
            let panic = self
                .dispatch_element_event(element, EventKind::CompositionUpdate, |event| {
                    if let Event::CompositionUpdate(handler) = event {
                        handler(CompositionEvent::new(
                            CompositionPhase::Update,
                            preedit.to_owned(),
                            caret,
                            selection,
                        ));
                    }
                })
                .err();
            self.finish_event(panic)
        })
    }

    /// 候选确认：用 `text` 替换组合区间，清除 preedit 并派发一次 `on:input`。
    pub fn compose_commit(&mut self, label: &str, text: &str) -> Result<TurnReport, MountError> {
        self.dispatch_label(
            FindTarget::Label(label.to_owned()),
            "composition target not found",
            "composition target is not a text input",
            move |runtime, node| runtime.compose_commit_node(node, text),
        )
    }

    /// 按 Node ID 确认候选；只有组合中的文本输入接受该操作。
    pub fn compose_commit_node(
        &mut self,
        node: NodeId,
        text: &str,
    ) -> Result<TurnReport, DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            with_state(|state| {
                state.require_enabled_text_input(
                    node,
                    element,
                    NodeOperation::CompositionCommit,
                )?;
                state.require_composing_text_input(node, element, NodeOperation::CompositionCommit)
            })?;
            let tree = with_state(|state| state.tree_for_element(element));
            let probe_before = tree.map(|tree| with_state(|state| state.reveal_probe(tree)));
            self.enter_event_phase();
            let (before, mut report) = match self.commit_composition(node, element, text) {
                Ok(done) => done,
                Err(DispatchError::Panic(panic)) => {
                    with_state(|state| abort_text_dispatch(state, element));
                    let _ = self.finish_command();
                    return Err(DispatchError::Panic(panic));
                }
                Err(error) => {
                    with_state(|state| state.phase = Phase::Idle);
                    return Err(error);
                }
            };
            if let Err(error) = self.leave_event_phase() {
                with_state(|state| abort_text_dispatch(state, element));
                return Err(DispatchError::Command(error));
            }
            with_state(|state| clamp_local_edit(state, element));
            if let Some(before) = before {
                self.record_text_edit(element, before, EditKind::Boundary);
            }
            report.mark_changed();
            finish_reveal(&mut report, tree, probe_before.as_ref());
            Ok(report)
        })
    }

    /// 取消组合：丢弃 preedit，恢复组合开始时的 caret/selection，不派发 `on:input`。
    pub fn compose_cancel(&mut self, label: &str) -> Result<(), MountError> {
        self.dispatch_label(
            FindTarget::Label(label.to_owned()),
            "composition target not found",
            "composition target is not a text input",
            Self::compose_cancel_node,
        )
    }

    /// 按 Node ID 取消组合；取消不写模型，因此组合期间被禁用的输入仍可取消。
    pub fn compose_cancel_node(&mut self, node: NodeId) -> Result<(), DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            // 取消组合是模态提示下的清理动作，不写入模型；挂起的父窗口也必须允许。
            let element = with_state(|state| state.element_for_node_with(node, true))?;
            with_state(|state| {
                state.require_text_input(node, element, NodeOperation::CompositionCancel)?;
                state.require_composing_text_input(node, element, NodeOperation::CompositionCancel)
            })?;
            self.enter_event_phase();
            match self.cancel_composition(node, element) {
                Ok(()) => self.finish_event(None),
                Err(DispatchError::Panic(panic)) => self.finish_event(Some(panic)),
                Err(error) => {
                    with_state(|state| state.phase = Phase::Idle);
                    Err(error)
                }
            }
        })
    }

    fn commit_composition(
        &mut self,
        node: NodeId,
        element: usize,
        text: &str,
    ) -> Result<(Option<TextSnapshot>, TurnReport), DispatchError> {
        // 组合期间外部 value 可能已更新；候选插入编辑器当前文本并按长度夹取组合区间。
        let current_text = with_state(|state| {
            let LocalState::TextInput(editor) = state.element_local(element) else {
                return Err(DispatchError::WrongKind {
                    node,
                    operation: NodeOperation::CompositionCommit,
                });
            };
            Ok::<_, DispatchError>(editor.value.clone())
        })?;
        let epoch = with_state(|state| state.element_value_epoch(element));
        let (before, new_text, caret) = with_state(|state| {
            let LocalState::TextInput(editor) = state.element_local_mut(element) else {
                return Err(DispatchError::WrongKind {
                    node,
                    operation: NodeOperation::CompositionCommit,
                });
            };
            let Some(composition) = editor.composition.take() else {
                return Err(DispatchError::Rejected {
                    node,
                    operation: NodeOperation::CompositionCommit,
                    reason: "text input is not composing",
                });
            };
            // 撤销回到组合开始前的已提交状态，而不是临时的 preedit caret。
            let before = TextSnapshot {
                value: current_text.clone(),
                caret: composition.base_caret,
                anchor: composition.base_anchor,
                selection: composition.base_selection,
            };
            let (start, end) = composition.base_range();
            let (new_text, next_caret) = text_edit::replace_range(&current_text, start, end, text);
            editor.caret = next_caret;
            editor.anchor = next_caret;
            editor.selection = None;
            Ok::<_, DispatchError>((before, new_text, next_caret))
        })?;
        with_state(|state| state.dispatch_text_edit(element, &new_text, epoch));
        if let Err(panic) =
            self.dispatch_element_event(element, EventKind::CompositionEnd, |event| {
                if let Event::CompositionEnd(handler) = event {
                    handler(CompositionEvent::new(
                        CompositionPhase::End,
                        text.to_owned(),
                        caret,
                        None,
                    ));
                }
            })
        {
            with_state(|state| state.phase = Phase::Idle);
            return Err(DispatchError::Panic(panic));
        }
        let mut report = TurnReport::default();
        if let Some((event_id, event)) =
            with_state(|state| state.take_element_event(element, EventKind::Input))
        {
            let outcome = invoke_event(event_id, event, |event| match event {
                Event::Input(handler) => handler(new_text),
                _ => HandlerOutcome::Ok,
            });
            match outcome {
                Ok(HandlerOutcome::Ok) => {}
                Ok(HandlerOutcome::Err(error)) => {
                    report.push_error(TurnError::HandlerFailed(error));
                }
                Err(panic) => {
                    with_state(|state| state.phase = Phase::Idle);
                    return Err(DispatchError::Panic(panic));
                }
            }
        }
        Ok((Some(before), report))
    }

    fn cancel_composition(&mut self, node: NodeId, element: usize) -> Result<(), DispatchError> {
        let (discarded, caret, selection) = with_state(|state| {
            let LocalState::TextInput(editor) = state.element_local_mut(element) else {
                return Err(DispatchError::WrongKind {
                    node,
                    operation: NodeOperation::CompositionCancel,
                });
            };
            let Some(composition) = editor.composition.take() else {
                return Err(DispatchError::Rejected {
                    node,
                    operation: NodeOperation::CompositionCancel,
                    reason: "text input is not composing",
                });
            };
            editor.caret = composition.base_caret;
            editor.anchor = composition.base_anchor;
            editor.selection = composition.base_selection;
            Ok((composition.preedit, editor.caret, editor.selection))
        })?;
        with_state(|state| state.mark_element_changed(element));
        self.dispatch_element_event(element, EventKind::CompositionCancel, |event| {
            if let Event::CompositionCancel(handler) = event {
                handler(CompositionEvent::new(
                    CompositionPhase::Cancel,
                    discarded.clone(),
                    caret,
                    selection,
                ));
            }
        })
        .map_err(DispatchError::Panic)
    }

    pub fn change(&mut self, label: &str, value: bool) -> Result<TurnReport, MountError> {
        self.dispatch_label(
            FindTarget::Label(label.to_owned()),
            "change target not found",
            "change target not found",
            move |runtime, node| runtime.change_node(node, value),
        )
    }

    /// 按 Node ID 派发一次复选框 change。
    ///
    /// 有 `on:change` 时先更新 proposed 再运行一次 handler；没有 handler 时
    /// 直接本地切换。受控 checkbox 在 flush 后按 `checked` 绑定对齐，非受控
    /// 的本地值由呈现层继续报告。
    pub fn change_node(&mut self, node: NodeId, value: bool) -> Result<TurnReport, DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            let tree = with_state(|state| state.tree_for_element(element));
            let reveal_before = tree.map(|tree| with_state(|state| state.reveal_probe(tree)));
            let before = with_state(|state| match state.element_local(element) {
                LocalState::Checkbox { checked } => Some(*checked),
                _ => None,
            });
            let Some(before) = before else {
                return Err(DispatchError::WrongKind {
                    node,
                    operation: NodeOperation::Change,
                });
            };
            let event = with_state(|state| state.take_element_event(element, EventKind::Change));
            let mut report = TurnReport::default();
            if let Some((event_id, event)) = event {
                with_state(|state| {
                    state.phase = Phase::Event;
                    state.fault = None;
                    state.write_error = None;
                    state.update_local(element, LocalUpdate::Change(value));
                });
                let panic = invoke_event(event_id, event, |event| {
                    if let Event::Change(handler) = event {
                        handler(value);
                    }
                })
                .err();
                with_state(|state| state.phase = Phase::Idle);
                if let Some(panic) = panic {
                    let _ = self.finish_command();
                    return Err(DispatchError::Panic(panic));
                }
            } else {
                with_state(|state| state.update_local(element, LocalUpdate::Change(value)));
            }
            self.finish_command().map_err(DispatchError::Command)?;
            let after = with_state(|state| match state.element_local(element) {
                LocalState::Checkbox { checked } => Some(*checked),
                _ => None,
            });
            if after.is_some_and(|after| after != before) {
                report.mark_changed();
            }
            finish_reveal(&mut report, tree, reveal_before.as_ref());
            Ok(report)
        })
    }

    pub fn submit(&mut self, label: &str) -> Result<TurnReport, MountError> {
        self.dispatch_label(
            FindTarget::Label(label.to_owned()),
            "submit target not found",
            "submit target not found",
            Self::submit_node,
        )
    }

    /// 按 Node ID 派发一次 submit；handler 的 `Err`/panic 进入 turn 报告。
    pub fn submit_node(&mut self, node: NodeId) -> Result<TurnReport, DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            let tree = with_state(|state| state.tree_for_element(element));
            let reveal_before = tree.map(|tree| with_state(|state| state.reveal_probe(tree)));
            let (event_id, event) =
                self.take_node_event(node, element, EventKind::Submit, NodeOperation::Submit)?;
            let mut submit_event = SubmitEvent::default();
            let outcome = invoke_event(event_id, event, |event| match event {
                Event::Submit(handler) => handler(&mut submit_event),
                _ => HandlerOutcome::Ok,
            });
            let mut report = TurnReport::default();
            record_handler_outcome(outcome, &mut report);
            with_state(|state| state.phase = Phase::Idle);
            self.finish_command().map_err(DispatchError::Command)?;
            report.set_outcome(submit_event.is_consumed(), submit_event.is_prevented());
            report.mark_changed();
            finish_reveal(&mut report, tree, reveal_before.as_ref());
            Ok(report)
        })
    }

    /// 组合中的候选 Enter 与取消 Escape 在用户 key handler 之前被消费；
    /// 候选文本由 IME 随后通过 compositionend/`compose_commit` 确认。
    /// 其他键仍按普通路由派发。未消费的 Enter 才是表单提交命令。
    pub fn keydown(&mut self, label: &str, key: Key) -> Result<TurnReport, MountError> {
        self.dispatch_label(
            FindTarget::Label(label.to_owned()),
            "keydown target not found",
            "keydown target not found",
            move |runtime, node| runtime.keydown_node(node, key),
        )
    }

    /// 按 Node ID 派发 keydown；组合中的 Enter/Escape 仍然优先被编辑器消费。
    /// 已解析目标的直接 keydown：只运行节点自己的普通 handler，不回溯祖先。
    pub fn keydown_node(&mut self, node: NodeId, key: Key) -> Result<TurnReport, DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            let tree = with_state(|state| state.tree_for_element(element));
            let reveal_before = tree.map(|tree| with_state(|state| state.reveal_probe(tree)));
            if with_state(|state| state.element_local(element).is_composing()) {
                let mut report = TurnReport::default();
                if key == Key::Escape {
                    self.enter_event_phase();
                    match self.cancel_composition(node, element) {
                        Ok(()) => {}
                        Err(DispatchError::Panic(panic)) => {
                            report.push_error(TurnError::HandlerPanicked(panic));
                        }
                        Err(error) => {
                            with_state(|state| state.phase = Phase::Idle);
                            return Err(error);
                        }
                    }
                }
                report.set_outcome(true, false);
                with_state(|state| state.phase = Phase::Idle);
                self.finish_command().map_err(DispatchError::Command)?;
                finish_reveal(&mut report, tree, reveal_before.as_ref());
                return Ok(report);
            }
            let (event_id, event) =
                self.take_node_event(node, element, EventKind::KeyDown, NodeOperation::KeyDown)?;
            let mut key_event = KeyEvent::new(key);
            let outcome = invoke_event(event_id, event, |event| match event {
                Event::KeyDown(handler) => handler(&mut key_event),
                _ => HandlerOutcome::Ok,
            });
            let mut report = TurnReport::default();
            match outcome {
                Ok(HandlerOutcome::Ok) => {}
                Ok(HandlerOutcome::Err(error)) => {
                    report.push_error(TurnError::HandlerFailed(error));
                }
                Err(panic) => report.push_error(TurnError::HandlerPanicked(panic)),
            }
            report.set_outcome(
                key_event.is_consumed()
                    || report
                        .errors()
                        .iter()
                        .any(|error| matches!(error, TurnError::HandlerPanicked(_))),
                key_event.prevents_default(),
            );
            with_state(|state| state.phase = Phase::Idle);
            self.finish_command().map_err(DispatchError::Command)?;
            finish_reveal(&mut report, tree, reveal_before.as_ref());
            Ok(report)
        })
    }

    /// headless 焦点命令：先对旧目标派发一次 blur，再对新目标派发一次 focus。
    pub fn focus(&mut self, label: &str) -> Result<(), MountError> {
        self.dispatch_label(
            FindTarget::Label(label.to_owned()),
            "focus target not found",
            "focus target is not focusable",
            move |runtime, node| runtime.focus_node(node).map(|_| ()),
        )
    }

    /// 按 Node ID 移动焦点；不可聚焦的目标返回 `WrongKind`，焦点保持不变。
    pub fn focus_node(&mut self, node: NodeId) -> Result<TurnReport, DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            let tree = with_state(|state| state.tree_for_element(element))
                .ok_or(DispatchError::StaleNode(node))?;
            if !with_state(|state| state.is_focusable(element)) {
                return Err(DispatchError::WrongKind {
                    node,
                    operation: NodeOperation::Focus,
                });
            }
            let before = with_state(|state| state.reveal_probe(tree));
            self.move_focus(tree, Some(element))
                .map_err(DispatchError::Command)?;
            let mut report = TurnReport::default();
            finish_reveal(&mut report, Some(tree), Some(&before));
            // 目标已经是焦点时 move_focus 不派发 handler、不推进 revision；
            // 只有焦点真的换了才报告 changed。
            if report.reveal().focus_changed() {
                report.mark_changed();
            }
            Ok(report)
        })
    }

    /// 按 Node ID 清除焦点：只有该元素仍是当前焦点时才清空，绝不改焦到其他元素。
    ///
    /// 已提交但未持有焦点的元素返回 `Rejected`；迟到的原生 blur 不会命中新的焦点。
    pub fn blur_node(&mut self, node: NodeId) -> Result<(), DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            let tree = with_state(|state| state.tree_for_element(element))
                .ok_or(DispatchError::StaleNode(node))?;
            let focused = with_state(|state| state.focused_in(tree));
            if focused != Some(element) {
                return Err(DispatchError::Rejected {
                    node,
                    operation: NodeOperation::Blur,
                    reason: "node is not focused",
                });
            }
            self.move_focus(tree, None).map_err(DispatchError::Command)
        })
    }

    /// 清空焦点窗口的焦点；没有当前目标时不派发事件。
    pub fn blur(&mut self) -> Result<(), MountError> {
        let state = self.state.clone();
        context::install(&state, || {
            let Some(tree) = with_state(|state| state.active_tree_key()) else {
                return Ok(());
            };
            self.move_focus(tree, None)
        })
    }

    /// Tab / 方向键的向前遍历；到末尾后回到第一个可聚焦控件。
    pub fn focus_next(&mut self) -> Result<(), MountError> {
        self.traverse_focus(true)
    }

    /// Tab / 方向键的向后遍历；到开头后回到最后一个可聚焦控件。
    pub fn focus_previous(&mut self) -> Result<(), MountError> {
        self.traverse_focus(false)
    }

    /// 在指定窗口内向前遍历焦点；焦点不会越过窗口边界。
    ///
    /// 窗口没有可聚焦控件时什么也不做；原生输入不应因为一个空窗口而报错。
    pub fn focus_next_in(&mut self, window: WindowCtx) -> Result<(), WindowError> {
        self.traverse_focus_in(window, true)
    }

    /// 在指定窗口内向后遍历焦点；焦点不会越过窗口边界。
    ///
    /// 窗口没有可聚焦控件时什么也不做。
    pub fn focus_previous_in(&mut self, window: WindowCtx) -> Result<(), WindowError> {
        self.traverse_focus_in(window, false)
    }

    /// 只清除指定窗口的焦点；其他窗口的焦点保持不变。
    pub fn blur_in(&mut self, window: WindowCtx) -> Result<(), WindowError> {
        let state = self.state.clone();
        if window.runtime != state.borrow().id {
            return Err(WindowError::WrongRuntime);
        }
        context::install(&state, || {
            if !with_state(|state| state.window_live_in_tree(window)) {
                return Err(WindowError::Stale);
            }
            self.move_focus(TreeSlot::Window(window.key), None)
                .map_err(WindowError::Mount)
        })
    }

    fn traverse_focus(&mut self, forward: bool) -> Result<(), MountError> {
        let state = self.state.clone();
        context::install(&state, || {
            let tree = with_state(|state| state.active_tree_key())
                .ok_or(MountError::Component("no focusable target"))?;
            self.traverse_focus_for(tree, forward)
        })
    }

    fn traverse_focus_in(&mut self, window: WindowCtx, forward: bool) -> Result<(), WindowError> {
        let state = self.state.clone();
        if window.runtime != state.borrow().id {
            return Err(WindowError::WrongRuntime);
        }
        context::install(&state, || {
            if !with_state(|state| state.window_live_in_tree(window)) {
                return Err(WindowError::Stale);
            }
            let tree = TreeSlot::Window(window.key);
            if with_state(|state| state.focus_order(tree)).is_empty() {
                return Ok(());
            }
            self.traverse_focus_for(tree, forward)
                .map_err(WindowError::Mount)
        })
    }

    fn traverse_focus_for(&mut self, tree: TreeSlot, forward: bool) -> Result<(), MountError> {
        let order = with_state(|state| state.focus_order(tree));
        if order.is_empty() {
            return Err(MountError::Component("no focusable target"));
        }
        let current = with_state(|state| state.focused_in(tree))
            .and_then(|focused| order.iter().position(|id| *id == focused));
        let index = match (forward, current) {
            (true, Some(index)) => (index + 1) % order.len(),
            (true, None) => 0,
            (false, Some(0)) => order.len() - 1,
            (false, Some(index)) => index - 1,
            (false, None) => order.len() - 1,
        };
        self.move_focus(tree, Some(order[index]))
    }

    fn move_focus(&mut self, tree: TreeSlot, target: Option<usize>) -> Result<(), MountError> {
        if let TreeSlot::Window(key) = tree {
            with_state(|state| state.focused_window = Some(key));
        }
        let current = with_state(|state| state.focused_in(tree));
        if current == target {
            return Ok(());
        }
        with_state(|state| {
            state.phase = Phase::Event;
            state.fault = None;
            state.write_error = None;
        });
        let mut panic = None;
        if let Some(old) = current {
            with_state(|state| state.set_focused_in(tree, None));
            if with_state(|state| state.element_is_mounted(old))
                && let Err(info) = self.fire_blur(old, FocusCause::Command)
            {
                panic = Some(info);
            }
        }
        if let Some(target) = target {
            with_state(|state| state.set_focused_in(tree, Some(target)));
            if panic.is_none()
                && let Err(info) = self.fire_focus(target, FocusCause::Command)
            {
                panic = Some(info);
            }
        }
        with_state(|state| state.phase = Phase::Idle);
        self.finish_event_mount(panic)
    }

    fn fire_blur(&mut self, element: usize, cause: FocusCause) -> Result<(), PanicInfo> {
        self.dispatch_element_event(element, EventKind::Blur, |event| {
            if let Event::Blur(handler) = event {
                handler(BlurEvent::new(cause));
            }
        })
    }

    fn fire_focus(&mut self, element: usize, cause: FocusCause) -> Result<(), PanicInfo> {
        self.dispatch_element_event(element, EventKind::Focus, |event| {
            if let Event::Focus(handler) = event {
                handler(FocusEvent::new(cause));
            }
        })
    }

    /// 取事件槽、运行 handler、无条件放回；panic 收敛为诊断。
    fn dispatch_element_event(
        &mut self,
        element: usize,
        kind: EventKind,
        invoke: impl FnOnce(&mut Event),
    ) -> Result<(), PanicInfo> {
        let Some((id, mut event)) = with_state(|state| state.take_element_event(element, kind))
        else {
            return Ok(());
        };
        let outcome = catch_panic(|| invoke(&mut event));
        with_state(|state| state.put_event(id, event));
        outcome
    }

    fn enter_event_phase(&self) {
        with_state(|state| {
            state.phase = Phase::Event;
            state.fault = None;
            state.write_error = None;
        });
    }

    fn leave_event_phase(&mut self) -> Result<(), MountError> {
        with_state(|state| state.phase = Phase::Idle);
        self.finish_command()
    }

    fn finish_command(&mut self) -> Result<(), MountError> {
        self.finish_command_without_ui()?;
        self.refresh_internal()
    }

    /// 事件 handler 结束后的统一收尾。
    ///
    /// panic 是主诊断：先把 phase 恢复为 Idle，并尽力刷新 panic 前已经提交的
    /// 写入；次生刷新错误不覆盖 panic。事件槽已经由 `invoke_event` 放回。
    fn finish_event(&mut self, panic: Option<PanicInfo>) -> Result<(), DispatchError> {
        let Some(panic) = panic else {
            return self.finish_command().map_err(DispatchError::Command);
        };
        with_state(|state| state.phase = Phase::Idle);
        let _ = self.finish_command();
        Err(DispatchError::Panic(panic))
    }

    /// [`Runtime::finish_event`] 的 `MountError` 版本；label 助手直接返回它。
    fn finish_event_mount(&mut self, panic: Option<PanicInfo>) -> Result<(), MountError> {
        let Some(panic) = panic else {
            return self.finish_command();
        };
        with_state(|state| state.phase = Phase::Idle);
        let _ = self.finish_command();
        Err(MountError::Panic(panic))
    }

    /// 没有挂载界面时的命令收尾：只报告 handler 的 fault 与写错误。
    fn finish_command_without_ui(&mut self) -> Result<(), MountError> {
        let fault = with_state(|state| state.fault.take());
        if let Some(fault) = fault {
            return Err(State::context_error(fault));
        }
        let write_error = with_state(|state| state.write_error.take());
        if let Some(error) = write_error {
            return Err(MountError::Write(error));
        }
        Ok(())
    }

    /// 当前焦点窗口中的文本；多窗口下不会混入其他窗口。
    pub fn texts(&self) -> Vec<String> {
        let state = self.state.borrow();
        let mut texts = Vec::new();
        if let Some(tree) = state.active_tree_key().and_then(|key| state.tree(key)) {
            collect_texts(&state, &tree.nodes, &mut texts);
        }
        texts
    }

    /// 当前焦点窗口中的 `For` 行；多窗口下不会混入其他窗口。
    pub fn for_items(&self) -> Vec<ItemInfo> {
        let state = self.state.borrow();
        let mut items = Vec::new();
        if let Some(tree) = state.active_tree_key().and_then(|key| state.tree(key)) {
            collect_items(&state, &tree.nodes, &mut items);
        }
        items
    }

    pub fn setup_count(&self, label: &str) -> u64 {
        self.state
            .borrow()
            .setup_counts
            .get(label)
            .copied()
            .unwrap_or(0)
    }

    pub fn once_reads(&self) -> u64 {
        self.state.borrow().once_reads
    }

    pub fn live_signals(&self) -> usize {
        self.state
            .borrow()
            .signals
            .iter()
            .filter(|cell| !cell.disposed)
            .count()
    }

    pub fn live_memos(&self) -> usize {
        self.state
            .borrow()
            .memos
            .iter()
            .filter(|cell| !cell.disposed)
            .count()
    }

    pub fn unmounts(&self) -> u64 {
        self.state.borrow().unmounts
    }

    pub fn counters(&self) -> Counters {
        self.state.borrow().store_counters
    }

    /// 当前焦点窗口的呈现 revision。
    ///
    /// 每个树维护自己的计数器；这里只读取当前活动树的版本，不再混合
    /// 全局计数器或其他窗口的变化。
    pub fn presentation_revision(&self) -> u64 {
        let state = self.state.borrow();
        state
            .active_tree_key()
            .and_then(|key| state.tree(key))
            .map_or(0, |tree| tree.revision)
    }

    /// 指定窗口所属树的呈现 revision；隐藏但存活的窗口也可以读取。
    pub fn presentation_revision_of(&self, window: WindowCtx) -> Result<u64, WindowError> {
        let state = self.state.borrow();
        if window.runtime != state.id {
            return Err(WindowError::WrongRuntime);
        }
        if !state.window_live_in_tree(window) {
            return Err(WindowError::Stale);
        }
        state
            .tree(TreeSlot::Window(window.key))
            .map(|tree| tree.revision)
            .ok_or(WindowError::Stale)
    }

    /// 实际构建 presentation snapshot 根的总次数；revision 未变的读取复用
    /// 共享根、不计数。用于验证滚动与指针输入不重建快照。
    pub fn presentation_builds(&self) -> u64 {
        self.state.borrow().presentation_builds.get()
    }

    pub fn caret(&self, label: &str) -> Option<usize> {
        self.state
            .borrow()
            .text_input(&FindTarget::Label(label.to_owned()))
    }

    /// 当前编辑器本地的 preedit；未在组合中时为 `None`。
    pub fn preedit(&self, label: &str) -> Option<String> {
        let state = self.state.borrow();
        let element = state.find_element(&FindTarget::Label(label.to_owned()))?;
        match state.element_local(element) {
            LocalState::TextInput(editor) => editor
                .composition
                .as_ref()
                .map(|composition| composition.preedit.clone()),
            _ => None,
        }
    }

    pub fn set_caret(&mut self, label: &str, offset: usize) -> bool {
        self.state
            .borrow_mut()
            .set_text_input_caret(&FindTarget::Label(label.to_owned()), offset)
    }

    /// 当前接收输入的窗口；没有可见窗口时为 `None`。
    pub fn focused_window(&self) -> Option<WindowCtx> {
        let state = self.state.borrow();
        state
            .focused_window
            .filter(|key| state.window_is_visible(key))
            .and_then(|key| state.window_ctx(key))
    }

    pub fn focused(&self) -> Option<FocusInfo> {
        let state = self.state.borrow();
        let element = state.presented_focus()?;
        Some(FocusInfo {
            tag: state.element_tag(element),
            label: state.element_label_text(element).map(ToOwned::to_owned),
        })
    }

    pub fn checked(&self, label: &str) -> Option<bool> {
        let state = self.state.borrow();
        let element = state.find_element(&FindTarget::Label(label.to_owned()))?;
        match state.element_local(element) {
            LocalState::Checkbox { checked } => Some(*checked),
            _ => None,
        }
    }

    /// 焦点窗口最近一次提交界面的只读呈现描述；生成过程不执行用户闭包，也不改变界面。
    pub fn presentation(&self) -> Result<PresentationSnapshot, MountError> {
        let state = self.state.borrow();
        state
            .active_tree_key()
            .and_then(|key| state.snapshot(key))
            .ok_or(MountError::Component(
                "presentation snapshot requires a visible committed interface",
            ))
    }

    /// 指定窗口的呈现快照；窗口已关闭、属于其他 Runtime 或没有已提交树时拒绝。
    ///
    /// 这是给 UIA、测试与查询使用的便捷入口，不发布 delta，也不推进发布基线。
    pub fn presentation_of(&self, window: WindowCtx) -> Result<PresentationSnapshot, WindowError> {
        let state = self.state.borrow();
        if window.runtime != state.id {
            return Err(WindowError::WrongRuntime);
        }
        if !state.window_is_live(window.key, window.activation, window.owner) {
            return Err(WindowError::Stale);
        }
        if !state.window_is_visible(window.key) {
            return Err(WindowError::Hidden);
        }
        state
            .snapshot(TreeSlot::Window(window.key))
            .ok_or(WindowError::Stale)
    }

    /// 当前活动树的呈现发布单位：快照与相对上次成功发布的累计 delta。
    ///
    /// 首次发布、隐藏窗口重新显示后的首次发布，或没有可安全累计的基线时，
    /// [`PresentationUpdate::delta`] 为 `None`，消费方按完整根重建。
    pub fn presentation_update(&self) -> Result<PresentationUpdate, MountError> {
        let state = self.state.borrow();
        state
            .active_tree_key()
            .and_then(|key| state.presentation_update(key))
            .ok_or(MountError::Component(
                "presentation update requires a visible committed interface",
            ))
    }

    /// 指定窗口的呈现发布单位；隐藏窗口拒绝发布，显示后的首次发布是完整根。
    pub fn presentation_update_of(
        &self,
        window: WindowCtx,
    ) -> Result<PresentationUpdate, WindowError> {
        let state = self.state.borrow();
        if window.runtime != state.id {
            return Err(WindowError::WrongRuntime);
        }
        if !state.window_is_live(window.key, window.activation, window.owner) {
            return Err(WindowError::Stale);
        }
        if !state.window_is_visible(window.key) {
            return Err(WindowError::Hidden);
        }
        state
            .presentation_update(TreeSlot::Window(window.key))
            .ok_or(WindowError::Stale)
    }

    /// 当前应用外观。
    pub fn appearance(&self) -> Appearance {
        self.state.borrow().appearance
    }

    /// 设置应用外观；只失效依赖外观的 memo，并在已挂载时刷新一帧。
    pub fn set_appearance(&mut self, appearance: Appearance) -> Result<(), MountError> {
        self.state.borrow_mut().set_appearance(appearance);
        if !self.state.borrow().trees.is_empty() {
            self.refresh_internal()?;
        }
        Ok(())
    }

    /// 对照当前已提交界面判定一个 Node ID：存活、已卸载或从未分配。
    pub fn node_status(&self, node: NodeId) -> NodeStatus {
        self.state.borrow().node_status(node)
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

fn contains_element(state: &State, nodes: &[MountedNode], wanted: usize) -> bool {
    nodes.iter().any(|node| match node {
        MountedNode::Text(_) => false,
        MountedNode::Element(id) => {
            *id == wanted || contains_element(state, &state.elements[*id].children, wanted)
        }
        MountedNode::Component(id) => {
            contains_element(state, &state.components[*id].children, wanted)
        }
        MountedNode::Show(payload) => {
            let show = state.nodes.show(*payload);
            show.active
                .as_ref()
                .is_some_and(|branch| contains_element(state, &branch.children, wanted))
        }
        MountedNode::For(payload) => {
            let for_node = state.nodes.for_node(*payload);
            if for_node
                .spacers
                .is_some_and(|(top, bottom)| top == wanted || bottom == wanted)
            {
                return true;
            }
            for_node.order.iter().any(|key| {
                for_node
                    .items
                    .get(key)
                    .is_some_and(|item| contains_element(state, &item.children, wanted))
            })
        }
    })
}

fn collect_band_keys(state: &State, nodes: &[MountedNode], out: &mut BTreeSet<u64>) {
    for node in nodes {
        match node {
            MountedNode::Text(_) => {}
            MountedNode::Element(id) => {
                collect_band_keys(state, &state.elements[*id].children, out);
            }
            MountedNode::Component(id) => {
                collect_band_keys(state, &state.components[*id].children, out);
            }
            MountedNode::Show(payload) => {
                let show = state.nodes.show(*payload);
                if let Some(branch) = &show.active {
                    collect_band_keys(state, &branch.children, out);
                }
            }
            MountedNode::For(payload) => {
                let for_node = state.nodes.for_node(*payload);
                // 只有声明了 `row-height` 的 For 才发布行带；其他 For 的键
                // 不是宿主可以寻址的窗口。
                if for_node.row_height.is_some() {
                    out.insert(for_node.band);
                }
                for key in &for_node.order {
                    if let Some(item) = for_node.items.get(key) {
                        collect_band_keys(state, &item.children, out);
                    }
                }
            }
        }
    }
}

fn find_for_payload(state: &State, nodes: &[MountedNode], band: u64) -> Option<u32> {
    for node in nodes {
        match node {
            MountedNode::Text(_) => {}
            MountedNode::Element(id) => {
                if let Some(payload) = find_for_payload(state, &state.elements[*id].children, band)
                {
                    return Some(payload);
                }
            }
            MountedNode::Component(id) => {
                if let Some(payload) =
                    find_for_payload(state, &state.components[*id].children, band)
                {
                    return Some(payload);
                }
            }
            MountedNode::Show(payload) => {
                let show = state.nodes.show(*payload);
                if let Some(branch) = &show.active
                    && let Some(payload) = find_for_payload(state, &branch.children, band)
                {
                    return Some(payload);
                }
            }
            MountedNode::For(payload) => {
                let for_node = state.nodes.for_node(*payload);
                if for_node.band == band && for_node.row_height.is_some() {
                    return Some(*payload);
                }
                for key in &for_node.order {
                    if let Some(item) = for_node.items.get(key)
                        && let Some(payload) = find_for_payload(state, &item.children, band)
                    {
                        return Some(payload);
                    }
                }
            }
        }
    }
    None
}

fn contains_band(state: &State, nodes: &[MountedNode], band: u64) -> bool {
    nodes.iter().any(|node| match node {
        MountedNode::Text(_) => false,
        MountedNode::Element(id) => contains_band(state, &state.elements[*id].children, band),
        MountedNode::Component(id) => contains_band(state, &state.components[*id].children, band),
        MountedNode::Show(payload) => {
            let show = state.nodes.show(*payload);
            show.active
                .as_ref()
                .is_some_and(|branch| contains_band(state, &branch.children, band))
        }
        MountedNode::For(payload) => {
            let for_node = state.nodes.for_node(*payload);
            // 只有声明了 `row-height` 的 For 才发布行带；其他 For 的键
            // 不是宿主可以寻址的窗口。
            (for_node.band == band && for_node.row_height.is_some())
                || for_node.order.iter().any(|key| {
                    for_node
                        .items
                        .get(key)
                        .is_some_and(|item| contains_band(state, &item.children, band))
                })
        }
    })
}

fn focus_replacement(
    state: &State,
    before: &[usize],
    after: &[usize],
    old: usize,
    tag: Option<&'static str>,
) -> Option<usize> {
    let index = before.iter().position(|id| *id == old)?;
    let tag = tag?;
    let live: BTreeSet<usize> = after.iter().copied().collect();
    let same_tag = |id: &usize| live.contains(id) && state.element_tag(*id) == tag;
    before[index + 1..]
        .iter()
        .find(|id| same_tag(id))
        .copied()
        .or_else(|| {
            before[..index]
                .iter()
                .rev()
                .find(|id| same_tag(id))
                .copied()
        })
        .or_else(|| {
            before[index + 1..]
                .iter()
                .find(|id| live.contains(id))
                .copied()
        })
        .or_else(|| {
            before[..index]
                .iter()
                .rev()
                .find(|id| live.contains(id))
                .copied()
        })
}

fn text_length(local: &LocalState) -> usize {
    match local {
        LocalState::TextInput(editor) => editor.value.chars().count(),
        _ => 0,
    }
}

fn abort_text_dispatch(state: &mut State, element: usize) {
    if let LocalState::TextInput(editor) = state.element_local_mut(element) {
        editor.lineage.abort_dispatch();
    }
}

/// 释放一批已卸载节点及其子树中的编辑器文本、历史与谱系观察值。
fn clear_history_in_nodes(state: &mut State, nodes: &[MountedNode]) {
    let mut elements = Vec::new();
    collect_elements(state, nodes, &mut elements);
    for element in elements {
        if let LocalState::TextInput(editor) = state.element_local_mut(element) {
            editor.reset();
        }
    }
}

/// handler 可能改写了值；刷新后把本地 caret/selection 夹回新值范围。
fn clamp_local_edit(state: &mut State, element: usize) {
    if let LocalState::TextInput(editor) = state.element_local_mut(element) {
        editor.clamp_offsets();
    }
}

fn set_local_selection(local: &mut LocalState, anchor: usize, focus: usize) {
    if let LocalState::TextInput(editor) = local {
        editor.caret = focus;
        editor.anchor = anchor;
        editor.selection = Some((anchor.min(focus), anchor.max(focus)));
    }
}

fn collect_elements(state: &State, nodes: &[MountedNode], order: &mut Vec<usize>) {
    for node in nodes {
        match node {
            MountedNode::Text(_) => {}
            MountedNode::Element(id) => {
                order.push(*id);
                collect_elements(state, &state.elements[*id].children, order);
            }
            MountedNode::Component(id) => {
                collect_elements(state, &state.components[*id].children, order);
            }
            MountedNode::Show(payload) => {
                let show = state.nodes.show(*payload);
                if let Some(branch) = &show.active {
                    collect_elements(state, &branch.children, order);
                }
            }
            MountedNode::For(payload) => {
                let for_node = state.nodes.for_node(*payload);
                if let Some((top, _)) = for_node.spacers {
                    order.push(top);
                }
                for key in &for_node.order {
                    if let Some(item) = for_node.items.get(key) {
                        collect_elements(state, &item.children, order);
                    }
                }
                if let Some((_, bottom)) = for_node.spacers {
                    order.push(bottom);
                }
            }
        }
    }
}

fn collect_focusable(state: &State, nodes: &[MountedNode], order: &mut Vec<usize>) {
    for node in nodes {
        match node {
            MountedNode::Text(_) => {}
            MountedNode::Element(id) => {
                if state.is_focusable(*id) {
                    order.push(*id);
                }
                collect_focusable(state, &state.elements[*id].children, order);
            }
            MountedNode::Component(id) => {
                collect_focusable(state, &state.components[*id].children, order);
            }
            MountedNode::Show(payload) => {
                let show = state.nodes.show(*payload);
                if let Some(branch) = &show.active {
                    collect_focusable(state, &branch.children, order);
                }
            }
            MountedNode::For(payload) => {
                let for_node = state.nodes.for_node(*payload);
                for key in &for_node.order {
                    if let Some(item) = for_node.items.get(key) {
                        collect_focusable(state, &item.children, order);
                    }
                }
            }
        }
    }
}

/// 只复制已提交的文本、属性值与本地编辑状态；不重新求值 slot，也不改变任何界面状态。
fn collect_snapshot(state: &State, nodes: &[MountedNode], out: &mut Vec<SnapshotNode>) {
    for node in nodes {
        match node {
            MountedNode::Text(payload) => {
                let key = PresentationKey::from_node(state.nodes.text(*payload).node);
                out.push(SnapshotNode::Text(SnapshotText {
                    key,
                    text: state.text_value(*payload),
                }));
            }
            MountedNode::Element(id) => collect_element(state, *id, out),
            MountedNode::Component(id) => {
                collect_snapshot(state, &state.components[*id].children, out);
            }
            MountedNode::Show(payload) => {
                let show = state.nodes.show(*payload);
                if let Some(branch) = &show.active {
                    collect_snapshot(state, &branch.children, out);
                }
            }
            MountedNode::For(payload) => {
                let for_node = state.nodes.for_node(*payload);
                if let Some((top, _)) = for_node.spacers {
                    collect_element(state, top, out);
                }
                for key in &for_node.order {
                    if let Some(item) = for_node.items.get(key) {
                        collect_snapshot(state, &item.children, out);
                    }
                }
                if let Some((_, bottom)) = for_node.spacers {
                    collect_element(state, bottom, out);
                }
            }
        }
    }
}

fn collect_element(state: &State, id: usize, out: &mut Vec<SnapshotNode>) {
    let element = &state.elements[id];
    let mut children = Vec::new();
    collect_snapshot(state, &element.children, &mut children);
    out.push(SnapshotNode::Element(SnapshotElement {
        id: element.id,
        tag: state.element_tag(id),
        attrs: snapshot_attrs(state, id),
        styles: snapshot_styles(state, id),
        edit: snapshot_edit(state.element_local(id)),
        semantics: state.element_semantics(id),
        children,
    }));
}

/// 收集窗口化 For 的行带元数据；窗口外行没有节点，几何全靠这条记录定位。
fn collect_row_bands(state: &State, nodes: &[MountedNode], out: &mut Vec<SnapshotRowBand>) {
    for node in nodes {
        match node {
            MountedNode::Text(_) => {}
            MountedNode::Element(id) => {
                collect_row_bands(state, &state.elements[*id].children, out);
            }
            MountedNode::Component(id) => {
                collect_row_bands(state, &state.components[*id].children, out);
            }
            MountedNode::Show(payload) => {
                let show = state.nodes.show(*payload);
                if let Some(branch) = &show.active {
                    collect_row_bands(state, &branch.children, out);
                }
            }
            MountedNode::For(payload) => {
                let for_node = state.nodes.for_node(*payload);
                if let (Some(row_height), Some((top, _))) = (for_node.row_height, for_node.spacers)
                {
                    out.push(SnapshotRowBand {
                        key: for_node.band,
                        top: state.elements[top].id,
                        row_height,
                        first_row: for_node.first_row,
                        total_rows: for_node.total_rows,
                    });
                }
                for key in &for_node.order {
                    if let Some(item) = for_node.items.get(key) {
                        collect_row_bands(state, &item.children, out);
                    }
                }
            }
        }
    }
}

fn snapshot_edit(local: &LocalState) -> Option<TextEditState> {
    let LocalState::TextInput(editor) = local else {
        return None;
    };
    Some(TextEditState {
        caret: editor.caret,
        anchor: editor.anchor,
        selection: editor.selection,
        preedit: editor
            .composition
            .as_ref()
            .map(|composition| composition.preedit.clone()),
        composition: editor
            .composition
            .as_ref()
            .map(|composition| composition.base_range()),
    })
}

/// 只复制已提交的声明；inline 槽的当前值在刷新时已经算好。
/// 没有 inline 声明时借用 class 切片，避免每个元素都分配。
fn snapshot_styles(
    state: &State,
    element: usize,
) -> std::borrow::Cow<'static, [zenui_style::StyleDeclaration]> {
    let class_styles = state.element_class_styles(element);
    let inline_styles = state.element_inline_styles(element);
    if inline_styles.is_empty() {
        return std::borrow::Cow::Borrowed(class_styles);
    }
    let table = state.element_binding_table(element);
    let mut styles = class_styles.to_vec();
    styles.extend(inline_styles.iter().filter_map(|style| {
        let value = table.and_then(|table| table.inline_style(style.binding))?;
        Some(zenui_style::StyleDeclaration::inline(style.property, value))
    }));
    std::borrow::Cow::Owned(styles)
}

fn snapshot_attrs(state: &State, element: usize) -> Vec<SnapshotAttr> {
    let table = state.element_binding_table(element);
    let mut attrs: Vec<SnapshotAttr> = state
        .element_attrs(element)
        .iter()
        .filter_map(|attr| {
            let table = table?;
            if table.is_text(attr.binding) {
                table.text_current(attr.binding).map(|text| SnapshotAttr {
                    name: attr.name,
                    value: SnapshotValue::Text(text.to_owned()),
                })
            } else if table.is_bool(attr.binding) {
                table.bool_current(attr.binding).map(|value| SnapshotAttr {
                    name: attr.name,
                    value: SnapshotValue::Bool(value),
                })
            } else {
                None
            }
        })
        .collect();
    // 无 `checked` 绑定的 checkbox 由本地编辑状态决定呈现；有绑定时两者在已提交帧一致。
    if let LocalState::Checkbox { checked } = state.element_local(element) {
        match attrs.iter_mut().find(|attr| attr.name == "checked") {
            Some(attr) => attr.value = SnapshotValue::Bool(*checked),
            None => attrs.push(SnapshotAttr {
                name: "checked",
                value: SnapshotValue::Bool(*checked),
            }),
        }
    }
    // 文本输入呈现编辑器当前文本；`value` 绑定可能尚未追上本地编辑。
    if let LocalState::TextInput(editor) = state.element_local(element)
        && let Some(SnapshotValue::Text(value)) = attrs
            .iter_mut()
            .find(|attr| attr.name == "value")
            .map(|attr| &mut attr.value)
    {
        *value = editor.value.clone();
    }
    attrs
}

fn collect_texts(state: &State, nodes: &[MountedNode], texts: &mut Vec<String>) {
    for node in nodes {
        match node {
            MountedNode::Text(payload) => texts.push(state.text_value(*payload)),
            MountedNode::Element(id) => collect_texts(state, &state.elements[*id].children, texts),
            MountedNode::Component(id) => {
                collect_texts(state, &state.components[*id].children, texts);
            }
            MountedNode::Show(payload) => {
                let show = state.nodes.show(*payload);
                if let Some(branch) = &show.active {
                    collect_texts(state, &branch.children, texts);
                }
            }
            MountedNode::For(payload) => {
                let for_node = state.nodes.for_node(*payload);
                for key in &for_node.order {
                    if let Some(item) = for_node.items.get(key) {
                        collect_texts(state, &item.children, texts);
                    }
                }
            }
        }
    }
}

fn collect_items(state: &State, nodes: &[MountedNode], items: &mut Vec<ItemInfo>) {
    for node in nodes {
        match node {
            MountedNode::Text(_) => {}
            MountedNode::Element(id) => {
                collect_items(state, &state.elements[*id].children, items);
            }
            MountedNode::Component(id) => {
                collect_items(state, &state.components[*id].children, items);
            }
            MountedNode::Show(payload) => {
                let show = state.nodes.show(*payload);
                if let Some(branch) = &show.active {
                    collect_items(state, &branch.children, items);
                }
            }
            MountedNode::For(payload) => {
                let for_node = state.nodes.for_node(*payload);
                for id in &for_node.order {
                    if let Some(item) = for_node.items.get(id) {
                        items.push(ItemInfo {
                            store: for_node.store,
                            row: *id,
                            todo: item.todo,
                            instance: item.instance,
                            owner: item.owner,
                        });
                        collect_items(state, &item.children, items);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod history_tests {
    #![allow(unused_braces)]

    use super::*;
    use crate::{component, memo, signal, view};

    #[component]
    fn Toggle() -> MountResult<MountPlan> {
        let visible = signal(true);
        let draft = signal(String::from("abcd"));
        let shown = memo(move || visible.get());

        view! {
            <view>
                <Show when={shown}>
                    <text-input
                        aria:label="toggle"
                        value={draft}
                        on:input={move |value: String| draft.set(value)}
                    />
                </Show>
                <button aria:label="hide-toggle" on:click={move || visible.set(false)}>"Hide"</button>
            </view>
        }
    }

    #[test]
    fn unmounting_clears_the_history_snapshot_text() {
        let mut app = Runtime::new();
        app.mount(|| view! { <Toggle /> }).expect("mount");
        let input = app
            .presentation()
            .expect("snapshot")
            .find_label("toggle")
            .expect("the input")
            .id();
        app.set_caret_node(input, 4).expect("caret");
        app.insert_text_node(input, "secret").expect("edit");

        app.click_label("hide-toggle").expect("unmount the input");

        let state = app.state.borrow();
        let slot = state
            .elements
            .iter()
            .position(|element| element.id == input)
            .expect("the retired element keeps its identity tombstone");
        let element = &state.elements[slot];
        assert_eq!(element.id, input, "the dead slot keeps the stale NodeId");
        assert!(
            element.children.is_empty() && state.element_aux_opt(slot).is_none(),
            "the dead slot releases its children and auxiliary payload"
        );
        assert!(
            state.nodes.get(input.node()).is_err(),
            "the arena node rejects the stale identity"
        );
        assert!(
            state.free_elements.contains(&slot),
            "the dead slot is available for reuse"
        );
    }

    #[test]
    fn replacing_the_value_dispatches_one_input_and_records_one_undo_entry() {
        use std::cell::{Cell, RefCell};
        use std::rc::Rc;

        let calls = Rc::new(Cell::new(0));
        let seen = Rc::new(RefCell::new(Vec::new()));
        let input_calls = Rc::clone(&calls);
        let input_seen = Rc::clone(&seen);
        let mut app = Runtime::new();
        app.mount(move || {
            let draft = signal(String::from("old"));
            view! {
                <text-input
                    aria:label="field"
                    value={draft}
                    on:input={move |value: String| {
                        input_calls.set(input_calls.get() + 1);
                        input_seen.borrow_mut().push(value.clone());
                        draft.set(value);
                    }}
                />
            }
        })
        .expect("mount");

        let input = app
            .presentation()
            .expect("snapshot")
            .find_label("field")
            .expect("the input")
            .id();
        app.replace_text_node(input, "new").expect("replace");
        assert_eq!(calls.get(), 1);
        assert_eq!(seen.borrow().as_slice(), ["new"]);
        let edited = app.presentation().expect("snapshot");
        let field = edited.find_label("field").expect("the input");
        assert_eq!(field.text_value(), Some("new"));
        assert_eq!(field.edit().expect("edit state").caret(), 3);

        app.undo_node(input).expect("undo");
        assert_eq!(calls.get(), 2);
        assert_eq!(seen.borrow().as_slice(), ["new", "old"]);
        app.redo_node(input).expect("redo");
        assert_eq!(calls.get(), 3);
        assert_eq!(seen.borrow().as_slice(), ["new", "old", "new"]);
    }
}

#[cfg(test)]
mod lineage_tests {
    #![allow(unused_braces)]

    use super::*;
    use crate::prelude::*;
    use crate::{component, memo, signal, view};

    fn text_of(app: &Runtime, label: &str) -> String {
        app.presentation()
            .expect("snapshot")
            .find_label(label)
            .expect("the input")
            .text_value()
            .unwrap_or_default()
            .to_owned()
    }

    fn input_id(app: &Runtime, label: &str) -> NodeId {
        app.presentation()
            .expect("snapshot")
            .find_label(label)
            .expect("the input")
            .id()
    }

    fn trace(app: &Runtime) -> Vec<TextTrace> {
        app.state.borrow().text_trace.iter().copied().collect()
    }

    /// 处理器不写 `value` 绑定：写入推迟到之后的按钮命令。
    #[component]
    fn Deferred() -> MountResult<MountPlan> {
        let draft = signal(String::from("a"));
        let first = signal(String::new());
        let second = signal(String::new());
        let calls = signal(0u32);

        view! {
            <view>
                <text-input
                    aria:label="deferred"
                    value={draft}
                    on:input={move |value: String| {
                        let count = calls.get();
                        calls.set(count + 1);
                        if count == 0 {
                            first.set(value);
                        } else {
                            second.set(value);
                        }
                    }}
                />
                <button aria:label="first" on:click={move || draft.set(first.get())}>
                    "Deliver first"
                </button>
                <button aria:label="second" on:click={move || draft.set(second.get())}>
                    "Deliver second"
                </button>
            </view>
        }
    }

    #[test]
    fn a_late_settlement_of_an_older_edit_is_ignored() {
        let mut app = Runtime::new();
        app.mount(|| view! { <Deferred /> }).expect("mount");
        let input = input_id(&app, "deferred");

        app.set_caret_node(input, 1).expect("caret");
        app.insert_text_node(input, "b").expect("first edit");
        // 移动 caret 结束连续输入 run，让两次编辑各有一个撤销步。
        app.set_caret_node(input, 2).expect("caret");
        app.insert_text_node(input, "c").expect("second edit");
        assert_eq!(text_of(&app, "deferred"), "abc");

        app.click_label("first")
            .expect("deliver the first edit's value");
        assert_eq!(
            text_of(&app, "deferred"),
            "abc",
            "the stale settlement must not roll the editor back"
        );

        app.click_label("second")
            .expect("deliver the second edit's value");
        assert_eq!(text_of(&app, "deferred"), "abc");

        let events = trace(&app);
        let origin = match events[0] {
            TextTrace::Remount { origin } => origin,
            other => panic!("the first event is the mount: {other:?}"),
        };
        assert_eq!(
            events,
            [
                TextTrace::Remount { origin },
                TextTrace::LocalEdit {
                    origin,
                    revision: 1,
                },
                TextTrace::LocalEdit {
                    origin,
                    revision: 2,
                },
                TextTrace::StaleSettlement {
                    origin,
                    current: 2,
                    stale: 1,
                },
                TextTrace::Settlement {
                    origin,
                    revision: 2,
                    normalized: false,
                },
            ],
            "each delivery is classified once, in revision order"
        );

        app.undo_node(input)
            .expect("the stale settlement kept the newest history");
        assert_eq!(text_of(&app, "deferred"), "ab");
    }

    /// 处理器把值截断到两个字符：归一化后的提交值仍是这次编辑的结算。
    #[component]
    fn Capped() -> MountResult<MountPlan> {
        let draft = signal(String::from("abc"));

        view! {
            <text-input
                aria:label="cap"
                value={draft}
                on:input={move |value: String| draft.set(value.chars().take(2).collect())}
            />
        }
    }

    #[test]
    fn a_normalizing_handler_settles_with_the_committed_value() {
        let mut app = Runtime::new();
        app.mount(|| view! { <Capped /> }).expect("mount");
        let input = input_id(&app, "cap");

        app.set_caret_node(input, 3).expect("caret");
        app.insert_text_node(input, "Z").expect("edit");

        let snapshot = app.presentation().expect("snapshot");
        let field = snapshot.find_label("cap").expect("the input");
        assert_eq!(field.text_value(), Some("ab"));
        assert_eq!(
            field.edit().expect("edit").caret(),
            2,
            "the caret is clamped to the committed value"
        );

        let events = trace(&app);
        let origin = match events[0] {
            TextTrace::Remount { origin } => origin,
            other => panic!("the first event is the mount: {other:?}"),
        };
        assert_eq!(
            events.last(),
            Some(&TextTrace::Settlement {
                origin,
                revision: 1,
                normalized: true,
            }),
            "the truncated value is the edit's settlement"
        );
    }

    /// 处理器把一位上限写回编辑前的值：相等写入仍表示应用处理了这次编辑。
    #[component]
    fn CappedAtOne() -> MountResult<MountPlan> {
        let draft = signal(String::from("a"));

        view! {
            <text-input
                aria:label="same"
                value={draft}
                on:input={move |value: String| draft.set(value.chars().take(1).collect())}
            />
        }
    }

    #[test]
    fn a_handler_that_writes_the_old_value_settles_the_edit() {
        let mut app = Runtime::new();
        app.mount(|| view! { <CappedAtOne /> }).expect("mount");
        let input = input_id(&app, "same");

        app.set_caret_node(input, 1).expect("caret");
        app.insert_text_node(input, "b").expect("edit");
        assert_eq!(
            text_of(&app, "same"),
            "a",
            "a write of the old value still settles the edit"
        );

        let events = trace(&app);
        let origin = match events[0] {
            TextTrace::Remount { origin } => origin,
            other => panic!("the first event is the mount: {other:?}"),
        };
        assert_eq!(
            events.last(),
            Some(&TextTrace::Settlement {
                origin,
                revision: 1,
                normalized: true,
            })
        );

        let recorded = events.len();
        app.undo_node(input)
            .expect("an edit with unchanged committed text records nothing");
        assert_eq!(trace(&app).len(), recorded, "undo dispatched no edit");
    }

    /// Store 字段与 Signal 一样：处理器写回旧值仍表示应用处理了这次编辑。
    #[component]
    fn StoreCappedAtOne() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));

        view! {
            <view>
                <button
                    aria:label="seed"
                    on:click={move || -> Result<(), WriteError> {
                        Transaction::new().write(store.insert("a")).finish()
                    }}
                >"seed"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                    {move |row: RowView<TodoRow>| {
                        let id = row.key();
                        view! {
                            <text-input
                                aria:label="store-same"
                                value={store.title(id)}
                                on:input={move |value: String| {
                                    let capped = value.chars().take(1).collect::<String>();
                                    let _ = Transaction::new()
                                        .write(store.set_title(id, capped))
                                        .finish();
                                }}
                            />
                        }
                    }}
                </For>
            </view>
        }
    }

    #[test]
    fn a_store_handler_that_writes_the_old_value_settles_the_edit() {
        let mut app = Runtime::new();
        app.mount(|| view! { <StoreCappedAtOne /> }).expect("mount");
        app.click_label("seed").expect("seed the row");
        let input = app
            .presentation()
            .expect("snapshot")
            .find_label("store-same")
            .expect("the input")
            .id();

        app.set_caret_node(input, 1).expect("caret");
        app.insert_text_node(input, "b").expect("edit");
        assert_eq!(
            text_of(&app, "store-same"),
            "a",
            "a store write of the old value still settles the edit"
        );

        let events = trace(&app);
        assert!(
            matches!(
                events.last(),
                Some(TextTrace::Settlement {
                    revision: 1,
                    normalized: true,
                    ..
                })
            ),
            "unexpected: {:?}",
            events.last()
        );
    }

    #[component]
    fn Toggle() -> MountResult<MountPlan> {
        let visible = signal(true);
        let draft = signal(String::from("abcd"));
        let shown = memo(move || visible.get());

        view! {
            <view>
                <Show when={shown}>
                    <text-input
                        aria:label="toggle"
                        value={draft}
                        on:input={move |value: String| draft.set(value)}
                    />
                </Show>
                <button aria:label="hide-toggle" on:click={move || visible.set(false)}>
                    "Hide"
                </button>
                <button aria:label="show-toggle" on:click={move || visible.set(true)}>
                    "Show"
                </button>
            </view>
        }
    }

    #[test]
    fn a_same_key_remount_starts_a_new_origin_and_revision() {
        let mut app = Runtime::new();
        app.mount(|| view! { <Toggle /> }).expect("mount");
        let first = input_id(&app, "toggle");
        app.set_caret_node(first, 4).expect("caret");
        app.insert_text_node(first, "X").expect("edit");

        app.click_label("hide-toggle").expect("hide the input");
        assert_eq!(app.node_status(first), NodeStatus::Stale);
        app.click_label("show-toggle")
            .expect("show the input again");
        let second = input_id(&app, "toggle");
        assert_ne!(first, second, "a remount is a new node identity");
        let before_undo = trace(&app).len();
        app.undo_node(second)
            .expect("the remount starts with an empty history");
        assert_eq!(trace(&app).len(), before_undo, "undo dispatched no edit");
        app.set_caret_node(second, 0).expect("caret");
        app.insert_text_node(second, "Y").expect("edit the remount");
        assert_eq!(
            app.insert_text_node(first, "Z"),
            Err(DispatchError::StaleNode(first)),
            "a delivery to the old mount is rejected"
        );
        assert_eq!(app.undo_node(first), Err(DispatchError::StaleNode(first)));

        let events = trace(&app);
        let origins: Vec<u64> = events
            .iter()
            .filter_map(|event| match event {
                TextTrace::Remount { origin } => Some(*origin),
                _ => None,
            })
            .collect();
        assert_eq!(origins.len(), 2, "each mount records its origin");
        assert_ne!(origins[0], origins[1], "a remount gets a fresh origin");
        assert!(
            events.contains(&TextTrace::LocalEdit {
                origin: origins[0],
                revision: 1,
            }),
            "the first mount starts at revision one"
        );
        assert!(
            events.contains(&TextTrace::LocalEdit {
                origin: origins[1],
                revision: 1,
            }),
            "the remount restarts revisions"
        );
        assert!(
            events.contains(&TextTrace::Settlement {
                origin: origins[1],
                revision: 1,
                normalized: false,
            }),
            "the remounted input settles its own edit"
        );
    }

    /// 最新编辑先结算后，旧 revision 的晚到值不能被当成新的当前结算。
    #[test]
    fn a_settled_edit_keeps_older_late_values_stale() {
        let mut app = Runtime::new();
        app.mount(|| view! { <Deferred /> }).expect("mount");
        let input = input_id(&app, "deferred");

        app.set_caret_node(input, 1).expect("caret");
        app.insert_text_node(input, "b").expect("first edit");
        app.set_caret_node(input, 2).expect("caret");
        app.insert_text_node(input, "c").expect("second edit");

        app.click_label("second")
            .expect("deliver the newest edit's value");
        assert_eq!(text_of(&app, "deferred"), "abc");
        app.click_label("first")
            .expect("deliver the older edit's value");
        assert_eq!(
            text_of(&app, "deferred"),
            "abc",
            "a settled revision must stay stale"
        );

        let events = trace(&app);
        let origin = match events[0] {
            TextTrace::Remount { origin } => origin,
            other => panic!("the first event is the mount: {other:?}"),
        };
        assert_eq!(
            events.last(),
            Some(&TextTrace::StaleSettlement {
                origin,
                current: 2,
                stale: 1,
            })
        );

        app.undo_node(input)
            .expect("the stale delivery kept the history");
        assert_eq!(text_of(&app, "deferred"), "ab");
    }

    /// Memo 值绑定读取 draft 与 cap；处理器只把 cap 写成相同值，推迟 draft。
    #[component]
    fn DeferredMemo() -> MountResult<MountPlan> {
        let draft = signal(String::from("a"));
        let cap = signal(10u32);
        let shown = memo(move || {
            draft
                .get()
                .chars()
                .take(cap.get() as usize)
                .collect::<String>()
        });

        view! {
            <text-input
                aria:label="deferred-memo"
                value={shown}
                on:input={move |_value: String| cap.set(cap.get())}
            />
        }
    }

    #[test]
    fn a_deferred_memo_write_does_not_roll_back_the_edit() {
        let mut app = Runtime::new();
        app.mount(|| view! { <DeferredMemo /> }).expect("mount");
        let input = input_id(&app, "deferred-memo");

        app.set_caret_node(input, 1).expect("caret");
        app.insert_text_node(input, "b").expect("edit");
        assert_eq!(
            text_of(&app, "deferred-memo"),
            "ab",
            "a written memo dependency is not the value write"
        );

        let events = trace(&app);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, TextTrace::Settlement { .. })),
            "an equal dependency write must not settle the edit"
        );
    }

    /// Memo 绑定的计算值随处理器写入变化时仍按结算处理。
    #[component]
    fn MemoIdentity() -> MountResult<MountPlan> {
        let draft = signal(String::from("a"));
        let shown = memo(move || draft.get());

        view! {
            <text-input
                aria:label="memo-identity"
                value={shown}
                on:input={move |value: String| draft.set(value)}
            />
        }
    }

    #[test]
    fn a_memo_value_binding_settles_a_changed_value() {
        let mut app = Runtime::new();
        app.mount(|| view! { <MemoIdentity /> }).expect("mount");
        let input = input_id(&app, "memo-identity");

        app.set_caret_node(input, 1).expect("caret");
        app.insert_text_node(input, "b").expect("edit");
        assert_eq!(text_of(&app, "memo-identity"), "ab");

        let events = trace(&app);
        let origin = match events[0] {
            TextTrace::Remount { origin } => origin,
            other => panic!("the first event is the mount: {other:?}"),
        };
        assert_eq!(
            events.last(),
            Some(&TextTrace::Settlement {
                origin,
                revision: 1,
                normalized: false,
            })
        );
    }

    #[component]
    fn Resettable(calls: Rc<Cell<u32>>) -> MountResult<MountPlan> {
        let draft = signal(String::from("abcd"));

        view! {
            <view>
                <text-input
                    aria:label="reset"
                    value={draft}
                    on:input={move |value: String| {
                        calls.set(calls.get() + 1);
                        draft.set(value);
                    }}
                />
                <button
                    aria:label="external"
                    on:click={move || draft.set(String::from("remote"))}
                >
                    "External"
                </button>
            </view>
        }
    }

    #[test]
    fn an_external_reset_during_composition_traces_the_cancellation() {
        let calls = Rc::new(Cell::new(0));
        let mut app = Runtime::new();
        app.mount({
            let calls = calls.clone();
            move || view! { <Resettable calls={calls} /> }
        })
        .expect("mount");
        let input = input_id(&app, "reset");

        app.set_selection_node(input, 1, 3).expect("selection");
        app.compose_start_node(input).expect("start composing");
        app.compose_update_node(input, "ni").expect("preedit");
        assert_eq!(
            app.presentation()
                .expect("snapshot")
                .find_label("reset")
                .expect("the input")
                .edit()
                .expect("edit")
                .preedit(),
            Some("ni")
        );

        app.click_label("external")
            .expect("replace the value from outside");
        let snapshot = app.presentation().expect("snapshot");
        let field = snapshot.find_label("reset").expect("the input");
        assert_eq!(field.text_value(), Some("remote"));
        let edit = field.edit().expect("edit");
        assert_eq!((edit.caret(), edit.anchor()), (6, 6));
        assert_eq!(edit.selection(), None);
        assert_eq!(edit.preedit(), None);
        assert_eq!(edit.composition(), None);
        assert_eq!(calls.get(), 0, "the cancellation dispatches no on:input");

        let events = trace(&app);
        let origin = match events[0] {
            TextTrace::Remount { origin } => origin,
            other => panic!("the first event is the mount: {other:?}"),
        };
        assert_eq!(
            events.last(),
            Some(&TextTrace::CompositionCancelledByReset {
                origin,
                revision: 0,
            })
        );
    }
}

#[cfg(test)]
mod recycling_tests {
    #![allow(unused_braces)]

    use super::*;
    use crate::prelude::*;

    /// 一行一个组件、一个元素与一个 click handler：覆盖元素、组件与事件槽位。
    #[component]
    fn RecycledRow(id: TodoId, store: Store) -> MountResult<MountPlan> {
        let title = store.title(id).read_once()?;
        let label = format!("Row {title}");
        view! {
            <view style:height={once(px(20.0))}>
                <button aria:label={once(label)} on:click={move || {}}>"row"</button>
            </view>
        }
    }

    #[component]
    fn RecycledList(store: Store) -> MountResult<MountPlan> {
        let rows = memo(move || store.rows(Filter::All));
        let seed = move || {
            let mut transaction = Transaction::new();
            for index in 0..200 {
                transaction = transaction.write(store.insert(format!("r{index}")));
            }
            transaction.finish().expect("seed the rows");
        };
        view! {
            <view role="main">
                <button aria:label="Seed" on:click={move || seed()}>"Seed"</button>
                <view class="gap-0">
                    <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                        {move |row: RowView<TodoRow>| view! {
                            <RecycledRow id={row.key()} store={store} />
                        }}
                    </For>
                </view>
            </view>
        }
    }

    fn fixture() -> (Runtime, WindowCtx, u64) {
        let mut app = Runtime::new();
        let store = app.setup(Store::new).expect("application setup");
        app.declare_windows(move |catalog| {
            catalog.window(
                WindowSpec::new("main").primary().sized(900, 700),
                move |_| view! { <RecycledList store={store} /> },
            )
        })
        .expect("declare the window");
        let window = app.show("main").expect("show the window");
        app.click_label("Seed").expect("seed the rows");
        let band = app.presentation_of(window).expect("snapshot").row_bands()[0].key();
        (app, window, band)
    }

    /// 当前所有挂载存储的长度；槽位复用时这些长度不随挂载次数增长。
    fn store_lengths(app: &Runtime) -> (usize, usize, usize, usize) {
        let state = app.state.borrow();
        (
            state.elements.len(),
            state.components.len(),
            state.events.len(),
            state.owners.len(),
        )
    }

    #[test]
    fn windowed_remounts_recycle_slots_and_keep_stale_ids() {
        let (mut app, window, band) = fixture();
        let baseline = store_lengths(&app);
        let first = app
            .presentation_of(window)
            .expect("snapshot")
            .find_label("Row r0")
            .expect("row zero")
            .id();

        // 两个窗口尺寸来回切换：挂载与卸载交错，槽位只应稳定在第一个
        // 生命周期内的峰值，而不是随切换次数增长。
        for _ in 0..40 {
            assert!(
                app.set_row_window(window, band, 40, 10)
                    .expect("shrink the row window")
                    .changed()
            );
            assert!(
                app.set_row_window(window, band, 0, 32)
                    .expect("restore the row window")
                    .changed()
            );
        }
        let steady = store_lengths(&app);
        assert_eq!(
            app.node_status(first),
            NodeStatus::Stale,
            "the first row was unmounted and remounted with a new identity"
        );
        for _ in 0..40 {
            assert!(
                app.set_row_window(window, band, 40, 10)
                    .expect("shrink the row window")
                    .changed()
            );
            assert!(
                app.set_row_window(window, band, 0, 32)
                    .expect("restore the row window")
                    .changed()
            );
        }
        assert_eq!(
            store_lengths(&app),
            steady,
            "repeated remounts reuse the same slots"
        );
        assert!(
            steady.0 <= baseline.0 + 32
                && steady.1 <= baseline.1 + 16
                && steady.2 <= baseline.2 + 16
                && steady.3 <= baseline.3 + 16,
            "the steady state stays near the live window: baseline {baseline:?}, steady {steady:?}"
        );
    }
}

#[cfg(test)]
mod arena_tests {
    #![allow(unused_braces)]

    use super::*;
    use crate::node::NodeError;
    use crate::prelude::*;

    /// 覆盖 For 行：每个行组件拥有元素与文本节点。
    #[component]
    fn ArenaRow(id: TodoId, store: Store) -> MountResult<MountPlan> {
        let title = store.title(id).read_once()?;
        let label = format!("Row {title}");
        view! {
            <view>
                <text>{once(label)}</text>
            </view>
        }
    }

    /// 覆盖 Show 的活动分支与卸载后的分支释放。
    #[component]
    fn ArenaShow() -> MountResult<MountPlan> {
        let visible = signal(true);
        let shown = memo(move || visible.get());
        view! {
            <view>
                <Show when={shown}>
                    <view aria:label="branch"><text>"shown"</text></view>
                </Show>
                <button
                    aria:label="toggle-show"
                    on:click={move || visible.set(!visible.get())}
                >"toggle"</button>
            </view>
        }
    }

    /// 覆盖元素、文本、组件、Show 与 For 五种节点。
    #[component]
    fn ArenaList(store: Store) -> MountResult<MountPlan> {
        let rows = memo(move || store.rows(Filter::All));
        let seed = move || {
            let mut transaction = Transaction::new();
            for index in 0..2 {
                transaction = transaction.write(store.insert(format!("r{index}")));
            }
            transaction.finish().expect("seed the rows");
        };
        view! {
            <view role="main">
                <button aria:label="Seed" on:click={move || seed()}>"Seed"</button>
                <text>"plain"</text>
                <ArenaShow />
                <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                    {move |row: RowView<TodoRow>| view! {
                        <ArenaRow id={row.key()} store={store} />
                    }}
                </For>
            </view>
        }
    }

    fn fixture() -> (Runtime, WindowCtx) {
        let mut app = Runtime::new();
        let store = app.setup(Store::new).expect("application setup");
        app.declare_windows(move |catalog| {
            catalog.window(WindowSpec::new("main").primary(), move |_| {
                view! { <ArenaList store={store} /> }
            })
        })
        .expect("declare the window");
        let window = app.show("main").expect("show the window");
        app.click_label("Seed").expect("seed the rows");
        (app, window)
    }

    /// 当前由 owner 回收的 arena 节点；覆盖文本与结构节点的登记路径。
    fn owned_nodes(state: &State) -> Vec<(NodeRef, NodeKindTag)> {
        let mut nodes = Vec::new();
        for record in state.owners.values() {
            for resource in &record.resources {
                if let Resource::Node(node) = resource
                    && let Ok(slot) = state.nodes.get(*node)
                {
                    nodes.push((*node, slot.kind.expect("listed nodes are live")));
                }
            }
        }
        nodes
    }

    fn count_kind(nodes: &[(NodeRef, NodeKindTag)], kind: NodeKindTag) -> usize {
        nodes.iter().filter(|(_, actual)| *actual == kind).count()
    }

    #[test]
    fn every_mounted_kind_is_reachable_through_the_node_arena() {
        let (app, window) = fixture();
        let state = app.state.borrow();
        let tree = state.arena_trees[&TreeSlot::Window(window.key)];
        let nodes = owned_nodes(&state);

        assert!(count_kind(&nodes, NodeKindTag::Element) >= 5);
        let texts = count_kind(&nodes, NodeKindTag::Text);
        assert!(texts >= 3);
        assert!(count_kind(&nodes, NodeKindTag::Component) >= 3);
        assert_eq!(count_kind(&nodes, NodeKindTag::Show), 1);
        assert_eq!(count_kind(&nodes, NodeKindTag::For), 1);
        assert_eq!(
            state.nodes.texts.live(),
            texts,
            "every text node owns one text payload"
        );
        assert_eq!(state.nodes.shows.live(), 1);
        assert_eq!(state.nodes.forces.live(), 1);

        let mut roots = 0;
        let mut elements = 0;
        for (node, kind) in &nodes {
            let slot = state.nodes.get(*node).expect("listed nodes are live");
            assert_eq!(
                slot.tree, tree,
                "every mounted node belongs to the window tree"
            );
            match slot.parent {
                Some(parent) => {
                    assert!(
                        state.nodes.get(parent).is_ok(),
                        "the parent link never dangles"
                    );
                }
                None => roots += 1,
            }
            match kind {
                NodeKindTag::Element => {
                    elements += 1;
                    let payload = slot.payload as usize;
                    let element = state
                        .elements
                        .get(payload)
                        .expect("the element payload index is live");
                    assert_eq!(
                        element.id.node(),
                        *node,
                        "the element resolves back to its node identity"
                    );
                    assert_eq!(
                        state.tree_for_element(payload),
                        Some(TreeSlot::Window(window.key))
                    );
                    let aux = state.element_aux(payload);
                    for handle in [
                        aux.attrs.raw(),
                        aux.class_styles.raw(),
                        aux.inline_styles.raw(),
                        aux.events.raw(),
                        aux.edit.raw(),
                        aux.bindings.raw(),
                        aux.semantics.raw(),
                    ] {
                        assert_ne!(handle, u32::MAX, "every element owns auxiliary handles");
                    }
                }
                NodeKindTag::Component => {
                    let payload = slot.payload as usize;
                    assert!(
                        state.components.get(payload).is_some(),
                        "the component payload index is live"
                    );
                }
                NodeKindTag::Text | NodeKindTag::Show | NodeKindTag::For => {}
            }
        }
        assert_eq!(roots, 1, "only the window root has no parent");
        assert_eq!(
            state.nodes.aux_live(),
            elements * 7,
            "one auxiliary handle set per live element"
        );
    }

    #[test]
    fn hiding_a_show_branch_releases_nodes_and_aux_handles() {
        let (mut app, _window) = fixture();
        let branch = app
            .presentation()
            .expect("snapshot")
            .find_label("branch")
            .expect("the active branch")
            .id();
        assert_eq!(
            branch.presentation_key().raw(),
            branch.node().raw(),
            "NodeId and PresentationKey share one encoding"
        );

        let before = {
            let state = app.state.borrow();
            (state.nodes.live(), state.nodes.storage_len())
        };
        let (aux_before, texts_before, shows_before, forces_before) = {
            let state = app.state.borrow();
            (
                state.nodes.aux_live(),
                state.nodes.texts.live(),
                state.nodes.shows.live(),
                state.nodes.forces.live(),
            )
        };
        assert_eq!(aux_before % 7, 0);

        app.click_label("toggle-show").expect("hide the branch");
        assert!(!app.texts().iter().any(|text| text == "shown"));
        assert_eq!(
            app.node_status(branch),
            NodeStatus::Stale,
            "the hide retires the old element identity"
        );
        {
            let state = app.state.borrow();
            assert_eq!(
                state.nodes.live(),
                before.0 - 3,
                "the branch view, its text element and raw text node were released"
            );
            assert_eq!(
                state.nodes.storage_len(),
                before.1,
                "release keeps slots for reuse"
            );
            assert_eq!(
                state.nodes.aux_live(),
                aux_before - 14,
                "both retired elements return their auxiliary handles"
            );
            assert_eq!(
                state.nodes.texts.live(),
                texts_before - 1,
                "the branch text payload is released"
            );
            assert_eq!(state.nodes.shows.live(), shows_before);
            assert_eq!(state.nodes.forces.live(), forces_before);
            assert!(
                matches!(
                    state.nodes.get(branch.node()),
                    Err(NodeError::Vacant { .. } | NodeError::StaleGeneration { .. })
                ),
                "the arena rejects the retired identity"
            );
        }

        app.click_label("toggle-show")
            .expect("show the branch again");
        let fresh = app
            .presentation()
            .expect("snapshot")
            .find_label("branch")
            .expect("the remounted branch")
            .id();
        assert_ne!(fresh, branch, "a reused arena slot gets a new generation");
        assert_eq!(app.node_status(fresh), NodeStatus::Live);
        let state = app.state.borrow();
        assert_eq!(state.nodes.live(), before.0, "the live set is restored");
        assert_eq!(
            state.nodes.storage_len(),
            before.1,
            "the remount reuses arena slots"
        );
        assert_eq!(
            state.nodes.aux_live(),
            aux_before,
            "the remount reserves a fresh handle set"
        );
        assert_eq!(state.nodes.texts.live(), texts_before);
        assert_eq!(state.nodes.shows.live(), shows_before);
        assert_eq!(state.nodes.forces.live(), forces_before);
    }
}

#[cfg(test)]
mod binding_tests {
    #![allow(unused_braces)]

    use super::*;
    use crate::prelude::*;
    use crate::view::{attr, child, element, style_color, styled_element};

    #[test]
    fn a_completed_flush_drains_pending_changes() {
        #[component]
        fn PendingHost(probe: Rc<Cell<usize>>) -> MountResult<MountPlan> {
            let store = Store::new();
            let rows = memo(move || store.rows(Filter::All));

            view! {
                <view>
                    <button aria:label="commit" on:click={move || {
                        Transaction::new()
                            .write(store.insert("A"))
                            .finish()
                            .expect("seed the row");
                        // 处理器执行时 flush 尚未开始：累计器已经看到提交。
                        crate::context::with_state(|state| {
                            probe.set(state.pending.borrow().len());
                        });
                    }}>"Commit"</button>
                    <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                        {move |_row: RowView<TodoRow>| view! { <text>"row"</text> }}
                    </For>
                </view>
            }
        }

        let probe = Rc::new(Cell::new(0usize));
        let mut app = Runtime::new();
        app.mount({
            let probe = Rc::clone(&probe);
            move || view! { <PendingHost probe={probe} /> }
        })
        .expect("mount");
        let pending = Rc::clone(&app.state.borrow().pending);

        app.click_label("commit").expect("commit during the turn");
        assert_eq!(
            probe.get(),
            1,
            "the handler observes the commit before the flush"
        );
        assert!(
            pending.borrow().is_empty(),
            "the completed flush drains the consumed PendingChanges"
        );
    }

    #[component]
    fn PatchHost() -> MountResult<MountPlan> {
        let expanded = signal(false);
        let label = signal(String::from("a"));
        view! {
            <view>
                <Show when={expanded}><text>"shown"</text></Show>
                <text>{label}</text>
                <button
                    aria:label="go"
                    on:click={move || {
                        expanded.set(true);
                        label.set(String::from("b"));
                    }}
                >"go"</button>
            </view>
        }
    }

    #[test]
    fn a_turn_records_structure_patches_before_value_patches() {
        let mut app = Runtime::new();
        app.mount(|| view! { <PatchHost /> }).expect("mount");
        app.click_label("go").expect("toggle the frame");

        let mut state = app.state.borrow_mut();
        let patches = state.binding_patches.take();
        let show = patches
            .iter()
            .position(|patch| patch.part == BindingPart::ShowWhen)
            .expect("the Show trigger changed");
        let text = patches
            .iter()
            .position(|patch| patch.part == BindingPart::Text)
            .expect("the text changed");
        assert!(
            show < text,
            "structure patches precede value patches: {:?}",
            patches.iter().map(|patch| patch.part).collect::<Vec<_>>()
        );
    }

    #[component]
    fn TwoValueHost() -> MountResult<MountPlan> {
        let left = signal(String::from("left-0"));
        let right = signal(String::from("right-0"));
        view! {
            <view>
                <text aria:label="left">{left}</text>
                <text aria:label="right">{right}</text>
                <button
                    aria:label="both"
                    on:click={move || {
                        left.set(String::from("left-1"));
                        right.set(String::from("right-1"));
                    }}
                >"both"</button>
            </view>
        }
    }

    #[test]
    fn a_presentation_update_publishes_a_cumulative_delta_for_one_turn() {
        let mut app = Runtime::new();
        app.mount(|| view! { <TwoValueHost /> }).expect("mount");
        let _ = app.refresh().expect("drain the mount report");
        let first = app.presentation_update().expect("the first publication");
        assert!(
            first.delta().is_none(),
            "the first publication has no baseline and is a full root"
        );
        let from = first.snapshot().revision();
        assert_eq!(from, app.presentation_revision());

        app.click_label("both").expect("change both values");
        let update = app.presentation_update().expect("the changed publication");
        let delta = update.delta().expect("a cumulative delta");
        assert_eq!(delta.from_revision(), from);
        assert_eq!(delta.to_revision(), update.snapshot().revision());
        assert_ne!(delta.to_revision(), from, "the turn advanced the revision");
        for label in ["left", "right"] {
            let key = update.snapshot().find_label(label).expect("label").key();
            assert!(
                delta.changed().contains(&key),
                "{label} is part of the cumulative delta: {:?}",
                delta.changed()
            );
        }
    }

    #[component]
    fn SharedWindowText(shared: Signal<String>) -> MountResult<MountPlan> {
        view! {
            <view>
                <text aria:label="shared">{shared}</text>
                <button
                    aria:label="update"
                    on:click={move || shared.set(String::from("updated"))}
                >"Update"</button>
            </view>
        }
    }

    #[test]
    fn a_hidden_window_publishes_a_full_root_after_show() {
        let mut app = Runtime::new();
        let shared = app
            .setup(|| signal(String::from("initial")))
            .expect("application setup");
        app.declare_windows(move |catalog| {
            catalog.window(WindowSpec::new("a").primary(), move |_| {
                view! { <SharedWindowText shared={shared} /> }
            })?;
            catalog.window(WindowSpec::new("b"), move |_| {
                view! { <SharedWindowText shared={shared} /> }
            })?;
            Ok(())
        })
        .expect("declare the windows");

        app.show("a").expect("show a");
        let b = app.show("b").expect("show b");
        let first = app
            .presentation_update_of(b)
            .expect("the first b publication");
        assert!(first.delta().is_none());
        assert_eq!(
            first.snapshot().revision(),
            app.presentation_revision_of(b).expect("b revision")
        );

        app.hide(b).expect("hide b");
        assert!(matches!(
            app.presentation_update_of(b),
            Err(WindowError::Hidden)
        ));
        let hidden_revision = app
            .presentation_revision_of(b)
            .expect("the hidden tree revision");
        app.click_label("update")
            .expect("update the shared value from the visible window");
        assert!(
            app.presentation_revision_of(b).expect("revision") > hidden_revision,
            "the hidden tree flushes the shared write synchronously"
        );

        let restored = app.show("b").expect("show b again");
        let update = app
            .presentation_update_of(restored)
            .expect("the publication after show");
        assert!(
            update.delta().is_none(),
            "a shown window publishes a full root, not a guessed delta"
        );
        assert_eq!(
            update.snapshot().revision(),
            app.presentation_revision_of(restored).expect("revision")
        );
        assert_eq!(
            update.snapshot().find_label("shared").expect("text").text(),
            "updated",
            "the full root already contains the flush performed while hidden"
        );
    }

    #[component]
    fn RootShowHost() -> MountResult<MountPlan> {
        let show = signal(false);
        view! {
            <Show when={show}><text aria:label="first">"first"</text></Show>
            <text aria:label="second">"second"</text>
            <button
                aria:label="toggle"
                on:click={move || show.set(!show.get())}
            >"toggle"</button>
        }
    }

    #[test]
    fn a_root_list_change_publishes_a_full_root_instead_of_an_empty_delta() {
        let mut app = Runtime::new();
        app.mount(|| view! { <RootShowHost /> }).expect("mount");
        let first = app.presentation_update().expect("the first publication");
        assert!(first.delta().is_none());
        let before = first.snapshot().roots().len();

        app.click_label("toggle").expect("add a top-level root");
        let update = app.presentation_update().expect("the changed publication");
        assert!(
            update.delta().is_none(),
            "a top-level root list change has no keyed representation"
        );
        assert_eq!(update.snapshot().roots().len(), before + 1);
    }

    #[component]
    fn NestedToggleHost() -> MountResult<MountPlan> {
        let show = signal(true);
        view! {
            <view>
                <Show when={show}><text aria:label="detail">"detail"</text></Show>
                <button
                    aria:label="toggle"
                    on:click={move || show.set(!show.get())}
                >"toggle"</button>
            </view>
        }
    }

    #[test]
    fn a_nested_removal_publishes_removed_keys_and_flags() {
        let mut app = Runtime::new();
        app.mount(|| view! { <NestedToggleHost /> }).expect("mount");
        let first = app.presentation_update().expect("the first publication");
        assert!(first.delta().is_none());
        let root = first
            .snapshot()
            .roots()
            .first()
            .and_then(SnapshotNode::as_element)
            .expect("the root view");
        let root_key = root.key();
        assert_eq!(first.snapshot().roots().len(), 1, "the view stays the root");

        app.click_label("toggle").expect("remove the detail text");
        let update = app.presentation_update().expect("the changed publication");
        let delta = update.delta().expect("a cumulative delta");
        assert!(
            !delta.removed().is_empty(),
            "the removed element and its text node are listed"
        );
        assert_eq!(
            delta.removed_subtrees().len(),
            1,
            "the removed subtree has one root"
        );
        assert!(
            delta
                .removed_subtrees()
                .iter()
                .all(|key| delta.removed().contains(key)),
            "subtree roots are part of the removed set"
        );
        assert!(
            delta.changed().contains(&root_key),
            "the surviving parent is on the changed path"
        );
        assert_eq!(
            delta.layout_roots(),
            &[root_key],
            "the topmost dirty node is the surviving root view"
        );
    }

    #[test]
    fn a_focus_change_publishes_its_flag() {
        let mut app = Runtime::new();
        app.mount(|| view! { <NestedToggleHost /> }).expect("mount");
        let first = app.presentation_update().expect("the first publication");
        assert!(first.delta().is_none());
        let button = first
            .snapshot()
            .find_label("toggle")
            .expect("the toggle button")
            .id();

        app.focus_node(button).expect("focus the button");
        let update = app.presentation_update().expect("the changed publication");
        let delta = update.delta().expect("a delta");
        assert!(delta.focus_changed(), "the committed focus moved");
        assert_eq!(update.snapshot().focus(), Some(button));
    }

    #[test]
    fn an_appearance_change_publishes_its_flag() {
        let mut app = Runtime::new();
        app.mount(|| view! { <NestedToggleHost /> }).expect("mount");
        let first = app.presentation_update().expect("the first publication");
        assert!(first.delta().is_none());

        app.set_appearance(Appearance::Dark)
            .expect("switch the appearance");
        let update = app.presentation_update().expect("the changed publication");
        let delta = update.delta().expect("a delta");
        assert!(delta.appearance_changed());
        assert_eq!(update.snapshot().appearance(), Appearance::Dark);
    }

    #[test]
    fn a_row_window_change_publishes_row_band_and_node_changes() {
        let (mut app, window, bands) = two_band_fixture();
        let first = app
            .presentation_update_of(window)
            .expect("the first publication");
        assert!(first.delta().is_none());

        app.set_row_window(window, bands[0], 4, 8)
            .expect("scroll the first band");
        let update = app
            .presentation_update_of(window)
            .expect("the changed publication");
        let delta = update.delta().expect("a delta");
        assert!(delta.row_bands_changed(), "the row band metadata changed");
        assert!(
            !delta.changed().is_empty()
                || !delta.inserted().is_empty()
                || !delta.removed().is_empty(),
            "entering and leaving rows are keyed"
        );
    }

    #[component]
    fn TwoTaskHost(window: WindowCtx) -> MountResult<MountPlan> {
        let left = signal(String::from("left-0"));
        let right = signal(String::from("right-0"));
        let scope = window.task_scope();
        view! {
            <view>
                <text aria:label="left">{left}</text>
                <text aria:label="right">{right}</text>
                <button
                    aria:label="spawn"
                    on:click={move || {
                        if let Ok(task) = scope.spawn(|_| String::from("left-1")) {
                            let _ = task.on_done(move |value| {
                                left.set(value);
                                Ok(())
                            });
                        }
                        if let Ok(task) = scope.spawn(|_| String::from("right-1")) {
                            let _ = task.on_done(move |value| {
                                right.set(value);
                                Ok(())
                            });
                        }
                    }}
                >"spawn"</button>
            </view>
        }
    }

    #[test]
    fn multiple_delivery_commits_publish_one_cumulative_delta() {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |window| {
                view! { <TwoTaskHost window={window} /> }
            })
        })
        .expect("declare the window");
        let window = app.show("main").expect("show the window");
        let first = app
            .presentation_update_of(window)
            .expect("the first publication");
        assert!(first.delta().is_none());
        let from = first.snapshot().revision();

        app.click_label("spawn").expect("spawn both tasks");
        let before_settle = app.presentation_revision_of(window).expect("revision");
        app.settle_tasks().expect("deliver both results");
        let after_settle = app.presentation_revision_of(window).expect("revision");
        assert!(
            after_settle >= before_settle + 2,
            "each delivery commits its own revision"
        );

        let update = app
            .presentation_update_of(window)
            .expect("the cumulative publication");
        let delta = update.delta().expect("a cumulative delta");
        assert_eq!(
            delta.from_revision(),
            from,
            "the delta starts at the last publication, not the last commit"
        );
        assert_eq!(delta.to_revision(), after_settle);
        for label in ["left", "right"] {
            let key = update.snapshot().find_label(label).expect("label").key();
            assert!(
                delta.changed().contains(&key),
                "both delivery commits are in the delta: {label}"
            );
        }
    }

    #[component]
    fn NestedShowHost() -> MountResult<MountPlan> {
        let outer = signal(true);
        let inner = signal(false);
        view! {
            <view>
                <Show when={outer}>
                    <Show when={inner}><text>"nested"</text></Show>
                    <button
                        aria:label="inner"
                        on:click={move || inner.set(!inner.get())}
                    >"toggle"</button>
                </Show>
                <button
                    aria:label="outer"
                    on:click={move || outer.set(!outer.get())}
                >"outer"</button>
            </view>
        }
    }

    #[test]
    fn a_nested_show_updates_while_the_outer_branch_stays_mounted() {
        let mut app = Runtime::new();
        app.mount(|| view! { <NestedShowHost /> }).expect("mount");
        assert!(app.texts().iter().all(|text| text != "nested"));

        app.click_label("inner").expect("show the nested branch");
        assert!(
            app.texts().iter().any(|text| text == "nested"),
            "the nested Show re-evaluates while the outer branch is unchanged"
        );
        app.click_label("inner").expect("hide the nested branch");
        assert!(app.texts().iter().all(|text| text != "nested"));
    }

    #[component]
    fn ForNestedShowHost() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        let inner = signal(false);
        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    Transaction::new().write(store.insert("row")).finish()?;
                    Ok(())
                }}>"seed"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                    {move |_row: RowView<TodoRow>| view! {
                        <view>
                            <Show when={inner}><text>"row-inner"</text></Show>
                            <text>"row"</text>
                        </view>
                    }}
                </For>
                <button
                    aria:label="inner"
                    on:click={move || inner.set(!inner.get())}
                >"inner"</button>
            </view>
        }
    }

    #[test]
    fn a_show_inside_a_for_row_updates_with_the_row_mounted() {
        let mut app = Runtime::new();
        app.mount(|| view! { <ForNestedShowHost /> })
            .expect("mount");
        app.click_label("seed").expect("seed a row");
        assert_eq!(app.for_items().len(), 1);
        assert!(app.texts().iter().all(|text| text != "row-inner"));

        app.click_label("inner").expect("show the nested branch");
        assert!(
            app.texts().iter().any(|text| text == "row-inner"),
            "structure triggers inside mounted rows still run"
        );
    }

    #[test]
    fn static_and_once_text_is_committed_at_mount_and_never_patched() {
        let mut app = Runtime::new();
        app.mount(|| {
            view! {
                <view>
                    <text>"static"</text>
                    <text>{once(String::from("snapshot"))}</text>
                </view>
            }
        })
        .expect("mount");
        assert_eq!(
            app.texts(),
            vec![String::from("static"), String::from("snapshot")],
            "static/once values are committed before the first refresh"
        );

        // 丢弃挂载期的补丁；下一次刷新不应再为静态槽产生任何补丁。
        let _ = app.state.borrow_mut().binding_patches.take();
        app.refresh().expect("refresh");
        let mut state = app.state.borrow_mut();
        assert!(
            state.binding_patches.take().is_empty(),
            "static and once slots are written once at mount"
        );
        for tree in state.trees.values() {
            for node in &tree.nodes {
                let MountedNode::Text(payload) = node else {
                    continue;
                };
                let data = state.nodes.text(*payload);
                let slot = state
                    .nodes
                    .binding_table(data.tree)
                    .and_then(|table| table.slot::<String>(data.binding))
                    .expect("a mounted text slot");
                assert!(slot.source.is_static());
                assert!(slot.current.is_some(), "the mount snapshot stays committed");
            }
        }
    }

    fn live_binding_slots(app: &Runtime) -> usize {
        let state = app.state.borrow();
        state
            .nodes
            .binding_tables
            .values()
            .map(BindingTable::live)
            .sum()
    }

    #[component]
    fn ToggleText() -> MountResult<MountPlan> {
        let visible = signal(true);
        let label = signal(String::from("branch"));
        view! {
            <view>
                <Show when={visible}>
                    <view aria:label="branch">
                        <text>{label}</text>
                        <button disabled={once(false)}>"inside"</button>
                    </view>
                </Show>
                <button
                    aria:label="toggle"
                    on:click={move || visible.set(!visible.get())}
                >"toggle"</button>
            </view>
        }
    }

    #[test]
    fn a_show_branch_switch_returns_its_binding_slots() {
        let mut app = Runtime::new();
        app.mount(|| view! { <ToggleText /> }).expect("mount");
        let mounted = live_binding_slots(&app);
        assert!(mounted > 0);

        app.click_label("toggle").expect("hide the branch");
        let hidden = live_binding_slots(&app);
        assert!(
            hidden < mounted,
            "hiding the branch returns its text and attr slots"
        );
        app.click_label("toggle").expect("show the branch");
        assert_eq!(
            live_binding_slots(&app),
            mounted,
            "the remounted branch allocates the same number of slots"
        );
        app.click_label("toggle").expect("hide the branch again");
        assert_eq!(
            live_binding_slots(&app),
            hidden,
            "repeated mount/unmount does not grow the binding table"
        );
    }

    #[test]
    fn closing_a_window_releases_every_binding_slot_and_list() {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| {
                view! {
                    <view>
                        <text>"hello"</text>
                        <button
                            aria:label="go"
                            disabled={once(false)}
                            style:width={once(px(10.0))}
                        >"go"</button>
                    </view>
                }
            })
        })
        .expect("declare the window");
        let window = app.show("main").expect("show the window");
        {
            let state = app.state.borrow();
            assert!(
                state
                    .nodes
                    .binding_tables
                    .values()
                    .any(|table| table.live() > 0),
                "the mount created typed binding slots"
            );
            assert!(
                state.nodes.aux.binding_lists.live() > 0,
                "elements keep their binding lists"
            );
        }

        app.close_window(window).expect("close the window");
        let state = app.state.borrow();
        assert!(
            state.nodes.binding_tables.is_empty(),
            "the retired tree drops its binding table"
        );
        assert_eq!(
            state.nodes.aux.binding_lists.live(),
            0,
            "every element returns its binding list"
        );
    }

    #[component]
    fn ForHost() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        view! {
            <view>
                <button aria:label="Seed" on:click={move || -> Result<(), WriteError> {
                    let mut transaction = Transaction::new();
                    for index in 0..2 {
                        transaction = transaction.write(store.insert(format!("r{index}")));
                    }
                    transaction.finish()?;
                    Ok(())
                }}>"Seed"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                    {move |row: RowView<TodoRow>| view! {
                        <text>{once(format!("row {}", row.key().raw()))}</text>
                    }}
                </For>
            </view>
        }
    }

    #[component]
    fn FaultFallbackHost() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    Transaction::new().write(store.insert("a")).finish()
                }}>"Seed"</button>
                <button aria:label="duplicate" on:click={move || -> Result<(), WriteError> {
                    Transaction::new().write(store.insert("b")).finish()
                }}>"Duplicate"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.title.len()}>
                    {move |row: RowView<TodoRow>| view! {
                        <text>{once(format!("row {}", row.get().title))}</text>
                    }}
                </For>
            </view>
        }
    }

    #[component]
    fn PanicText() -> MountResult<MountPlan> {
        let failing: Memo<String> = memo(|| panic!("virgin memo boom"));
        view! { <text>{failing}</text> }
    }

    #[component]
    fn RejectHost() -> MountResult<MountPlan> {
        let label = signal(String::from("a"));
        let open = signal(false);
        view! {
            <view>
                <Show when={open}>
                    <text>{label}</text>
                    <PanicText />
                </Show>
                <button
                    aria:label="go"
                    on:click={move || {
                        label.set(String::from("b"));
                        open.set(true);
                    }}
                >"go"</button>
            </view>
        }
    }

    #[test]
    fn a_rejected_show_branch_rolls_back_its_pre_refresh_patches() {
        let mut app = Runtime::new();
        app.mount(|| view! { <RejectHost /> }).expect("mount");
        let _ = app.refresh().expect("drain the mount turn");
        let revision = app.presentation_revision();
        let before = app.presentation().expect("snapshot");
        let failure = app
            .click_label("go")
            .expect_err("the proposed branch panics during pre-refresh");
        assert!(matches!(failure, MountError::Panic(_)));
        assert_eq!(
            app.presentation_revision(),
            revision,
            "a rejected branch is not a visible change"
        );
        assert_eq!(
            app.presentation().expect("snapshot"),
            before,
            "the snapshot is byte-identical after the rejection"
        );

        let mut state = app.state.borrow_mut();
        let patches = state.binding_patches.take();
        assert!(
            patches.iter().all(|patch| patch.part != BindingPart::Text),
            "patches for the rejected branch are rolled back: {:?}",
            patches.iter().map(|patch| patch.part).collect::<Vec<_>>()
        );
        assert!(
            patches
                .iter()
                .any(|patch| patch.part == BindingPart::ShowWhen),
            "the committed trigger patch is kept"
        );
    }

    #[test]
    fn a_root_first_refresh_failure_rolls_back_the_committed_tree() {
        let mut app = Runtime::new();
        let error = app
            .mount(|| view! { <PanicText /> })
            .expect_err("the first refresh evaluates the poisoned memo");
        assert!(matches!(error, MountError::Panic(_)));
        {
            let state = app.state.borrow();
            assert!(
                state.trees.is_empty(),
                "a failed mount leaves no committed root tree"
            );
            assert_eq!(state.nodes.live(), 0, "every node is released");
            assert!(
                state.nodes.binding_tables.is_empty(),
                "the failed root mount drops its binding table"
            );
            assert_eq!(
                state.nodes.aux.binding_lists.live(),
                0,
                "every binding list is returned"
            );
        }
        assert_eq!(app.live_memos(), 0, "the component memo is released");

        app.mount(|| view! { <text>"calm"</text> })
            .expect("the failed mount can be retried");
        assert_eq!(app.texts(), vec![String::from("calm")]);
    }

    #[component]
    fn WriteThenPanicShow() -> MountResult<MountPlan> {
        let store = Store::new();
        let failing: Memo<bool> = memo(move || {
            let _ = Transaction::new().write(store.insert("ghost")).finish();
            panic!("write then panic");
        });
        view! { <Show when={failing}><text>"x"</text></Show> }
    }

    #[test]
    fn a_failed_window_activation_discards_its_write_error() {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("a").primary(), |_| {
                view! { <text>"a"</text> }
            })?;
            catalog.window(WindowSpec::new("b"), |_| {
                view! { <WriteThenPanicShow /> }
            })?;
            Ok(())
        })
        .expect("declare the windows");
        app.show("a").expect("show the calm window");
        let _ = app.refresh().expect("drain the mount report");
        let error = app
            .show("b")
            .expect_err("the b source writes and then panics");
        assert!(matches!(error, WindowError::Mount(MountError::Panic(_))));
        let report = app
            .refresh()
            .expect("the aborted activation's write error is discarded");
        assert!(!report.changed(), "nothing visible changed");
    }

    #[component]
    fn LiveThenPanicText() -> MountResult<MountPlan> {
        let live = signal(String::from("live"));
        let failing: Memo<String> = memo(|| panic!("virgin memo boom"));
        view! { <view><text>{live}</text><text>{failing}</text></view> }
    }

    #[test]
    fn a_failed_window_first_refresh_removes_its_released_patches() {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| {
                view! { <LiveThenPanicText /> }
            })
        })
        .expect("declare the window");
        let error = app
            .show("main")
            .expect_err("the second text evaluates a poisoned memo");
        assert!(matches!(error, WindowError::Mount(MountError::Panic(_))));
        let mut state = app.state.borrow_mut();
        assert!(
            state.binding_patches.take().is_empty(),
            "no patch survives the released window tree"
        );
        assert!(
            state.nodes.binding_tables.is_empty(),
            "the failed window mount drops its binding table"
        );
    }

    #[component]
    fn WindowedRowsHost() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    let mut transaction = Transaction::new();
                    for index in 0..40 {
                        transaction = transaction.write(store.insert(format!("r{index}")));
                    }
                    transaction.finish()?;
                    Ok(())
                }}>"seed"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                    {move |row: RowView<TodoRow>| view! {
                        <text>{once(format!("row {}", row.key().raw()))}</text>
                    }}
                </For>
            </view>
        }
    }

    #[test]
    fn row_window_ticks_do_not_accumulate_stale_patches() {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| {
                view! { <WindowedRowsHost /> }
            })
        })
        .expect("declare the window");
        let window = app.show("main").expect("show the window");
        app.click_label("seed").expect("seed the rows");
        let band = app.presentation_of(window).expect("snapshot").row_bands()[0].key();
        for tick in 0..6 {
            let revision = app.presentation_revision_of(window).expect("revision");
            app.set_row_window(window, band, 4 + tick * 4, 4)
                .expect("scroll down");
            assert_ne!(
                app.presentation_revision_of(window).expect("revision"),
                revision,
                "tick {tick} advances the revision for a changed window"
            );
            let mut state = app.state.borrow_mut();
            let patches = state.binding_patches.parts();
            assert!(
                !patches.is_empty(),
                "tick {tick} changes the spacer heights"
            );
            for (tree, node, part) in &patches {
                assert!(
                    state.nodes.get(*node).is_ok(),
                    "tick {tick} left a patch for a released node: {part:?}"
                );
                assert_eq!(
                    state.nodes.tree_of(*node),
                    Some(*tree),
                    "tick {tick} patch names its tree"
                );
            }
            assert!(
                patches.len() <= 8,
                "tick {tick} keeps a bounded per-turn log: {}",
                patches.len()
            );
            let _ = state.binding_patches.take();
        }
    }

    #[component]
    fn ShowTextHost() -> MountResult<MountPlan> {
        let open = signal(false);
        let label = signal(String::from("a"));
        view! {
            <view>
                <Show when={open}><text>{label}</text></Show>
                <button
                    aria:label="open"
                    on:click={move || {
                        label.set(String::from("b"));
                        open.set(true);
                    }}
                >"open"</button>
            </view>
        }
    }

    #[test]
    fn a_show_mount_turn_evaluates_each_binding_once() {
        let mut app = Runtime::new();
        app.mount(|| view! { <ShowTextHost /> }).expect("mount");
        let _ = app.refresh().expect("drain the mount turn");
        let _ = crate::view::take_binding_evaluations();
        app.click_label("open").expect("open the branch");
        assert_eq!(
            crate::view::take_binding_evaluations(),
            2,
            "one when evaluation and one text evaluation in the turn"
        );
    }

    #[component]
    fn ForShowHost() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        let inner = signal(true);
        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    Transaction::new().write(store.insert("row")).finish()
                }}>"seed"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                    {move |_row: RowView<TodoRow>| view! {
                        <Show when={inner}><text>"row"</text></Show>
                    }}
                </For>
            </view>
        }
    }

    #[test]
    fn a_for_row_mount_evaluates_nested_triggers_once() {
        let mut app = Runtime::new();
        app.mount(|| view! { <ForShowHost /> }).expect("mount");
        let _ = app.refresh().expect("drain the mount turn");
        let _ = crate::view::take_binding_evaluations();
        app.click_label("seed").expect("seed a row");
        assert_eq!(
            crate::view::take_binding_evaluations(),
            2,
            "one ForEach evaluation and one nested when evaluation in the turn"
        );
    }

    #[component]
    fn ForRejectHost() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        let calls = Rc::new(Cell::new(0usize));
        let item_calls = calls.clone();
        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    Transaction::new()
                        .write(store.insert("A"))
                        .write(store.insert("B"))
                        .finish()
                }}>"seed"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                    {move |_row: RowView<TodoRow>| -> MountResult<MountPlan> {
                        let index = item_calls.get();
                        item_calls.set(index + 1);
                        if index == 1 {
                            return Err(MountError::Component("row factory boom"));
                        }
                        view! { <Show when={once(true)}><text>"row"</text></Show> }
                    }}
                </For>
            </view>
        }
    }

    #[test]
    fn a_rejected_for_row_does_not_bump_the_tree_revision() {
        let mut app = Runtime::new();
        app.mount(|| view! { <ForRejectHost /> }).expect("mount");
        let _ = app.refresh().expect("drain the mount turn");
        let revision = app.presentation_revision();
        let before = app.presentation().expect("snapshot");
        let _ = app
            .click_label("seed")
            .expect_err("the second fresh row fails");
        assert_eq!(
            app.presentation_revision(),
            revision,
            "rejected rows are not a visible change"
        );
        assert_eq!(
            app.presentation().expect("snapshot"),
            before,
            "the snapshot is unchanged after the rejected rows"
        );
    }

    #[component]
    fn FailingBranch() -> MountResult<MountPlan> {
        Err(MountError::Component("branch boom"))
    }

    #[component]
    fn BrokenShowHost() -> MountResult<MountPlan> {
        let first = signal(false);
        let second = signal(false);
        view! {
            <view>
                <Show when={first}><FailingBranch /></Show>
                <Show when={second}><text>"second shown"</text></Show>
                <button
                    aria:label="go"
                    on:click={move || {
                        first.set(true);
                        second.set(true);
                    }}
                >"go"</button>
            </view>
        }
    }

    #[test]
    fn a_rejected_branch_does_not_freeze_later_structure() {
        let mut app = Runtime::new();
        app.mount(|| view! { <BrokenShowHost /> }).expect("mount");
        let _ = app.refresh().expect("drain the mount turn");
        let failure = app
            .click_label("go")
            .expect_err("the first branch cannot mount");
        assert!(
            matches!(failure, MountError::Component("branch boom")),
            "unexpected failure: {failure:?}"
        );
        assert!(
            app.texts().iter().any(|text| text == "second shown"),
            "the sibling Show still evaluates and mounts"
        );

        let failure = app
            .refresh()
            .expect_err("the broken branch is retried next turn");
        assert!(matches!(failure, MountError::Component("branch boom")));
        assert!(
            app.texts().iter().any(|text| text == "second shown"),
            "the sibling stays mounted while the failure persists"
        );
    }

    #[component]
    fn FailingWindowA(store: Store) -> MountResult<MountPlan> {
        let open = signal(false);
        view! {
            <view>
                <button aria:label="break" on:click={move || -> Result<(), WriteError> {
                    Transaction::new().write(store.insert("row")).finish()?;
                    open.set(true);
                    Ok(())
                }}>"break"</button>
                <Show when={open}><FailingBranch /></Show>
            </view>
        }
    }

    #[component]
    fn StatusWindowB(store: Store) -> MountResult<MountPlan> {
        let status = memo(move || format!("{} rows", store.rows(Filter::All).len()));
        view! { <text aria:label="status">{status}</text> }
    }

    fn status_text(app: &Runtime, window: WindowCtx) -> String {
        app.presentation_of(window)
            .expect("snapshot")
            .find_label("status")
            .expect("the status element")
            .text()
    }

    #[test]
    fn a_failing_window_does_not_freeze_other_windows() {
        let mut app = Runtime::new();
        let store = app.setup(Store::new).expect("application setup");
        app.declare_windows(move |catalog| {
            catalog.window(WindowSpec::new("a").primary(), move |_| {
                view! { <FailingWindowA store={store} /> }
            })?;
            catalog.window(WindowSpec::new("b"), move |_| {
                view! { <StatusWindowB store={store} /> }
            })?;
            Ok(())
        })
        .expect("declare the windows");
        let a = app.show("a").expect("show window a");
        let b = app.show("b").expect("show window b");
        assert_eq!(status_text(&app, b), "0 rows");

        let node = app
            .presentation_of(a)
            .expect("snapshot")
            .find_label("break")
            .expect("the break button")
            .id();
        let error = app.click_node(node).expect_err("the a branch fails");
        assert!(
            matches!(
                error,
                DispatchError::Command(MountError::Component("branch boom"))
            ),
            "unexpected failure: {error:?}"
        );
        assert_eq!(
            status_text(&app, b),
            "1 rows",
            "the later window still refreshes in the failing turn"
        );
    }

    #[component]
    fn RowFaultWindow() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        let calls = Rc::new(Cell::new(0usize));
        let item_calls = calls.clone();
        let armed = signal(false);
        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    let mut transaction = Transaction::new();
                    for index in 0..40 {
                        transaction = transaction.write(store.insert(format!("r{index}")));
                    }
                    transaction.finish()
                }}>"seed"</button>
                <button aria:label="arm" on:click={move || armed.set(true)}>"arm"</button>
                <button aria:label="disarm" on:click={move || armed.set(false)}>"disarm"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                    {move |_row: RowView<TodoRow>| -> MountResult<MountPlan> {
                        let index = item_calls.get();
                        item_calls.set(index + 1);
                        if armed.get() && index >= 34 {
                            return Err(MountError::Component("row boom"));
                        }
                        view! { <text>"row"</text> }
                    }}
                </For>
            </view>
        }
    }

    #[test]
    fn a_failed_row_window_tick_keeps_the_revision_and_can_retry() {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| {
                view! { <RowFaultWindow /> }
            })
        })
        .expect("declare the window");
        let window = app.show("main").expect("show the window");
        app.click_label("seed").expect("seed the rows");
        app.click_label("arm").expect("arm the row fault");
        let band = app.presentation_of(window).expect("snapshot").row_bands()[0].key();
        let revision = app.presentation_revision_of(window).expect("revision");

        let failure = app
            .set_row_window(window, band, 32, 8)
            .expect_err("the fresh row fails");
        assert!(
            matches!(
                &failure,
                WindowError::Mount(MountError::FactoryFailed(error))
                    if **error == MountError::Component("row boom")
            ),
            "unexpected failure: {failure:?}"
        );
        assert_eq!(
            app.presentation_revision_of(window).expect("revision"),
            revision,
            "a rejected window request does not advance the revision"
        );
        assert_eq!(
            app.presentation_of(window).expect("snapshot").row_bands()[0].first_row(),
            0,
            "the failed request is rolled back"
        );

        let failure = app
            .set_row_window(window, band, 32, 8)
            .expect_err("the retry still hits the armed fault");
        assert!(
            matches!(
                &failure,
                WindowError::Mount(MountError::FactoryFailed(error))
                    if **error == MountError::Component("row boom")
            ),
            "an identical retry is not swallowed: {failure:?}"
        );

        app.click_label("disarm").expect("disarm the row fault");
        assert!(
            app.set_row_window(window, band, 32, 8)
                .expect("the retry applies")
                .changed(),
            "the window change is applied once the rows mount"
        );
        assert_eq!(
            app.presentation_of(window).expect("snapshot").row_bands()[0].first_row(),
            32
        );
    }

    #[component]
    fn CommittedTickWindow() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        let broken = signal(false);
        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    let mut transaction = Transaction::new();
                    for index in 0..40 {
                        transaction = transaction.write(store.insert(format!("r{index}")));
                    }
                    transaction.finish()
                }}>"seed"</button>
                <button aria:label="break" on:click={move || broken.set(true)}>"break"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                    {move |_row: RowView<TodoRow>| view! { <text>"row"</text> }}
                </For>
                <Show when={broken}><FailingBranch /></Show>
            </view>
        }
    }

    fn row_band_first(app: &Runtime, window: WindowCtx) -> usize {
        app.presentation_of(window).expect("snapshot").row_bands()[0].first_row()
    }

    #[test]
    fn a_failed_batch_restores_the_committed_windows() {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| {
                view! { <CommittedTickWindow /> }
            })
        })
        .expect("declare the window");
        let window = app.show("main").expect("show the window");
        app.click_label("seed").expect("seed the rows");
        let band = app.presentation_of(window).expect("snapshot").row_bands()[0].key();
        assert!(
            app.set_row_window(window, band, 2, 8)
                .expect("the first window applies")
                .changed()
        );
        assert_eq!(row_band_first(&app, window), 2);

        // 后续兄弟节点失败：整个批量结构 turn 失败，已经提交的新窗口也要
        // 搬回旧窗口，请求映射与已提交的 first_row/spacer 不能分叉。
        let failure = app.click_label("break").expect_err("the show branch fails");
        assert!(
            matches!(failure, MountError::Component("branch boom")),
            "unexpected failure: {failure:?}"
        );
        let revision = app.presentation_revision_of(window).expect("revision");
        let failure = app
            .set_row_window(window, band, 4, 8)
            .expect_err("the later sibling fails the turn");
        assert!(
            matches!(
                failure,
                WindowError::Mount(MountError::Component("branch boom"))
            ),
            "unexpected failure: {failure:?}"
        );
        assert_eq!(
            row_band_first(&app, window),
            2,
            "the failed batch restores the committed window"
        );
        {
            let state = app.state.borrow();
            assert!(
                state.row_window_batch.is_none(),
                "the failed turn leaves no batch marker behind"
            );
        }
        assert_ne!(
            app.presentation_revision_of(window).expect("revision"),
            revision,
            "the failed batch reports the restored window as a change"
        );
        assert!(
            !app.set_row_window(window, band, 2, 8)
                .expect("the restored request is recorded")
                .changed(),
            "the restored request matches the committed window"
        );

        // 普通刷新失败也不能把窗口悄悄挪回默认值：请求映射必须描述
        // 已提交的行窗口。
        let _ = app.refresh().expect_err("the branch still fails");
        assert_eq!(
            row_band_first(&app, window),
            2,
            "a failing refresh keeps the committed window"
        );

        // 被回滚的请求不会因为“请求已存在”被吞掉：重试仍然先经过
        // 结构 turn，并再次失败。
        let failure = app
            .set_row_window(window, band, 4, 8)
            .expect_err("the retry still fails the turn");
        assert!(
            matches!(
                failure,
                WindowError::Mount(MountError::Component("branch boom"))
            ),
            "unexpected failure: {failure:?}"
        );
        assert_eq!(
            row_band_first(&app, window),
            2,
            "the retried batch restores the committed window again"
        );
    }

    #[component]
    fn RetainedRowStructureHost() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        let inner = signal(false);
        let armed = signal(false);
        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    Transaction::new()
                        .write(store.insert("a"))
                        .write(store.insert("b"))
                        .finish()
                }}>"seed"</button>
                <button aria:label="go" on:click={move || -> Result<(), WriteError> {
                    Transaction::new().write(store.insert("c")).finish()?;
                    armed.set(true);
                    inner.set(true);
                    Ok(())
                }}>"go"</button>
                <button aria:label="disarm" on:click={move || armed.set(false)}>"disarm"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                    {move |_row: RowView<TodoRow>| -> MountResult<MountPlan> {
                        if armed.get() {
                            return Err(MountError::Component("row boom"));
                        }
                        view! { <Show when={inner}><text>"row-inner"</text></Show> }
                    }}
                </For>
            </view>
        }
    }

    #[test]
    fn a_failed_for_row_does_not_freeze_retained_row_structure() {
        let mut app = Runtime::new();
        app.mount(|| view! { <RetainedRowStructureHost /> })
            .expect("mount");
        let _ = app.refresh().expect("drain the mount turn");
        app.click_label("seed").expect("seed the retained rows");

        // 新插入的行挂载失败，但已提交行的嵌套 Show 仍要在这一帧求值。
        let failure = app.click_label("go").expect_err("the fresh row fails");
        assert!(
            matches!(&failure, MountError::FactoryFailed(error) if **error == MountError::Component("row boom")),
            "unexpected failure: {failure:?}"
        );
        let inners = app
            .texts()
            .iter()
            .filter(|text| text.as_str() == "row-inner")
            .count();
        assert_eq!(
            inners, 2,
            "both retained rows mount their nested branch in the failing turn"
        );

        let _ = app.refresh().expect_err("the fresh row still fails");
        assert_eq!(
            app.texts()
                .iter()
                .filter(|text| text.as_str() == "row-inner")
                .count(),
            2,
            "the retained branches stay mounted while the failure persists"
        );

        app.click_label("disarm").expect("disarm the row fault");
        assert_eq!(
            app.texts()
                .iter()
                .filter(|text| text.as_str() == "row-inner")
                .count(),
            3,
            "the fresh row mounts its nested branch once the fault clears"
        );
    }

    #[component]
    fn RetainedFallbackStructureHost() -> MountResult<MountPlan> {
        let inner = signal(false);
        let broken = signal(false);
        view! {
            <view>
                <button aria:label="go" on:click={move || {
                    broken.set(true);
                    inner.set(true);
                }}>"go"</button>
                <Show when={broken} fallback={move || view! {
                    <Show when={inner}><text>"fallback-inner"</text></Show>
                }}>
                    <FailingBranch />
                </Show>
            </view>
        }
    }

    #[test]
    fn a_failed_show_branch_does_not_freeze_the_retained_branch_structure() {
        let mut app = Runtime::new();
        app.mount(|| view! { <RetainedFallbackStructureHost /> })
            .expect("mount");
        let _ = app.refresh().expect("drain the mount turn");

        let failure = app
            .click_label("go")
            .expect_err("the proposed branch cannot mount");
        assert!(
            matches!(failure, MountError::Component("branch boom")),
            "unexpected failure: {failure:?}"
        );
        assert!(
            app.texts().iter().any(|text| text == "fallback-inner"),
            "the retained fallback still evaluates its nested Show"
        );

        let failure = app
            .refresh()
            .expect_err("the proposed branch is retried next turn");
        assert!(matches!(failure, MountError::Component("branch boom")));
        assert!(
            app.texts().iter().any(|text| text == "fallback-inner"),
            "the retained fallback stays mounted while the failure persists"
        );
    }

    #[component]
    fn RetainedThenStructureHost() -> MountResult<MountPlan> {
        let inner = signal(false);
        let shown = signal(true);
        view! {
            <view>
                <button aria:label="go" on:click={move || {
                    shown.set(false);
                    inner.set(true);
                }}>"go"</button>
                <Show
                    when={shown}
                    fallback={move || -> MountResult<MountPlan> {
                        Err(MountError::Component("fallback boom"))
                    }}
                >
                    <Show when={inner}><text>"then-inner"</text></Show>
                </Show>
            </view>
        }
    }

    #[test]
    fn a_failed_fallback_factory_does_not_freeze_the_retained_then_branch() {
        let mut app = Runtime::new();
        app.mount(|| view! { <RetainedThenStructureHost /> })
            .expect("mount");
        let _ = app.refresh().expect("drain the mount turn");

        let failure = app
            .click_label("go")
            .expect_err("the proposed fallback cannot mount");
        assert!(
            matches!(failure, MountError::Component("fallback boom")),
            "unexpected failure: {failure:?}"
        );
        assert!(
            app.texts().iter().any(|text| text == "then-inner"),
            "the retained then branch still evaluates its nested Show"
        );
    }

    #[component]
    fn WindowedForInShowHost() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        let open = signal(false);
        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    let mut transaction = Transaction::new();
                    for index in 0..40 {
                        transaction = transaction.write(store.insert(format!("r{index}")));
                    }
                    transaction.finish()
                }}>"seed"</button>
                <button aria:label="toggle" on:click={move || open.set(!open.get())}>"toggle"</button>
                <Show when={open}>
                    <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                        {move |_row: RowView<TodoRow>| view! { <text>"row"</text> }}
                    </For>
                </Show>
            </view>
        }
    }

    fn assert_no_row_window_batch_state(app: &Runtime) {
        let state = app.state.borrow();
        assert!(
            state.row_window_batch.is_none(),
            "no row-window batch state survives its turn"
        );
    }

    #[test]
    fn row_window_batch_state_does_not_survive_or_accumulate() {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| {
                view! { <WindowedForInShowHost /> }
            })
        })
        .expect("declare the window");
        let window = app.show("main").expect("show the window");
        app.click_label("seed").expect("seed the rows");
        // 普通刷新（没有宿主批量 turn）不留下任何批状态。
        app.click_label("toggle").expect("open the windowed for");
        assert_no_row_window_batch_state(&app);
        let band = app.presentation_of(window).expect("snapshot").row_bands()[0].key();

        // 成功的批量 turn 结束后同样清零，而不是把批标记留给下一轮。
        assert!(
            app.set_row_window(window, band, 2, 4)
                .expect("the batch applies")
                .changed()
        );
        assert_no_row_window_batch_state(&app);
        assert_eq!(
            app.presentation_of(window).expect("snapshot").row_bands()[0].first_row(),
            2,
            "the successful batch applies its window"
        );

        // 卸载/重挂多轮也不会累积批状态；行带请求随 owner 释放。
        for _ in 0..4 {
            app.click_label("toggle").expect("close the windowed for");
            app.click_label("toggle").expect("reopen the windowed for");
            assert_no_row_window_batch_state(&app);
        }
        {
            let state = app.state.borrow();
            assert_eq!(
                state.owner_bands.len(),
                1,
                "the live windowed for owns exactly one band"
            );
        }
        app.click_label("toggle").expect("close the windowed for");
        assert_no_row_window_batch_state(&app);
        let state = app.state.borrow();
        assert!(
            state.row_windows.is_empty(),
            "row-window requests die with their for"
        );
        assert!(
            state.owner_bands.is_empty(),
            "band ownership dies with its for"
        );
    }

    /// 两个兄弟窗口化 `For`：A 正常，B 在 armed 时让新行工厂失败。
    #[component]
    fn TwoBandRows() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        let armed = signal(false);
        // 动态文本绑定：失败回滚重挂的行必须重新求值。
        let mark = signal(String::from("m"));
        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    let mut transaction = Transaction::new();
                    for index in 0..40 {
                        transaction = transaction.write(store.insert(format!("r{index}")));
                    }
                    transaction.finish()
                }}>"seed"</button>
                <button aria:label="arm" on:click={move || armed.set(true)}>"arm"</button>
                <button aria:label="disarm" on:click={move || armed.set(false)}>"disarm"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                    {move |row: RowView<TodoRow>| view! {
                        <button aria:label={once(format!("a {}", row.key().raw()))}>
                            <text>{mark}</text>
                        </button>
                    }}
                </For>
                <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                    {move |row: RowView<TodoRow>| {
                        if armed.get() {
                            return Err(MountError::Component("b boom"));
                        }
                        view! {
                            <button aria:label={once(format!("b {}", row.key().raw()))}>"b"</button>
                        }
                    }}
                </For>
            </view>
        }
    }

    fn count_labels(app: &Runtime, window: WindowCtx, prefix: &str) -> usize {
        app.presentation_of(window)
            .expect("snapshot")
            .elements()
            .filter_map(SnapshotElement::label)
            .filter(|label| label.starts_with(prefix))
            .count()
    }

    fn two_band_fixture() -> (Runtime, WindowCtx, [u64; 2]) {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| {
                view! { <TwoBandRows /> }
            })
        })
        .expect("declare the window");
        let window = app.show("main").expect("show the window");
        app.click_label("seed").expect("seed the rows");
        let snapshot = app.presentation_of(window).expect("snapshot");
        assert_eq!(snapshot.row_bands().len(), 2, "two windowed fors");
        let bands = [snapshot.row_bands()[0].key(), snapshot.row_bands()[1].key()];
        (app, window, bands)
    }

    #[test]
    fn a_batch_applies_every_band_and_unchanged_batches_reuse_the_scene() {
        let (mut app, window, bands) = two_band_fixture();
        let revision = app.presentation_revision_of(window).expect("revision");
        let report = app
            .set_row_windows(
                window,
                &[
                    (bands[0], RowWindow::new(40, 10)),
                    (bands[1], RowWindow::new(20, 5)),
                ],
            )
            .expect("both bands apply");
        assert!(report.changed(), "a changed batch reports a new scene");
        let snapshot = app.presentation_of(window).expect("snapshot");
        assert_eq!(snapshot.row_bands()[0].first_row(), 40);
        assert_eq!(snapshot.row_bands()[1].first_row(), 20);
        assert_eq!(
            count_labels(&app, window, "b "),
            5,
            "the second band mounts exactly its own window"
        );
        assert_eq!(
            count_labels(&app, window, "a "),
            0,
            "the first band unmounts rows outside its empty window"
        );
        assert_ne!(
            app.presentation_revision_of(window).expect("revision"),
            revision,
            "a changed batch advances the revision"
        );

        let revision = app.presentation_revision_of(window).expect("revision");
        let report = app
            .set_row_windows(
                window,
                &[
                    (bands[0], RowWindow::new(40, 10)),
                    (bands[1], RowWindow::new(20, 5)),
                ],
            )
            .expect("the identical batch is a no-op");
        assert!(
            !report.changed(),
            "an identical batch reuses the committed scene"
        );
        assert_eq!(
            app.presentation_revision_of(window).expect("revision"),
            revision,
            "an identical batch does not advance the revision"
        );
    }

    #[test]
    fn an_empty_batch_is_a_no_op() {
        let (mut app, window, _bands) = two_band_fixture();
        let revision = app.presentation_revision_of(window).expect("revision");
        let report = app
            .set_row_windows(window, &[])
            .expect("an empty batch succeeds");
        assert!(!report.changed());
        assert_eq!(
            app.presentation_revision_of(window).expect("revision"),
            revision
        );
    }

    #[test]
    fn a_batch_with_one_stale_band_applies_none_of_its_bands() {
        let (mut app, window, bands) = two_band_fixture();
        let revision = app.presentation_revision_of(window).expect("revision");
        let failure = app
            .set_row_windows(
                window,
                &[
                    (bands[0], RowWindow::new(40, 10)),
                    (bands[1].wrapping_add(1), RowWindow::new(20, 5)),
                ],
            )
            .expect_err("the stale band rejects the batch");
        assert!(
            matches!(failure, WindowError::Stale),
            "unexpected failure: {failure:?}"
        );
        let snapshot = app.presentation_of(window).expect("snapshot");
        assert_eq!(snapshot.row_bands()[0].first_row(), 0);
        assert_eq!(snapshot.row_bands()[1].first_row(), 0);
        assert_eq!(
            app.presentation_revision_of(window).expect("revision"),
            revision,
            "a rejected batch does not refresh"
        );
        assert!(
            app.state.borrow().row_windows.is_empty(),
            "a rejected batch installs no requests"
        );
    }

    #[test]
    fn a_batch_with_a_foreign_band_applies_none_of_its_bands() {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| {
                view! { <TwoBandRows /> }
            })?;
            catalog.window(WindowSpec::new("other"), |_| {
                view! { <TwoBandRows /> }
            })?;
            Ok(())
        })
        .expect("declare the windows");
        let main = app.show("main").expect("show main");
        let other = app.show("other").expect("show other");
        app.show("main").expect("focus main again");
        let main_band = app.presentation_of(main).expect("snapshot").row_bands()[0].key();
        let other_band = app.presentation_of(other).expect("snapshot").row_bands()[0].key();
        let failure = app
            .set_row_windows(
                main,
                &[
                    (main_band, RowWindow::new(20, 5)),
                    (other_band, RowWindow::new(20, 5)),
                ],
            )
            .expect_err("a band from another window rejects the batch");
        assert!(
            matches!(failure, WindowError::Stale),
            "unexpected failure: {failure:?}"
        );
        assert!(
            app.state.borrow().row_windows.is_empty(),
            "the foreign band applies nothing"
        );
        assert_eq!(
            app.presentation_of(main).expect("snapshot").row_bands()[0].first_row(),
            0
        );
    }

    /// 两个行带的工厂共享同一个失败条件：回滚时旧行也挂不回去。
    #[component]
    fn SharedFaultBands() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        let armed = signal(false);
        // 动态文本绑定：修复重挂的行必须重新求值，否则呈现为空。
        let mark = signal(String::from("m"));
        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    let mut transaction = Transaction::new();
                    for index in 0..40 {
                        transaction = transaction.write(store.insert(format!("r{index}")));
                    }
                    transaction.finish()
                }}>"seed"</button>
                <button aria:label="arm" on:click={move || armed.set(true)}>"arm"</button>
                <button aria:label="disarm" on:click={move || armed.set(false)}>"disarm"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                    {move |row: RowView<TodoRow>| {
                        if armed.get() {
                            return Err(MountError::Component("a boom"));
                        }
                        view! {
                            <button aria:label={once(format!("a {}", row.key().raw()))}>
                                <text>{mark}</text>
                            </button>
                        }
                    }}
                </For>
                <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                    {move |row: RowView<TodoRow>| {
                        if armed.get() {
                            return Err(MountError::Component("b boom"));
                        }
                        view! {
                            <button aria:label={once(format!("b {}", row.key().raw()))}>"b"</button>
                        }
                    }}
                </For>
            </view>
        }
    }

    #[test]
    fn a_failed_rollback_keeps_requests_describing_the_committed_windows() {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| {
                view! { <SharedFaultBands /> }
            })
        })
        .expect("declare the window");
        let window = app.show("main").expect("show the window");
        app.click_label("seed").expect("seed the rows");
        let snapshot = app.presentation_of(window).expect("snapshot");
        let bands = [snapshot.row_bands()[0].key(), snapshot.row_bands()[1].key()];
        app.click_label("arm").expect("arm the shared fault");
        let failure = app
            .set_row_windows(
                window,
                &[
                    // 第一个行带的目标区间已经挂载，不需要运行工厂即可提交。
                    (bands[0], RowWindow::new(5, 10)),
                    // 第二个行带必须新挂载行，共享故障让它失败。
                    (bands[1], RowWindow::new(35, 5)),
                ],
            )
            .expect_err("the second band's factory fails");
        assert!(
            matches!(
                &failure,
                WindowError::Mount(MountError::FactoryFailed(error))
                    if **error == MountError::Component("b boom")
            ),
            "unexpected failure: {failure:?}"
        );
        let snapshot = app.presentation_of(window).expect("snapshot");
        assert_eq!(
            snapshot.row_bands()[0].first_row(),
            5,
            "the first band keeps its committed window when its old rows cannot remount"
        );
        assert_eq!(
            app.texts()
                .iter()
                .filter(|text| text.as_str() == "m")
                .count(),
            10,
            "the repaired rows evaluate their value bindings instead of rendering empty"
        );
        assert_eq!(
            app.state.borrow().row_windows.get(&bands[0]).copied(),
            Some(RowWindow::new(5, 10)),
            "the request map describes the window that is actually committed"
        );
        assert_eq!(
            app.state.borrow().row_windows.get(&bands[1]).copied(),
            None,
            "the failed band keeps its old default window"
        );

        // 失败的修复不能留下一个会在下一次普通刷新时偷偷移动的窗口。
        app.click_label("disarm").expect("disarm the shared fault");
        app.refresh().expect("the ordinary refresh succeeds");
        assert_eq!(
            app.presentation_of(window).expect("snapshot").row_bands()[0].first_row(),
            5,
            "the committed window stays where the failed batch left it"
        );
        assert_eq!(
            count_labels(&app, window, "a "),
            10,
            "the committed window still mounts exactly its rows"
        );
    }

    /// 外层、嵌套内层与兄弟三个行带共享同一个失败条件；外层与内层的目标
    /// 区间都已挂载（提交不需要工厂），兄弟行带需要新行并失败。
    #[component]
    fn NestedFaultBands() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        let armed = signal(false);
        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    let mut transaction = Transaction::new();
                    for index in 0..40 {
                        transaction = transaction.write(store.insert(format!("r{index}")));
                    }
                    transaction.finish()
                }}>"seed"</button>
                <button aria:label="arm" on:click={move || armed.set(true)}>"arm"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(40.0)}>
                    {move |group: RowView<TodoRow>| view! {
                        <view>
                            <text>{once(format!("g{}", group.key().raw()))}</text>
                            <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                                {move |item: RowView<TodoRow>| {
                                    if armed.get() {
                                        return Err(MountError::Component("item boom"));
                                    }
                                    view! {
                                        <text>{once(format!("i{}", item.key().raw()))}</text>
                                    }
                                }}
                            </For>
                        </view>
                    }}
                </For>
                <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                    {move |_item: RowView<TodoRow>| {
                        if armed.get() {
                            return Err(MountError::Component("c boom"));
                        }
                        view! { <text>"c"</text> }
                    }}
                </For>
            </view>
        }
    }

    #[test]
    fn a_nested_applied_band_is_repaired_in_its_own_refresh_round() {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| {
                view! { <NestedFaultBands /> }
            })
        })
        .expect("declare the window");
        let window = app.show("main").expect("show the window");
        app.click_label("seed").expect("seed the rows");
        let snapshot = app.presentation_of(window).expect("snapshot");
        // O band，随后每个已挂载外层行一个 B band，最后是兄弟 C band。
        let outer = snapshot.row_bands()[0].key();
        let nested = snapshot.row_bands()[1 + 5].key();
        let sibling = snapshot.row_bands().last().expect("the sibling band").key();
        // 内层 band 的 id 总是大于外层；确认选择的是一个嵌套行带。
        assert!(
            nested > outer,
            "the chosen band is nested in the outer rows"
        );
        app.click_label("arm").expect("arm the shared fault");
        let failure = app
            .set_row_windows(
                window,
                &[
                    (outer, RowWindow::new(5, 10)),
                    (nested, RowWindow::new(5, 10)),
                    (sibling, RowWindow::new(35, 5)),
                ],
            )
            .expect_err("the sibling needs new rows from a failing factory");
        assert!(
            matches!(&failure, WindowError::Mount(MountError::FactoryFailed(_))),
            "unexpected failure: {failure:?}"
        );
        let requests = app.state.borrow().row_windows.clone();
        assert_eq!(
            requests.get(&nested).copied(),
            Some(RowWindow::new(5, 10)),
            "the nested band's request describes its committed window"
        );
        assert_eq!(
            requests.get(&outer).copied(),
            Some(RowWindow::new(5, 10)),
            "the outer band's request describes its committed window"
        );
        assert_eq!(
            requests.get(&sibling).copied(),
            None,
            "the failed sibling keeps its old window"
        );
        let snapshot = app.presentation_of(window).expect("snapshot");
        let nested_band = snapshot
            .row_bands()
            .iter()
            .find(|band| band.key() == nested)
            .expect("the nested band stays mounted");
        assert_eq!(nested_band.first_row(), 5);

        // 请求与已提交窗口一致后，普通刷新不会搬动任何行带。
        app.refresh().expect("the ordinary refresh succeeds");
        let snapshot = app.presentation_of(window).expect("snapshot");
        assert_eq!(
            snapshot.row_bands()[0].first_row(),
            5,
            "the outer window stays where the failed batch left it"
        );
        assert_eq!(
            app.state.borrow().row_windows.get(&nested).copied(),
            Some(RowWindow::new(5, 10)),
            "the nested request is stable across an ordinary refresh"
        );
    }

    #[test]
    fn a_batch_with_a_duplicate_band_is_rejected() {
        let (mut app, window, bands) = two_band_fixture();
        let failure = app
            .set_row_windows(
                window,
                &[
                    (bands[0], RowWindow::new(4, 2)),
                    (bands[0], RowWindow::new(8, 2)),
                ],
            )
            .expect_err("a duplicate band is ambiguous");
        assert!(
            matches!(failure, WindowError::DuplicateRowBand),
            "unexpected failure: {failure:?}"
        );
        assert!(
            app.state.borrow().row_windows.is_empty(),
            "a duplicate band installs no requests"
        );
    }

    #[test]
    fn one_failed_band_rolls_back_every_band_in_the_batch() {
        let (mut app, window, bands) = two_band_fixture();
        app.click_label("arm").expect("arm the second factory");
        let failure = app
            .set_row_windows(
                window,
                &[
                    (bands[0], RowWindow::new(40, 10)),
                    (bands[1], RowWindow::new(35, 5)),
                ],
            )
            .expect_err("the second band's factory fails");
        assert!(
            matches!(
                &failure,
                WindowError::Mount(MountError::FactoryFailed(error))
                    if **error == MountError::Component("b boom")
            ),
            "unexpected failure: {failure:?}"
        );
        let snapshot = app.presentation_of(window).expect("snapshot");
        assert_eq!(
            snapshot.row_bands()[0].first_row(),
            0,
            "the committed first band is rolled back with the failed batch"
        );
        assert_eq!(
            count_labels(&app, window, "a "),
            32,
            "the first band's old rows are restored"
        );
        assert_eq!(
            app.texts()
                .iter()
                .filter(|text| text.as_str() == "m")
                .count(),
            32,
            "the restored rows evaluate their value bindings"
        );
        assert_eq!(snapshot.row_bands()[1].first_row(), 0);
        assert!(
            app.state.borrow().row_windows.is_empty(),
            "the rollback leaves no requests behind"
        );
        assert!(
            app.state.borrow().row_window_batch.is_none(),
            "the failed batch leaves no marker behind"
        );

        app.click_label("disarm")
            .expect("disarm the second factory");
        let report = app
            .set_row_windows(
                window,
                &[
                    (bands[0], RowWindow::new(40, 10)),
                    (bands[1], RowWindow::new(35, 5)),
                ],
            )
            .expect("the retried batch applies");
        assert!(report.changed());
        let snapshot = app.presentation_of(window).expect("snapshot");
        assert_eq!(snapshot.row_bands()[0].first_row(), 40);
        assert_eq!(snapshot.row_bands()[1].first_row(), 35);
        assert_eq!(
            count_labels(&app, window, "b "),
            5,
            "the retried batch mounts the second band's new rows"
        );
    }

    /// 一个外层窗口化行，每行里面再挂一个窗口化 `For`。
    ///
    /// Todo facade 的 `Store::new()` 是同一个共享模型的句柄，因此内外两层
    /// 读同一份行集；行工厂只在内层挂文本，不会递归。
    #[component]
    fn NestedBandsHost() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    let mut transaction = Transaction::new();
                    for index in 0..26 {
                        transaction = transaction.write(store.insert(format!("r{index}")));
                    }
                    transaction.finish()
                }}>"seed"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(40.0)}>
                    {move |group: RowView<TodoRow>| view! {
                        <view>
                            <text>{once(format!("g{}", group.key().raw()))}</text>
                            <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                                {move |item: RowView<TodoRow>| view! {
                                    <text>{once(format!("i{}", item.key().raw()))}</text>
                                }}
                            </For>
                        </view>
                    }}
                </For>
            </view>
        }
    }

    #[test]
    fn a_batch_applies_nested_bands_without_rejecting_unmounted_inner_bands() {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| {
                view! { <NestedBandsHost /> }
            })
        })
        .expect("declare the window");
        let window = app.show("main").expect("show the window");
        app.click_label("seed").expect("seed both levels");
        let snapshot = app.presentation_of(window).expect("snapshot");
        // 第一项是外层行带；其余按已挂载外层行的顺序排列内层行带。
        let total = snapshot.row_bands()[0].total_rows();
        assert_eq!(
            snapshot.row_bands().len(),
            1 + total,
            "one outer band and one inner band per mounted outer row"
        );
        let outer = snapshot.row_bands()[0].key();
        let leaving_inner = snapshot.row_bands()[1].key();
        let staying_inner = snapshot.row_bands()[3].key();

        let report = app
            .set_row_windows(
                window,
                &[
                    (outer, RowWindow::new(2, 4)),
                    // 外层窗口卸载第 0 行后，这个内层行带随行一起释放；它的
                    // 请求可以留在同一批里，不能因此拒绝整批。
                    (leaving_inner, RowWindow::new(0, 2)),
                    (staying_inner, RowWindow::new(5, 2)),
                ],
            )
            .expect("nested bands apply atomically");
        assert!(report.changed());
        let snapshot = app.presentation_of(window).expect("snapshot");
        assert_eq!(snapshot.row_bands()[0].first_row(), 2);
        assert_eq!(
            snapshot.row_bands().len(),
            1 + 4,
            "only the surviving outer rows keep their inner bands"
        );
        let inner = snapshot
            .row_bands()
            .iter()
            .find(|band| band.key() == staying_inner)
            .expect("the surviving inner band stays mounted");
        assert_eq!(inner.first_row(), 5);
    }

    #[test]
    fn a_failed_batch_clears_focus_on_rows_it_remounted() {
        let (mut app, window, bands) = two_band_fixture();
        let snapshot = app.presentation_of(window).expect("snapshot");
        let label = snapshot
            .elements()
            .find_map(|element| {
                element
                    .label()
                    .filter(|label| label.starts_with("a "))
                    .map(str::to_owned)
            })
            .expect("the first band has a focusable row");
        app.focus(&label).expect("focus a row");
        let focused = app
            .presentation_of(window)
            .expect("snapshot")
            .focus()
            .expect("the row holds focus");
        app.click_label("arm").expect("arm the second factory");
        app.set_row_windows(
            window,
            &[
                (bands[0], RowWindow::new(40, 10)),
                (bands[1], RowWindow::new(35, 5)),
            ],
        )
        .expect_err("the second band's factory fails");
        // 已提交的行带被回滚刷新重挂：旧行身份不存在，焦点按行窗口卸载
        // 策略清除，而不是结算到相邻行或复活一个 Stale 节点。
        assert_eq!(
            app.presentation_of(window).expect("snapshot").focus(),
            None,
            "a failed batch never leaves focus on a remounted row"
        );
        assert_eq!(app.node_status(focused), NodeStatus::Stale);
        assert!(app.focused().is_none());
        app.focus(&label)
            .expect("the restored row is focusable again");
    }

    #[test]
    fn a_batch_clears_focus_that_leaves_its_window_instead_of_settling() {
        let (mut app, window, bands) = two_band_fixture();
        let snapshot = app.presentation_of(window).expect("snapshot");
        let focused_label = snapshot
            .elements()
            .find_map(|element| {
                element
                    .label()
                    .filter(|label| label.starts_with("a "))
                    .map(str::to_owned)
            })
            .expect("the first band has a focusable row");
        app.focus(&focused_label).expect("focus a row");
        assert!(app.focused().is_some());
        app.set_row_windows(
            window,
            &[
                (bands[0], RowWindow::new(40, 10)),
                (bands[1], RowWindow::new(20, 5)),
            ],
        )
        .expect("the batch applies");
        let snapshot = app.presentation_of(window).expect("snapshot");
        assert_eq!(
            snapshot.focus(),
            None,
            "a focused row that leaves the window is cleared, not reassigned"
        );
        assert!(app.focused().is_none());
    }

    #[component]
    fn PlainForHost() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        view! {
            <view>
                <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                    {move |_row: RowView<TodoRow>| view! { <text>"row"</text> }}
                </For>
            </view>
        }
    }

    #[test]
    fn a_band_without_a_row_height_is_not_a_window() {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| {
                view! { <PlainForHost /> }
            })
        })
        .expect("declare the window");
        let window = app.show("main").expect("show the window");
        let band = {
            let state = app.state.borrow();
            *state
                .owner_bands
                .values()
                .flatten()
                .next()
                .expect("the plain for owns its band")
        };
        let failure = app
            .set_row_windows(window, &[(band, RowWindow::new(0, 4))])
            .expect_err("a non-windowed band is not addressable");
        assert!(
            matches!(failure, WindowError::Stale),
            "unexpected failure: {failure:?}"
        );
    }

    #[component]
    fn EqualWriteHost() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    Transaction::new().write(store.insert("row")).finish()
                }}>"seed"</button>
                <button aria:label="equal" on:click={move || -> Result<(), WriteError> {
                    if let Some(id) = store.keys(Filter::All).into_iter().next() {
                        Transaction::new()
                            .write(store.set_title(id, "row"))
                            .write(store.set_completed(id, false))
                            .write(store.reorder(&[id]))
                            .finish()?;
                    }
                    Ok(())
                }}>"equal"</button>
                <button aria:label="change" on:click={move || -> Result<(), WriteError> {
                    if let Some(id) = store.keys(Filter::All).into_iter().next() {
                        Transaction::new().write(store.set_title(id, "changed")).finish()?;
                    }
                    Ok(())
                }}>"change"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                    {move |row: RowView<TodoRow>| view! {
                        <text>{store.title(row.key())}</text>
                    }}
                </For>
            </view>
        }
    }

    #[test]
    fn equal_store_writes_do_not_bump_field_or_row_dependencies() {
        use crate::context::{FieldKind, filter_slot};

        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| {
                view! { <EqualWriteHost /> }
            })
        })
        .expect("declare the window");
        let window = app.show("main").expect("show the window");
        app.click_label("seed").expect("seed the row");
        let id = app
            .for_items()
            .first()
            .expect("row item")
            .todo
            .expect("todo row");
        assert!(app.texts().iter().any(|text| text == "row"));

        let fields = [
            DepKey::Field(id, FieldKind::Title),
            DepKey::Field(id, FieldKind::Completed),
        ];
        let rows = [
            DepKey::Rows(filter_slot(Filter::All)),
            DepKey::Rows(filter_slot(Filter::Active)),
            DepKey::Rows(filter_slot(Filter::Completed)),
        ];
        let before: Vec<u64> = {
            let state = app.state.borrow();
            fields
                .iter()
                .chain(rows.iter())
                .map(|key| state.write_epochs.get(key).copied().unwrap_or(0))
                .collect()
        };
        let revision = app.presentation_revision_of(window).expect("revision");
        let attempts = {
            let state = app.state.borrow();
            [
                state.field_write_epoch(id, FieldKind::Title),
                state.field_write_epoch(id, FieldKind::Completed),
            ]
        };
        let io_writes = app.counters().io_writes;

        app.click_label("equal").expect("equal writes are allowed");
        {
            let state = app.state.borrow();
            let after: Vec<u64> = fields
                .iter()
                .chain(rows.iter())
                .map(|key| state.write_epochs.get(key).copied().unwrap_or(0))
                .collect();
            assert_eq!(after, before, "equal writes bump no field or row epoch");
            assert!(
                state.field_write_epoch(id, FieldKind::Title) > attempts[0],
                "the title write attempt is still recorded"
            );
            assert!(
                state.field_write_epoch(id, FieldKind::Completed) > attempts[1],
                "the completed write attempt is still recorded"
            );
        }
        assert_eq!(
            app.counters().io_writes,
            io_writes,
            "equal writes are not io writes"
        );
        assert_eq!(
            app.presentation_revision_of(window).expect("revision"),
            revision,
            "equal writes change nothing visible"
        );

        app.click_label("change").expect("real title write");
        assert!(app.presentation_revision_of(window).expect("revision") > revision);
        assert_eq!(
            app.counters().io_writes,
            io_writes + 1,
            "a real write counts once"
        );
        assert!(app.texts().iter().any(|text| text == "changed"));
    }

    #[component]
    fn WriteCountingHost() -> MountResult<MountPlan> {
        let store = Store::new();
        view! {
            <view>
                <button aria:label="two-inserts" on:click={move || -> Result<(), WriteError> {
                    Transaction::new()
                        .write(store.insert("a"))
                        .write(store.insert("b"))
                        .finish()
                }}>"two"</button>
                <button aria:label="empty-clear" on:click={move || -> Result<(), WriteError> {
                    Transaction::new().write(store.clear_completed()).finish()
                }}>"clear"</button>
            </view>
        }
    }

    #[test]
    fn a_transaction_counts_each_effective_write() {
        let mut app = Runtime::new();
        app.mount(|| view! { <WriteCountingHost /> })
            .expect("mount");

        let before = app.counters();
        app.click_label("two-inserts").expect("two inserts");
        let after = app.counters();
        assert_eq!(
            after.io_writes - before.io_writes,
            2,
            "each insert is one write"
        );
        assert_eq!(
            after.commits - before.commits,
            1,
            "one transaction commits once"
        );

        let before = app.counters();
        app.click_label("empty-clear").expect("nothing to clear");
        let after = app.counters();
        assert_eq!(
            after.io_writes, before.io_writes,
            "an empty clear batch writes nothing"
        );
    }

    fn first_for(state: &State, nodes: &[MountedNode]) -> Option<u32> {
        for node in nodes {
            match node {
                MountedNode::Text(_) => {}
                MountedNode::Element(id) => {
                    if let Some(found) = first_for(state, &state.elements[*id].children) {
                        return Some(found);
                    }
                }
                MountedNode::Component(id) => {
                    if let Some(found) = first_for(state, &state.components[*id].children) {
                        return Some(found);
                    }
                }
                MountedNode::Show(payload) => {
                    let show = state.nodes.show(*payload);
                    if let Some(branch) = &show.active
                        && let Some(found) = first_for(state, &branch.children)
                    {
                        return Some(found);
                    }
                }
                MountedNode::For(payload) => return Some(*payload),
            }
        }
        None
    }

    #[test]
    fn duplicate_attribute_and_style_declarations_keep_one_live_slot() {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| {
                Ok(MountPlan::new(vec![
                    element(
                        "view",
                        vec![attr("role", "a"), attr("role", "b")],
                        vec![child("dup")],
                    ),
                    styled_element(
                        "view",
                        Vec::new(),
                        &[],
                        vec![
                            style_color(once(Color::Literal([1, 2, 3, 255]))),
                            style_color(once(Color::Literal([9, 8, 7, 255]))),
                        ],
                        Vec::new(),
                    ),
                ]))
            })
        })
        .expect("declare the window");
        let window = app.show("main").expect("show the window");
        {
            let state = app.state.borrow();
            let tree = state
                .trees
                .get(&TreeSlot::Window("main"))
                .expect("the window tree");
            let MountedNode::Element(bound) = tree.nodes[0] else {
                panic!("the first node is the bound element");
            };
            assert_eq!(
                state.element_attrs(bound).len(),
                1,
                "duplicate attribute declarations collapse into one"
            );
            assert_eq!(
                state.element_attr_text(bound, "role"),
                Some("b"),
                "the last attribute declaration wins"
            );
            let MountedNode::Element(styled) = tree.nodes[1] else {
                panic!("the second node is the styled element");
            };
            let styles = state.element_inline_styles(styled);
            assert_eq!(
                styles.len(),
                1,
                "duplicate style declarations collapse into one"
            );
            let table = state
                .element_binding_table(styled)
                .expect("the styled element's binding table");
            assert_eq!(
                table.inline_style(styles[0].binding),
                Some(StyleValue::Color(Color::Literal([9, 8, 7, 255]))),
                "the last style declaration wins"
            );
            assert_eq!(
                live_binding_slots(&app),
                3,
                "role, one text and one style slot stay live"
            );
        }

        app.close_window(window).expect("close the window");
        assert!(
            app.state.borrow().nodes.binding_tables.is_empty(),
            "the retired tree returned every slot before its table was dropped"
        );
    }

    #[component]
    fn PanicShow() -> MountResult<MountPlan> {
        let failing: Memo<bool> = memo(|| panic!("show memo boom"));
        view! { <Show when={failing}><text>"x"</text></Show> }
    }

    #[component]
    fn PanicFor() -> MountResult<MountPlan> {
        let store = Store::new();
        let failing: Memo<RowSet> = memo(|| panic!("for memo boom"));
        view! {
            <For each={failing} store={store} key={|row: &TodoRow| row.id}>
                {move |_row: RowView<TodoRow>| view! { <text>"row"</text> }}
            </For>
        }
    }

    #[test]
    fn a_show_source_panic_at_mount_is_a_contained_mount_error() {
        let mut app = Runtime::new();
        let error = app
            .mount(|| view! { <PanicShow /> })
            .expect_err("the memo panic fails the mount");
        assert!(matches!(error, MountError::Panic(_)));
        let state = app.state.borrow();
        assert!(
            state.nodes.binding_tables.is_empty(),
            "the failed mount drops its binding table"
        );
        assert_eq!(state.nodes.live(), 0, "no node stays live");
        assert_eq!(
            state.nodes.aux.binding_lists.live(),
            0,
            "no binding list stays live"
        );
    }

    #[test]
    fn a_for_source_panic_at_mount_is_a_contained_mount_error() {
        let mut app = Runtime::new();
        let error = app
            .mount(|| view! { <PanicFor /> })
            .expect_err("the memo panic fails the mount");
        assert!(matches!(error, MountError::Panic(_)));
        let state = app.state.borrow();
        assert!(
            state.nodes.binding_tables.is_empty(),
            "the failed mount drops its binding table"
        );
        assert_eq!(state.nodes.live(), 0, "no node stays live");
    }

    #[test]
    fn a_root_mount_fault_releases_root_text_slots() {
        let mut app = Runtime::new();
        let error = app
            .mount(|| {
                let value = signal(0u32);
                let _derived: Memo<u32> = memo(move || value.get() + 1);
                value.set(1);
                Ok(MountPlan::new(vec![child("root text")]))
            })
            .expect_err("an idle-phase write is a context fault");
        assert!(matches!(error, MountError::Context { .. }));
        {
            let state = app.state.borrow();
            assert!(
                state.nodes.binding_tables.is_empty(),
                "the faulted root mount drops its binding table"
            );
            assert_eq!(state.nodes.live(), 0, "the root text node is released");
        }
        assert_eq!(
            app.live_signals(),
            0,
            "the failed mount releases owner-0 signals"
        );
        assert_eq!(
            app.live_memos(),
            0,
            "the failed mount releases owner-0 memos"
        );
    }

    #[component]
    fn BrokenSibling() -> MountResult<MountPlan> {
        Err(MountError::Component("broken sibling"))
    }

    #[component]
    fn PatchLeakHost() -> MountResult<MountPlan> {
        let open = signal(true);
        view! {
            <view>
                <Show when={open}><text>"branch"</text></Show>
                <BrokenSibling />
            </view>
        }
    }

    #[test]
    fn a_failed_mount_rolls_back_the_patches_of_its_mounted_siblings() {
        let mut app = Runtime::new();
        let error = app
            .mount(|| view! { <PatchLeakHost /> })
            .expect_err("the sibling fails the mount");
        assert!(matches!(error, MountError::Component("broken sibling")));
        let mut state = app.state.borrow_mut();
        assert!(
            state.binding_patches.take().is_empty(),
            "no patch survives for a released mount"
        );
        assert!(
            state.nodes.binding_tables.is_empty(),
            "the failed mount drops its binding table"
        );
    }

    #[test]
    fn a_failed_window_mount_rolls_back_its_patches() {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| {
                view! { <PatchLeakHost /> }
            })
        })
        .expect("declare the window");
        let error = app
            .show("main")
            .expect_err("the sibling fails the window mount");
        assert!(matches!(
            error,
            WindowError::Mount(MountError::Component("broken sibling"))
        ));
        let mut state = app.state.borrow_mut();
        assert!(
            state.binding_patches.take().is_empty(),
            "no patch survives for a released window mount"
        );
        assert!(
            state.nodes.binding_tables.is_empty(),
            "the failed window mount drops its binding table"
        );
    }

    #[component]
    fn StyledFaultHost() -> MountResult<MountPlan> {
        let width = signal(px(10.0));
        view! { <button aria:label="styled" style:width={width}>"styled"</button> }
    }

    fn styled_width(app: &Runtime) -> Option<StyleValue> {
        let snapshot = app.presentation().expect("snapshot");
        snapshot
            .find_label("styled")
            .expect("the styled element")
            .styles()
            .iter()
            .find(|declaration| declaration.property() == Property::Width)
            .map(|declaration| declaration.value())
    }

    /// 找到带 `aria:label` 的元素并释放它 `style:width` 的来源。
    fn dispose_width_source(app: &Runtime, clear_current: bool) {
        let mut state = app.state.borrow_mut();
        let element = (0..state.elements.len())
            .find(|element| state.element_attr_text(*element, "aria:label") == Some("styled"))
            .expect("the styled element");
        let tree = state
            .element_tree(element)
            .expect("the element belongs to a tree");
        let binding = state
            .element_inline_styles(element)
            .iter()
            .find(|style| style.property == Property::Width)
            .expect("the width declaration")
            .binding;
        let slot = state
            .nodes
            .binding_table_mut(tree)
            .slot_mut::<Length>(binding)
            .expect("the width slot");
        let ReadSlot::Signal(source) = slot.source else {
            panic!("the width is bound to a signal");
        };
        if clear_current {
            slot.current = None;
            slot.fault = None;
        }
        state
            .signals
            .get_mut(source.id as usize)
            .expect("the width signal")
            .disposed = true;
    }

    #[test]
    fn a_style_slot_keeps_its_last_committed_value_after_a_source_fault() {
        let mut app = Runtime::new();
        app.mount(|| view! { <StyledFaultHost /> }).expect("mount");
        assert_eq!(
            styled_width(&app),
            Some(StyleValue::Length(px(10.0))),
            "the mounted style is committed"
        );

        dispose_width_source(&app, false);
        let report = app.refresh().expect("refresh reports the fault");
        assert!(
            report
                .faults()
                .iter()
                .any(|fault| matches!(fault, ReadFault::Disposed)),
            "the disposed style source is a typed fault: {:?}",
            report.faults()
        );
        assert_eq!(
            styled_width(&app),
            Some(StyleValue::Length(px(10.0))),
            "a later failure keeps the last committed style"
        );
    }

    #[test]
    fn a_first_style_failure_uses_the_intrinsic_default() {
        let mut app = Runtime::new();
        app.mount(|| view! { <StyledFaultHost /> }).expect("mount");
        dispose_width_source(&app, true);

        let report = app.refresh().expect("refresh reports the fault");
        assert!(
            report
                .faults()
                .iter()
                .any(|fault| matches!(fault, ReadFault::Disposed)),
            "the disposed style source is a typed fault: {:?}",
            report.faults()
        );
        assert_eq!(
            styled_width(&app),
            None,
            "a first failure falls back to the intrinsic default instead of a value"
        );
    }

    #[test]
    fn a_first_failed_for_evaluation_mounts_an_empty_row_set() {
        let mut app = Runtime::new();
        app.mount(|| view! { <ForHost /> }).expect("mount");
        assert!(app.for_items().is_empty());

        {
            let mut state = app.state.borrow_mut();
            let tree_key = state.arena_trees[&TreeSlot::Root];
            let payload = {
                let tree = state.trees.get(&TreeSlot::Root).expect("root tree");
                first_for(&state, &tree.nodes).expect("the For node")
            };
            let MountedForRows::Read(binding) = state.nodes.for_node(payload).rows else {
                panic!("the For is bound to a memo");
            };
            let slot = state
                .nodes
                .binding_table_mut(tree_key)
                .slot_mut::<RowSet>(binding)
                .expect("the For binding slot");
            let ReadSlot::Memo(source) = slot.source else {
                panic!("For is bound to a memo");
            };
            // 注入“首次求值前源已释放”：没有最后值，也没有可重算的 compute。
            slot.current = None;
            slot.fault = None;
            let memo = state
                .memos
                .get_mut(source.id as usize)
                .expect("the For memo cell");
            memo.disposed = true;
            memo.compute = None;
            memo.value = None;
            memo.dirty = true;
            memo.poisoned = None;
        }

        let report = app.refresh().expect("refresh");
        assert!(
            app.for_items().is_empty(),
            "a source that never produced rows mounts an empty row set"
        );
        assert!(
            report
                .faults()
                .iter()
                .any(|fault| matches!(fault, ReadFault::Disposed)),
            "the disposed For source is a typed fault: {:?}",
            report.faults()
        );
    }

    #[test]
    fn a_disposed_for_source_keeps_the_committed_rows() {
        let mut app = Runtime::new();
        app.mount(|| view! { <ForHost /> }).expect("mount");
        app.click_label("Seed").expect("seed the rows");
        assert_eq!(app.for_items().len(), 2);

        {
            let mut state = app.state.borrow_mut();
            let tree_key = state.arena_trees[&TreeSlot::Root];
            let payload = {
                let tree = state.trees.get(&TreeSlot::Root).expect("root tree");
                first_for(&state, &tree.nodes).expect("the For node")
            };
            let MountedForRows::Read(binding) = state.nodes.for_node(payload).rows else {
                panic!("the For is bound to a memo");
            };
            let slot = state
                .nodes
                .binding_table_mut(tree_key)
                .slot_mut::<RowSet>(binding)
                .expect("the For binding slot");
            let ReadSlot::Memo(source) = slot.source else {
                panic!("For is bound to a memo");
            };
            // 槽位没有最后值，但已提交投影仍在：失败帧必须保留它。
            slot.current = None;
            slot.fault = None;
            state.memos[source.id as usize].disposed = true;
        }

        let report = app
            .refresh()
            .expect("a fault keeps the committed rows and reports the fault");
        assert!(
            report
                .faults()
                .iter()
                .any(|fault| matches!(fault, ReadFault::Disposed)),
            "the disposed source is still reported: {:?}",
            report.faults()
        );
        assert_eq!(
            app.for_items().len(),
            2,
            "a source fault keeps the last committed rows"
        );
    }

    #[test]
    fn a_faulting_for_source_keeps_the_committed_projection() {
        let mut app = Runtime::new();
        app.mount(|| view! { <FaultFallbackHost /> })
            .expect("mount");
        app.click_label("seed").expect("seed a");
        let failure = app
            .click_label("duplicate")
            .expect_err("b collides with a by title length");
        assert_eq!(failure, MountError::DuplicateKey);
        assert_eq!(
            app.for_items().len(),
            1,
            "the rejected frame kept the old mapping"
        );

        {
            let mut state = app.state.borrow_mut();
            let tree_key = state.arena_trees[&TreeSlot::Root];
            let payload = {
                let tree = state.trees.get(&TreeSlot::Root).expect("root tree");
                first_for(&state, &tree.nodes).expect("the For node")
            };
            let MountedForRows::Read(binding) = state.nodes.for_node(payload).rows else {
                panic!("the For is bound to a memo");
            };
            let memo_id = {
                let slot = state
                    .nodes
                    .binding_table_mut(tree_key)
                    .slot_mut::<RowSet>(binding)
                    .expect("the For binding slot");
                let ReadSlot::Memo(source) = slot.source else {
                    panic!("For is bound to a memo");
                };
                source.id as usize
            };
            // 修复 b 的键，并在同一帧让源失效：失败帧不能把拟议行集变成已提交顺序。
            let b = state.todo_store.ids()[1];
            state
                .todo_store
                .update(b, |row| row.title = String::from("bb"))
                .expect("heal b");
            state.memos[memo_id].disposed = true;
        }

        let report = app.refresh().expect("a fault keeps the committed rows");
        assert!(
            report
                .faults()
                .iter()
                .any(|fault| matches!(fault, ReadFault::Disposed)),
            "the disposed source is still reported: {:?}",
            report.faults()
        );
        assert_eq!(
            app.for_items().len(),
            1,
            "the never-committed b row must not mount"
        );
        let rows: Vec<String> = app
            .texts()
            .into_iter()
            .filter(|text| text.starts_with("row "))
            .collect();
        assert_eq!(rows, vec!["row a"]);
    }
}
