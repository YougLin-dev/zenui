use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

use zenui_core::{Counters, RowSet};
use zenui_style::{Length, Property, StyleValue};

use crate::context::{self, ContextFault, DepKey, Phase, with_state};
use crate::dispatch::{DispatchError, NodeOperation};
use crate::menu::{
    CommandError, CommandEvent, CommandHandler, CommandSource, MenuCommand, MenuCommandState,
    MenuSpec, MenuStateError, MenuStateHandler,
};
use crate::presentation::{
    Appearance, NodeId, NodeStatus, PresentationSnapshot, SnapshotAction, SnapshotActions,
    SnapshotAttr, SnapshotElement, SnapshotNode, SnapshotRowBand, SnapshotSemantics, SnapshotValue,
    TextEditState,
};
use crate::signal::{Memo, MemoCell, SignalCell};
use crate::store::{TodoId, WriteError};
use crate::task::{TaskRecord, WakeCallback};
use crate::text_edit::{self, Edit, Movement};
use crate::tray::{TrayError, TraySpec};
use crate::view::{
    Attr, AttrValue, BlurEvent, ChildrenFactory, CompositionEvent, CompositionPhase, Event,
    FocusCause, FocusEvent, InlineStyle, Key, KeyEvent, MountError, MountPlan, MountResult,
    PlanNode, ResultBoolValue, StyleSlot, SubmitEvent, TextSlot, ValueSlot,
};
use crate::window::{
    CatalogEntry, CatalogError, CloseError, CloseHandler, ClosePolicy, CloseReply, CloseRequest,
    WindowAction, WindowCatalog, WindowCtx, WindowError, WindowInfo, WindowRecord, WindowRequest,
    WindowRole,
};

const HIDDEN_TARGET: &str = "target window is hidden";
const SUSPENDED_TARGET: &str = "target window input is suspended";

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

/// 一棵已提交树的身份：headless 根，或目录中的窗口键。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum TreeKey {
    Root,
    Window(&'static str),
}

/// 一棵已提交界面，以及它拥有的 Owner、焦点与结算前序。
struct MountedTree {
    nodes: Vec<MountedNode>,
    owner: u32,
    focused: Option<usize>,
    settle_before: Option<Vec<usize>>,
}

/// 宿主为一行带请求的挂载窗口；`count` 为零表示该行带当前不需要行。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RowWindow {
    first: usize,
    count: usize,
}

/// 没有宿主窗口时的默认挂载行数；ADR-0011 要求它远大于常见视口行数。
const DEFAULT_WINDOW_ROWS: usize = 32;

pub(crate) struct State {
    pub(crate) id: u32,
    pub(crate) read_only: u32,
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
    events: Vec<Event>,
    elements: Vec<MountedElement>,
    components: Vec<MountedComponent>,
    trees: BTreeMap<TreeKey, MountedTree>,
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
    /// 每个 activation 在 factory 中注册的命令处理器，随窗口关闭释放。
    command_handlers: BTreeMap<&'static str, BTreeMap<&'static str, CommandHandler>>,
    /// 应用级命令处理器；不随窗口关闭释放，运行时启动后拒绝注册。
    app_command_handlers: BTreeMap<&'static str, CommandHandler>,
    /// 应用级托盘声明；由宿主在启动时读取。
    tray: Option<TraySpec>,
    /// 每个 activation 的只读菜单状态回调，随窗口关闭释放。
    menu_state_handlers: BTreeMap<&'static str, MenuStateHandler>,
    /// 按父窗口键索引的待决关闭 epoch；存在即表示父窗口输入被挂起。
    close_epochs: BTreeMap<&'static str, CloseEpoch>,
    /// UI 处理期排队的 Allow/Deny 回复，在事件/交付结算后执行。
    close_replies: Vec<CloseReplyRequest>,
    next_close_epoch: u64,
    /// 窗口打开/关闭/隐藏/显示时递增；原生宿主据此跳过无变化的扫描。
    window_revision: u64,
    /// 本地编辑状态（焦点、caret、selection、组合、勾选）的版本。
    presentation_revision: u64,
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
    next_node_mount: u64,
    unmounts: u64,
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
            command_handlers: BTreeMap::new(),
            app_command_handlers: BTreeMap::new(),
            tray: None,
            menu_state_handlers: BTreeMap::new(),
            close_epochs: BTreeMap::new(),
            close_replies: Vec::new(),
            next_close_epoch: 1,
            window_revision: 0,
            presentation_revision: 0,
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
            next_node_mount: 0,
            unmounts: 0,
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

    fn tree(&self, key: TreeKey) -> Option<&MountedTree> {
        self.trees.get(&key)
    }

    fn tree_mut(&mut self, key: TreeKey) -> Option<&mut MountedTree> {
        self.trees.get_mut(&key)
    }

    /// 取走指定窗口的树供刷新使用。
    fn take_tree(&mut self, key: TreeKey) -> Option<MountedTree> {
        self.trees.remove(&key)
    }

    fn put_tree(&mut self, key: TreeKey, tree: MountedTree) {
        self.trees.insert(key, tree);
    }

    /// 当前接收输入的窗口；没有焦点窗口时退到目录中的第一个可见窗口或 headless 根。
    fn active_tree_key(&self) -> Option<TreeKey> {
        if let Some(key) = self.focused_window
            && self.tree_is_visible(TreeKey::Window(key))
        {
            return Some(TreeKey::Window(key));
        }
        self.windows
            .keys()
            .copied()
            .find(|key| self.window_is_visible(key))
            .map(TreeKey::Window)
            .or_else(|| {
                self.trees
                    .contains_key(&TreeKey::Root)
                    .then_some(TreeKey::Root)
            })
    }

    fn window_ctx(&self, key: &'static str) -> Option<WindowCtx> {
        let record = self.windows.get(key)?;
        let live = self.window_is_live(key, record.activation, record.owner)
            && self.trees.contains_key(&TreeKey::Window(key));
        live.then(|| WindowCtx::new(self.id, key, record.activation, record.owner))
    }

    pub(crate) fn window_is_visible(&self, key: &'static str) -> bool {
        self.windows
            .get(key)
            .is_some_and(|record| record.open && record.visible)
            && self.trees.contains_key(&TreeKey::Window(key))
    }

    fn tree_is_visible(&self, key: TreeKey) -> bool {
        match key {
            TreeKey::Root => self.trees.contains_key(&key),
            TreeKey::Window(key) => self.window_is_visible(key),
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
        }
    }

    fn show_window(&mut self, key: &'static str) {
        let Some(record) = self.windows.get_mut(key) else {
            return;
        };
        if !record.visible {
            record.visible = true;
            self.bump_window_revision();
        }
    }

    fn bump_window_revision(&mut self) {
        self.window_revision = self.window_revision.wrapping_add(1);
    }

    /// 记录一次快照可见的本地状态变化；宿主据此决定是否重建快照。
    fn bump_presentation(&mut self) {
        self.presentation_revision = self.presentation_revision.wrapping_add(1);
    }

    fn tree_for_element(&self, element: usize) -> Option<TreeKey> {
        self.trees
            .iter()
            .find_map(|(key, tree)| contains_element(self, &tree.nodes, element).then_some(*key))
    }

    /// 指定树里是否还有这个行带的行窗口化 For；过期行带由宿主忽略。
    fn tree_has_band(&self, key: TreeKey, band: u64) -> bool {
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
            .get(&TreeKey::Window(window.key))
            .is_some_and(|tree| {
                tree.owner == window.owner
                    && self.window_is_live(window.key, window.activation, window.owner)
            })
    }

    /// 当前焦点目标；焦点属于树，而不是整个应用。
    fn focused_in(&self, key: TreeKey) -> Option<usize> {
        self.tree(key).and_then(|tree| tree.focused)
    }

    fn set_focused_in(&mut self, key: TreeKey, focused: Option<usize>) {
        if let Some(tree) = self.tree_mut(key)
            && tree.focused != focused
        {
            tree.focused = focused;
            self.bump_presentation();
        }
    }

    /// 焦点窗口中的呈现焦点：仍在已提交树中且可聚焦。
    fn presented_focus(&self) -> Option<usize> {
        self.active_tree_key()
            .and_then(|key| self.presented_focus_in(key))
    }

    fn presented_focus_in(&self, key: TreeKey) -> Option<usize> {
        self.focused_in(key)
            .filter(|element| self.element_is_mounted(*element) && self.is_focusable(*element))
    }

    fn snapshot(&self, key: TreeKey) -> Option<PresentationSnapshot> {
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
        self.bump_presentation();
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
            TreeKey::Root => true,
            TreeKey::Window(key) => self.window_accepts_input(key),
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
        self.events[id] = event;
    }

    fn update_local(&mut self, element: usize, value: LocalUpdate) {
        match (&mut self.elements[element].local, value) {
            (
                LocalState::TextInput {
                    caret,
                    anchor,
                    selection,
                    ..
                },
                LocalUpdate::Input(value),
            ) => {
                *caret = value.chars().count();
                *anchor = *caret;
                *selection = None;
            }
            (LocalState::Checkbox { checked }, LocalUpdate::Change(value)) => {
                *checked = value;
            }
            _ => return,
        }
        self.bump_presentation();
    }

    fn text_input(&self, target: &FindTarget) -> Option<usize> {
        let element = self.find_element(target)?;
        match self.elements[element].local {
            LocalState::TextInput { caret, .. } => Some(caret),
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
        if matches!(self.elements[element].local, LocalState::TextInput { .. }) {
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
        if self.elements[element].is_disabled() {
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
        if self.elements[element].local.is_composing() {
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
        if self.elements[element].local.is_composing() {
            Ok(())
        } else {
            Err(DispatchError::Rejected {
                node,
                operation,
                reason: "text input is not composing",
            })
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
        let Some(element) = self.elements.get(node.slot() as usize) else {
            return Err(DispatchError::UnknownNode(node));
        };
        if element.id.slot() != node.slot() || element.id.generation() != node.generation() {
            return Err(DispatchError::StaleNode(node));
        }
        match self.tree_for_element(node.slot() as usize) {
            Some(key) if self.tree_is_visible(key) => {
                if allow_suspended || self.element_accepts_input(node.slot() as usize) {
                    Ok(node.slot() as usize)
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
        if let LocalState::TextInput {
            caret,
            anchor,
            selection,
            composition,
            ..
        } = &mut self.elements[element].local
        {
            if composition.is_some() {
                return false;
            }
            *caret = offset;
            *anchor = offset;
            *selection = Some((offset, offset));
        } else {
            return false;
        }
        self.bump_presentation();
        true
    }

    /// 原生控件与显式挂接 focus/blur 的元素参与遍历；禁用元素不参与接任或 Tab 顺序。
    fn is_focusable(&self, element: usize) -> bool {
        !self.elements[element].is_disabled() && self.element_is_inherently_focusable(element)
    }

    /// 元素本身的键盘可聚焦语义；`disabled` 与可见性由调用方另行组合。
    fn element_is_inherently_focusable(&self, element: usize) -> bool {
        let element = &self.elements[element];
        matches!(element.tag, "button" | "text-input" | "checkbox")
            || element.events.iter().any(|id| {
                matches!(
                    event_kind(&self.events[*id]),
                    EventKind::Focus | EventKind::Blur
                )
            })
    }

    /// Tab/方向键共用的确定性前序：控件先于自己的子树，Show 只看活动分支，For 按业务顺序。
    fn focus_order(&self, key: TreeKey) -> Vec<usize> {
        let mut order = Vec::new();
        if let Some(tree) = self.tree(key) {
            collect_focusable(self, &tree.nodes, &mut order);
        }
        order
    }

    /// 结算快照包含旧一代的全部挂载元素；某个控件当时禁用、本帧重新启用时仍可接任。
    fn element_order(&self, key: TreeKey) -> Vec<usize> {
        let mut order = Vec::new();
        if let Some(tree) = self.tree(key) {
            collect_elements(self, &tree.nodes, &mut order);
        }
        order
    }

    fn element_is_mounted(&self, element: usize) -> bool {
        self.trees
            .values()
            .any(|tree| contains_element(self, &tree.nodes, element))
    }

    /// 旧焦点不在新的可聚焦集合时，下一次结算才产生接任目标。
    ///
    /// 只在旧一代的挂载顺序里选：同 tag 的下一控件、同 tag 的上一控件，
    /// 再退到任意下一控件、任意上一控件；本帧新挂载的控件不会因返场抢焦点，
    /// 当时禁用、本帧重新启用的旧控件仍然可以接任。
    fn focus_settlement(&mut self, key: TreeKey) -> Option<FocusSettlement> {
        let old = self.focused_in(key)?;
        let after = self.focus_order(key);
        if after.contains(&old) {
            return None;
        }
        let before = self.tree_mut(key)?.settle_before.take().unwrap_or_default();
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
    class_styles: &'static [zenui_style::StyleDeclaration],
    inline_styles: Vec<MountedInlineStyle>,
    events: Vec<usize>,
    local: LocalState,
    children: Vec<MountedNode>,
}

struct MountedInlineStyle {
    property: zenui_style::Property,
    slot: StyleSlot,
    current: Option<zenui_style::StyleValue>,
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

    fn text_value(&self) -> Option<&str> {
        self.attrs
            .iter()
            .find(|attr| attr.name == "value")
            .and_then(|attr| attr.current.as_ref())
            .and_then(|current| match current {
                AttrCurrent::Text(text) => Some(text.as_str()),
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
        /// 选择的活动端（焦点）；`caret` 与它相同表示折叠。
        anchor: usize,
        selection: Option<(usize, usize)>,
        composition: Option<Composition>,
        /// 这个输入自己的撤销/重做历史；随元素卸载释放。
        /// Box 让每个元素槽位保持小尺寸，只有文本输入承担历史分配。
        history: Box<TextHistory>,
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

/// 一次可撤销编辑前后的完整本地状态；`value` 是已经提交到模型的值。
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

const HISTORY_ENTRIES: usize = 100;
const HISTORY_BYTES: usize = 256 * 1024;

/// 每个文本输入自己的有界编辑历史。
///
/// 历史只保存已提交值的快照；撤销和重做都通过一次 `on:input` 写回模型，因此
/// 应用始终是值的权威。`observed` 是最近一次刷新看到的已提交值，外部替换据此
/// 清空两个栈；`settle_local` 让一次本地编辑自己的刷新不被误判为外部变化。
#[derive(Default)]
struct TextHistory {
    undo: VecDeque<TextEntry>,
    redo: VecDeque<TextEntry>,
    undo_bytes: usize,
    redo_bytes: usize,
    observed: Option<String>,
    settle_local: bool,
}

impl TextHistory {
    /// 接受一次刷新看到的值；不是本地编辑结算时，变化即为外部替换。
    fn observe(&mut self, value: &str) {
        if self.settle_local {
            self.settle_local = false;
            self.observed = Some(value.to_owned());
        } else if self.observed.as_deref() != Some(value) {
            self.observed = Some(value.to_owned());
            self.clear();
        }
    }

    fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.undo_bytes = 0;
        self.redo_bytes = 0;
    }

    /// 元素卸载时释放历史与观察到的值，不在死槽位里保留文本。
    fn reset(&mut self) {
        self.clear();
        self.observed = None;
        self.settle_local = false;
    }

    /// 记录一次已提交编辑；提交前后值相同则不记录，也不清空 redo。
    fn record(&mut self, before: TextSnapshot, after: TextSnapshot) {
        if before.value == after.value {
            return;
        }
        self.redo.clear();
        self.redo_bytes = 0;
        self.push_undo(TextEntry { before, after });
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

fn text_snapshot(element: &MountedElement) -> Option<TextSnapshot> {
    let LocalState::TextInput {
        caret,
        anchor,
        selection,
        ..
    } = &element.local
    else {
        return None;
    };
    Some(TextSnapshot {
        value: element.text_value().unwrap_or_default().to_owned(),
        caret: *caret,
        anchor: *anchor,
        selection: *selection,
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
fn ensure_spacers(state: &mut State, owner: u32) -> (usize, usize) {
    let mut make = || {
        let id = state.mint_node();
        let slot = state.elements.len();
        state.elements.push(MountedElement {
            id,
            tag: "view",
            owner,
            attrs: vec![MountedAttr {
                name: "role",
                value: AttrValue::Text(TextSlot::Static("spacer")),
                current: Some(AttrCurrent::Text(String::from("spacer"))),
            }],
            class_styles: &[],
            inline_styles: vec![MountedInlineStyle {
                property: Property::Height,
                slot: StyleSlot::Length(ValueSlot::Static(Length::px(0.0))),
                current: Some(StyleValue::Length(Length::px(0.0))),
            }],
            events: Vec::new(),
            local: LocalState::None,
            children: Vec::new(),
        });
        slot
    };
    (make(), make())
}

/// 更新 spacer 的已提交高度；数量和单位一起保留，em 在场景里按最终字号解析。
fn set_spacer_height(state: &mut State, spacer: usize, rows: usize, row_height: Length) {
    let height = rows_height(row_height, rows);
    let element = &mut state.elements[spacer];
    if let Some(style) = element
        .inline_styles
        .iter_mut()
        .find(|style| style.property == Property::Height)
    {
        style.slot = StyleSlot::Length(ValueSlot::Static(height));
        style.current = Some(StyleValue::Length(height));
    }
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
            let value = with_owner(0, || {
                with_state(|state| state.phase = Phase::Setup);
                let value = setup();
                with_state(|state| state.phase = Phase::Idle);
                value
            });
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
                state.put_tree(
                    TreeKey::Root,
                    MountedTree {
                        nodes: tree,
                        owner: 0,
                        focused: None,
                        settle_before: None,
                    },
                );
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
    pub fn show(&mut self, key: &'static str) -> Result<WindowCtx, WindowError> {
        let factory = {
            let state = self.state.borrow();
            let entry = state
                .catalog
                .get(key)
                .ok_or(WindowError::UnknownWindow(key))?;
            Rc::clone(&entry.factory)
        };
        if self.state.borrow().trees.contains_key(&TreeKey::Root) {
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
            let plan = match plan {
                Ok(plan) => plan,
                Err(error) => {
                    self.abort_open(key, activation, owner, None);
                    return Err(WindowError::Mount(error));
                }
            };
            if let Some(id) = with_state(|state| state.missing_command_handler(key)) {
                self.abort_open(key, activation, owner, None);
                return Err(WindowError::Command(CommandError::MissingHandler(id)));
            }
            let tree = match self.mount_plan(plan, owner) {
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
                    TreeKey::Window(key),
                    MountedTree {
                        nodes: tree,
                        owner,
                        focused: None,
                        settle_before: None,
                    },
                );
                state.phase = Phase::Idle;
            });
            if let Err(error) = self.refresh() {
                let nodes = with_state(|state| {
                    state
                        .trees
                        .remove(&TreeKey::Window(key))
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

    /// 走目录声明的关闭策略：Immediate 直接释放；Confirm 建立或复用 epoch。
    ///
    /// 隐藏的 Confirm 窗口先恢复可见，再走关闭协议，避免不可见的确认提示；
    /// 确认窗口在待决 epoch 中的关闭请求等价于 Deny。
    pub fn request_close(&mut self, window: WindowCtx) -> Result<(), WindowError> {
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
                return self.begin_confirm_close(window);
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
                        && state.tree_for_element(target.slot() as usize)
                            == Some(TreeKey::Window(window.key))
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
            let previous = state.phase;
            state.phase = Phase::Event;
            state.fault = None;
            state.write_error = None;
            previous
        });
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(event)));
        with_state(|state| state.phase = previous);
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
            WindowAction::Close => self.request_close(request.from),
        }
    }

    /// 创建 pending epoch、挂起父窗口输入并调用注册的 close handler。
    ///
    /// handler 通过 `request_show` 显示目录声明的确认窗口；这里立即结算这些
    /// show 请求并验证确认窗口可见，失败时绝不回退为 Allow。
    fn begin_confirm_close(&mut self, window: WindowCtx) -> Result<(), WindowError> {
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
                    .focused_in(TreeKey::Window(window.key))
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
            self.invoke_close_handler(window, &handler, request, queued_before)?;

            // 处理器可以直接 allow/deny（例如草稿为空时立即关闭），无需显示确认
            // 窗口；这次决定在本次 request_close 内结算。
            if let Some(reply) = self.take_handler_reply(window, request, confirmation) {
                return match reply {
                    CloseReply::Allow => self.release_close_group(window.key),
                    CloseReply::Deny => self.deny_close(window),
                };
            }

            let settled = self.settle_show_requests();
            let shown = with_state(|state| state.window_is_visible(confirmation));
            if let Err(error) = settled {
                self.abort_close_epoch(window.key, queued_before);
                self.close_visible_confirmation(confirmation);
                return Err(error);
            }
            if !shown {
                self.abort_close_epoch(window.key, queued_before);
                self.close_visible_confirmation(confirmation);
                return Err(WindowError::Close(CloseError::ConfirmationNotShown));
            }
            Ok(())
        })
    }

    /// 在 Event 阶段调用 close handler；panic 时撤销 epoch 并报告失败。
    fn invoke_close_handler(
        &mut self,
        window: WindowCtx,
        handler: &CloseHandler,
        request: CloseRequest,
        queued_before: usize,
    ) -> Result<(), WindowError> {
        let previous_phase = with_state(|state| {
            let previous = state.phase;
            state.phase = Phase::Event;
            previous
        });
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            handler(request);
        }));
        with_state(|state| state.phase = previous_phase);
        if outcome.is_err() {
            self.abort_close_epoch(window.key, queued_before);
            return Err(WindowError::Close(CloseError::HandlerPanicked));
        }
        Ok(())
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
                        (state.node_status(focus) == NodeStatus::Live
                            && state.is_focusable(focus.slot() as usize))
                        .then_some(focus.slot() as usize)
                    });
                    if let Some(target) = target {
                        self.move_focus(TreeKey::Window(parent.key), Some(target))?;
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
            let tree = with_state(|state| state.trees.remove(&TreeKey::Window(window.key)));
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
            let key = TreeKey::Window(window.key);
            if !with_state(|state| state.tree_has_band(key, band)) {
                return Err(WindowError::Stale);
            }
            let changed = with_state(|state| {
                let request = RowWindow { first, count };
                if state.row_windows.get(&band) == Some(&request) {
                    return false;
                }
                state.row_windows.insert(band, request);
                state.bump_presentation();
                true
            });
            if changed {
                self.refresh_tree_once(key).map_err(WindowError::Mount)?;
            }
            Ok(changed)
        })
    }

    pub fn refresh(&mut self) -> Result<(), MountError> {
        if self.tree_keys().is_empty() {
            return Err(MountError::Component("no committed interface is mounted"));
        }
        if let Err(error) = self.refresh_all_once() {
            // 已完成的窗口保留本帧结果；不能让焦点身份停留在已被移除或禁用的旧目标上。
            self.clear_invalid_focus();
            return Err(error);
        }
        self.settle_focus()
    }

    fn tree_keys(&self) -> Vec<TreeKey> {
        self.state.borrow().trees.keys().copied().collect()
    }

    fn refresh_all_once(&mut self) -> Result<(), MountError> {
        // 刷新会重新绑定组件并可能改写文本、属性和可见性；即使没有计数器
        // 变化，也必须让宿主知道快照可能过期。
        self.state.borrow_mut().bump_presentation();
        for key in self.tree_keys() {
            self.refresh_tree_once(key)?;
        }
        Ok(())
    }

    fn refresh_tree_once(&mut self, key: TreeKey) -> Result<(), MountError> {
        let state = self.state.clone();
        context::install(&state, || {
            let taken = with_state(|state| {
                state.fault = None;
                state.phase = Phase::Binding;
                let focused = state.focused_in(key);
                // 窗口化 For 在刷新里据此判断聚焦行是否仍在挂载窗口内。
                state.refresh_focus = focused;
                let settle_before = focused.is_some().then(|| state.element_order(key));
                if let Some(entry) = state.tree_mut(key) {
                    entry.settle_before = settle_before;
                }
                state.take_tree(key)
            });
            let Some(mut tree) = taken else {
                with_state(|state| state.refresh_focus = None);
                return Ok(());
            };
            let result = self.refresh_nodes(&mut tree.nodes);
            let fault = with_state(|state| {
                state.put_tree(key, tree);
                state.refresh_focus = None;
                // 焦点行被滚动窗口卸载时直接清除焦点；它不能结算到相邻行，
                // 那会把 Tab 的语义变成一次隐式焦点跳转（ADR-0011）。
                if let Some(dropped) = state.window_focus_dropped.take()
                    && state.focused_in(key) == Some(dropped)
                    && !state.element_is_mounted(dropped)
                {
                    state.set_focused_in(key, None);
                }
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
        key: TreeKey,
        settlement: FocusSettlement,
        keep_target: bool,
    ) -> Result<(), MountError> {
        with_state(|state| {
            state.phase = Phase::Event;
            state.fault = None;
            state.write_error = None;
        });
        if settlement.blur_old {
            with_state(|state| state.set_focused_in(key, None));
            self.fire_blur(settlement.old, FocusCause::Settlement);
        }
        match (keep_target, settlement.target) {
            (true, Some(target)) => {
                with_state(|state| state.set_focused_in(key, Some(target)));
                self.fire_focus(target, FocusCause::Settlement);
            }
            _ => with_state(|state| state.set_focused_in(key, None)),
        }
        with_state(|state| state.phase = Phase::Idle);
        if let Some(fault) = with_state(|state| state.fault) {
            return Err(State::context_error(fault));
        }
        if let Some(error) = with_state(|state| state.write_error.take()) {
            return Err(MountError::Write(error));
        }
        Ok(())
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
                            anchor: 0,
                            selection: None,
                            composition: None,
                            history: Box::default(),
                        },
                        "checkbox" => LocalState::Checkbox { checked: false },
                        _ => LocalState::None,
                    };
                    let id = state.mint_node();
                    let slot = state.elements.len();
                    let inline_styles = plan
                        .inline_styles
                        .into_iter()
                        .map(|InlineStyle { property, slot }| MountedInlineStyle {
                            property,
                            slot,
                            current: None,
                        })
                        .collect();
                    state.elements.push(MountedElement {
                        id,
                        tag,
                        owner,
                        attrs,
                        class_styles: plan.class_styles,
                        inline_styles,
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
                let band = with_state(|state| {
                    let band = state.next_row_band;
                    state.next_row_band += 1;
                    state.owner_bands.entry(owner).or_default().push(band);
                    band
                });
                let mut node = MountedFor {
                    owner,
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
        with_state(|state| clear_history_in_nodes(state, nodes));
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
        let mut styles = with_state(|state| std::mem::take(&mut state.elements[id].inline_styles));
        for style in &mut styles {
            style.current = Some(style.slot.evaluate());
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
            element.inline_styles = styles;
            if let LocalState::TextInput { history, .. } = &mut element.local {
                let value = element
                    .attrs
                    .iter()
                    .find(|attr| attr.name == "value")
                    .and_then(|attr| attr.current.as_ref())
                    .and_then(|current| match current {
                        AttrCurrent::Text(text) => Some(text.as_str()),
                        AttrCurrent::Bool(_) => None,
                    })
                    .unwrap_or_default();
                history.observe(value);
            }
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
        if let Some(branch) = &mut show.active {
            self.refresh_nodes(&mut branch.children)?;
        }
        Ok(())
    }

    fn refresh_for(&mut self, for_node: &mut MountedFor) -> Result<(), MountError> {
        let rows: RowSet = for_node.each.get();
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
                let mapped = (for_node.key)(todo);
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

        let mut first = 0;
        let mut count = total;
        if for_node.row_height.is_some() {
            if for_node.spacers.is_none() {
                let spacers = with_state(|state| ensure_spacers(state, for_node.owner));
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
            let plan = match with_owner(owner, || (for_node.item)(todo)) {
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
            DispatchError::Hidden(_) => MountError::Component(HIDDEN_TARGET),
            DispatchError::Suspended(_) => MountError::Component(SUSPENDED_TARGET),
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

    /// label 助手的整体值替换；视为外部替换并清空该输入的撤销/重做历史。
    ///
    /// 它仍然派发一次 `on:input`，应用模型保持值的权威。
    pub fn input(&mut self, label: &str, value: &str) -> Result<(), MountError> {
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
                if let LocalState::TextInput { history, .. } = &mut state.elements[element].local {
                    history.clear();
                }
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

    /// 在 caret/selection 处插入文本；一次编辑恰好派发一次 `on:input`。
    pub fn insert_text_node(&mut self, node: NodeId, text: &str) -> Result<(), DispatchError> {
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
    pub fn replace_text_node(&mut self, node: NodeId, value: &str) -> Result<(), DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            self.apply_text_edit(node, element, NodeOperation::SetValue, Edit::Replace(value))
        })
    }

    /// 删除 selection，或 caret 前一个字符；没有可删除内容时是不派发事件的无操作。
    pub fn backspace_node(&mut self, node: NodeId) -> Result<(), DispatchError> {
        let state = self.state.clone();
        context::install(&state, || {
            let element = with_state(|state| state.element_for_node(node))?;
            self.apply_text_edit(node, element, NodeOperation::Backspace, Edit::Backspace)
        })
    }

    /// 删除 selection，或 caret 后一个字符；没有可删除内容时是不派发事件的无操作。
    pub fn delete_forward_node(&mut self, node: NodeId) -> Result<(), DispatchError> {
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
        let length = text_length(&state.elements[element]);
        let (anchor, focus) = match movement {
            Movement::Caret(offset) => {
                let offset = offset.min(length);
                (offset, offset)
            }
            Movement::Selection(anchor, focus) => (anchor.min(length), focus.min(length)),
            Movement::SelectAll => (0, length),
        };
        set_local_selection(&mut state.elements[element].local, anchor, focus);
        state.bump_presentation();
        Ok(())
    }

    fn apply_text_edit(
        &mut self,
        node: NodeId,
        element: usize,
        operation: NodeOperation,
        edit: Edit<'_>,
    ) -> Result<(), DispatchError> {
        let (before, caret, selection) = with_state(|state| {
            state.require_editable_text_input(node, element, operation)?;
            let LocalState::TextInput {
                caret,
                anchor,
                selection,
                ..
            } = &state.elements[element].local
            else {
                unreachable!("require_editable_text_input validated the input");
            };
            let before = TextSnapshot {
                value: state.elements[element]
                    .text_value()
                    .unwrap_or_default()
                    .to_owned(),
                caret: *caret,
                anchor: *anchor,
                selection: *selection,
            };
            Ok::<_, DispatchError>((before, *caret, *selection))
        })?;
        let Some(outcome) = text_edit::apply(&before.value, caret, selection, edit) else {
            return Ok(());
        };
        let text_edit::EditOutcome {
            value: new_value,
            caret: next_caret,
        } = outcome;
        let (event_id, mut event) =
            self.take_node_event(node, element, EventKind::Input, operation)?;
        with_state(|state| {
            let LocalState::TextInput {
                caret,
                anchor,
                selection,
                history,
                ..
            } = &mut state.elements[element].local
            else {
                unreachable!("require_editable_text_input validated the input");
            };
            *caret = next_caret;
            *anchor = next_caret;
            *selection = None;
            // 这次编辑自己的模型写入不能被当作外部替换。
            history.settle_local = true;
        });
        with_state(State::bump_presentation);
        // handler unwind 不能把占位事件或 Event 阶段留在槽位里。
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Event::Input(handler) = &mut event {
                handler(new_value);
            }
        }));
        with_state(|state| {
            state.put_event(event_id, event);
            state.phase = Phase::Idle;
        });
        if let Err(payload) = outcome {
            with_state(|state| clear_history_settle(state, element));
            std::panic::resume_unwind(payload);
        }
        if let Err(error) = self.finish_command() {
            with_state(|state| clear_history_settle(state, element));
            return Err(DispatchError::Command(error));
        }
        with_state(|state| clamp_local_edit(state, element));
        self.record_text_edit(element, before);
        Ok(())
    }

    /// 把一次编辑前后的已提交状态记入该输入自己的历史。
    ///
    /// 只记录仍然挂载的元素；处理器改写的值以刷新后的已提交值为准，
    /// 提交前后值相同的编辑不产生条目。
    fn record_text_edit(&mut self, element: usize, before: TextSnapshot) {
        with_state(|state| {
            if !state.element_is_mounted(element) {
                return;
            }
            let Some(after) = text_snapshot(&state.elements[element]) else {
                return;
            };
            let LocalState::TextInput { history, .. } = &mut state.elements[element].local else {
                return;
            };
            history.record(before, after);
        });
    }

    /// 撤销该输入最近一次已提交编辑；空栈是静默无操作。
    ///
    /// 撤销通过一次 `on:input` 写回编辑前的值，应用模型仍是值的权威。
    /// 精确恢复 caret/anchor/selection 只对原样提交值的处理器成立。
    pub fn undo_node(&mut self, node: NodeId) -> Result<(), DispatchError> {
        self.apply_history(node, NodeOperation::Undo, HistoryDirection::Undo)
    }

    /// 重做该输入最近一次被撤销的编辑；空栈是静默无操作。
    ///
    /// 重做同样通过一次 `on:input` 写回，恢复的是那次编辑提交后的状态。
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
            let entry = with_state(|state| {
                let LocalState::TextInput { history, .. } = &state.elements[element].local else {
                    unreachable!("require_editable_text_input validated the input");
                };
                match direction {
                    HistoryDirection::Undo => history.undo.back(),
                    HistoryDirection::Redo => history.redo.back(),
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
            let (event_id, mut event) =
                self.take_node_event(node, element, EventKind::Input, operation)?;
            let (caret, anchor, selection) = (target.caret, target.anchor, target.selection);
            with_state(|state| {
                let LocalState::TextInput {
                    caret: current_caret,
                    anchor: current_anchor,
                    selection: current_selection,
                    history,
                    ..
                } = &mut state.elements[element].local
                else {
                    unreachable!("require_editable_text_input validated the input");
                };
                let length = target.value.chars().count();
                *current_caret = caret.min(length);
                *current_anchor = anchor.min(length);
                *current_selection =
                    selection.map(|(start, end)| text_edit::normalized_range(start, end, length));
                history.settle_local = true;
            });
            with_state(State::bump_presentation);
            // handler unwind 不能把占位事件或 Event 阶段留在槽位里。
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if let Event::Input(handler) = &mut event {
                    handler(target.value);
                }
            }));
            with_state(|state| {
                state.put_event(event_id, event);
                state.phase = Phase::Idle;
            });
            if let Err(payload) = outcome {
                with_state(|state| clear_history_settle(state, element));
                std::panic::resume_unwind(payload);
            }
            if let Err(error) = self.finish_command() {
                with_state(|state| clear_history_settle(state, element));
                return Err(DispatchError::Command(error));
            }
            with_state(|state| {
                clamp_local_edit(state, element);
                if !state.element_is_mounted(element) {
                    return;
                }
                let LocalState::TextInput { history, .. } = &mut state.elements[element].local
                else {
                    return;
                };
                match direction {
                    HistoryDirection::Undo => {
                        if history.pop_undo().is_some() {
                            history.push_redo(entry);
                        }
                    }
                    HistoryDirection::Redo => {
                        if history.pop_redo().is_some() {
                            history.push_undo(entry);
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
                let LocalState::TextInput {
                    caret,
                    anchor,
                    selection,
                    composition,
                    ..
                } = &mut state.elements[element].local
                else {
                    unreachable!("require_enabled_text_input validated the input");
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
                    base_anchor: *anchor,
                    base_selection: *selection,
                    preedit: String::new(),
                });
                Ok((*caret, *selection))
            })?;
            with_state(State::bump_presentation);
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
            with_state(|state| {
                state.require_enabled_text_input(node, element, NodeOperation::CompositionUpdate)
            })?;
            let (caret, selection) = with_state(|state| {
                let LocalState::TextInput {
                    caret,
                    anchor,
                    selection,
                    composition,
                    ..
                } = &mut state.elements[element].local
                else {
                    unreachable!("require_enabled_text_input validated the input");
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
                *anchor = *caret;
                *selection = None;
                Ok((*caret, *selection))
            })?;
            with_state(State::bump_presentation);
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
            with_state(|state| {
                state.require_enabled_text_input(
                    node,
                    element,
                    NodeOperation::CompositionCommit,
                )?;
                state.require_composing_text_input(node, element, NodeOperation::CompositionCommit)
            })?;
            self.enter_event_phase();
            let before = self.commit_composition(node, element, text)?;
            if let Err(error) = self.leave_event_phase() {
                with_state(|state| clear_history_settle(state, element));
                return Err(DispatchError::Command(error));
            }
            with_state(|state| clamp_local_edit(state, element));
            if let Some(before) = before {
                self.record_text_edit(element, before);
            }
            Ok(())
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
            self.cancel_composition(node, element)?;
            self.leave_event_phase().map_err(DispatchError::Command)
        })
    }

    fn commit_composition(
        &mut self,
        node: NodeId,
        element: usize,
        text: &str,
    ) -> Result<Option<TextSnapshot>, DispatchError> {
        // 组合期间外部 value 可能已更新；候选插入当前文本并按长度夹取组合区间。
        let current_text = with_state(|state| {
            state.elements[element]
                .text_value()
                .unwrap_or_default()
                .to_owned()
        });
        let (before, new_text, caret) = with_state(|state| {
            let LocalState::TextInput {
                caret,
                anchor,
                selection,
                composition,
                history,
                ..
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
            // 撤销回到组合开始前的已提交状态，而不是临时的 preedit caret。
            let before = TextSnapshot {
                value: current_text.clone(),
                caret: composition.base_caret,
                anchor: composition.base_anchor,
                selection: composition.base_selection,
            };
            let (start, end) = composition.base_range();
            let (new_text, next_caret) = text_edit::replace_range(&current_text, start, end, text);
            *caret = next_caret;
            *anchor = next_caret;
            *selection = None;
            history.settle_local = true;
            Ok((before, new_text, *caret))
        })?;
        with_state(State::bump_presentation);
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
        Ok(Some(before))
    }

    fn cancel_composition(&mut self, node: NodeId, element: usize) -> Result<(), DispatchError> {
        let (discarded, caret, selection) = with_state(|state| {
            let LocalState::TextInput {
                caret,
                anchor,
                selection,
                composition,
                ..
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
            *anchor = composition.base_anchor;
            *selection = composition.base_selection;
            Ok((composition.preedit, *caret, *selection))
        })?;
        with_state(State::bump_presentation);
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
            self.move_focus(TreeKey::Window(window.key), None)
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
            let tree = TreeKey::Window(window.key);
            if with_state(|state| state.focus_order(tree)).is_empty() {
                return Ok(());
            }
            self.traverse_focus_for(tree, forward)
                .map_err(WindowError::Mount)
        })
    }

    fn traverse_focus_for(&mut self, tree: TreeKey, forward: bool) -> Result<(), MountError> {
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

    fn move_focus(&mut self, tree: TreeKey, target: Option<usize>) -> Result<(), MountError> {
        if let TreeKey::Window(key) = tree {
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
        if let Some(old) = current {
            with_state(|state| state.set_focused_in(tree, None));
            if with_state(|state| state.element_is_mounted(old)) {
                self.fire_blur(old, FocusCause::Command);
            }
        }
        if let Some(target) = target {
            with_state(|state| state.set_focused_in(tree, Some(target)));
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
        self.finish_command_without_ui()?;
        self.refresh()
    }

    /// 没有挂载界面时的命令收尾：只报告 handler 的 fault 与写错误。
    fn finish_command_without_ui(&mut self) -> Result<(), MountError> {
        let fault = with_state(|state| state.fault);
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

    /// 呈现版本：任何可能改变 presentation snapshot 的状态变化都会让它变化。
    ///
    /// 原生宿主在重绘前比较它；版本未变时可以直接复用上一次快照，滚动与
    /// 指针瞬态因此不需要重建运行时快照。
    pub fn presentation_revision(&self) -> u64 {
        let state = self.state.borrow();
        let counters = state.core.counters();
        let mut revision = state.presentation_revision;
        let mut mix = |value: u64| {
            revision = revision.rotate_left(9) ^ value;
            revision = revision.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        };
        // 任何核心计数变化都可能对应一次组件刷新或挂载；宁可多重建一次，
        // 也不能复用已经过期的快照。
        mix(counters.total());
        mix(state.unmounts);
        mix(state.next_node_mount);
        mix(state.window_revision);
        // `Runtime::presentation()` 跟随焦点窗口；切换到已可见窗口不改变
        // window_revision，必须单独进入版本。
        mix(state.focused_window.map_or(0, |key| {
            key.as_bytes()
                .iter()
                .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
                    (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
                })
        }));
        mix(match state.appearance {
            Appearance::Light => 0,
            Appearance::Dark => 1,
        });
        revision
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
            .snapshot(TreeKey::Window(window.key))
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
            self.refresh()?;
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
        MountedNode::Show(show) => show
            .active
            .as_ref()
            .is_some_and(|branch| contains_element(state, &branch.children, wanted)),
        MountedNode::For(for_node) => {
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
        MountedNode::Show(show) => show
            .active
            .as_ref()
            .is_some_and(|branch| contains_band(state, &branch.children, band)),
        MountedNode::For(for_node) => {
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

fn text_length(element: &MountedElement) -> usize {
    element
        .text_value()
        .map_or(0, |value| value.chars().count())
}

fn clear_history_settle(state: &mut State, element: usize) {
    if let LocalState::TextInput { history, .. } = &mut state.elements[element].local {
        history.settle_local = false;
    }
}

/// 释放一批已卸载节点及其子树中的文本历史与观察值。
fn clear_history_in_nodes(state: &mut State, nodes: &[MountedNode]) {
    let mut elements = Vec::new();
    collect_elements(state, nodes, &mut elements);
    for element in elements {
        if let LocalState::TextInput { history, .. } = &mut state.elements[element].local {
            history.reset();
        }
    }
}

/// handler 可能改写了值；刷新后把本地 caret/selection 夹回新值范围。
fn clamp_local_edit(state: &mut State, element: usize) {
    let length = text_length(&state.elements[element]);
    if let LocalState::TextInput {
        caret,
        anchor,
        selection,
        ..
    } = &mut state.elements[element].local
    {
        *caret = (*caret).min(length);
        *anchor = (*anchor).min(length);
        if let Some((start, end)) = *selection {
            *selection = Some(text_edit::normalized_range(start, end, length));
        }
    }
}

fn set_local_selection(local: &mut LocalState, anchor: usize, focus: usize) {
    if let LocalState::TextInput {
        caret,
        anchor: fixed,
        selection,
        ..
    } = local
    {
        *caret = focus;
        *fixed = anchor;
        *selection = Some((anchor.min(focus), anchor.max(focus)));
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
            MountedNode::Show(show) => {
                if let Some(branch) = &show.active {
                    collect_elements(state, &branch.children, order);
                }
            }
            MountedNode::For(for_node) => {
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
            MountedNode::Element(id) => collect_element(state, *id, out),
            MountedNode::Component(id) => {
                collect_snapshot(state, &state.components[*id].children, out);
            }
            MountedNode::Show(show) => {
                if let Some(branch) = &show.active {
                    collect_snapshot(state, &branch.children, out);
                }
            }
            MountedNode::For(for_node) => {
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
        tag: element.tag,
        attrs: snapshot_attrs(element),
        styles: snapshot_styles(element),
        edit: snapshot_edit(&element.local),
        semantics: SnapshotSemantics::new(
            state.element_is_inherently_focusable(id),
            snapshot_actions(state, element),
        ),
        children,
    }));
}

/// 已挂载的用户动作集合；焦点/组合等运行时事件不在此投影里。
fn snapshot_actions(state: &State, element: &MountedElement) -> SnapshotActions {
    let mut actions = SnapshotActions::default();
    for id in &element.events {
        match event_kind(&state.events[*id]) {
            EventKind::Click => actions.insert(SnapshotAction::Click),
            EventKind::Change => actions.insert(SnapshotAction::Change),
            EventKind::Input => actions.insert(SnapshotAction::Input),
            _ => {}
        }
    }
    actions
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
            MountedNode::Show(show) => {
                if let Some(branch) = &show.active {
                    collect_row_bands(state, &branch.children, out);
                }
            }
            MountedNode::For(for_node) => {
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
    match local {
        LocalState::TextInput {
            caret,
            anchor,
            selection,
            composition,
            ..
        } => Some(TextEditState {
            caret: *caret,
            anchor: *anchor,
            selection: *selection,
            preedit: composition
                .as_ref()
                .map(|composition| composition.preedit.clone()),
            composition: composition
                .as_ref()
                .map(|composition| composition.base_range()),
        }),
        _ => None,
    }
}

/// 只复制已提交的声明；inline 槽的当前值在刷新时已经算好。
/// 没有 inline 声明时借用 class 切片，避免每个元素都分配。
fn snapshot_styles(
    element: &MountedElement,
) -> std::borrow::Cow<'static, [zenui_style::StyleDeclaration]> {
    if element.inline_styles.is_empty() {
        return std::borrow::Cow::Borrowed(element.class_styles);
    }
    let mut styles = element.class_styles.to_vec();
    styles.extend(element.inline_styles.iter().filter_map(|style| {
        style
            .current
            .map(|value| zenui_style::StyleDeclaration::inline(style.property, value))
    }));
    std::borrow::Cow::Owned(styles)
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
        let LocalState::TextInput { history, .. } = &state.elements[input.slot() as usize].local
        else {
            panic!("the slot was a text input");
        };
        assert!(history.undo.is_empty(), "the undo stack was released");
        assert!(history.redo.is_empty(), "the redo stack was released");
        assert_eq!(history.undo_bytes, 0);
        assert_eq!(history.redo_bytes, 0);
        assert!(
            history.observed.is_none(),
            "the observed value does not stay in the dead slot"
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
