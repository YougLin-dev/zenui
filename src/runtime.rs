use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

use zenui_core::{Counters, PanicInfo, RowSet, catch_panic};
use zenui_style::{Length, Property, StyleValue};

use crate::context::{self, ContextFault, DepKey, Phase, with_state};
use crate::dispatch::{DispatchError, NodeOperation};
use crate::menu::{
    CommandError, CommandEvent, CommandHandler, CommandSource, MenuCommand, MenuCommandState,
    MenuSpec, MenuStateError, MenuStateHandler,
};
use crate::node::{
    AttrListId, AuxHandles, BindingList, BindingListId, EditStateId, ElementData, ElementFlags,
    EventListId, NO_PAYLOAD, NodeKindTag, NodeRef, Pool, SemanticsId, SlotArena, StyleListId, TagId,
    TreeKey,
};
use crate::presentation::{
    Appearance, NodeId, NodeStatus, PresentationSnapshot, SnapshotAction, SnapshotActions,
    SnapshotAttr, SnapshotElement, SnapshotNode, SnapshotRowBand, SnapshotSemantics, SnapshotValue,
    TextEditState,
};
use crate::session::{SessionAction, SessionCtx, SessionError, SessionPolicy};
use crate::signal::{Memo, MemoCell, SignalCell};
use crate::store::{TodoId, WriteError};
use crate::task::{TaskRecord, WakeCallback};
use crate::text_edit::{self, Edit, Movement};
use crate::tray::{TrayError, TraySpec};
use crate::view::{
    Attr, AttrValue, BlurEvent, ChildrenFactory, CompositionEvent, CompositionPhase, Event,
    FocusCause, FocusEvent, HandlerError, HandlerOutcome, InlineStyle, Key, KeyEvent, MountError,
    MountPlan, MountResult, PlanNode, Read, ReadFault, ReadValue, StyleSlot, SubmitEvent,
};
use crate::window::{
    CatalogEntry, CatalogError, CloseError, CloseEvent, CloseHandler, CloseOutcome, ClosePolicy,
    CloseReply, CloseRequest, CloseSource, DropError, DropEvent, DropHandler, WindowAction,
    WindowCatalog, WindowCtx, WindowError, WindowInfo, WindowRecord, WindowRequest, WindowRole,
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

    pub const fn changed(&self) -> bool {
        self.changed
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

    /// 合并一个嵌套默认动作的报告：错误、flow 与 changed 都要回到外层 turn。
    fn absorb(&mut self, other: Self) {
        self.errors.extend(other.errors);
        self.changed |= other.changed;
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

    /// 释放节点身份、它的元素 payload 与辅助列表；重复释放无操作。
    fn release(&mut self, node: NodeRef) {
        if let Ok(slot) = self.slots.get(node) {
            let kind = slot.kind;
            let payload = slot.payload;
            match kind {
                Some(NodeKindTag::Element) => {
                    if let Some(data) = self
                        .elements
                        .get_mut(payload as usize)
                        .and_then(Option::take)
                    {
                        self.aux.release(data.aux);
                    }
                }
                Some(NodeKindTag::Text) => {
                    self.texts.release(payload);
                }
                Some(NodeKindTag::Show) => {
                    self.shows.release(payload);
                }
                Some(NodeKindTag::For) => {
                    self.forces.release(payload);
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

    fn text_mut(&mut self, payload: u32) -> &mut TextData {
        self.texts
            .get_mut(payload)
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

/// 文本节点的 payload：求值来源与已提交文本。
struct TextData {
    source: Read<String>,
    /// 最后已提交文本；读取失败时保持不变。
    value: String,
    /// 本次求值失败；可与 `value` 同时存在。
    fault: Option<ReadFault>,
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
}

/// 宿主为一行带请求的挂载窗口；`count` 为零表示该行带当前不需要行。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RowWindow {
    first: usize,
    count: usize,
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
    /// 源写入代数；字段绑定用它辨认处理器是否写过这次编辑。
    pub(crate) write_epochs: BTreeMap<DepKey, u64>,
    pub(crate) write_error: Option<WriteError>,
    pub(crate) core: zenui_core::Runtime,
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
    pub todo: TodoId,
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

/// 记录一个 `Read` 源的 memo 槽位；其他来源没有 memo 依赖。
fn collect_read_memo<T>(read: &Read<T>, live: &mut BTreeSet<u32>) {
    if let Read::Memo(memo) = read {
        live.insert(memo.memo_id());
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
    /// 这是 #336 BindingTable 之前的 liveness 近似：从当前保留树的 `Read`
    /// 源出发，不把只被处理器读取的派生值提前结算。
    pub(crate) fn live_memo_set(&self) -> BTreeSet<u32> {
        let mut live = BTreeSet::new();
        for tree in self.trees.values() {
            self.collect_node_memos(&tree.nodes, &mut live);
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

    fn collect_node_memos(&self, nodes: &[MountedNode], live: &mut BTreeSet<u32>) {
        for node in nodes {
            match node {
                MountedNode::Text(payload) => {
                    collect_read_memo(&self.nodes.text(*payload).source, live);
                }
                MountedNode::Element(id) => {
                    for attr in self.element_attrs(*id) {
                        match &attr.value {
                            AttrValue::Text(read) => collect_read_memo(read, live),
                            AttrValue::Bool(read) => collect_read_memo(read, live),
                        }
                    }
                    for style in self.element_inline_styles(*id) {
                        match &style.slot {
                            StyleSlot::Color(read) => collect_read_memo(read, live),
                            StyleSlot::Length(read) => collect_read_memo(read, live),
                            StyleSlot::Opacity(read) => collect_read_memo(read, live),
                        }
                    }
                    self.collect_node_memos(&self.elements[*id].children, live);
                }
                MountedNode::Component(id) => {
                    self.collect_node_memos(&self.components[*id].children, live);
                }
                MountedNode::Show(payload) => {
                    let show = self.nodes.show(*payload);
                    collect_read_memo(&show.when, live);
                    if let Some(branch) = &show.active {
                        self.collect_node_memos(&branch.children, live);
                    }
                }
                MountedNode::For(payload) => {
                    let data = self.nodes.for_node(*payload);
                    live.insert(data.each.memo_id());
                    for item in data.items.values() {
                        self.collect_node_memos(&item.children, live);
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
            write_epochs: BTreeMap::new(),
            write_error: None,
            core: zenui_core::Runtime::new(),
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
        live.then(|| WindowCtx::new(self.id, key, record.activation, record.owner))
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
        self.element_attrs(element)
            .iter()
            .find(|attr| attr.name == "aria:label")
            .and_then(|attr| attr.current.as_ref())
            .and_then(|current| match current {
                AttrCurrent::Text(text) => Some(text.as_str()),
                AttrCurrent::Bool(_) => None,
            })
    }

    /// 已提交的文本属性；`type` 等标记只读取当前值。
    fn element_attr_text(&self, element: usize, name: &str) -> Option<&str> {
        self.element_attrs(element)
            .iter()
            .find(|attr| attr.name == name)
            .and_then(|attr| attr.current.as_ref())
            .and_then(|current| match current {
                AttrCurrent::Text(text) => Some(text.as_str()),
                AttrCurrent::Bool(_) => None,
            })
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
        self.element_attrs(element)
            .iter()
            .find(|attr| attr.name == "disabled")
            .and_then(|attr| attr.current.as_ref())
            .is_some_and(|current| matches!(current, AttrCurrent::Bool(true)))
    }

    /// `value` 绑定的写入代数；静态值或缺失绑定返回 `None`。
    fn element_value_epoch(&self, element: usize) -> Option<u64> {
        self.element_attrs(element)
            .iter()
            .find(|attr| attr.name == "value")
            .and_then(|attr| match &attr.value {
                AttrValue::Text(read) => read.epoch(self),
                AttrValue::Bool(_) => None,
            })
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
        self.presentation_builds
            .set(self.presentation_builds.get().wrapping_add(1));
        let tree = self.tree(key)?;
        let mut roots = Vec::new();
        collect_snapshot(self, &tree.nodes, &mut roots);
        let mut row_bands = Vec::new();
        collect_row_bands(self, &tree.nodes, &mut row_bands);
        let focus = self
            .presented_focus_in(key)
            .map(|element| self.elements[element].id);
        Some(PresentationSnapshot {
            roots,
            focus,
            appearance: self.appearance,
            row_bands,
        })
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
                Resource::Node(node) => self.nodes.release(node),
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

    pub(crate) fn begin_window(&mut self, key: &'static str) -> (u32, u32) {
        self.window_factory_ran = true;
        // 旧 activation 的处理器不能进入新 factory；正常关闭已经清理过。
        self.drop_handlers.remove(key);
        self.keydown_handlers.remove(key);
        self.command_handlers.remove(key);
        self.menu_state_handlers.remove(key);
        let activation = self
            .windows
            .get(key)
            .map_or(1, |record| record.activation + 1);
        let owner = self.create_owner(Some(0));
        self.windows.insert(
            key,
            WindowRecord {
                activation,
                owner,
                open: true,
                visible: true,
            },
        );
        self.bump_window_revision();
        (activation, owner)
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
    slot: StyleSlot,
    current: Option<zenui_style::StyleValue>,
    fault: Option<ReadFault>,
}

pub struct MountedAttr {
    name: &'static str,
    value: AttrValue,
    current: Option<AttrCurrent>,
    /// 本次求值失败；`current` 仍是最后已提交值。
    fault: Option<ReadFault>,
}

#[derive(Clone, PartialEq, Eq)]
enum AttrCurrent {
    Text(String),
    Bool(bool),
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
    when: Read<bool>,
    then: ChildrenFactory,
    fallback: Option<ChildrenFactory>,
    active: Option<MountedBranch>,
}

pub struct MountedBranch {
    which: bool,
    owner: u32,
    children: Vec<MountedNode>,
}

struct ForData {
    owner: u32,
    node: NodeRef,
    each: Memo<RowSet>,
    key: Box<dyn Fn(TodoId) -> TodoId>,
    item: Box<dyn Fn(TodoId) -> MountResult<MountPlan>>,
    /// 窗口化 For 的统一行高；`None` 挂载全部行（ADR-0010）。
    row_height: Option<Length>,
    /// 跨快照稳定的行带键；宿主按它声明行窗口。
    band: u64,
    total_rows: usize,
    first_row: usize,
    /// 窗口化 For 的上下 spacer 元素槽位；保持内容总高度不变。
    spacers: Option<(usize, usize)>,
    /// 最近一次完整映射的 row set 投影；同一份投影不重复映射与查重。
    rows: Option<RowSet>,
    /// 完整的 keyed 顺序 `(mapped, todo)`，只在 row set 变化时重建。
    entries: Vec<(TodoId, TodoId)>,
    /// 完整顺序里每个 key 的行号；窗口更新与焦点 grace 的 O(log n) 查表。
    row_of: BTreeMap<TodoId, usize>,
    items: BTreeMap<TodoId, MountedItem>,
    /// 当前已挂载的行键，按业务顺序；窗口外的行不在其中。
    order: Vec<TodoId>,
}

pub struct MountedItem {
    owner: u32,
    todo: TodoId,
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
fn ensure_spacers(state: &mut State, owner: u32, parent: Option<NodeRef>) -> (usize, usize) {
    let mut make = || {
        let slot = state.reserve_element(owner);
        let id = state.mint_element_node(slot, owner, parent);
        let aux = state.nodes.reserve_element_aux(AuxSeed {
            attrs: vec![MountedAttr {
                name: "role",
                value: AttrValue::Text(Read::Static(String::from("spacer"))),
                current: Some(AttrCurrent::Text(String::from("spacer"))),
                fault: None,
            }],
            class_styles: &[],
            inline_styles: vec![MountedInlineStyle {
                property: Property::Height,
                slot: StyleSlot::Length(Read::Static(Length::px(0.0))),
                current: Some(StyleValue::Length(Length::px(0.0))),
                fault: None,
            }],
            events: Vec::new(),
            edit: LocalState::None,
            semantics: SnapshotSemantics::new(false, SnapshotActions::default()),
        });
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
    let changed = {
        let Some(style) = state
            .element_inline_styles_mut(spacer)
            .iter_mut()
            .find(|style| style.property == Property::Height)
        else {
            return;
        };
        let next = Some(StyleValue::Length(height));
        let changed = style.current != next;
        style.current = next;
        style.slot = StyleSlot::Length(Read::Static(height));
        changed
    };
    if changed {
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

    fn attrs_mut(&mut self) -> &mut Vec<MountedAttr> {
        self.attrs.as_mut().expect("element attrs are taken")
    }

    fn styles_mut(&mut self) -> &mut Vec<MountedInlineStyle> {
        self.styles.as_mut().expect("element styles are taken")
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
                                if state.nodes.text(*node).value == *text
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
        context::install(&state, || {
            with_state(|state| state.fault = None);
            let plan = match with_owner(0, || catch_panic(root)) {
                Ok(Ok(plan)) => plan,
                Ok(Err(error)) => {
                    with_state(State::release_root_resources);
                    return Err(error);
                }
                Err(panic) => {
                    with_state(State::release_root_resources);
                    return Err(MountError::Panic(panic));
                }
            };
            let mut tree =
                match with_tree_context(TreeSlot::Root, || self.mount_plan(plan, 0, None)) {
                    Ok(tree) => tree,
                    Err(error) => {
                        with_state(State::release_root_resources);
                        return Err(error);
                    }
                };
            if let Some(fault) = with_state(|state| state.fault.take()) {
                self.release_nodes(&mut tree);
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
                    },
                );
                state.mark_tree_changed(TreeSlot::Root);
                state.phase = Phase::Idle;
            });
            self.refresh_internal()
        })
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
            state.fault = None;
        });
        if let Some(mut nodes) = tree {
            self.release_nodes(&mut nodes);
        }
        with_state(|state| state.dispose_owner(owner));
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
            with_state(|state| state.fault = None);
            let (activation, owner) = with_state(|state| state.begin_window(key));
            let window = WindowCtx::new(state.borrow().id, key, activation, owner);
            let plan = with_owner(owner, || {
                let _phase = PhaseGuard::enter(Phase::Setup);
                catch_panic(|| factory(window))
            });
            let plan = match plan {
                Ok(Ok(plan)) => plan,
                Ok(Err(error)) => {
                    self.abort_open(key, activation, owner, None);
                    return Err(WindowError::Mount(error));
                }
                Err(panic) => {
                    self.abort_open(key, activation, owner, None);
                    return Err(WindowError::Mount(MountError::Panic(panic)));
                }
            };
            if let Some(id) = with_state(|state| state.missing_command_handler(key)) {
                self.abort_open(key, activation, owner, None);
                return Err(WindowError::Command(CommandError::MissingHandler(id)));
            }
            let tree = match with_tree_context(TreeSlot::Window(key), || {
                self.mount_plan(plan, owner, None)
            }) {
                Ok(tree) => tree,
                Err(error) => {
                    self.abort_open(key, activation, owner, None);
                    return Err(WindowError::Mount(error));
                }
            };
            if let Some(fault) = with_state(|state| state.fault.take()) {
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

    /// 声明一个窗口化 `For` 的挂载窗口。
    ///
    /// `band` 来自快照的 [`SnapshotRowBand::key`]；`first`/`count` 是宿主从行带
    /// 几何和滚动偏移换算出的行区间。运行时在这里挂载进入窗口的行、卸载离开
    /// 窗口的行并更新上下 spacer；聚焦行在窗口外一行以内时窗口自动扩大一行，
    /// 更远时该行被卸载且焦点清除。返回是否真的有变化；未变化的请求不刷新
    /// 也不改变呈现版本。窗口、行带已失效时返回 [`WindowError::Stale`]。
    pub fn set_row_window(
        &mut self,
        window: WindowCtx,
        band: u64,
        first: usize,
        count: usize,
    ) -> Result<bool, WindowError> {
        let state = self.state.clone();
        if window.runtime != state.borrow().id {
            return Err(WindowError::WrongRuntime);
        }
        context::install(&state, || {
            if !with_state(|state| state.window_live_in_tree(window)) {
                return Err(WindowError::Stale);
            }
            let key = TreeSlot::Window(window.key);
            if !with_state(|state| state.tree_has_band(key, band)) {
                return Err(WindowError::Stale);
            }
            let changed = with_state(|state| {
                let request = RowWindow { first, count };
                if state.row_windows.get(&band) == Some(&request) {
                    return false;
                }
                state.row_windows.insert(band, request);
                state.mark_tree_changed(key);
                true
            });
            if changed {
                self.refresh_tree_once(key).map_err(WindowError::Mount)?;
            }
            Ok(changed)
        })
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
        self.state.borrow_mut().read_faults.clear();
        self.settle_memos();
        // settle 在 Phase::Memo 中运行；非法写入要像刷新期写入一样冒泡为
        // Context 错误，而不是被随后每棵树的 fault 重置吞掉。
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
        if let Err(error) = self.refresh_all_once() {
            // 已完成的窗口保留本帧结果；不能让焦点身份停留在已被移除或禁用的旧目标上。
            self.clear_invalid_focus();
            return Err(error);
        }
        self.settle_focus()
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
    /// 处理器读取的 memo 保持惰性，也不会有挂载期副作用。
    fn settle_memos(&mut self) {
        let state = self.state.clone();
        context::install(&state, || {
            let live = with_state(|state| state.live_memo_set());
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
        for key in self.tree_keys() {
            self.refresh_tree_once(key)?;
        }
        Ok(())
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
    fn settle_focus(&mut self) -> Result<(), MountError> {
        const MAX_SETTLEMENTS: usize = 8;

        let state = self.state.clone();
        context::install(&state, || {
            for _ in 0..MAX_SETTLEMENTS {
                if !self.settle_round(true)? {
                    return Ok(());
                }
                self.refresh_all_once()?;
            }
            // 第 8 轮恰好收敛时焦点仍然有效；只有确实还有待结算的目标才做兜底清理。
            if self.settle_round(false)? {
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

    fn mount_node(
        &mut self,
        node: PlanNode,
        parent: u32,
        parent_node: Option<NodeRef>,
    ) -> Result<MountedNode, MountError> {
        match node {
            PlanNode::Text(slot) => {
                let payload = with_state(|state| {
                    let node = state.register_node(NodeKindTag::Text, parent, parent_node);
                    let payload = state.nodes.reserve_text(TextData {
                        source: slot,
                        value: String::new(),
                        fault: None,
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
                    let mut attrs = Vec::new();
                    let mut events = Vec::new();
                    for attr in plan.attrs {
                        match attr {
                            Attr::Value { name, value } => attrs.push(MountedAttr {
                                name,
                                value,
                                current: None,
                                fault: None,
                            }),
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
                    let inline_styles = plan
                        .inline_styles
                        .into_iter()
                        .map(|InlineStyle { property, slot }| MountedInlineStyle {
                            property,
                            slot,
                            current: None,
                            fault: None,
                        })
                        .collect();
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
                    let payload = state.nodes.reserve_show(ShowData {
                        owner,
                        node: show_node,
                        when: plan.when,
                        then: plan.then,
                        fallback: plan.fallback,
                        active: None,
                    });
                    state
                        .nodes
                        .set_payload(show_node, payload)
                        .expect("the Show node is live");
                    payload
                });
                if let Err(error) = self.refresh_show(payload) {
                    with_state(|state| state.dispose_owner(owner));
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
                let payload = with_state(|state| {
                    let payload = state.nodes.reserve_for(ForData {
                        owner,
                        node: for_node,
                        each: plan.each,
                        key: plan.key,
                        item: plan.item,
                        row_height: plan.row_height,
                        band,
                        total_rows: 0,
                        first_row: 0,
                        spacers: None,
                        rows: None,
                        entries: Vec::new(),
                        row_of: BTreeMap::new(),
                        items: BTreeMap::new(),
                        order: Vec::new(),
                    });
                    state
                        .nodes
                        .set_payload(for_node, payload)
                        .expect("the For node is live");
                    payload
                });
                if let Err(error) = self.refresh_for(payload) {
                    with_state(|state| state.dispose_owner(owner));
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
            let owner = with_state(|state| match node {
                MountedNode::Text(_) => None,
                MountedNode::Element(id) => Some(state.element_owner(id)),
                MountedNode::Component(id) => Some(state.components[id].owner),
                MountedNode::Show(payload) => Some(state.nodes.show(payload).owner),
                MountedNode::For(payload) => Some(state.nodes.for_node(payload).owner),
            });
            if let Some(owner) = owner {
                with_state(|state| {
                    state.dispose_owner(owner);
                    state.unmounts += 1;
                });
            }
        }
    }

    fn refresh_nodes(&mut self, nodes: &mut [MountedNode]) -> Result<(), MountError> {
        for node in nodes {
            self.refresh_node(node)?;
        }
        Ok(())
    }

    fn refresh_node(&mut self, node: &mut MountedNode) -> Result<(), MountError> {
        match node {
            MountedNode::Text(payload) => {
                let source = with_state(|state| state.nodes.text(*payload).source.clone());
                let next = source.evaluate();
                let fault = with_state(|state| match next {
                    ReadValue::Value(value) => {
                        let changed = {
                            let text = state.nodes.text_mut(*payload);
                            text.fault = None;
                            if text.value == value {
                                false
                            } else {
                                text.value = value;
                                true
                            }
                        };
                        if changed {
                            state.mark_building_tree_changed();
                        }
                        None
                    }
                    ReadValue::Fault(fault) => {
                        // 读取失败保留最后已提交文本；fault 与值并列。
                        state.nodes.text_mut(*payload).fault = Some(fault.clone());
                        Some(fault)
                    }
                });
                if let Some(fault) = fault {
                    with_state(|state| state.record_read_fault(fault));
                }
                Ok(())
            }
            MountedNode::Element(id) => {
                self.refresh_element(*id);
                let mut guard = ChildrenGuard::take(ChildrenOwner::Element(*id));
                let result = self.refresh_nodes(guard.children_mut());
                guard.restore();
                result
            }
            MountedNode::Component(id) => {
                let mut guard = ChildrenGuard::take(ChildrenOwner::Component(*id));
                let result = self.refresh_nodes(guard.children_mut());
                guard.restore();
                result
            }
            MountedNode::Show(payload) => self.refresh_show(*payload),
            MountedNode::For(payload) => self.refresh_for(*payload),
        }
    }

    fn refresh_element(&mut self, id: usize) {
        // 属性求值会读取用户 memo；panic 时 guard 把旧 attrs/styles 放回元素，
        // 已提交界面因此保持完整。
        let mut scratch = ElementScratchGuard::take(id);
        let mut attrs_changed = false;
        let mut faults = Vec::new();
        for attr in scratch.attrs_mut() {
            let evaluated = match &attr.value {
                AttrValue::Text(read) => match read.evaluate() {
                    ReadValue::Value(value) => ReadValue::Value(AttrCurrent::Text(value)),
                    ReadValue::Fault(fault) => ReadValue::Fault(fault),
                },
                AttrValue::Bool(read) => match read.evaluate() {
                    ReadValue::Value(value) => ReadValue::Value(AttrCurrent::Bool(value)),
                    ReadValue::Fault(fault) => ReadValue::Fault(fault),
                },
            };
            match evaluated {
                ReadValue::Value(next) => {
                    let next = Some(next);
                    if attr.current != next {
                        attrs_changed = true;
                    }
                    attr.current = next;
                    attr.fault = None;
                }
                ReadValue::Fault(fault) => {
                    // 失败不替换 current：属性保持最后已提交值，首次失败则缺席。
                    attr.fault = Some(fault.clone());
                    faults.push(fault);
                }
            }
        }
        let mut styles_changed = false;
        for style in scratch.styles_mut() {
            match style.slot.evaluate() {
                ReadValue::Value(value) => {
                    let next = Some(value);
                    if style.current != next {
                        styles_changed = true;
                    }
                    style.current = next;
                    style.fault = None;
                }
                ReadValue::Fault(fault) => {
                    style.fault = Some(fault.clone());
                    faults.push(fault);
                }
            }
        }
        if !faults.is_empty() {
            with_state(|state| {
                for fault in faults {
                    state.record_read_fault(fault);
                }
            });
        }
        // 值绑定的写入代数要在同一份新属性上读取，且不能嵌套借用 State。
        let (committed, epoch) = with_state(|state| {
            let attrs = scratch.attrs_mut();
            let committed = attrs
                .iter()
                .find(|attr| attr.name == "value")
                .and_then(|attr| attr.current.as_ref())
                .and_then(|current| match current {
                    AttrCurrent::Text(text) => Some(text.as_str()),
                    AttrCurrent::Bool(_) => None,
                })
                .unwrap_or_default()
                .to_owned();
            let epoch = attrs
                .iter()
                .find(|attr| attr.name == "value")
                .and_then(|attr| match &attr.value {
                    AttrValue::Text(read) => read.epoch(state),
                    AttrValue::Bool(_) => None,
                });
            (committed, epoch)
        });
        with_state(|state| {
            let attrs = scratch.take_attrs();
            let styles = scratch.take_styles();
            let mut local_changed = false;
            let event = {
                if let LocalState::Checkbox { checked } = state.element_local_mut(id)
                    && let Some(AttrCurrent::Bool(value)) = attrs
                        .iter()
                        .find(|attr| attr.name == "checked")
                        .and_then(|attr| attr.current.clone())
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
        // `when` 的求值可能读到毒化且没有最后值的 memo；旧分支保持挂载。
        let active = match catch_panic(|| show.when.evaluate()) {
            Ok(ReadValue::Value(active)) => active,
            Ok(ReadValue::Fault(fault)) => {
                // 读取失败不伪造 false：保持当前分支，并报告 typed fault。
                // 分支里的其他绑定仍要在这一帧刷新，不能被结构触发冻结。
                with_state(|state| state.record_read_fault(fault));
                if let Some(branch) = &mut show.active {
                    return self.refresh_nodes(&mut branch.children);
                }
                return Ok(());
            }
            Err(panic) => return Err(MountError::Panic(panic)),
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
            if let Some(branch) = &mut show.active {
                return self.refresh_nodes(&mut branch.children);
            }
            return Ok(());
        }

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
                        with_state(|state| state.dispose_owner(owner));
                        return Err(error);
                    }
                    Err(panic) => {
                        with_state(|state| state.dispose_owner(owner));
                        return Err(MountError::Panic(panic));
                    }
                };
                let children = match self.mount_plan(plan, owner, Some(show.node)) {
                    Ok(children) => children,
                    Err(error) => {
                        with_state(|state| state.dispose_owner(owner));
                        return Err(error);
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
                    state.unmounts += 1;
                });
                return Err(error);
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
        show.active = replacement;
        if branch_changed {
            with_state(State::mark_building_tree_changed);
        }
        Ok(())
    }

    fn refresh_for(&mut self, payload: u32) -> Result<(), MountError> {
        let data = with_state(|state| state.nodes.take_for(payload));
        let mut guard = ForPayloadGuard {
            payload,
            data: Some(data),
        };
        let for_node = guard.data_mut();
        // 源求值失败时旧行与旧窗口保持不变。
        let rows: RowSet = match catch_panic(|| for_node.each.get()) {
            Ok(rows) => rows,
            Err(panic) => return Err(MountError::Panic(panic)),
        };
        // 完整的 keyed 顺序只在 row set 投影变化时重建：重复键在窗口化之前
        // 就拒绝，窗口只影响挂载哪些行，绝不改变行顺序（ADR-0011）。key 只应
        // 依赖行身份，因此同一份投影可以复用映射结果；窗口更新是 O(窗口)。
        if for_node
            .rows
            .as_ref()
            .is_none_or(|cached| !cached.shares_with(&rows))
        {
            let mut entries = Vec::with_capacity(rows.len());
            let mut seen = BTreeSet::new();
            for key in rows.iter() {
                let todo = TodoId::from_key(key);
                // key 选择器是用户代码；panic 时不改写已提交的行映射。
                let mapped = match catch_panic(|| (for_node.key)(todo)) {
                    Ok(mapped) => mapped,
                    Err(panic) => return Err(MountError::Panic(panic)),
                };
                if !seen.insert(mapped) {
                    return Err(MountError::DuplicateKey);
                }
                entries.push((mapped, todo));
            }
            for_node.row_of = entries
                .iter()
                .enumerate()
                .map(|(index, (mapped, _))| (*mapped, index))
                .collect();
            for_node.entries = entries;
            for_node.rows = Some(rows);
        }
        let total = for_node.entries.len();
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
                        .and_then(|(key, _)| for_node.row_of.get(key).copied())
                })
            });
            if let Some(row) = focused {
                if row + 1 == first {
                    first = row;
                    count += 1;
                } else if row == first + count {
                    count += 1;
                }
                count = count.min(total - first);
                if row < first || row >= first + count {
                    with_state(|state| state.window_focus_dropped = state.refresh_focus);
                }
            }
        }

        let mut order = Vec::with_capacity(count);
        let mut fresh = Vec::new();
        for index in first..first + count {
            let (mapped, todo) = for_node.entries[index];
            order.push(mapped);
            if for_node.items.contains_key(&mapped) {
                continue;
            }
            let owner = with_state(|state| state.create_owner(Some(for_node.owner)));
            let plan = match with_owner(owner, || catch_panic(|| (for_node.item)(todo))) {
                Ok(Ok(plan)) => plan,
                Ok(Err(error)) => {
                    with_state(|state| state.dispose_owner(owner));
                    self.release_items(&mut fresh);
                    return Err(error);
                }
                Err(panic) => {
                    with_state(|state| state.dispose_owner(owner));
                    self.release_items(&mut fresh);
                    return Err(MountError::Panic(panic));
                }
            };
            let children = match self.mount_plan(plan, owner, Some(for_node.node)) {
                Ok(children) => children,
                Err(error) => {
                    with_state(|state| state.dispose_owner(owner));
                    self.release_items(&mut fresh);
                    return Err(error);
                }
            };
            let instance = with_state(|state| {
                let instance = state.next_instance;
                state.next_instance += 1;
                instance
            });
            fresh.push((
                mapped,
                MountedItem {
                    owner,
                    todo,
                    instance,
                    children,
                },
            ));
        }

        let keep: BTreeSet<TodoId> = order.iter().copied().collect();
        let removed: Vec<TodoId> = for_node
            .items
            .keys()
            .filter(|key| !keep.contains(key))
            .copied()
            .collect();
        for key in removed {
            if let Some(item) = for_node.items.remove(&key) {
                with_state(|state| clear_history_in_nodes(state, &item.children));
                with_state(|state| {
                    state.dispose_owner(item.owner);
                    state.unmounts += 1;
                });
            }
        }
        for (key, item) in fresh {
            for_node.items.insert(key, item);
        }
        for_node.order = order;
        let structural_changed = previous_order != for_node.order
            || previous_first != first
            || previous_total != total
            || (!had_spacers && for_node.spacers.is_some());
        if structural_changed {
            with_state(State::mark_building_tree_changed);
        }
        for item in for_node.items.values_mut() {
            self.refresh_nodes(&mut item.children)?;
        }
        if let (Some(row_height), Some((top, bottom))) = (for_node.row_height, for_node.spacers) {
            let mounted = for_node.order.len().min(total - first);
            with_state(|state| {
                set_spacer_height(state, top, first, row_height);
                set_spacer_height(state, bottom, total - first - mounted, row_height);
            });
        }
        for_node.total_rows = total;
        for_node.first_row = first;
        Ok(())
    }

    fn release_items(&mut self, items: &mut Vec<(TodoId, MountedItem)>) {
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
            Ok(report)
        })
    }

    /// 路由后的激活：先运行最近 click handler，再执行 `type=submit` 表单默认。
    fn activate_element(&mut self, element: usize) -> Result<TurnReport, DispatchError> {
        let node = with_state(|state| state.elements[element].id);
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
        context::install(&state, || match target {
            EventTarget::Node(node) => {
                let element = with_state(|state| state.element_for_node(node))?;
                if with_state(|state| state.element_local(element).is_composing()) {
                    return self.keydown_node(node, key);
                }
                let window = with_state(|state| {
                    state.tree_for_element(element).and_then(|tree| match tree {
                        TreeSlot::Window(key) => state.window_ctx(key),
                        TreeSlot::Root => None,
                    })
                });
                self.route_keydown(Some(element), window, key, modifiers)
            }
            EventTarget::Window(window) => {
                if !with_state(|state| state.intent_window_usable(window)) {
                    return Err(DispatchError::TargetClosed);
                }
                let focused =
                    with_state(|state| state.presented_focus_in(TreeSlot::Window(window.key)));
                self.route_keydown(focused, Some(window), key, modifiers)
            }
            EventTarget::App => Ok(TurnReport::default()),
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
    pub fn set_caret_node(&mut self, node: NodeId, offset: usize) -> Result<(), DispatchError> {
        self.place_selection(node, NodeOperation::SetCaret, Movement::Caret(offset))
    }

    /// 设置 selection；起点终点按字符夹取并归一化，不派发事件。
    pub fn set_selection_node(
        &mut self,
        node: NodeId,
        start: usize,
        end: usize,
    ) -> Result<(), DispatchError> {
        self.place_selection(
            node,
            NodeOperation::SetSelection,
            Movement::Selection(start, end),
        )
    }

    /// 选中全部文本；caret 停在末尾，不派发事件。
    pub fn select_all_node(&mut self, node: NodeId) -> Result<(), DispatchError> {
        self.place_selection(node, NodeOperation::SelectAll, Movement::SelectAll)
    }

    fn place_selection(
        &mut self,
        node: NodeId,
        operation: NodeOperation,
        movement: Movement,
    ) -> Result<(), DispatchError> {
        let mut state = self.state.borrow_mut();
        let element = state.element_for_node(node)?;
        state.require_editable_text_input(node, element, operation)?;
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
        let key = state.tree_for_element(element);
        state.mark_element_changed(element);
        // 直接 selection 原语不经过 refresh；提交 revision 让宿主看到编辑状态变化。
        if let Some(key) = key {
            state.commit_tree_revision(key);
        }
        Ok(())
    }

    fn apply_text_edit(
        &mut self,
        node: NodeId,
        element: usize,
        operation: NodeOperation,
        edit: Edit<'_>,
    ) -> Result<TurnReport, DispatchError> {
        with_state(|state| state.require_editable_text_input(node, element, operation))?;
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
            return Ok(TurnReport::default());
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
            return Ok(report);
        }
        if let Err(error) = self.finish_command() {
            with_state(|state| abort_text_dispatch(state, element));
            return Err(DispatchError::Command(error));
        }
        with_state(|state| clamp_local_edit(state, element));
        self.record_text_edit(element, before, kind);
        report.mark_changed();
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
    pub fn undo_node(&mut self, node: NodeId) -> Result<(), DispatchError> {
        self.apply_history(node, NodeOperation::Undo, HistoryDirection::Undo)
    }

    /// 重做该输入最近一个被撤销的撤销步；空栈是静默无操作。
    ///
    /// 重做同样通过一次 `on:input` 写回，恢复的是该步提交后的状态；合并 run
    /// 的重做仍是一步。
    pub fn redo_node(&mut self, node: NodeId) -> Result<(), DispatchError> {
        self.apply_history(node, NodeOperation::Redo, HistoryDirection::Redo)
    }

    fn apply_history(
        &mut self,
        node: NodeId,
        operation: NodeOperation,
        direction: HistoryDirection,
    ) -> Result<(), DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            with_state(|state| state.require_editable_text_input(node, element, operation))?;
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
                return Ok(());
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
            Ok(())
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
            Ok(report)
        })
    }

    /// headless 焦点命令：先对旧目标派发一次 blur，再对新目标派发一次 focus。
    pub fn focus(&mut self, label: &str) -> Result<(), MountError> {
        self.dispatch_label(
            FindTarget::Label(label.to_owned()),
            "focus target not found",
            "focus target is not focusable",
            Self::focus_node,
        )
    }

    /// 按 Node ID 移动焦点；不可聚焦的目标返回 `WrongKind`，焦点保持不变。
    pub fn focus_node(&mut self, node: NodeId) -> Result<(), DispatchError> {
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
            self.move_focus(tree, Some(element))
                .map_err(DispatchError::Command)
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
        self.state.borrow().core.counters()
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

    /// 已构建的 presentation snapshot 次数；用于验证滚动与指针输入不重建快照。
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
            for_node.band == band
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
                out.push(SnapshotNode::Text(state.nodes.text(*payload).value.clone()));
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
    let mut styles = class_styles.to_vec();
    styles.extend(inline_styles.iter().filter_map(|style| {
        style
            .current
            .map(|value| zenui_style::StyleDeclaration::inline(style.property, value))
    }));
    std::borrow::Cow::Owned(styles)
}

fn snapshot_attrs(state: &State, element: usize) -> Vec<SnapshotAttr> {
    let mut attrs: Vec<SnapshotAttr> = state
        .element_attrs(element)
        .iter()
        .filter_map(|attr| {
            attr.current.as_ref().map(|current| SnapshotAttr {
                name: attr.name,
                value: match current {
                    AttrCurrent::Text(text) => SnapshotValue::Text(text.clone()),
                    AttrCurrent::Bool(value) => SnapshotValue::Bool(*value),
                },
            })
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
            MountedNode::Text(payload) => texts.push(state.nodes.text(*payload).value.clone()),
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
                for key in &for_node.order {
                    if let Some(item) = for_node.items.get(key) {
                        items.push(ItemInfo {
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
                    <For each={rows} key={|id: TodoId| id} row-height={px(20.0)}>
                        {move |id: TodoId| view! { <RecycledRow id={id} store={store} /> }}
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
            );
            assert!(
                app.set_row_window(window, band, 0, 32)
                    .expect("restore the row window")
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
            );
            assert!(
                app.set_row_window(window, band, 0, 32)
                    .expect("restore the row window")
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
                <For each={rows} key={|id: TodoId| id}>
                    {move |id: TodoId| view! { <ArenaRow id={id} store={store} /> }}
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
