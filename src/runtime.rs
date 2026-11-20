use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

use zenui_core::{Counters, RowSet};

use crate::context::{self, ContextFault, DepKey, Phase, with_state};
use crate::dispatch::{DispatchError, NodeOperation};
use crate::presentation::{
    NodeId, NodeStatus, PresentationSnapshot, SnapshotAttr, SnapshotElement, SnapshotNode,
    SnapshotValue, TextEditState,
};
use crate::signal::{Memo, MemoCell, SignalCell};
use crate::store::{TodoId, WriteError};
use crate::task::{TaskRecord, WakeCallback};
use crate::view::{
    Attr, AttrValue, BlurEvent, ChildrenFactory, CompositionEvent, CompositionPhase, Event,
    FocusCause, FocusEvent, Key, KeyEvent, MountError, MountPlan, MountResult, PlanNode,
    ResultBoolValue, SubmitEvent, TextSlot,
};
use crate::window::{WindowCtx, WindowError, WindowRecord};

pub(crate) enum Resource {
    Signal(u32),
    Memo(u32),
    Event(usize),
}

pub(crate) struct OwnerRecord {
    parent: Option<u32>,
    children: Vec<u32>,
    resources: Vec<Resource>,
    disposed: bool,
}

static NEXT_RUNTIME: AtomicU32 = AtomicU32::new(1);

pub(crate) struct State {
    pub(crate) id: u32,
    pub(crate) read_only: u32,
    pub(crate) write_error: Option<WriteError>,
    pub(crate) core: zenui_core::Runtime,
    pub(crate) phase: Phase,
    pub(crate) fault: Option<ContextFault>,
    pub(crate) signals: Vec<SignalCell>,
    pub(crate) memos: Vec<MemoCell>,
    pub(crate) reverse: BTreeMap<DepKey, BTreeSet<u32>>,
    pub(crate) computing: Vec<(u32, Vec<DepKey>)>,
    pub(crate) once_reads: u64,
    owners: BTreeMap<u32, OwnerRecord>,
    next_owner: u32,
    pub(crate) current_owner: Option<u32>,
    events: Vec<Event>,
    elements: Vec<MountedElement>,
    components: Vec<MountedComponent>,
    tree: Option<Vec<MountedNode>>,
    tree_owner: Option<u32>,
    tree_window: Option<&'static str>,
    windows: BTreeMap<&'static str, WindowRecord>,
    pub(crate) tasks: BTreeMap<u32, TaskRecord>,
    pub(crate) next_task: u32,
    pub(crate) wake_callback: Arc<RwLock<Option<WakeCallback>>>,
    setup_counts: BTreeMap<&'static str, u64>,
    next_instance: u64,
    next_node_mount: u64,
    unmounts: u64,
    focused: Option<usize>,
    settle_before: Option<Vec<usize>>,
}

/// 旧焦点离开可见集合后的一次结算：旧目标（可选 blur）与接任目标。
struct FocusSettlement {
    old: usize,
    /// 旧目标仍在挂载树中（例如刚被禁用）时，需要向它派发一次 blur。
    blur_old: bool,
    target: Option<usize>,
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

impl State {
    fn new() -> Self {
        let mut owners = BTreeMap::new();
        owners.insert(
            0,
            OwnerRecord {
                parent: None,
                children: Vec::new(),
                resources: Vec::new(),
                disposed: false,
            },
        );
        Self {
            id: NEXT_RUNTIME.fetch_add(1, Ordering::Relaxed),
            read_only: 0,
            write_error: None,
            core: zenui_core::Runtime::new(),
            phase: Phase::Idle,
            fault: None,
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
            tree: None,
            tree_owner: None,
            tree_window: None,
            windows: BTreeMap::new(),
            tasks: BTreeMap::new(),
            next_task: 1,
            wake_callback: Arc::new(RwLock::new(None)),
            setup_counts: BTreeMap::new(),
            next_instance: 1,
            next_node_mount: 0,
            unmounts: 0,
            focused: None,
            settle_before: None,
        }
    }

    /// 每次元素挂载分配新的 generation；旧挂载的身份不会命中重新挂载的元素。
    fn mint_node(&mut self) -> NodeId {
        let id = NodeId::mint(
            self.id,
            u32::try_from(self.elements.len()).expect("node identity exhausted"),
            self.next_node_mount,
        );
        self.next_node_mount += 1;
        id
    }

    /// 当前呈现焦点：仍在已提交树中且可聚焦。
    fn presented_focus(&self) -> Option<usize> {
        self.focused
            .filter(|element| self.element_is_mounted(*element) && self.is_focusable(*element))
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
        let children = record.children.clone();
        let resources = std::mem::take(&mut record.resources);
        let parent = record.parent;

        self.release_resources(resources);
        for child in children {
            self.dispose_owner(child);
        }
        if let Some(parent) = parent
            && let Some(record) = self.owners.get_mut(&parent)
        {
            record.children.retain(|child| *child != id);
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
                    }
                }
                Resource::Event(event) => {
                    if let Some(cell) = self.events.get_mut(event) {
                        *cell = Event::Click(Box::new(|| {}));
                    }
                }
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
            },
        );
        (activation, owner)
    }

    pub(crate) fn mark_window_closed(&mut self, key: &'static str) {
        if let Some(record) = self.windows.get_mut(key) {
            record.open = false;
        }
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
        self.tree.is_some()
    }

    fn context_error(fault: ContextFault) -> MountError {
        MountError::Context {
            operation: fault.operation,
            phase: fault.phase,
        }
    }

    fn find_element(&self, target: &FindTarget) -> Option<usize> {
        self.tree
            .as_ref()
            .and_then(|tree| find_element(self, tree, target))
    }

    fn put_event(&mut self, id: usize, event: Event) {
        self.events[id] = event;
    }

    fn update_local(&mut self, element: usize, value: LocalUpdate) {
        match (&mut self.elements[element].local, value) {
            (
                LocalState::TextInput {
                    caret, selection, ..
                },
                LocalUpdate::Input(value),
            ) => {
                *caret = value.chars().count();
                *selection = None;
            }
            (LocalState::Checkbox { checked }, LocalUpdate::Change(value)) => {
                *checked = value;
            }
            _ => {}
        }
    }

    fn text_input(&self, target: &FindTarget) -> Option<usize> {
        let element = self.find_element(target)?;
        match self.elements[element].local {
            LocalState::TextInput { caret, .. } => Some(caret),
            _ => None,
        }
    }

    /// `None` 表示该元素不是文本输入；`Some(true)` 表示正在组合。
    fn text_input_composing(&self, element: usize) -> Option<bool> {
        match &self.elements[element].local {
            LocalState::TextInput { composition, .. } => Some(composition.is_some()),
            _ => None,
        }
    }

    /// 判定一个 Node ID 相对已提交界面的状态。
    fn node_status(&self, node: NodeId) -> NodeStatus {
        if node.runtime_id() != self.id {
            return NodeStatus::Unknown;
        }
        let Some(element) = self.elements.get(node.slot() as usize) else {
            return NodeStatus::Unknown;
        };
        // 槽位已由查表确定；代际仍须匹配，否则这是同一槽位更早一次挂载的身份。
        if element.id.slot() != node.slot() || element.id.generation() != node.generation() {
            return NodeStatus::Stale;
        }
        if self.element_is_mounted(node.slot() as usize) {
            NodeStatus::Live
        } else {
            NodeStatus::Stale
        }
    }

    /// 解析一个存活的 Node ID；未知与已卸载身份分别报错，绝不猜测替代目标。
    fn element_for_node(&self, node: NodeId) -> Result<usize, DispatchError> {
        match self.node_status(node) {
            NodeStatus::Live => Ok(node.slot() as usize),
            NodeStatus::Stale => Err(DispatchError::StaleNode(node)),
            NodeStatus::Unknown => Err(DispatchError::UnknownNode(node)),
        }
    }

    fn set_text_input_caret(&mut self, target: &FindTarget, offset: usize) -> bool {
        let Some(element) = self.find_element(target) else {
            return false;
        };
        if let LocalState::TextInput {
            caret,
            selection,
            composition,
        } = &mut self.elements[element].local
        {
            if composition.is_some() {
                return false;
            }
            *caret = offset;
            *selection = Some((offset, offset));
            true
        } else {
            false
        }
    }

    /// 原生控件与显式挂接 focus/blur 的元素参与遍历；禁用元素不参与接任或 Tab 顺序。
    fn is_focusable(&self, element: usize) -> bool {
        let element = &self.elements[element];
        if element.is_disabled() {
            return false;
        }
        matches!(element.tag, "button" | "text-input" | "checkbox")
            || element.events.iter().any(|id| {
                matches!(
                    event_kind(&self.events[*id]),
                    EventKind::Focus | EventKind::Blur
                )
            })
    }

    /// Tab/方向键共用的确定性前序：控件先于自己的子树，Show 只看活动分支，For 按业务顺序。
    fn focus_order(&self) -> Vec<usize> {
        let mut order = Vec::new();
        if let Some(tree) = &self.tree {
            collect_focusable(self, tree, &mut order);
        }
        order
    }

    /// 结算快照包含旧一代的全部挂载元素；某个控件当时禁用、本帧重新启用时仍可接任。
    fn element_order(&self) -> Vec<usize> {
        let mut order = Vec::new();
        if let Some(tree) = &self.tree {
            collect_elements(self, tree, &mut order);
        }
        order
    }

    fn element_is_mounted(&self, element: usize) -> bool {
        self.tree
            .as_ref()
            .is_some_and(|tree| contains_element(self, tree, element))
    }

    /// 旧焦点不在新的可聚焦集合时，下一次结算才产生接任目标。
    ///
    /// 只在旧一代的挂载顺序里选：同 tag 的下一控件、同 tag 的上一控件，
    /// 再退到任意下一控件、任意上一控件；本帧新挂载的控件不会因返场抢焦点，
    /// 当时禁用、本帧重新启用的旧控件仍然可以接任。
    fn focus_settlement(&mut self) -> Option<FocusSettlement> {
        let old = self.focused?;
        let after = self.focus_order();
        if after.contains(&old) {
            return None;
        }
        let before = self.settle_before.take().unwrap_or_default();
        let target = focus_replacement(self, &before, &after, old);
        Some(FocusSettlement {
            old,
            blur_old: self.element_is_mounted(old),
            target,
        })
    }

    fn take_element_event(&mut self, element: usize, kind: EventKind) -> Option<(usize, Event)> {
        let event = self
            .elements
            .get(element)?
            .events
            .iter()
            .copied()
            .find(|id| event_kind(&self.events[*id]) == kind)?;
        Some((
            event,
            std::mem::replace(&mut self.events[event], Event::Click(Box::new(|| {}))),
        ))
    }
}

pub enum MountedNode {
    Text(MountedText),
    Element(usize),
    Component(usize),
    Show(MountedShow),
    For(MountedFor),
}

pub struct MountedText {
    slot: TextSlot,
    value: String,
}

pub struct MountedElement {
    id: NodeId,
    tag: &'static str,
    owner: u32,
    attrs: Vec<MountedAttr>,
    events: Vec<usize>,
    local: LocalState,
    children: Vec<MountedNode>,
}

impl MountedElement {
    fn label(&self) -> Option<String> {
        self.label_text().map(ToOwned::to_owned)
    }

    fn label_text(&self) -> Option<&str> {
        self.attrs
            .iter()
            .find(|attr| attr.name == "aria:label")
            .and_then(|attr| attr.current.as_ref())
            .and_then(|current| match current {
                AttrCurrent::Text(text) => Some(text.as_str()),
                AttrCurrent::Bool(_) => None,
            })
    }

    fn text_value(&self) -> Option<String> {
        self.attrs
            .iter()
            .find(|attr| attr.name == "value")
            .and_then(|attr| attr.current.as_ref())
            .and_then(|current| match current {
                AttrCurrent::Text(text) => Some(text.clone()),
                AttrCurrent::Bool(_) => None,
            })
    }

    fn is_disabled(&self) -> bool {
        self.attrs
            .iter()
            .find(|attr| attr.name == "disabled")
            .and_then(|attr| attr.current.as_ref())
            .is_some_and(|current| matches!(current, AttrCurrent::Bool(true)))
    }
}

pub struct MountedAttr {
    name: &'static str,
    value: AttrValue,
    current: Option<AttrCurrent>,
}

#[derive(Clone)]
enum AttrCurrent {
    Text(String),
    Bool(bool),
}

enum LocalState {
    None,
    TextInput {
        caret: usize,
        selection: Option<(usize, usize)>,
        composition: Option<Composition>,
    },
    Checkbox {
        checked: bool,
    },
}

impl LocalState {
    fn is_composing(&self) -> bool {
        matches!(
            self,
            Self::TextInput {
                composition: Some(_),
                ..
            }
        )
    }
}

/// preedit 只存在于编辑器本地；`base_*` 记录组合开始时的已提交状态，取消时恢复。
struct Composition {
    base_caret: usize,
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

enum LocalUpdate {
    Input(String),
    Change(bool),
}

pub struct MountedComponent {
    owner: u32,
    children: Vec<MountedNode>,
}

pub struct MountedShow {
    owner: u32,
    when: Memo<bool>,
    then: ChildrenFactory,
    fallback: Option<ChildrenFactory>,
    active: Option<MountedBranch>,
}

pub struct MountedBranch {
    which: bool,
    owner: u32,
    children: Vec<MountedNode>,
}

pub struct MountedFor {
    owner: u32,
    each: Memo<RowSet>,
    key: Box<dyn Fn(TodoId) -> TodoId>,
    item: Box<dyn Fn(TodoId) -> MountResult<MountPlan>>,
    items: BTreeMap<TodoId, MountedItem>,
    order: Vec<TodoId>,
}

pub struct MountedItem {
    owner: u32,
    todo: TodoId,
    instance: u64,
    children: Vec<MountedNode>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EventKind {
    Click,
    Input,
    Change,
    Submit,
    KeyDown,
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
fn with_owner<R>(owner: u32, f: impl FnOnce() -> R) -> R {
    let previous = with_state(|state| {
        let previous = state.current_owner;
        state.current_owner = Some(owner);
        previous
    });
    let result = f();
    with_state(|state| state.current_owner = previous);
    result
}

fn find_element(state: &State, nodes: &[MountedNode], target: &FindTarget) -> Option<usize> {
    for node in nodes {
        match node {
            MountedNode::Text(_) => {}
            MountedNode::Element(id) => {
                let element = &state.elements[*id];
                let matches = match target {
                    FindTarget::Label(label) => {
                        element.label_text().is_some_and(|value| value == label)
                    }
                    FindTarget::Text(text) => element.children.iter().any(
                        |child| matches!(child, MountedNode::Text(node) if node.value == *text),
                    ),
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
            MountedNode::Show(show) => {
                if let Some(branch) = &show.active
                    && let Some(found) = find_element(state, &branch.children, target)
                {
                    return Some(found);
                }
            }
            MountedNode::For(for_node) => {
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

pub struct Runtime {
    pub(crate) state: Rc<RefCell<State>>,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(State::new())),
        }
    }

    pub fn mount(
        &mut self,
        root: impl FnOnce() -> MountResult<MountPlan> + 'static,
    ) -> Result<(), MountError> {
        if self.state.borrow().tree.is_some() {
            return Err(MountError::Component("root is already mounted"));
        }
        let state = self.state.clone();
        context::install(&state, || {
            with_state(|state| state.fault = None);
            let plan = match with_owner(0, root) {
                Ok(plan) => plan,
                Err(error) => {
                    with_state(State::release_root_resources);
                    return Err(error);
                }
            };
            let mut tree = match self.mount_plan(plan, 0) {
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
                state.tree = Some(tree);
                state.tree_owner = Some(0);
                state.tree_window = None;
                state.phase = Phase::Idle;
            });
            self.refresh()
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

    /// 打开一个 headless 窗口 activation；factory 每次打开都会重新执行。
    ///
    /// stage-one 一次只支持一个已挂载窗口，其他宿主能力返回明确错误。
    pub fn open_window(
        &mut self,
        key: &'static str,
        factory: impl Fn(WindowCtx) -> MountResult<MountPlan> + 'static,
    ) -> Result<WindowCtx, WindowError> {
        if self.state.borrow().tree.is_some() {
            return Err(WindowError::Unsupported(
                "a second headless window while one is mounted",
            ));
        }
        let state = self.state.clone();
        context::install(&state, || {
            with_state(|state| state.fault = None);
            let (activation, owner) = with_state(|state| state.begin_window(key));
            let window = WindowCtx::new(state.borrow().id, key, activation, owner);
            let previous = with_state(|state| {
                let previous = (state.current_owner, state.phase);
                state.current_owner = Some(owner);
                state.phase = Phase::Setup;
                previous
            });
            let plan = factory(window);
            with_state(|state| {
                state.current_owner = previous.0;
                state.phase = previous.1;
            });
            let tree = match plan {
                Ok(plan) => match self.mount_plan(plan, owner) {
                    Ok(tree) => tree,
                    Err(error) => {
                        self.abort_open(key, activation, owner, None);
                        return Err(WindowError::Mount(error));
                    }
                },
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
                state.tree = Some(tree);
                state.tree_owner = Some(owner);
                state.tree_window = Some(key);
                state.phase = Phase::Idle;
            });
            if let Err(error) = self.refresh() {
                let nodes = with_state(|state| state.tree.take());
                self.abort_open(key, activation, owner, nodes);
                with_state(|state| {
                    state.tree_owner = None;
                    state.tree_window = None;
                });
                return Err(WindowError::Mount(error));
            }
            Ok(window)
        })
    }

    /// 关闭 activation：释放窗口 UI、取消窗口所有权交付；应用级模型保留。
    pub fn close_window(&mut self, window: WindowCtx) -> Result<(), WindowError> {
        let state = self.state.clone();
        if window.runtime != state.borrow().id {
            return Err(WindowError::WrongRuntime);
        }
        context::install(&state, || {
            let live = {
                let state = state.borrow();
                state.window_is_live(window.key, window.activation, window.owner)
                    && state.tree_window == Some(window.key)
            };
            if !live {
                return Err(WindowError::Stale);
            }
            with_state(|state| {
                state.cancel_window(window.key, window.activation);
            });
            let nodes = with_state(|state| state.tree.take());
            if let Some(mut nodes) = nodes {
                self.release_nodes(&mut nodes);
            }
            with_state(|state| {
                state.focused = None;
                state.dispose_owner(window.owner);
                state.tree_owner = None;
                state.tree_window = None;
            });
            Ok(())
        })
    }

    pub fn window_alive(&self, window: WindowCtx) -> bool {
        let state = self.state.borrow();
        window.runtime == state.id
            && state.window_is_live(window.key, window.activation, window.owner)
    }

    pub fn refresh(&mut self) -> Result<(), MountError> {
        if let Err(error) = self.refresh_once() {
            // 失败的帧没有发布；不能让焦点身份停留在已被移除或禁用的旧目标上。
            let mut state = self.state.borrow_mut();
            let focused = state.focused;
            if focused.is_some_and(|id| !state.element_is_mounted(id) || !state.is_focusable(id)) {
                state.focused = None;
            }
            return Err(error);
        }
        self.settle_focus()
    }

    fn refresh_once(&mut self) -> Result<(), MountError> {
        let state = self.state.clone();
        context::install(&state, || {
            let Some(mut tree) = with_state(|state| {
                state.fault = None;
                state.phase = Phase::Binding;
                state.settle_before = state.focused.is_some().then(|| state.element_order());
                state.tree.take()
            }) else {
                return Err(MountError::Component("root is not mounted"));
            };
            let result = self.refresh_nodes(&mut tree);
            let fault = with_state(|state| {
                state.tree = Some(tree);
                state.phase = Phase::Idle;
                state.fault
            });
            result?;
            if let Some(fault) = fault {
                return Err(State::context_error(fault));
            }
            Ok(())
        })
    }

    /// 结构变化后的焦点结算只做一次批量决定；handler 造成的进一步变化在下一轮结算。
    fn settle_focus(&mut self) -> Result<(), MountError> {
        const MAX_SETTLEMENTS: usize = 8;

        let state = self.state.clone();
        context::install(&state, || {
            for _ in 0..MAX_SETTLEMENTS {
                let Some(settlement) = with_state(|state| state.focus_settlement()) else {
                    return Ok(());
                };
                with_state(|state| {
                    state.phase = Phase::Event;
                    state.fault = None;
                    state.write_error = None;
                });
                if settlement.blur_old {
                    with_state(|state| state.focused = None);
                    self.fire_blur(settlement.old, FocusCause::Settlement);
                }
                match settlement.target {
                    Some(target) => {
                        with_state(|state| state.focused = Some(target));
                        self.fire_focus(target, FocusCause::Settlement);
                    }
                    None => with_state(|state| state.focused = None),
                }
                with_state(|state| state.phase = Phase::Idle);
                if let Some(fault) = with_state(|state| state.fault) {
                    return Err(State::context_error(fault));
                }
                if let Some(error) = with_state(|state| state.write_error.take()) {
                    return Err(MountError::Write(error));
                }
                self.refresh_once()?;
            }
            // 第 8 轮恰好收敛时焦点仍然有效；只有确实还有待结算的目标才做兜底清理。
            let Some(settlement) = with_state(|state| state.focus_settlement()) else {
                return Ok(());
            };
            with_state(|state| {
                state.phase = Phase::Event;
                state.fault = None;
                state.write_error = None;
            });
            if settlement.blur_old {
                with_state(|state| state.focused = None);
                self.fire_blur(settlement.old, FocusCause::Settlement);
            }
            with_state(|state| state.focused = None);
            with_state(|state| state.phase = Phase::Idle);
            if let Some(fault) = with_state(|state| state.fault) {
                return Err(State::context_error(fault));
            }
            if let Some(error) = with_state(|state| state.write_error.take()) {
                return Err(MountError::Write(error));
            }
            self.refresh_once()?;
            Ok(())
        })
    }

    fn mount_plan(&mut self, plan: MountPlan, parent: u32) -> Result<Vec<MountedNode>, MountError> {
        let mut nodes = Vec::with_capacity(plan.nodes.len());
        for node in plan.nodes {
            let result = match node {
                PlanNode::Children(children) => with_owner(parent, || children.mount())
                    .and_then(|plan| self.mount_plan(plan, parent)),
                PlanNode::Factory(factory) => with_owner(parent, || factory.mount())
                    .and_then(|plan| self.mount_plan(plan, parent)),
                node => self.mount_node(node, parent).map(|node| vec![node]),
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

    fn mount_node(&mut self, node: PlanNode, parent: u32) -> Result<MountedNode, MountError> {
        match node {
            PlanNode::Text(slot) => Ok(MountedNode::Text(MountedText {
                value: String::new(),
                slot,
            })),
            PlanNode::Children(_) | PlanNode::Factory(_) => {
                unreachable!("slots are spliced by mount_plan")
            }
            PlanNode::Element(plan) => {
                let owner = with_state(|state| state.create_owner(Some(parent)));
                let children = match self.mount_plan(MountPlan::new(plan.children), owner) {
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
                            }),
                            Attr::Event(event) => {
                                let id = state.events.len();
                                state.events.push(event);
                                events.push(id);
                                if let Some(record) = state.owners.get_mut(&owner) {
                                    record.resources.push(Resource::Event(id));
                                }
                            }
                        }
                    }
                    let local = match tag {
                        "text-input" => LocalState::TextInput {
                            caret: 0,
                            selection: None,
                            composition: None,
                        },
                        "checkbox" => LocalState::Checkbox { checked: false },
                        _ => LocalState::None,
                    };
                    let id = state.mint_node();
                    let slot = state.elements.len();
                    state.elements.push(MountedElement {
                        id,
                        tag,
                        owner,
                        attrs,
                        events,
                        local,
                        children,
                    });
                    slot
                });
                Ok(MountedNode::Element(id))
            }
            PlanNode::Component(plan) => {
                let label = plan.label;
                let owner = with_state(|state| state.create_owner(Some(parent)));
                let previous = with_state(|state| {
                    let previous = (state.current_owner, state.phase);
                    state.current_owner = Some(owner);
                    state.phase = Phase::Setup;
                    previous
                });
                let mounted = (plan.mount)();
                let fault = with_state(|state| {
                    state.current_owner = previous.0;
                    state.phase = previous.1;
                    state.fault.take()
                });
                let plan = match mounted {
                    Ok(plan) => plan,
                    Err(error) => {
                        with_state(|state| state.dispose_owner(owner));
                        return Err(error);
                    }
                };
                if let Some(fault) = fault {
                    with_state(|state| state.dispose_owner(owner));
                    return Err(State::context_error(fault));
                }
                let children = match self.mount_plan(plan, owner) {
                    Ok(children) => children,
                    Err(error) => {
                        with_state(|state| state.dispose_owner(owner));
                        return Err(error);
                    }
                };
                let id = with_state(|state| {
                    *state.setup_counts.entry(label).or_default() += 1;
                    let id = state.components.len();
                    state.components.push(MountedComponent { owner, children });
                    id
                });
                Ok(MountedNode::Component(id))
            }
            PlanNode::Show(plan) => {
                let owner = with_state(|state| state.create_owner(Some(parent)));
                let mut node = MountedShow {
                    owner,
                    when: plan.when,
                    then: plan.then,
                    fallback: plan.fallback,
                    active: None,
                };
                if let Err(error) = self.refresh_show(&mut node) {
                    with_state(|state| state.dispose_owner(owner));
                    return Err(error);
                }
                Ok(MountedNode::Show(node))
            }
            PlanNode::For(plan) => {
                let owner = with_state(|state| state.create_owner(Some(parent)));
                let mut node = MountedFor {
                    owner,
                    each: plan.each,
                    key: plan.key,
                    item: plan.item,
                    items: BTreeMap::new(),
                    order: Vec::new(),
                };
                if let Err(error) = self.refresh_for(&mut node) {
                    with_state(|state| state.dispose_owner(owner));
                    return Err(error);
                }
                Ok(MountedNode::For(node))
            }
        }
    }

    fn release_nodes(&mut self, nodes: &mut Vec<MountedNode>) {
        let nodes = std::mem::take(nodes);
        for node in nodes {
            let owner = with_state(|state| match node {
                MountedNode::Text(_) => None,
                MountedNode::Element(id) => Some(state.elements[id].owner),
                MountedNode::Component(id) => Some(state.components[id].owner),
                MountedNode::Show(show) => Some(show.owner),
                MountedNode::For(for_node) => Some(for_node.owner),
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
            MountedNode::Text(text) => {
                text.value = text.slot.evaluate();
                Ok(())
            }
            MountedNode::Element(id) => {
                self.refresh_element(*id);
                let mut children =
                    with_state(|state| std::mem::take(&mut state.elements[*id].children));
                let result = self.refresh_nodes(&mut children);
                with_state(|state| state.elements[*id].children = children);
                result
            }
            MountedNode::Component(id) => {
                let mut children =
                    with_state(|state| std::mem::take(&mut state.components[*id].children));
                let result = self.refresh_nodes(&mut children);
                with_state(|state| state.components[*id].children = children);
                result
            }
            MountedNode::Show(show) => self.refresh_show(show),
            MountedNode::For(for_node) => self.refresh_for(for_node),
        }
    }

    fn refresh_element(&mut self, id: usize) {
        let mut attrs = with_state(|state| std::mem::take(&mut state.elements[id].attrs));
        for attr in &mut attrs {
            attr.current = match &attr.value {
                AttrValue::Text(slot) => Some(AttrCurrent::Text(slot.evaluate())),
                AttrValue::Bool(slot) => Some(AttrCurrent::Bool(slot.evaluate())),
                AttrValue::ResultBool(ResultBoolValue::Field(field)) => {
                    field.get().ok().map(AttrCurrent::Bool)
                }
                AttrValue::ResultBool(ResultBoolValue::Memo(memo)) => {
                    memo.get().ok().map(AttrCurrent::Bool)
                }
            };
        }
        with_state(|state| {
            let element = &mut state.elements[id];
            if let LocalState::Checkbox { checked } = &mut element.local
                && let Some(AttrCurrent::Bool(value)) = attrs
                    .iter()
                    .find(|attr| attr.name == "checked")
                    .and_then(|attr| attr.current.clone())
            {
                *checked = value;
            }
            element.attrs = attrs;
        });
    }

    fn refresh_show(&mut self, show: &mut MountedShow) -> Result<(), MountError> {
        let active = show.when.get();
        let wanted = if active {
            Some(true)
        } else if show.fallback.is_some() {
            Some(false)
        } else {
            None
        };
        if show.active.as_ref().map(|branch| branch.which) == wanted {
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
                let plan = match with_owner(owner, || factory.mount()) {
                    Ok(plan) => plan,
                    Err(error) => {
                        with_state(|state| state.dispose_owner(owner));
                        return Err(error);
                    }
                };
                let children = match self.mount_plan(plan, owner) {
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

        if let Some(branch) = show.active.take() {
            with_state(|state| {
                state.dispose_owner(branch.owner);
                state.unmounts += 1;
            });
        }
        show.active = replacement;
        if let Some(branch) = &mut show.active {
            self.refresh_nodes(&mut branch.children)?;
        }
        Ok(())
    }

    fn refresh_for(&mut self, for_node: &mut MountedFor) -> Result<(), MountError> {
        let rows: RowSet = for_node.each.get();
        let mut entries = Vec::with_capacity(rows.len());
        let mut seen = BTreeSet::new();
        for key in rows.iter() {
            let todo = TodoId::from_key(key);
            let mapped = (for_node.key)(todo);
            if !seen.insert(mapped) {
                return Err(MountError::DuplicateKey);
            }
            entries.push((mapped, todo));
        }

        let mut fresh = Vec::new();
        for (mapped, todo) in &entries {
            if for_node.items.contains_key(mapped) {
                continue;
            }
            let owner = with_state(|state| state.create_owner(Some(for_node.owner)));
            let plan = match with_owner(owner, || (for_node.item)(*todo)) {
                Ok(plan) => plan,
                Err(error) => {
                    with_state(|state| state.dispose_owner(owner));
                    self.release_items(&mut fresh);
                    return Err(error);
                }
            };
            let children = match self.mount_plan(plan, owner) {
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
                *mapped,
                MountedItem {
                    owner,
                    todo: *todo,
                    instance,
                    children,
                },
            ));
        }

        let keep: BTreeSet<TodoId> = entries.iter().map(|(mapped, _)| *mapped).collect();
        let removed: Vec<TodoId> = for_node
            .items
            .keys()
            .filter(|key| !keep.contains(key))
            .copied()
            .collect();
        for key in removed {
            if let Some(item) = for_node.items.remove(&key) {
                with_state(|state| {
                    state.dispose_owner(item.owner);
                    state.unmounts += 1;
                });
            }
        }
        for (key, item) in fresh {
            for_node.items.insert(key, item);
        }
        for_node.order = entries.iter().map(|(mapped, _)| *mapped).collect();
        for item in for_node.items.values_mut() {
            self.refresh_nodes(&mut item.children)?;
        }
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

    pub fn click_text(&mut self, text: &str) -> Result<(), MountError> {
        self.dispatch_label(
            FindTarget::Text(text.to_owned()),
            "click target not found",
            "click target not found",
            Self::click_node,
        )
    }

    pub fn click_label(&mut self, label: &str) -> Result<(), MountError> {
        self.dispatch_label(
            FindTarget::Label(label.to_owned()),
            "click target not found",
            "click target not found",
            Self::click_node,
        )
    }

    /// 按 Node ID 派发 click；只运行该元素自己的 handler。
    pub fn click_node(&mut self, node: NodeId) -> Result<(), DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            let (event_id, mut event) =
                self.take_node_event(node, element, EventKind::Click, NodeOperation::Click)?;
            if let Event::Click(handler) = &mut event {
                handler();
            }
            with_state(|state| {
                state.put_event(event_id, event);
                state.phase = Phase::Idle;
            });
            self.finish_command().map_err(DispatchError::Command)
        })
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
    fn dispatch_label(
        &mut self,
        target: FindTarget,
        not_found: &'static str,
        wrong_kind: &'static str,
        dispatch: impl FnOnce(&mut Self, NodeId) -> Result<(), DispatchError>,
    ) -> Result<(), MountError> {
        let Some(node) = self.label_node(&target) else {
            return Err(MountError::Component(not_found));
        };
        dispatch(self, node).map_err(|error| match error {
            DispatchError::UnknownNode(_) | DispatchError::StaleNode(_) => {
                MountError::Component(not_found)
            }
            DispatchError::WrongKind { .. } => MountError::Component(wrong_kind),
            DispatchError::Rejected { reason, .. } => MountError::Component(reason),
            DispatchError::Command(error) => error,
        })
    }

    fn label_node(&self, target: &FindTarget) -> Option<NodeId> {
        let state = self.state.borrow();
        let element = state.find_element(target)?;
        Some(state.elements[element].id)
    }

    pub fn input(&mut self, label: &str, value: &str) -> Result<(), MountError> {
        let target = FindTarget::Label(label.to_owned());
        let state = self.state.clone();
        context::install(&state, || {
            let Some(element) = with_state(|state| state.find_element(&target)) else {
                return Err(MountError::Component("input target not found"));
            };
            if with_state(|state| state.elements[element].local.is_composing()) {
                return Err(MountError::Component("text input is composing"));
            }
            let Some((event_id, mut event)) =
                with_state(|state| state.take_element_event(element, EventKind::Input))
            else {
                return Err(MountError::Component("input target not found"));
            };
            with_state(|state| {
                state.phase = Phase::Event;
                state.fault = None;
                state.write_error = None;
                state.update_local(element, LocalUpdate::Input(value.to_owned()));
            });
            if let Event::Input(handler) = &mut event {
                handler(value.to_owned());
            }
            with_state(|state| {
                state.put_event(event_id, event);
                state.phase = Phase::Idle;
            });
            self.finish_command()
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
            let (caret, selection) = with_state(|state| {
                let LocalState::TextInput {
                    caret,
                    selection,
                    composition,
                } = &mut state.elements[element].local
                else {
                    return Err(DispatchError::WrongKind {
                        node,
                        operation: NodeOperation::CompositionStart,
                    });
                };
                if composition.is_some() {
                    return Err(DispatchError::Rejected {
                        node,
                        operation: NodeOperation::CompositionStart,
                        reason: "text input is already composing",
                    });
                }
                *composition = Some(Composition {
                    base_caret: *caret,
                    base_selection: *selection,
                    preedit: String::new(),
                });
                Ok((*caret, *selection))
            })?;
            self.enter_event_phase();
            self.dispatch_element_event(element, EventKind::CompositionStart, |event| {
                if let Event::CompositionStart(handler) = event {
                    handler(CompositionEvent::new(
                        CompositionPhase::Start,
                        String::new(),
                        caret,
                        selection,
                    ));
                }
            });
            self.leave_event_phase().map_err(DispatchError::Command)
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
            let (caret, selection) = with_state(|state| {
                let LocalState::TextInput {
                    caret,
                    selection,
                    composition,
                } = &mut state.elements[element].local
                else {
                    return Err(DispatchError::WrongKind {
                        node,
                        operation: NodeOperation::CompositionUpdate,
                    });
                };
                let Some(composition) = composition else {
                    return Err(DispatchError::Rejected {
                        node,
                        operation: NodeOperation::CompositionUpdate,
                        reason: "text input is not composing",
                    });
                };
                composition.preedit = preedit.to_owned();
                *caret = composition.base_range().0 + preedit.chars().count();
                *selection = None;
                Ok((*caret, *selection))
            })?;
            self.enter_event_phase();
            self.dispatch_element_event(element, EventKind::CompositionUpdate, |event| {
                if let Event::CompositionUpdate(handler) = event {
                    handler(CompositionEvent::new(
                        CompositionPhase::Update,
                        preedit.to_owned(),
                        caret,
                        selection,
                    ));
                }
            });
            self.leave_event_phase().map_err(DispatchError::Command)
        })
    }

    /// 候选确认：用 `text` 替换组合区间，清除 preedit 并派发一次 `on:input`。
    pub fn compose_commit(&mut self, label: &str, text: &str) -> Result<(), MountError> {
        self.dispatch_label(
            FindTarget::Label(label.to_owned()),
            "composition target not found",
            "composition target is not a text input",
            move |runtime, node| runtime.compose_commit_node(node, text),
        )
    }

    /// 按 Node ID 确认候选；只有组合中的文本输入接受该操作。
    pub fn compose_commit_node(&mut self, node: NodeId, text: &str) -> Result<(), DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            match with_state(|state| state.text_input_composing(element)) {
                None => {
                    return Err(DispatchError::WrongKind {
                        node,
                        operation: NodeOperation::CompositionCommit,
                    });
                }
                Some(false) => {
                    return Err(DispatchError::Rejected {
                        node,
                        operation: NodeOperation::CompositionCommit,
                        reason: "text input is not composing",
                    });
                }
                Some(true) => {}
            }
            self.enter_event_phase();
            self.commit_composition(node, element, text)?;
            self.leave_event_phase().map_err(DispatchError::Command)
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

    /// 按 Node ID 取消组合；只有组合中的文本输入接受该操作。
    pub fn compose_cancel_node(&mut self, node: NodeId) -> Result<(), DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            match with_state(|state| state.text_input_composing(element)) {
                None => {
                    return Err(DispatchError::WrongKind {
                        node,
                        operation: NodeOperation::CompositionCancel,
                    });
                }
                Some(false) => {
                    return Err(DispatchError::Rejected {
                        node,
                        operation: NodeOperation::CompositionCancel,
                        reason: "text input is not composing",
                    });
                }
                Some(true) => {}
            }
            self.enter_event_phase();
            self.cancel_composition(node, element)?;
            self.leave_event_phase().map_err(DispatchError::Command)
        })
    }

    fn commit_composition(
        &mut self,
        node: NodeId,
        element: usize,
        text: &str,
    ) -> Result<(), DispatchError> {
        // 组合期间外部 value 可能已更新；候选插入当前文本并按长度夹取组合区间。
        let current_text =
            with_state(|state| state.elements[element].text_value().unwrap_or_default());
        let (new_text, caret) = with_state(|state| {
            let LocalState::TextInput {
                caret,
                selection,
                composition,
            } = &mut state.elements[element].local
            else {
                return Err(DispatchError::WrongKind {
                    node,
                    operation: NodeOperation::CompositionCommit,
                });
            };
            let Some(composition) = composition.take() else {
                return Err(DispatchError::Rejected {
                    node,
                    operation: NodeOperation::CompositionCommit,
                    reason: "text input is not composing",
                });
            };
            let length = current_text.chars().count();
            let (start, end) = composition.base_range();
            let start = start.min(length);
            let end = end.min(length).max(start);
            let new_text = splice_chars(&current_text, start, end, text);
            *caret = start + text.chars().count();
            *selection = None;
            Ok((new_text, *caret))
        })?;
        self.dispatch_element_event(element, EventKind::CompositionEnd, |event| {
            if let Event::CompositionEnd(handler) = event {
                handler(CompositionEvent::new(
                    CompositionPhase::End,
                    text.to_owned(),
                    caret,
                    None,
                ));
            }
        });
        self.dispatch_element_event(element, EventKind::Input, |event| {
            if let Event::Input(handler) = event {
                handler(new_text.clone());
            }
        });
        Ok(())
    }

    fn cancel_composition(&mut self, node: NodeId, element: usize) -> Result<(), DispatchError> {
        let (discarded, caret, selection) = with_state(|state| {
            let LocalState::TextInput {
                caret,
                selection,
                composition,
            } = &mut state.elements[element].local
            else {
                return Err(DispatchError::WrongKind {
                    node,
                    operation: NodeOperation::CompositionCancel,
                });
            };
            let Some(composition) = composition.take() else {
                return Err(DispatchError::Rejected {
                    node,
                    operation: NodeOperation::CompositionCancel,
                    reason: "text input is not composing",
                });
            };
            *caret = composition.base_caret;
            *selection = composition.base_selection;
            Ok((composition.preedit, *caret, *selection))
        })?;
        self.dispatch_element_event(element, EventKind::CompositionCancel, |event| {
            if let Event::CompositionCancel(handler) = event {
                handler(CompositionEvent::new(
                    CompositionPhase::Cancel,
                    discarded.clone(),
                    caret,
                    selection,
                ));
            }
        });
        Ok(())
    }

    pub fn change(&mut self, label: &str, value: bool) -> Result<(), MountError> {
        self.dispatch_label(
            FindTarget::Label(label.to_owned()),
            "change target not found",
            "change target not found",
            move |runtime, node| runtime.change_node(node, value),
        )
    }

    /// 按 Node ID 派发 change；先确认该元素拥有 change handler，再更新本地状态。
    pub fn change_node(&mut self, node: NodeId, value: bool) -> Result<(), DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            let (event_id, mut event) =
                self.take_node_event(node, element, EventKind::Change, NodeOperation::Change)?;
            with_state(|state| state.update_local(element, LocalUpdate::Change(value)));
            if let Event::Change(handler) = &mut event {
                handler(value);
            }
            with_state(|state| {
                state.put_event(event_id, event);
                state.phase = Phase::Idle;
            });
            self.finish_command().map_err(DispatchError::Command)
        })
    }

    pub fn submit(&mut self, label: &str) -> Result<(), MountError> {
        self.dispatch_label(
            FindTarget::Label(label.to_owned()),
            "submit target not found",
            "submit target not found",
            Self::submit_node,
        )
    }

    /// 按 Node ID 派发 submit。
    pub fn submit_node(&mut self, node: NodeId) -> Result<(), DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            let (event_id, mut event) =
                self.take_node_event(node, element, EventKind::Submit, NodeOperation::Submit)?;
            if let Event::Submit(handler) = &mut event {
                handler(SubmitEvent::default());
            }
            with_state(|state| {
                state.put_event(event_id, event);
                state.phase = Phase::Idle;
            });
            self.finish_command().map_err(DispatchError::Command)
        })
    }

    /// 组合中的候选 Enter 与取消 Escape 在用户 key handler 之前被消费；
    /// 候选文本由 IME 随后通过 compositionend/`compose_commit` 确认。
    /// 其他键仍按普通路由派发。未消费的 Enter 才是表单提交命令。
    pub fn keydown(&mut self, label: &str, key: Key) -> Result<(), MountError> {
        self.dispatch_label(
            FindTarget::Label(label.to_owned()),
            "keydown target not found",
            "keydown target not found",
            move |runtime, node| runtime.keydown_node(node, key),
        )
    }

    /// 按 Node ID 派发 keydown；组合中的 Enter/Escape 仍然优先被编辑器消费。
    pub fn keydown_node(&mut self, node: NodeId, key: Key) -> Result<(), DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            if with_state(|state| state.elements[element].local.is_composing()) {
                match key {
                    Key::Enter => {
                        // 确认候选属于 IME；这里只保证按键不会进入表单提交或用户 handler。
                        return Ok(());
                    }
                    Key::Escape => {
                        self.enter_event_phase();
                        self.cancel_composition(node, element)?;
                        return self.leave_event_phase().map_err(DispatchError::Command);
                    }
                    Key::Other => {}
                }
            }
            let (event_id, mut event) =
                self.take_node_event(node, element, EventKind::KeyDown, NodeOperation::KeyDown)?;
            if let Event::KeyDown(handler) = &mut event {
                handler(KeyEvent::new(key));
            }
            with_state(|state| {
                state.put_event(event_id, event);
                state.phase = Phase::Idle;
            });
            self.finish_command().map_err(DispatchError::Command)
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
            if !with_state(|state| state.is_focusable(element)) {
                return Err(DispatchError::WrongKind {
                    node,
                    operation: NodeOperation::Focus,
                });
            }
            self.move_focus(Some(element))
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
            let focused = with_state(|state| state.focused);
            if focused != Some(element) {
                return Err(DispatchError::Rejected {
                    node,
                    operation: NodeOperation::Blur,
                    reason: "node is not focused",
                });
            }
            self.move_focus(None).map_err(DispatchError::Command)
        })
    }

    /// 清空焦点；没有当前目标时不派发事件。
    pub fn blur(&mut self) -> Result<(), MountError> {
        let state = self.state.clone();
        context::install(&state, || self.move_focus(None))
    }

    /// Tab / 方向键的向前遍历；到末尾后回到第一个可聚焦控件。
    pub fn focus_next(&mut self) -> Result<(), MountError> {
        self.traverse_focus(true)
    }

    /// Tab / 方向键的向后遍历；到开头后回到最后一个可聚焦控件。
    pub fn focus_previous(&mut self) -> Result<(), MountError> {
        self.traverse_focus(false)
    }

    fn traverse_focus(&mut self, forward: bool) -> Result<(), MountError> {
        let state = self.state.clone();
        context::install(&state, || {
            let order = with_state(|state| state.focus_order());
            if order.is_empty() {
                return Err(MountError::Component("no focusable target"));
            }
            let current = with_state(|state| state.focused)
                .and_then(|focused| order.iter().position(|id| *id == focused));
            let index = match (forward, current) {
                (true, Some(index)) => (index + 1) % order.len(),
                (true, None) => 0,
                (false, Some(0)) => order.len() - 1,
                (false, Some(index)) => index - 1,
                (false, None) => order.len() - 1,
            };
            self.move_focus(Some(order[index]))
        })
    }

    fn move_focus(&mut self, target: Option<usize>) -> Result<(), MountError> {
        let current = with_state(|state| state.focused);
        if current == target {
            return Ok(());
        }
        with_state(|state| {
            state.phase = Phase::Event;
            state.fault = None;
            state.write_error = None;
        });
        if let Some(old) = current {
            with_state(|state| state.focused = None);
            if with_state(|state| state.element_is_mounted(old)) {
                self.fire_blur(old, FocusCause::Command);
            }
        }
        if let Some(target) = target {
            with_state(|state| state.focused = Some(target));
            self.fire_focus(target, FocusCause::Command);
        }
        with_state(|state| state.phase = Phase::Idle);
        self.finish_command()
    }

    fn fire_blur(&mut self, element: usize, cause: FocusCause) {
        self.dispatch_element_event(element, EventKind::Blur, |event| {
            if let Event::Blur(handler) = event {
                handler(BlurEvent::new(cause));
            }
        });
    }

    fn fire_focus(&mut self, element: usize, cause: FocusCause) {
        self.dispatch_element_event(element, EventKind::Focus, |event| {
            if let Event::Focus(handler) = event {
                handler(FocusEvent::new(cause));
            }
        });
    }

    fn dispatch_element_event(
        &mut self,
        element: usize,
        kind: EventKind,
        invoke: impl FnOnce(&mut Event),
    ) {
        let Some((id, mut event)) = with_state(|state| state.take_element_event(element, kind))
        else {
            return;
        };
        // handler unwind 不能把占位事件留在槽位里；恢复后再继续展开。
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| invoke(&mut event)));
        with_state(|state| state.put_event(id, event));
        if let Err(payload) = outcome {
            std::panic::resume_unwind(payload);
        }
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
        let fault = with_state(|state| state.fault);
        if let Some(fault) = fault {
            return Err(State::context_error(fault));
        }
        let write_error = with_state(|state| state.write_error.take());
        if let Some(error) = write_error {
            return Err(MountError::Write(error));
        }
        self.refresh()
    }

    pub fn texts(&self) -> Vec<String> {
        let state = self.state.borrow();
        let mut texts = Vec::new();
        if let Some(tree) = &state.tree {
            collect_texts(&state, tree, &mut texts);
        }
        texts
    }

    pub fn for_items(&self) -> Vec<ItemInfo> {
        let state = self.state.borrow();
        let mut items = Vec::new();
        if let Some(tree) = &state.tree {
            collect_items(&state, tree, &mut items);
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

    pub fn caret(&self, label: &str) -> Option<usize> {
        self.state
            .borrow()
            .text_input(&FindTarget::Label(label.to_owned()))
    }

    /// 当前编辑器本地的 preedit；未在组合中时为 `None`。
    pub fn preedit(&self, label: &str) -> Option<String> {
        let state = self.state.borrow();
        let element = state.find_element(&FindTarget::Label(label.to_owned()))?;
        match &state.elements[element].local {
            LocalState::TextInput {
                composition: Some(composition),
                ..
            } => Some(composition.preedit.clone()),
            _ => None,
        }
    }

    pub fn set_caret(&mut self, label: &str, offset: usize) -> bool {
        self.state
            .borrow_mut()
            .set_text_input_caret(&FindTarget::Label(label.to_owned()), offset)
    }

    pub fn focused(&self) -> Option<FocusInfo> {
        let state = self.state.borrow();
        let element = &state.elements[state.presented_focus()?];
        Some(FocusInfo {
            tag: element.tag,
            label: element.label(),
        })
    }

    pub fn checked(&self, label: &str) -> Option<bool> {
        let state = self.state.borrow();
        let element = state.find_element(&FindTarget::Label(label.to_owned()))?;
        match state.elements[element].local {
            LocalState::Checkbox { checked } => Some(checked),
            _ => None,
        }
    }

    /// 最近一次提交界面的只读呈现描述；生成过程不执行用户闭包，也不改变界面。
    pub fn presentation(&self) -> Result<PresentationSnapshot, MountError> {
        let state = self.state.borrow();
        let Some(tree) = state.tree.as_ref() else {
            return Err(MountError::Component(
                "presentation snapshot requires a committed interface",
            ));
        };
        let mut roots = Vec::new();
        collect_snapshot(&state, tree, &mut roots);
        let focus = state
            .presented_focus()
            .map(|element| state.elements[element].id);
        Ok(PresentationSnapshot { roots, focus })
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
        MountedNode::Show(show) => show
            .active
            .as_ref()
            .is_some_and(|branch| contains_element(state, &branch.children, wanted)),
        MountedNode::For(for_node) => for_node.order.iter().any(|key| {
            for_node
                .items
                .get(key)
                .is_some_and(|item| contains_element(state, &item.children, wanted))
        }),
    })
}

fn focus_replacement(
    state: &State,
    before: &[usize],
    after: &[usize],
    old: usize,
) -> Option<usize> {
    let index = before.iter().position(|id| *id == old)?;
    let tag = state.elements[old].tag;
    let live: BTreeSet<usize> = after.iter().copied().collect();
    let same_tag = |id: &usize| live.contains(id) && state.elements[*id].tag == tag;
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

fn splice_chars(text: &str, start: usize, end: usize, insert: &str) -> String {
    let mut result: String = text.chars().take(start).collect();
    result.push_str(insert);
    result.extend(text.chars().skip(end));
    result
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
            MountedNode::Show(show) => {
                if let Some(branch) = &show.active {
                    collect_elements(state, &branch.children, order);
                }
            }
            MountedNode::For(for_node) => {
                for key in &for_node.order {
                    if let Some(item) = for_node.items.get(key) {
                        collect_elements(state, &item.children, order);
                    }
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
            MountedNode::Show(show) => {
                if let Some(branch) = &show.active {
                    collect_focusable(state, &branch.children, order);
                }
            }
            MountedNode::For(for_node) => {
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
            MountedNode::Text(text) => out.push(SnapshotNode::Text(text.value.clone())),
            MountedNode::Element(id) => {
                let element = &state.elements[*id];
                let mut children = Vec::new();
                collect_snapshot(state, &element.children, &mut children);
                out.push(SnapshotNode::Element(SnapshotElement {
                    id: element.id,
                    tag: element.tag,
                    attrs: snapshot_attrs(element),
                    edit: snapshot_edit(&element.local),
                    children,
                }));
            }
            MountedNode::Component(id) => {
                collect_snapshot(state, &state.components[*id].children, out);
            }
            MountedNode::Show(show) => {
                if let Some(branch) = &show.active {
                    collect_snapshot(state, &branch.children, out);
                }
            }
            MountedNode::For(for_node) => {
                for key in &for_node.order {
                    if let Some(item) = for_node.items.get(key) {
                        collect_snapshot(state, &item.children, out);
                    }
                }
            }
        }
    }
}

fn snapshot_edit(local: &LocalState) -> Option<TextEditState> {
    match local {
        LocalState::TextInput {
            caret,
            selection,
            composition,
        } => Some(TextEditState {
            caret: *caret,
            selection: *selection,
            preedit: composition
                .as_ref()
                .map(|composition| composition.preedit.clone()),
        }),
        _ => None,
    }
}

fn snapshot_attrs(element: &MountedElement) -> Vec<SnapshotAttr> {
    let mut attrs: Vec<SnapshotAttr> = element
        .attrs
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
    if let LocalState::Checkbox { checked } = element.local {
        match attrs.iter_mut().find(|attr| attr.name == "checked") {
            Some(attr) => attr.value = SnapshotValue::Bool(checked),
            None => attrs.push(SnapshotAttr {
                name: "checked",
                value: SnapshotValue::Bool(checked),
            }),
        }
    }
    attrs
}

fn collect_texts(state: &State, nodes: &[MountedNode], texts: &mut Vec<String>) {
    for node in nodes {
        match node {
            MountedNode::Text(text) => texts.push(text.value.clone()),
            MountedNode::Element(id) => collect_texts(state, &state.elements[*id].children, texts),
            MountedNode::Component(id) => {
                collect_texts(state, &state.components[*id].children, texts);
            }
            MountedNode::Show(show) => {
                if let Some(branch) = &show.active {
                    collect_texts(state, &branch.children, texts);
                }
            }
            MountedNode::For(for_node) => {
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
            MountedNode::Show(show) => {
                if let Some(branch) = &show.active {
                    collect_items(state, &branch.children, items);
                }
            }
            MountedNode::For(for_node) => {
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
