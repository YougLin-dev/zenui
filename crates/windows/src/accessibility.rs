//! 辅助技术接入（UI Automation）。
//!
//! 每个目录窗口在每次已提交的场景发布时生成一棵只读的 [`AccessibilityTree`]。
//! 树从 presentation snapshot 与同一份 `Scene` 派生：角色、名称、AutomationId
//! 与状态来自快照，边界来自场景已经平移过的逻辑几何。它是派生的发布物，
//! 不是第二棵保留树，也不新增 Node ID。
//!
//! provider 持有窗口发布单元的弱引用，并在每次查询时解析当前发布，因此缓存的
//! 旧 provider 能看到滚动后的边界；已卸载的 Node ID 返回
//! `UIA_E_ELEMENTNOTAVAILABLE`。隐藏窗口与关闭中的窗口不发布树。
//!
//! 写操作只经排队通道：Invoke、Toggle、Value 与 SetFocus 在 provider 线程上
//! 检查当前树与启用状态后把 [`AccessibilityAction`] 入队并唤醒窗口，UI 线程
//! 再按最新快照重验，最后走与指针、键盘相同的 Runtime 命令路径。provider 线程
//! 从不直接修改 Runtime；`UIA_ACTION` 消息只负责唤醒，具体动作以队列顺序为准。

use std::collections::{HashMap, VecDeque};
use std::mem::ManuallyDrop;
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};

use windows::Win32::Foundation::{
    E_INVALIDARG, E_OUTOFMEMORY, HWND, POINT, VARIANT_BOOL, VARIANT_FALSE, VARIANT_TRUE,
};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayDestroy, SafeArrayPutElement};
use windows::Win32::System::Variant::{
    InitVariantFromDoubleArray, VARENUM, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BOOL,
    VT_BSTR, VT_I4,
};
use windows::Win32::UI::Accessibility::{
    IInvokeProvider, IInvokeProvider_Impl, IRawElementProviderFragment,
    IRawElementProviderFragment_Impl, IRawElementProviderFragmentRoot,
    IRawElementProviderFragmentRoot_Impl, IRawElementProviderSimple,
    IRawElementProviderSimple_Impl, IToggleProvider, IToggleProvider_Impl, IValueProvider,
    IValueProvider_Impl, NavigateDirection, NavigateDirection_FirstChild,
    NavigateDirection_LastChild, NavigateDirection_NextSibling, NavigateDirection_Parent,
    NavigateDirection_PreviousSibling, ProviderOptions, ProviderOptions_ServerSideProvider,
    ToggleState, ToggleState_Off, ToggleState_On, UIA_AutomationIdPropertyId,
    UIA_BoundingRectanglePropertyId, UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId,
    UIA_ClassNamePropertyId, UIA_ControlTypePropertyId, UIA_CustomControlTypeId,
    UIA_E_ELEMENTNOTAVAILABLE, UIA_E_NOTSUPPORTED, UIA_EditControlTypeId,
    UIA_FrameworkIdPropertyId, UIA_GroupControlTypeId, UIA_HasKeyboardFocusPropertyId,
    UIA_InvokePatternId, UIA_IsContentElementPropertyId, UIA_IsControlElementPropertyId,
    UIA_IsEnabledPropertyId, UIA_IsKeyboardFocusablePropertyId, UIA_IsOffscreenPropertyId,
    UIA_ListControlTypeId, UIA_ListItemControlTypeId, UIA_NamePropertyId,
    UIA_NativeWindowHandlePropertyId, UIA_ProcessIdPropertyId, UIA_ProviderDescriptionPropertyId,
    UIA_TextControlTypeId, UIA_TogglePatternId, UIA_ToggleToggleStatePropertyId,
    UIA_ToolBarControlTypeId, UIA_ValuePatternId, UIA_WindowControlTypeId, UiaAppendRuntimeId,
    UiaHostProviderFromHwnd, UiaRect,
};
use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
use windows::core::{BOOL, BSTR, Error, HRESULT, PCWSTR, Result as WinResult, implement};
use zenui::{
    NodeId, PresentationSnapshot, SnapshotAction, SnapshotActions, SnapshotAttr, SnapshotElement,
    SnapshotNode,
};
use zenui_render::{Rect, Scene, Viewport};

use crate::host::post_uia_action;

/// 一个已提交窗口的只读辅助功能树。
///
/// 树在每次场景发布时整体替换；同一棵树内部不可变。Node ID 来自 presentation
/// snapshot，`runtime_id` 在多次发布之间保持稳定，直到对应节点卸载。
#[derive(Debug)]
pub struct AccessibilityTree {
    nodes: Vec<AccessibleNode>,
    by_node: HashMap<NodeId, usize>,
    scale: f32,
}

/// 树中一个可访问节点。
///
/// 根节点的 `node_id` 与 `runtime_id` 为 `None`；其余节点对应一个已提交元素。
/// 没有 Node ID 的裸文本不产生节点，只并入最近元素的名称。
#[derive(Debug)]
pub struct AccessibleNode {
    node_id: Option<NodeId>,
    runtime_id: Option<i32>,
    control_type: ControlType,
    tag: &'static str,
    name: String,
    automation_id: String,
    enabled: bool,
    focusable: bool,
    focused: bool,
    offscreen: bool,
    toggle_state: Option<bool>,
    actions: SnapshotActions,
    value: Option<String>,
    composing: bool,
    rect: Rect,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// UIA 控制类型中本框架使用的最小子集。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlType {
    /// 窗口根。
    Window,
    /// 分组容器。
    Group,
    /// 工具栏。
    ToolBar,
    /// 列表容器。
    List,
    /// 列表项。
    ListItem,
    /// 只读文本。
    Text,
    /// 按钮。
    Button,
    /// 复选框。
    CheckBox,
    /// 文本输入。
    Edit,
    /// 没有映射的标签。
    Custom,
}

/// 物理像素屏幕矩形；成员为 UIA `double` 语义。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicalRect {
    /// 左边。
    pub left: f64,
    /// 上边。
    pub top: f64,
    /// 宽度。
    pub width: f64,
    /// 高度。
    pub height: f64,
}

impl AccessibilityTree {
    /// 根节点。
    pub fn root(&self) -> &AccessibleNode {
        &self.nodes[0]
    }

    /// 按 Node ID 查找节点。
    pub fn node(&self, id: NodeId) -> Option<&AccessibleNode> {
        self.by_node.get(&id).map(|index| &self.nodes[*index])
    }

    /// 节点数量，包含根。
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// 按前序（先序）遍历所有节点。
    pub fn iter(&self) -> impl Iterator<Item = &AccessibleNode> {
        self.nodes.iter()
    }

    /// 按前序返回某节点的子节点。
    ///
    /// # Panics
    ///
    /// `node` 必须来自本树。跨树的节点带有另一棵树的索引，无法解析。
    pub fn children<'a>(
        &'a self,
        node: &'a AccessibleNode,
    ) -> impl Iterator<Item = &'a AccessibleNode> {
        node.children.iter().map(|index| &self.nodes[*index])
    }

    /// 按名称查找节点；测试与验收使用。
    pub fn find_name(&self, name: &str) -> Option<&AccessibleNode> {
        self.nodes.iter().find(|node| node.name == name)
    }

    /// 发布时的 Window scale。
    pub const fn scale(&self) -> f32 {
        self.scale
    }

    /// 节点已平移的逻辑矩形；`node` 应来自本树。
    pub const fn logical_rect(&self, node: &AccessibleNode) -> Rect {
        node.rect
    }

    /// 节点在屏幕坐标中的物理边界；`node` 应来自本树。
    pub fn bounds(&self, node: &AccessibleNode, origin: (i32, i32)) -> PhysicalRect {
        physical_rect(node.rect, origin, self.scale)
    }

    /// 根窗口在屏幕坐标中的物理边界。
    pub fn root_bounds(&self, origin: (i32, i32)) -> PhysicalRect {
        physical_rect(self.root().rect, origin, self.scale)
    }

    fn index_of(&self, id: NodeId) -> Option<usize> {
        self.by_node.get(&id).copied()
    }

    fn node_at(&self, index: usize) -> Option<&AccessibleNode> {
        self.nodes.get(index)
    }

    fn parent_index(&self, index: usize) -> Option<usize> {
        self.nodes.get(index).and_then(|node| node.parent)
    }

    fn child_indices(&self, index: usize) -> Option<&[usize]> {
        self.nodes.get(index).map(|node| node.children.as_slice())
    }

    fn focused_index(&self) -> Option<usize> {
        self.nodes
            .iter()
            .position(|node| node.focused && node.node_id.is_some())
    }
}

impl AccessibleNode {
    /// 对应的 Node ID；根节点为 `None`。
    pub const fn node_id(&self) -> Option<NodeId> {
        self.node_id
    }

    /// provider 本地运行时 ID；根节点为 `None`。
    pub const fn runtime_id(&self) -> Option<i32> {
        self.runtime_id
    }

    /// UIA 控制类型。
    pub const fn control_type(&self) -> ControlType {
        self.control_type
    }

    /// intrinsic 标签；根节点为宿主窗口类名。
    pub const fn tag(&self) -> &'static str {
        self.tag
    }

    /// UIA 名称：优先 `aria:label`，否则直接可见文本。
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `id` 属性映射的 AutomationId。
    pub fn automation_id(&self) -> &str {
        &self.automation_id
    }

    /// 元素与窗口都启用时为真。
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// 固有可聚焦且在发布时启用；不考虑原生窗口的后续启用变化。
    pub const fn focusable(&self) -> bool {
        self.focusable
    }

    /// 发布时快照焦点是否在该节点。
    pub const fn focused(&self) -> bool {
        self.focused
    }

    /// 节点是否不与发布时的视口相交。
    pub const fn offscreen(&self) -> bool {
        self.offscreen
    }

    /// `checked` 或 `aria:pressed` 的布尔状态；没有该状态时为 `None`。
    pub const fn toggle_state(&self) -> Option<bool> {
        self.toggle_state
    }

    /// 快照声明的用户动作；provider 据此决定 pattern 是否存在。
    pub const fn has_action(&self, action: SnapshotAction) -> bool {
        self.actions.contains(action)
    }

    /// 文本输入已提交的值；其它节点为 `None`。
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// 文本输入当前是否处于 IME 组合中。
    pub const fn composing(&self) -> bool {
        self.composing
    }
}

/// 每个窗口的 provider 本地运行时 ID 分配器；Node ID 卸载后可回收表项。
pub(crate) struct AccessibilityIds {
    next: i32,
    by_node: HashMap<NodeId, i32>,
}

impl AccessibilityIds {
    pub(crate) fn new() -> Self {
        Self {
            next: 1,
            by_node: HashMap::new(),
        }
    }

    fn id_for(&mut self, node: NodeId) -> i32 {
        if let Some(id) = self.by_node.get(&node) {
            return *id;
        }
        let id = self.next;
        self.next = self.next.saturating_add(1).max(1);
        self.by_node.insert(node, id);
        id
    }

    fn retain(&mut self, live: &HashMap<NodeId, usize>) {
        self.by_node.retain(|node, _| live.contains_key(node));
    }
}

/// provider 线程排入、UI 线程重验后执行的动作。
pub(crate) enum AccessibilityAction {
    Invoke(NodeId),
    Toggle(NodeId),
    SetValue(NodeId, String),
    SetFocus(NodeId),
}

impl AccessibilityAction {
    pub(crate) const fn node(&self) -> NodeId {
        match self {
            Self::Invoke(node)
            | Self::Toggle(node)
            | Self::SetValue(node, _)
            | Self::SetFocus(node) => *node,
        }
    }
}

/// 单个窗口最多保留的待执行动作数。
///
/// provider 线程可能比 UI 线程快得多；达到上限后拒绝新动作，已排队的动作仍按序执行。
pub(crate) const MAX_PENDING_ACTIONS: usize = 1024;

/// 一个窗口的可访问性发布单元；provider 通过弱引用解析当前发布。
pub(crate) struct WindowAccessibility {
    hwnd: WindowHandle,
    published: RwLock<Option<Arc<AccessibilityTree>>>,
    actions: Mutex<VecDeque<AccessibilityAction>>,
}

impl WindowAccessibility {
    pub(crate) fn new(hwnd: HWND) -> Self {
        Self {
            hwnd: WindowHandle(hwnd),
            published: RwLock::new(None),
            actions: Mutex::new(VecDeque::new()),
        }
    }

    pub(crate) fn publish(&self, tree: AccessibilityTree) {
        let mut slot = self
            .published
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        *slot = Some(Arc::new(tree));
    }

    pub(crate) fn clear(&self) {
        let mut slot = self
            .published
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        *slot = None;
        self.actions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    pub(crate) fn tree(&self) -> Option<Arc<AccessibilityTree>> {
        self.published
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// 记录一次 provider 动作并唤醒 UI 线程；调用可能来自任意 COM 线程。
    ///
    /// 队列满或投递失败时返回 false，且不留下动作；调用方据此向 UIA 报告
    /// 元素暂不可用。持锁调用 `PostMessageW` 只排队，不会回调本进程。
    pub(crate) fn enqueue(&self, action: AccessibilityAction) -> bool {
        let mut actions = self.actions.lock().unwrap_or_else(PoisonError::into_inner);
        if actions.len() >= MAX_PENDING_ACTIONS {
            return false;
        }
        actions.push_back(action);
        if post_uia_action(self.hwnd()) {
            return true;
        }
        actions.pop_back();
        false
    }

    /// 按入队顺序取走动作；宿主在 UI 线程上调用。
    pub(crate) fn take_actions(&self) -> Vec<AccessibilityAction> {
        self.actions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }

    fn hwnd(&self) -> HWND {
        self.hwnd.0
    }
}

/// `HWND` 的 `Send + Sync` 包装。
///
/// 句柄值只用于线程安全的 Win32 查询（`ClientToScreen`、`IsWindowEnabled`、
/// `UiaHostProviderFromHwnd`），从不跨线程解引用。窗口销毁后这些查询只会失败。
#[derive(Clone, Copy)]
struct WindowHandle(HWND);

// 安全：如上，句柄值只作为不透明参数传给线程安全的 Win32 函数。
unsafe impl Send for WindowHandle {}
// 安全：同上。
unsafe impl Sync for WindowHandle {}

/// 构造某个窗口的 fragment root provider。
pub(crate) fn fragment_root(window: &Arc<WindowAccessibility>) -> IRawElementProviderSimple {
    ZenProvider {
        window: Arc::downgrade(window),
        key: ProviderKey::Window,
    }
    .into()
}

/// 在 provider 调用线程上解析当前发布中的节点；元素 provider 与模式 provider 共用。
fn resolve_node(
    window: &Weak<WindowAccessibility>,
    node: NodeId,
) -> WinResult<(Arc<WindowAccessibility>, Arc<AccessibilityTree>, usize)> {
    let window = window.upgrade().ok_or_else(element_not_available)?;
    let tree = window.tree().ok_or_else(element_not_available)?;
    let index = tree.index_of(node).ok_or_else(element_not_available)?;
    Ok((window, tree, index))
}

/// 发布时的启用状态与当前原生窗口启用状态都为真。
fn node_enabled(window: &WindowAccessibility, node: &AccessibleNode) -> bool {
    node.enabled && window_enabled(window.hwnd())
}

/// 解析当前发布中仍启用的节点；写 pattern 共用的身份、发布与启用检查。
///
/// 禁用目标按 UIA 契约报告元素不可用；只读查询不要使用这个入口，它们仍需
/// 能读取禁用元素的名称、状态与边界。
fn available_node(
    window: &Weak<WindowAccessibility>,
    node: NodeId,
) -> WinResult<(Arc<WindowAccessibility>, Arc<AccessibilityTree>, usize)> {
    let (window, tree, index) = resolve_node(window, node)?;
    let published = tree.node_at(index).ok_or_else(element_not_available)?;
    if !node_enabled(&window, published) {
        return Err(element_not_available());
    }
    Ok((window, tree, index))
}

fn window_enabled(hwnd: HWND) -> bool {
    // 安全：句柄只用于线程安全的启用状态查询；销毁后返回 false。
    unsafe { IsWindowEnabled(hwnd) }.as_bool()
}

/// Invoke 模式：只把动作入队，由 UI 线程重验后执行。
#[implement(IInvokeProvider)]
struct InvokePattern {
    window: Weak<WindowAccessibility>,
    node: NodeId,
}

impl IInvokeProvider_Impl for InvokePattern_Impl {
    fn Invoke(&self) -> WinResult<()> {
        let (window, tree, index) = available_node(&self.window, self.node)?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        if !node.has_action(SnapshotAction::Click) {
            return Err(not_supported());
        }
        if !window.enqueue(AccessibilityAction::Invoke(self.node)) {
            return Err(element_not_available());
        }
        Ok(())
    }
}

/// Toggle 模式：状态来自发布树，切换动作由 UI 线程读取最新状态后执行。
#[implement(IToggleProvider)]
struct TogglePattern {
    window: Weak<WindowAccessibility>,
    node: NodeId,
}

impl IToggleProvider_Impl for TogglePattern_Impl {
    fn Toggle(&self) -> WinResult<()> {
        let (window, tree, index) = available_node(&self.window, self.node)?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        if !node.has_action(SnapshotAction::Change) || node.toggle_state().is_none() {
            return Err(not_supported());
        }
        if !window.enqueue(AccessibilityAction::Toggle(self.node)) {
            return Err(element_not_available());
        }
        Ok(())
    }

    #[allow(non_snake_case)]
    fn ToggleState(&self) -> WinResult<ToggleState> {
        let (_, tree, index) = resolve_node(&self.window, self.node)?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        let state = node.toggle_state().ok_or_else(not_supported)?;
        Ok(if state {
            ToggleState_On
        } else {
            ToggleState_Off
        })
    }
}

/// Value 模式：返回值立即读取，写入排入队列。
#[implement(IValueProvider)]
struct ValuePattern {
    window: Weak<WindowAccessibility>,
    node: NodeId,
}

impl IValueProvider_Impl for ValuePattern_Impl {
    fn SetValue(&self, val: &PCWSTR) -> WinResult<()> {
        let (window, tree, index) = available_node(&self.window, self.node)?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        // 安全：val 由 UIA 传入，按契约是以 NUL 结尾的有效宽字符串。
        let value = unsafe { val.to_string() }.map_err(|_| Error::from_hresult(E_INVALIDARG))?;
        if node.has_action(SnapshotAction::Input)
            && !node.composing
            && window.enqueue(AccessibilityAction::SetValue(self.node, value))
        {
            return Ok(());
        }
        Err(element_not_available())
    }

    fn Value(&self) -> WinResult<BSTR> {
        let (_, tree, index) = resolve_node(&self.window, self.node)?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        let value = node.value().ok_or_else(not_supported)?;
        Ok(BSTR::from(value))
    }

    #[allow(non_snake_case)]
    fn IsReadOnly(&self) -> WinResult<BOOL> {
        let (window, tree, index) = resolve_node(&self.window, self.node)?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        let writable = node_enabled(&window, node)
            && node.has_action(SnapshotAction::Input)
            && !node.composing;
        Ok(BOOL::from(!writable))
    }
}

#[derive(Clone, Copy)]
enum ProviderKey {
    Window,
    Node(NodeId),
}

#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot
)]
struct ZenProvider {
    window: Weak<WindowAccessibility>,
    key: ProviderKey,
}

impl ZenProvider {
    fn resolve(&self) -> WinResult<(Arc<WindowAccessibility>, Arc<AccessibilityTree>, usize)> {
        match self.key {
            ProviderKey::Window => {
                let window = self.window.upgrade().ok_or_else(element_not_available)?;
                let tree = window.tree().ok_or_else(element_not_available)?;
                Ok((window, tree, 0))
            }
            ProviderKey::Node(node) => resolve_node(&self.window, node),
        }
    }

    fn provider_for(
        &self,
        tree: &AccessibilityTree,
        index: usize,
    ) -> WinResult<IRawElementProviderFragment> {
        let key = if index == 0 {
            ProviderKey::Window
        } else {
            let node = tree
                .node_at(index)
                .and_then(AccessibleNode::node_id)
                .ok_or_else(element_not_available)?;
            ProviderKey::Node(node)
        };
        Ok(ZenProvider {
            window: self.window.clone(),
            key,
        }
        .into())
    }

    fn root_provider(&self) -> WinResult<IRawElementProviderFragmentRoot> {
        Ok(ZenProvider {
            window: self.window.clone(),
            key: ProviderKey::Window,
        }
        .into())
    }

    fn live_enabled(&self, window: &WindowAccessibility) -> bool {
        window_enabled(window.hwnd())
    }

    fn window_bounds(
        &self,
        window: &WindowAccessibility,
        tree: &AccessibilityTree,
        node: &AccessibleNode,
    ) -> PhysicalRect {
        tree.bounds(node, client_origin(window.hwnd()))
    }
}

impl IRawElementProviderSimple_Impl for ZenProvider_Impl {
    #[allow(non_snake_case)]
    fn ProviderOptions(&self) -> WinResult<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider)
    }

    #[allow(non_snake_case)]
    fn GetPatternProvider(
        &self,
        patternid: windows::Win32::UI::Accessibility::UIA_PATTERN_ID,
    ) -> WinResult<windows::core::IUnknown> {
        let (_, tree, index) = self.resolve()?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        if patternid == UIA_InvokePatternId && node.has_action(SnapshotAction::Click) {
            let id = node.node_id().ok_or_else(element_not_available)?;
            return Ok(InvokePattern {
                window: self.window.clone(),
                node: id,
            }
            .into());
        }
        if patternid == UIA_TogglePatternId
            && node.has_action(SnapshotAction::Change)
            && node.toggle_state().is_some()
        {
            let id = node.node_id().ok_or_else(element_not_available)?;
            return Ok(TogglePattern {
                window: self.window.clone(),
                node: id,
            }
            .into());
        }
        if patternid == UIA_ValuePatternId && node.value().is_some() {
            let id = node.node_id().ok_or_else(element_not_available)?;
            return Ok(ValuePattern {
                window: self.window.clone(),
                node: id,
            }
            .into());
        }
        Err(Error::empty())
    }

    #[allow(non_snake_case)]
    fn GetPropertyValue(
        &self,
        propertyid: windows::Win32::UI::Accessibility::UIA_PROPERTY_ID,
    ) -> WinResult<VARIANT> {
        let (window, tree, index) = self.resolve()?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        let enabled = node.enabled && self.live_enabled(&window);
        if propertyid == UIA_NamePropertyId {
            return Ok(bstr_variant(&node.name));
        }
        if propertyid == UIA_AutomationIdPropertyId {
            return Ok(bstr_variant(&node.automation_id));
        }
        if propertyid == UIA_ControlTypePropertyId {
            return Ok(i32_variant(node.control_type.uia_id()));
        }
        if propertyid == UIA_ClassNamePropertyId {
            return Ok(bstr_variant(node.tag));
        }
        if propertyid == UIA_FrameworkIdPropertyId {
            return Ok(bstr_variant("ZenUI"));
        }
        if propertyid == UIA_ProviderDescriptionPropertyId {
            return Ok(bstr_variant("ZenUI accessibility provider"));
        }
        if propertyid == UIA_ProcessIdPropertyId {
            return Ok(i32_variant(std::process::id() as i32));
        }
        if propertyid == UIA_IsEnabledPropertyId {
            return Ok(bool_variant(enabled));
        }
        if propertyid == UIA_IsKeyboardFocusablePropertyId {
            return Ok(bool_variant(node.focusable && enabled));
        }
        if propertyid == UIA_HasKeyboardFocusPropertyId {
            return Ok(bool_variant(
                node.focused && enabled && window_is_foreground(window.hwnd()),
            ));
        }
        if propertyid == UIA_IsOffscreenPropertyId {
            return Ok(bool_variant(node.offscreen));
        }
        if propertyid == UIA_IsControlElementPropertyId
            || propertyid == UIA_IsContentElementPropertyId
        {
            return Ok(bool_variant(true));
        }
        if propertyid == UIA_ToggleToggleStatePropertyId
            && let Some(value) = node.toggle_state
        {
            let state = if value {
                ToggleState_On
            } else {
                ToggleState_Off
            };
            return Ok(i32_variant(state.0));
        }
        if propertyid == UIA_BoundingRectanglePropertyId {
            let bounds = self.window_bounds(&window, &tree, node);
            // 安全：四个 f64 复制进 VARIANT 持有的 SAFEARRAY。
            return unsafe {
                InitVariantFromDoubleArray(&[bounds.left, bounds.top, bounds.width, bounds.height])
            };
        }
        if propertyid == UIA_NativeWindowHandlePropertyId && index == 0 {
            return Ok(i32_variant(window.hwnd().0 as isize as i32));
        }
        Ok(VARIANT::default())
    }

    #[allow(non_snake_case)]
    fn HostRawElementProvider(&self) -> WinResult<IRawElementProviderSimple> {
        let (window, _, index) = self.resolve()?;
        if index != 0 {
            return Err(Error::empty());
        }
        // 安全：句柄只用于查询宿主 provider；窗口销毁时返回错误。
        unsafe { UiaHostProviderFromHwnd(window.hwnd()) }
    }
}

impl IRawElementProviderFragment_Impl for ZenProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> WinResult<IRawElementProviderFragment> {
        let (_, tree, index) = self.resolve()?;
        let target = if direction == NavigateDirection_Parent {
            tree.parent_index(index)
        } else if direction == NavigateDirection_FirstChild {
            tree.child_indices(index)
                .and_then(|children| children.first().copied())
        } else if direction == NavigateDirection_LastChild {
            tree.child_indices(index)
                .and_then(|children| children.last().copied())
        } else if direction == NavigateDirection_NextSibling
            || direction == NavigateDirection_PreviousSibling
        {
            let parent = tree.parent_index(index).ok_or_else(Error::empty)?;
            let siblings = tree.child_indices(parent).ok_or_else(Error::empty)?;
            let position = siblings
                .iter()
                .position(|sibling| *sibling == index)
                .ok_or_else(Error::empty)?;
            if direction == NavigateDirection_NextSibling {
                siblings.get(position + 1).copied()
            } else {
                position
                    .checked_sub(1)
                    .and_then(|position| siblings.get(position).copied())
            }
        } else {
            None
        };
        match target {
            Some(target) => self.provider_for(&tree, target),
            None => Err(Error::empty()),
        }
    }

    fn GetRuntimeId(&self) -> WinResult<*mut SAFEARRAY> {
        let (_, tree, index) = self.resolve()?;
        if index == 0 {
            return Ok(std::ptr::null_mut());
        }
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        let runtime_id = node.runtime_id.ok_or_else(element_not_available)?;
        safe_array_i32(&[UiaAppendRuntimeId as i32, runtime_id])
    }

    fn BoundingRectangle(&self) -> WinResult<UiaRect> {
        let (window, tree, index) = self.resolve()?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        let bounds = self.window_bounds(&window, &tree, node);
        Ok(UiaRect {
            left: bounds.left,
            top: bounds.top,
            width: bounds.width,
            height: bounds.height,
        })
    }

    fn GetEmbeddedFragmentRoots(&self) -> WinResult<*mut SAFEARRAY> {
        let _ = self.resolve()?;
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> WinResult<()> {
        let (window, tree, index) = self.resolve()?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        if index == 0 {
            return Err(not_supported());
        }
        if !node_enabled(&window, node) {
            return Err(element_not_available());
        }
        if !node.focusable {
            return Err(not_supported());
        }
        let id = node.node_id().ok_or_else(element_not_available)?;
        // 前台检查留给 UI 线程：从 provider 调用到执行之间窗口可能已经切到前台。
        if !window.enqueue(AccessibilityAction::SetFocus(id)) {
            return Err(element_not_available());
        }
        Ok(())
    }

    fn FragmentRoot(&self) -> WinResult<IRawElementProviderFragmentRoot> {
        let _ = self.resolve()?;
        self.root_provider()
    }
}

impl IRawElementProviderFragmentRoot_Impl for ZenProvider_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> WinResult<IRawElementProviderFragment> {
        let (window, tree, _) = self.resolve()?;
        let scale = f64::from(tree.scale());
        if scale <= 0.0 {
            return Err(Error::empty());
        }
        let origin = client_origin(window.hwnd());
        let local_x = (x - f64::from(origin.0)) / scale;
        let local_y = (y - f64::from(origin.1)) / scale;
        let viewport = tree.root().rect;
        if local_x < 0.0
            || local_y < 0.0
            || local_x >= f64::from(viewport.width)
            || local_y >= f64::from(viewport.height)
        {
            return Err(Error::empty());
        }
        for index in (1..tree.nodes.len()).rev() {
            let node = &tree.nodes[index];
            if node.offscreen {
                continue;
            }
            let rect = node.rect;
            if local_x >= f64::from(rect.x)
                && local_x < f64::from(rect.x + rect.width)
                && local_y >= f64::from(rect.y)
                && local_y < f64::from(rect.y + rect.height)
            {
                return self.provider_for(&tree, index);
            }
        }
        // 视口内没有子节点命中时，窗口根就是该点上的元素。
        self.provider_for(&tree, 0)
    }

    fn GetFocus(&self) -> WinResult<IRawElementProviderFragment> {
        let (window, tree, _) = self.resolve()?;
        if !self.live_enabled(&window) || !window_is_foreground(window.hwnd()) {
            return Err(Error::empty());
        }
        match tree.focused_index() {
            Some(index) => self.provider_for(&tree, index),
            None => Err(Error::empty()),
        }
    }
}

impl ControlType {
    fn uia_id(self) -> i32 {
        let id = match self {
            Self::Window => UIA_WindowControlTypeId,
            Self::Group => UIA_GroupControlTypeId,
            Self::ToolBar => UIA_ToolBarControlTypeId,
            Self::List => UIA_ListControlTypeId,
            Self::ListItem => UIA_ListItemControlTypeId,
            Self::Text => UIA_TextControlTypeId,
            Self::Button => UIA_ButtonControlTypeId,
            Self::CheckBox => UIA_CheckBoxControlTypeId,
            Self::Edit => UIA_EditControlTypeId,
            Self::Custom => UIA_CustomControlTypeId,
        };
        id.0
    }
}

fn element_not_available() -> Error {
    Error::from_hresult(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32))
}

fn not_supported() -> Error {
    Error::from_hresult(HRESULT(UIA_E_NOTSUPPORTED as i32))
}

pub(crate) fn window_is_foreground(hwnd: HWND) -> bool {
    // 安全：只比较前台窗口句柄。
    unsafe { GetForegroundWindow() == hwnd }
}

fn client_origin(hwnd: HWND) -> (i32, i32) {
    let mut point = POINT { x: 0, y: 0 };
    // 安全：point 是本地输出缓冲；句柄只用于线程安全的坐标查询。
    if unsafe { ClientToScreen(hwnd, &mut point) }.as_bool() {
        (point.x, point.y)
    } else {
        (0, 0)
    }
}

fn physical_rect(rect: Rect, origin: (i32, i32), scale: f32) -> PhysicalRect {
    let scale = f64::from(scale);
    PhysicalRect {
        left: f64::from(origin.0) + f64::from(rect.x) * scale,
        top: f64::from(origin.1) + f64::from(rect.y) * scale,
        width: f64::from(rect.width) * scale,
        height: f64::from(rect.height) * scale,
    }
}

fn bstr_variant(value: &str) -> VARIANT {
    scalar_variant(
        VT_BSTR,
        VARIANT_0_0_0 {
            bstrVal: ManuallyDrop::new(BSTR::from(value)),
        },
    )
}

fn bool_variant(value: bool) -> VARIANT {
    scalar_variant(
        VT_BOOL,
        VARIANT_0_0_0 {
            boolVal: VARIANT_BOOL(if value {
                VARIANT_TRUE.0
            } else {
                VARIANT_FALSE.0
            }),
        },
    )
}

fn i32_variant(value: i32) -> VARIANT {
    scalar_variant(VT_I4, VARIANT_0_0_0 { lVal: value })
}

fn scalar_variant(vt: VARENUM, value: VARIANT_0_0_0) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: value,
            }),
        },
    }
}

fn safe_array_i32(values: &[i32]) -> WinResult<*mut SAFEARRAY> {
    // 安全：数组与索引都是本地值；创建的 SAFEARRAY 交给 UIA 持有。
    unsafe {
        let array = SafeArrayCreateVector(VT_I4, 0, values.len() as u32);
        if array.is_null() {
            return Err(Error::from_hresult(E_OUTOFMEMORY));
        }
        for (position, value) in values.iter().enumerate() {
            let index = position as i32;
            if let Err(error) = SafeArrayPutElement(array, &index, (value as *const i32).cast()) {
                let _ = SafeArrayDestroy(array);
                return Err(error);
            }
        }
        Ok(array)
    }
}

/// 从快照与场景派生一棵只读树。
pub(crate) fn build_tree(
    snapshot: &PresentationSnapshot,
    scene: &Scene,
    title: &str,
    window_enabled: bool,
    ids: &mut AccessibilityIds,
) -> AccessibilityTree {
    let viewport = scene.viewport();
    let mut builder = TreeBuilder {
        nodes: Vec::new(),
        by_node: HashMap::new(),
        scene,
        window_enabled,
        focus: snapshot.focus(),
        ids,
        viewport,
        scale: viewport.scale(),
    };
    builder.nodes.push(AccessibleNode {
        node_id: None,
        runtime_id: None,
        control_type: ControlType::Window,
        tag: "ZenUIHostWindow",
        name: title.to_owned(),
        automation_id: String::new(),
        enabled: window_enabled,
        focusable: false,
        focused: false,
        offscreen: false,
        toggle_state: None,
        actions: SnapshotActions::default(),
        value: None,
        composing: false,
        rect: Rect::new(0.0, 0.0, viewport.width(), viewport.height()),
        parent: None,
        children: Vec::new(),
    });
    for node in snapshot.roots() {
        if let SnapshotNode::Element(element) = node {
            builder.push_element(element, 0);
        }
    }
    {
        let TreeBuilder { by_node, ids, .. } = &mut builder;
        ids.retain(by_node);
    }
    AccessibilityTree {
        nodes: builder.nodes,
        by_node: builder.by_node,
        scale: builder.scale,
    }
}

struct TreeBuilder<'a> {
    nodes: Vec<AccessibleNode>,
    by_node: HashMap<NodeId, usize>,
    scene: &'a Scene,
    window_enabled: bool,
    focus: Option<NodeId>,
    ids: &'a mut AccessibilityIds,
    viewport: Viewport,
    scale: f32,
}

impl TreeBuilder<'_> {
    fn push_element(&mut self, element: &SnapshotElement, parent: usize) {
        if element.tag() == "view"
            && element.attr("role").and_then(SnapshotAttr::as_text) == Some("spacer")
        {
            self.push_children(element, parent);
            return;
        }
        let id = element.id();
        let Some(scene_element) = self.scene.node(id) else {
            self.push_children(element, parent);
            return;
        };
        let role = element.attr("role").and_then(SnapshotAttr::as_text);
        let control_type = control_type(element.tag(), role);
        let rect = scene_element.rect();
        let value = (control_type == ControlType::Edit)
            .then(|| element.text_value().unwrap_or_default().to_owned());
        let composing = element
            .edit()
            .is_some_and(|edit| edit.composition().is_some());
        let index = self.nodes.len();
        self.nodes.push(AccessibleNode {
            node_id: Some(id),
            runtime_id: Some(self.ids.id_for(id)),
            control_type,
            tag: element.tag(),
            name: accessible_name(element),
            automation_id: element
                .attr("id")
                .and_then(SnapshotAttr::as_text)
                .unwrap_or_default()
                .to_owned(),
            enabled: !element.disabled() && self.window_enabled,
            focusable: element.semantics().focusable()
                && !element.disabled()
                && self.window_enabled,
            focused: self.focus == Some(id),
            offscreen: self.offscreen(rect),
            toggle_state: element.toggle_state(),
            actions: element.semantics().actions(),
            value,
            composing,
            rect,
            parent: Some(parent),
            children: Vec::new(),
        });
        self.nodes[parent].children.push(index);
        self.by_node.insert(id, index);
        self.push_children(element, index);
    }

    fn push_children(&mut self, element: &SnapshotElement, parent: usize) {
        for child in element.children() {
            if let SnapshotNode::Element(child) = child {
                self.push_element(child, parent);
            }
        }
    }

    fn offscreen(&self, rect: Rect) -> bool {
        rect.width <= 0.0
            || rect.height <= 0.0
            || rect.x >= self.viewport.width()
            || rect.x + rect.width <= 0.0
            || rect.y >= self.viewport.height()
            || rect.y + rect.height <= 0.0
    }
}

fn accessible_name(element: &SnapshotElement) -> String {
    if let Some(label) = element.label() {
        let label = label.trim();
        if !label.is_empty() {
            return label.to_owned();
        }
    }
    element.text().trim().to_owned()
}

fn control_type(tag: &str, role: Option<&str>) -> ControlType {
    match tag {
        "button" => ControlType::Button,
        "checkbox" => ControlType::CheckBox,
        "text-input" => ControlType::Edit,
        "text" => ControlType::Text,
        "form" => ControlType::Group,
        "view" => match role {
            Some("toolbar") => ControlType::ToolBar,
            Some("list") => ControlType::List,
            Some("listitem") => ControlType::ListItem,
            // spacer 在 push_element 中透明处理，不会到达这里。
            _ => ControlType::Group,
        },
        _ => ControlType::Custom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_and_publications_are_send_and_sync() {
        // COM 接口句柄本身在 windows-rs 中不是 Sync；可跨线程共享的是
        // provider 持有的不可变状态，它只通过 Weak 解析当前发布。
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ZenProvider>();
        assert_send_sync::<WindowAccessibility>();
        assert_send_sync::<AccessibilityTree>();
    }
}
