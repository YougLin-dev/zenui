use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::rc::Rc;

use crate::menu::{
    CommandError, CommandEvent, MENU_ITEM_LIMIT, MenuCommand, MenuCommandState, MenuShortcut,
    MenuSpec, ShortcutError,
};
use crate::presentation::Appearance;
use crate::task::TaskScope;
use crate::view::{MountError, MountPlan, MountResult};

/// 一次窗口打开的 identity；它不等于目录键，也不保证窗口仍然存活。
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct WindowCtx {
    pub(crate) runtime: u32,
    pub(crate) key: &'static str,
    pub(crate) activation: u32,
    pub(crate) owner: u32,
    _not_send: PhantomData<*const ()>,
}

impl WindowCtx {
    pub(crate) const fn new(runtime: u32, key: &'static str, activation: u32, owner: u32) -> Self {
        Self {
            runtime,
            key,
            activation,
            owner,
            _not_send: PhantomData,
        }
    }

    pub const fn key(self) -> &'static str {
        self.key
    }

    /// 目录键之外的打开代际；旧 activation 的结果不得命中重新打开的窗口。
    pub const fn activation(self) -> u32 {
        self.activation
    }

    /// 窗口 activation 拥有的工作：关闭后交付被取消。
    pub const fn task_scope(self) -> TaskScope {
        TaskScope::for_window(self.runtime, self)
    }

    /// 应用所有权的工作：窗口关闭后仍可提交共享模型。
    pub const fn app_task_scope(self) -> TaskScope {
        TaskScope::app(self.runtime)
    }

    /// 当前应用外观；在 memo 计算期间读取会建立依赖。
    pub fn appearance(self) -> Result<Appearance, WindowError> {
        crate::context::with_state_of(self.runtime, |state| {
            if !state.window_is_live(self.key, self.activation, self.owner) {
                return Err(WindowError::Stale);
            }
            Ok(state.read_appearance())
        })
    }

    /// 设置应用外观；窗口已关闭时拒绝。
    pub fn set_appearance(self, appearance: Appearance) -> Result<(), WindowError> {
        crate::context::with_state_of(self.runtime, |state| {
            if !state.window_is_live(self.key, self.activation, self.owner) {
                return Err(WindowError::Stale);
            }
            state.set_appearance(appearance);
            Ok(())
        })
    }

    /// 请求打开或聚焦目录窗口；宿主在本次事件/交付结算后执行。
    ///
    /// 只能在 `Event` 或 `Delivery` 处理器中调用。目标窗口已可见时只聚焦，
    /// 不重跑 factory；来源 activation 在结算前关闭时请求被丢弃。
    pub fn request_show(self, key: &'static str) -> Result<(), WindowError> {
        self.request(WindowAction::Show(key))
    }

    /// 请求隐藏自身窗口；保留 activation、树与本地编辑状态。
    ///
    /// 若该 activation 有待决关闭，先按 Deny 结算；作为确认窗口时隐藏等价于
    /// 关闭确认窗口并恢复父窗口。
    pub fn request_hide(self) -> Result<(), WindowError> {
        self.request(WindowAction::Hide)
    }

    /// 请求关闭自身窗口；遵循目录的 `ClosePolicy`。
    ///
    /// `Immediate` 释放 activation 的树、资源与窗口任务；`Confirm` 创建或
    /// 复用父窗口的 close epoch，等 Allow 后才释放。
    pub fn request_close(self) -> Result<(), WindowError> {
        self.request(WindowAction::Close)
    }

    fn request(self, action: WindowAction) -> Result<(), WindowError> {
        crate::context::with_state_of(self.runtime, |state| {
            state.queue_window_request(WindowRequest { from: self, action })
        })
    }

    /// 在窗口 factory 中注册关闭处理器；`Confirm` 策略的关闭请求会调用它。
    ///
    /// 每个 activation 只能注册一次。处理器收到当前关闭 epoch 的一次性句柄；
    /// 它可以立即 allow/deny，或显示目录中声明给这个父窗口的确认窗口。
    pub fn on_close(self, handler: impl Fn(CloseRequest) + 'static) -> Result<(), WindowError> {
        crate::context::with_state_of(self.runtime, |state| {
            state.register_close_handler(self, Rc::new(handler))
        })
    }

    /// 在窗口 factory 中注册文件拖放处理器；每个 activation 只能注册一次。
    ///
    /// 处理器由 [`crate::Runtime::drop_files`] 在 `Phase::Event` 中调用，可以写
    /// Store/Signal、排队窗口请求并启动窗口或应用所有权任务。没有注册处理器的
    /// 窗口拒绝拖放；关闭窗口释放处理器，重新打开必须重新注册。
    pub fn on_drop(self, handler: impl Fn(DropEvent) + 'static) -> Result<(), WindowError> {
        crate::context::with_state_of(self.runtime, |state| {
            state.register_drop_handler(self, Rc::new(handler))
        })
    }

    /// 在窗口 factory 中为菜单命令注册处理器；每个 activation、每个 id 一次。
    ///
    /// 菜单栏与上下文菜单声明的每个命令都必须有窗口或应用级处理器，否则这次
    /// 打开失败。应用级处理器由 `Runtime::on_app_command` 注册，可被多个窗口
    /// 共享；窗口处理器优先于应用级处理器。注册只发生在窗口 factory 自己的
    /// Setup 中；嵌套组件的 Setup、事件与交付阶段都拒绝。关闭窗口释放全部
    /// 窗口处理器，重新打开必须重新注册。
    pub fn on_command(
        self,
        id: &'static str,
        handler: impl Fn(CommandEvent) + 'static,
    ) -> Result<(), WindowError> {
        crate::context::with_state_of(self.runtime, |state| {
            state.register_command_handler(self, id, Rc::new(handler))
        })
    }

    /// 在窗口 factory 中注册只读菜单状态回调；每个 activation 只能注册一次。
    ///
    /// 宿主在菜单打开前按命令调用它，用返回值设置 enabled/checked。回调运行在
    /// 只读作用域：Signal 写与 Store 事务被拒绝并返回 typed error，模型不变。
    /// 没有注册回调时宿主使用命令声明的默认状态。关闭窗口释放回调，重新打开
    /// 必须在新的 factory 中重新注册。
    pub fn on_menu_state(
        self,
        handler: impl Fn(&MenuCommand) -> MenuCommandState + 'static,
    ) -> Result<(), WindowError> {
        crate::context::with_state_of(self.runtime, |state| {
            state.register_menu_state_handler(self, Rc::new(handler))
        })
    }

    /// 作为关闭确认窗口时，读取父窗口当前 epoch 的请求。
    ///
    /// 没有待决关闭请求、或本窗口不是该父窗口声明的确认窗口时返回 `None`；
    /// 组件据此把按钮接到 `allow` / `deny`。
    pub fn close_request(self) -> Result<Option<CloseRequest>, WindowError> {
        crate::context::with_state_of(self.runtime, |state| state.close_request_for(self))
    }
}

/// 一个待决关闭 epoch 的一次性句柄；`Copy` 不能延长期限。
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CloseRequest {
    runtime: u32,
    pub(crate) parent: WindowCtx,
    pub(crate) epoch: u64,
    _not_send: PhantomData<*const ()>,
}

impl CloseRequest {
    pub(crate) const fn new(runtime: u32, parent: WindowCtx, epoch: u64) -> Self {
        Self {
            runtime,
            parent,
            epoch,
            _not_send: PhantomData,
        }
    }

    /// 父窗口本次关闭请求的 epoch；重开后不会复用。
    pub const fn epoch(self) -> u64 {
        self.epoch
    }

    /// 允许关闭：父窗口与确认窗口作为一组释放。
    pub fn allow(self) -> Result<(), WindowError> {
        self.reply(CloseReply::Allow)
    }

    /// 拒绝关闭：关闭确认窗口，恢复父窗口输入与焦点。
    pub fn deny(self) -> Result<(), WindowError> {
        self.reply(CloseReply::Deny)
    }

    fn reply(self, reply: CloseReply) -> Result<(), WindowError> {
        crate::context::with_state_of(self.runtime, |state| state.queue_close_reply(self, reply))
    }
}

impl fmt::Debug for CloseRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloseRequest")
            .field("parent", &self.parent)
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseReply {
    Allow,
    Deny,
}

pub(crate) type CloseHandler = Rc<dyn Fn(CloseRequest)>;

/// 一次已接受的窗口文件拖放。
///
/// 只在 `on_drop` 处理器调用期间存在。路径保持源顺序、重复项与 Unicode；
/// 宿主不规范化、不去重，也不把文件夹展开成子项。
#[derive(Debug)]
pub struct DropEvent {
    window: WindowCtx,
    paths: Vec<PathBuf>,
}

impl DropEvent {
    pub(crate) fn new(window: WindowCtx, paths: Vec<PathBuf>) -> Self {
        Self { window, paths }
    }

    /// 接收这次拖放的窗口 activation。
    pub const fn window(&self) -> WindowCtx {
        self.window
    }

    /// 按源顺序读取路径；处理器可以检查后克隆需要保留的部分。
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// 取走有序路径，通常交给应用所有权 worker。
    pub fn into_paths(self) -> Vec<PathBuf> {
        self.paths
    }
}

pub(crate) type DropHandler = Rc<dyn Fn(DropEvent)>;

/// 文件拖放被拒绝的原因。
///
/// 验证发生在用户处理器运行之前；被拒绝的拖放不写模型、不启动任务，也不
/// 进入 `Phase::Event`。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DropError {
    /// `WindowCtx` 属于另一个 Runtime。
    WrongRuntime,
    /// 该 activation 已关闭或被新的一代取代。
    Stale,
    /// 窗口仍然存活但处于隐藏状态。
    Hidden,
    /// 窗口正在等待关闭确认；父窗口与确认窗口都拒绝拖放。
    Suspended,
    /// 路径列表为空。
    Empty,
    /// 窗口没有在 factory 中注册 `on_drop` 处理器。
    NoHandler,
    /// 处理器 panic；容器被捕获，运行时保持可用。
    HandlerPanicked,
    /// 处理器或其后的界面刷新失败。
    Command(MountError),
}

impl fmt::Display for DropError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongRuntime => {
                f.write_str("a WindowCtx from another ZenUI Runtime cannot receive this drop")
            }
            Self::Stale => f.write_str("the window activation is no longer live"),
            Self::Hidden => f.write_str("the window is hidden"),
            Self::Suspended => f.write_str("the window is awaiting close confirmation"),
            Self::Empty => f.write_str("a file drop needs at least one path"),
            Self::NoHandler => f.write_str("the window has no on_drop handler"),
            Self::HandlerPanicked => f.write_str("the on_drop handler panicked"),
            Self::Command(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for DropError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Command(error) => Some(error),
            _ => None,
        }
    }
}

/// 关闭确认协议本身失败；父窗口保持打开，绝不回退为 Allow。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CloseError {
    /// `Confirm` 窗口没有在 factory 中注册 close handler。
    MissingHandler,
    /// 目录没有为这个父窗口声明确认窗口。
    ConfirmationNotDeclared,
    /// 处理器没有让声明的确认窗口变为可见。
    ConfirmationNotShown,
    /// 处理器 panic；取消本次 epoch。
    HandlerPanicked,
}

/// UI 处理期挂起的窗口生命周期请求；由 `Runtime::apply_window_requests` 结算。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WindowRequest {
    pub(crate) from: WindowCtx,
    pub(crate) action: WindowAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowAction {
    Show(&'static str),
    Hide,
    Close,
}

impl fmt::Debug for WindowCtx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowCtx")
            .field("runtime", &self.runtime)
            .field("key", &self.key)
            .field("activation", &self.activation)
            .finish_non_exhaustive()
    }
}

/// 窗口收到关闭请求时的策略；确认窗口自身必须使用 `Immediate`，目录拒绝嵌套确认。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClosePolicy {
    /// 关闭请求立即释放 activation。
    #[default]
    Immediate,
    /// 关闭请求创建一个 epoch，并调用窗口注册的 close handler。
    Confirm,
}

/// 窗口在目录中的角色；确认窗口绑定到一个 `Confirm` 父窗口。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowRole {
    /// 普通应用窗口。
    #[default]
    Normal,
    /// 父窗口关闭时用来放置确认界面的窗口。
    CloseConfirmation { parent: &'static str },
}

/// 声明期的窗口描述；登记身份、原生标题/尺寸、primary 标记、关闭策略与角色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowSpec {
    key: &'static str,
    title: &'static str,
    width: u32,
    height: u32,
    primary: bool,
    close_policy: ClosePolicy,
    role: WindowRole,
}

impl WindowSpec {
    /// 以目录键作为标题、900×700 逻辑客户区的普通窗口。
    pub const fn new(key: &'static str) -> Self {
        Self {
            key,
            title: key,
            width: 900,
            height: 700,
            primary: false,
            close_policy: ClosePolicy::Immediate,
            role: WindowRole::Normal,
        }
    }

    /// 标记目录中的 primary 窗口；每个目录恰好有一个。
    pub const fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    /// 原生窗口标题；缺省是目录键。
    pub const fn titled(mut self, title: &'static str) -> Self {
        self.title = title;
        self
    }

    /// 初始客户区的逻辑尺寸；原生宿主按系统 DPI 换算为物理像素。
    pub const fn sized(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// 关闭策略；`Confirm` 会先创建一个 close epoch。
    pub const fn close_policy(mut self, policy: ClosePolicy) -> Self {
        self.close_policy = policy;
        self
    }

    /// 把窗口声明为父窗口的关闭确认窗口。
    pub const fn confirmation_for(mut self, parent: &'static str) -> Self {
        self.role = WindowRole::CloseConfirmation { parent };
        self
    }

    /// 目录中的稳定键。
    pub const fn key(self) -> &'static str {
        self.key
    }

    /// 原生窗口标题。
    pub const fn title(self) -> &'static str {
        self.title
    }

    /// 初始客户区的逻辑尺寸。
    pub const fn size(self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 是否为目录中的 primary 窗口。
    pub const fn is_primary(self) -> bool {
        self.primary
    }

    /// 收到关闭请求时的策略。
    pub const fn policy(self) -> ClosePolicy {
        self.close_policy
    }

    /// 目录角色。
    pub const fn role(self) -> WindowRole {
        self.role
    }

    /// 作为确认窗口时绑定的父窗口键。
    pub const fn confirmation_parent(self) -> Option<&'static str> {
        match self.role {
            WindowRole::Normal => None,
            WindowRole::CloseConfirmation { parent } => Some(parent),
        }
    }
}

/// 目录中的一个窗口：静态描述、每次打开都执行的 factory 与可选静态菜单。
pub(crate) struct CatalogEntry {
    pub(crate) spec: WindowSpec,
    pub(crate) factory: WindowFactory,
    pub(crate) menu: Option<MenuSpec>,
    pub(crate) context_menu: Option<MenuSpec>,
}

/// 一个仍然存活的窗口及它的目录描述；原生宿主据此同步 HWND。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    /// 本次打开的 identity。
    pub window: WindowCtx,
    /// 目录声明的标题与初始尺寸。
    pub spec: WindowSpec,
    /// 运行时可见性；隐藏窗口保留 activation 与树。
    pub visible: bool,
}

pub(crate) type WindowFactory = Rc<dyn Fn(WindowCtx) -> MountResult<MountPlan>>;

/// 声明窗口目录的临时收集器；`Runtime::declare_windows` 接收它。
pub struct WindowCatalog {
    entries: Vec<(WindowSpec, WindowFactory)>,
    menus: Vec<(&'static str, MenuSpec)>,
    context_menus: Vec<(&'static str, MenuSpec)>,
}

impl WindowCatalog {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
            menus: Vec::new(),
            context_menus: Vec::new(),
        }
    }

    /// 登记一个窗口；重复键立即报错，不保留第二份 factory。
    pub fn window(
        &mut self,
        spec: WindowSpec,
        factory: impl Fn(WindowCtx) -> MountResult<MountPlan> + 'static,
    ) -> Result<(), CatalogError> {
        if self.entries.iter().any(|(entry, _)| entry.key == spec.key) {
            return Err(CatalogError::DuplicateKey(spec.key));
        }
        self.entries.push((spec, Rc::new(factory)));
        Ok(())
    }

    /// 为一个目录窗口附加静态菜单；菜单不创建原生界面。
    ///
    /// 菜单项可以由窗口 factory 或应用级处理器承接。可以在 `window` 之前或
    /// 之后调用；未知窗口键、重复菜单、重复命令 id 与超限在
    /// `Runtime::declare_windows` 完成时校验，重复菜单立即报错。
    pub fn menu(&mut self, window: &'static str, menu: MenuSpec) -> Result<(), CatalogError> {
        if self.menus.iter().any(|(key, _)| *key == window) {
            return Err(CatalogError::DuplicateMenu(window));
        }
        self.menus.push((window, menu));
        Ok(())
    }

    /// 为一个目录窗口附加窗口级上下文菜单；菜单不创建原生界面。
    ///
    /// 与菜单栏共享窗口或应用级命令处理器，但两者的命令 id 不能重复；上下文
    /// 菜单不声明快捷键，也不参与 `HACCEL`。未知窗口、重复声明、重复命令 id
    /// 与超限在 `Runtime::declare_windows` 完成时校验，重复上下文菜单立即报错。
    pub fn context_menu(
        &mut self,
        window: &'static str,
        menu: MenuSpec,
    ) -> Result<(), CatalogError> {
        if self.context_menus.iter().any(|(key, _)| *key == window) {
            return Err(CatalogError::DuplicateContextMenu(window));
        }
        self.context_menus.push((window, menu));
        Ok(())
    }

    pub(crate) fn finish(
        self,
    ) -> Result<(BTreeMap<&'static str, CatalogEntry>, &'static str), CatalogError> {
        let mut primary = None;
        let mut entries = BTreeMap::new();
        for (spec, factory) in self.entries {
            if spec.primary && primary.replace(spec.key).is_some() {
                return Err(CatalogError::MultiplePrimaries);
            }
            entries.insert(
                spec.key,
                CatalogEntry {
                    spec,
                    factory,
                    menu: None,
                    context_menu: None,
                },
            );
        }
        let Some(primary) = primary else {
            return Err(CatalogError::MissingPrimary);
        };
        for (key, menu) in self.menus {
            let Some(entry) = entries.get_mut(key) else {
                return Err(CatalogError::UnknownMenuWindow(key));
            };
            validate_menu_commands(key, &menu)?;
            if let Some((id, shortcut)) = menu.first_duplicate_shortcut() {
                return Err(CatalogError::DuplicateMenuShortcut {
                    window: key,
                    id,
                    shortcut,
                });
            }
            entry.menu = Some(menu);
        }
        for (key, menu) in self.context_menus {
            let Some(entry) = entries.get_mut(key) else {
                return Err(CatalogError::UnknownContextMenuWindow(key));
            };
            validate_menu_commands(key, &menu)?;
            if let Some(id) = menu.first_shortcut_command() {
                return Err(CatalogError::ContextMenuShortcut { window: key, id });
            }
            if let Some(menu_bar) = entry.menu.as_ref() {
                let declared: BTreeSet<_> = menu_bar.command_ids().into_iter().collect();
                if let Some(id) = menu
                    .command_ids()
                    .into_iter()
                    .find(|id| declared.contains(id))
                {
                    return Err(CatalogError::DuplicateMenuCommand { window: key, id });
                }
            }
            entry.context_menu = Some(menu);
        }
        let mut confirmation_parents = BTreeSet::new();
        for (key, entry) in &entries {
            let WindowRole::CloseConfirmation { parent } = entry.spec.role else {
                continue;
            };
            if parent == *key {
                return Err(CatalogError::ConfirmationParentIsSelf(key));
            }
            let Some(parent_entry) = entries.get(parent) else {
                return Err(CatalogError::UnknownConfirmationParent(key));
            };
            if parent_entry.spec.close_policy != ClosePolicy::Confirm {
                return Err(CatalogError::ConfirmationParentNotConfirm(parent));
            }
            if entry.spec.close_policy != ClosePolicy::Immediate {
                return Err(CatalogError::ConfirmationMustBeImmediate(key));
            }
            if !confirmation_parents.insert(parent) {
                return Err(CatalogError::DuplicateConfirmation(parent));
            }
        }
        Ok((entries, primary))
    }
}

/// 菜单栏与上下文菜单共享的声明校验：项数上限与树内重复命令 id。
fn validate_menu_commands(window: &'static str, menu: &MenuSpec) -> Result<(), CatalogError> {
    if let Some(items) = menu.items_over_limit() {
        return Err(CatalogError::MenuTooLarge { window, items });
    }
    if let Some(id) = menu.first_duplicate_command() {
        return Err(CatalogError::DuplicateMenuCommand { window, id });
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CatalogError {
    /// 同一个目录键被登记了两次。
    DuplicateKey(&'static str),
    /// 目录必须恰好有一个 primary 窗口。
    MissingPrimary,
    /// 目录声明了不止一个 primary 窗口。
    MultiplePrimaries,
    /// 目录只能声明一次。
    AlreadyDeclared,
    /// 窗口目录必须先于任何窗口 factory。
    DeclaredAfterWindow,
    /// 确认窗口绑定的父窗口不在目录中。
    UnknownConfirmationParent(&'static str),
    /// 确认窗口只能绑定到 `Confirm` 父窗口。
    ConfirmationParentNotConfirm(&'static str),
    /// 窗口不能作为自己的关闭确认窗口。
    ConfirmationParentIsSelf(&'static str),
    /// 同一个父窗口只能声明一个关闭确认窗口。
    DuplicateConfirmation(&'static str),
    /// 关闭确认窗口自身必须使用 Immediate 策略，避免嵌套确认。
    ConfirmationMustBeImmediate(&'static str),
    /// 菜单绑定的窗口键不在目录中。
    UnknownMenuWindow(&'static str),
    /// 同一个目录窗口声明了不止一个菜单。
    DuplicateMenu(&'static str),
    /// 上下文菜单绑定的窗口键不在目录中。
    UnknownContextMenuWindow(&'static str),
    /// 同一个目录窗口声明了不止一个上下文菜单。
    DuplicateContextMenu(&'static str),
    /// 上下文菜单声明了快捷键；窗口级上下文菜单不参与 HACCEL。
    ContextMenuShortcut {
        window: &'static str,
        id: &'static str,
    },
    /// 一个窗口的菜单树里命令 id 重复。
    DuplicateMenuCommand {
        window: &'static str,
        id: &'static str,
    },
    /// 菜单命令声明了无法解析的快捷键。
    InvalidMenuShortcut(ShortcutError),
    /// 一个窗口的菜单树里两个命令使用同一个归一化快捷键。
    DuplicateMenuShortcut {
        window: &'static str,
        id: &'static str,
        shortcut: MenuShortcut,
    },
    /// 菜单树超过可表示的命令项上限。
    MenuTooLarge { window: &'static str, items: usize },
}

impl From<ShortcutError> for CatalogError {
    fn from(error: ShortcutError) -> Self {
        Self::InvalidMenuShortcut(error)
    }
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateKey(key) => write!(f, "window key {key} is declared twice"),
            Self::MissingPrimary => {
                f.write_str("the window catalog needs exactly one primary window")
            }
            Self::MultiplePrimaries => {
                f.write_str("the window catalog declares more than one primary window")
            }
            Self::AlreadyDeclared => f.write_str("the window catalog was already declared"),
            Self::DeclaredAfterWindow => {
                f.write_str("the window catalog must be declared before any window opens")
            }
            Self::UnknownConfirmationParent(key) => {
                write!(f, "close confirmation window {key} has no declared parent")
            }
            Self::ConfirmationParentNotConfirm(parent) => {
                write!(f, "window {parent} is not a Confirm parent")
            }
            Self::ConfirmationParentIsSelf(key) => {
                write!(f, "window {key} cannot confirm its own close")
            }
            Self::DuplicateConfirmation(parent) => {
                write!(
                    f,
                    "window {parent} declares more than one close confirmation window"
                )
            }
            Self::ConfirmationMustBeImmediate(key) => {
                write!(
                    f,
                    "close confirmation window {key} must use Immediate close policy"
                )
            }
            Self::UnknownMenuWindow(key) => {
                write!(f, "menu window {key} is not in the catalog")
            }
            Self::DuplicateMenu(key) => {
                write!(f, "window {key} declares more than one menu")
            }
            Self::UnknownContextMenuWindow(key) => {
                write!(f, "context menu window {key} is not in the catalog")
            }
            Self::DuplicateContextMenu(key) => {
                write!(f, "window {key} declares more than one context menu")
            }
            Self::ContextMenuShortcut { window, id } => write!(
                f,
                "window {window} declares a shortcut on context menu command {id}"
            ),
            Self::DuplicateMenuCommand { window, id } => {
                write!(f, "window {window} declares command id {id} more than once")
            }
            Self::InvalidMenuShortcut(error) => write!(f, "{error}"),
            Self::DuplicateMenuShortcut {
                window,
                id,
                shortcut,
            } => write!(
                f,
                "window {window} declares shortcut {shortcut} for command {id} more than once"
            ),
            Self::MenuTooLarge { window, items } => {
                write!(
                    f,
                    "window {window} declares {items} menu items; the limit is {MENU_ITEM_LIMIT}"
                )
            }
        }
    }
}

impl std::error::Error for CatalogError {}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WindowError {
    /// headless 边界不执行该能力；返回错误而不是空成功。
    Unsupported(&'static str),
    /// 当前生命周期状态不允许该操作。
    State(&'static str),
    /// 窗口仍然存活，但处于隐藏状态。
    Hidden,
    /// 该 activation 已关闭或被更新的一代取代。
    Stale,
    WrongRuntime,
    /// 目录中没有这个窗口键。
    UnknownWindow(&'static str),
    /// 目录声明失败。
    Catalog(CatalogError),
    /// 关闭确认协议失败，父窗口保持打开。
    Close(CloseError),
    /// 命令注册或派发失败。
    Command(CommandError),
    Mount(MountError),
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(operation) => {
                write!(
                    f,
                    "{operation} is not supported by the headless window boundary"
                )
            }
            Self::State(message) => f.write_str(message),
            Self::Hidden => f.write_str("the window is hidden"),
            Self::Stale => f.write_str("the window activation is no longer live"),
            Self::WrongRuntime => {
                f.write_str("a WindowCtx from another ZenUI Runtime cannot access this one")
            }
            Self::UnknownWindow(key) => write!(f, "window key {key} is not in the catalog"),
            Self::Catalog(error) => write!(f, "{error}"),
            Self::Close(error) => write!(f, "{error}"),
            Self::Command(error) => write!(f, "{error}"),
            Self::Mount(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for WindowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog(error) => Some(error),
            Self::Close(error) => Some(error),
            Self::Command(error) => Some(error),
            Self::Mount(error) => Some(error),
            _ => None,
        }
    }
}

impl fmt::Display for CloseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHandler => {
                f.write_str("a Confirm window needs a close handler registered during its factory")
            }
            Self::ConfirmationNotDeclared => {
                f.write_str("the catalog declares no confirmation window for this parent")
            }
            Self::ConfirmationNotShown => {
                f.write_str("the close handler did not show the declared confirmation window")
            }
            Self::HandlerPanicked => {
                f.write_str("the close handler panicked; the window stays open")
            }
        }
    }
}

impl std::error::Error for CloseError {}

impl From<MountError> for WindowError {
    fn from(error: MountError) -> Self {
        Self::Mount(error)
    }
}

impl From<CatalogError> for WindowError {
    fn from(error: CatalogError) -> Self {
        Self::Catalog(error)
    }
}

/// 目录键的稳定记录；activation 只随真正打开递增。
pub(crate) struct WindowRecord {
    pub(crate) activation: u32,
    pub(crate) owner: u32,
    pub(crate) open: bool,
    /// `visible` 只在 `open` 时有效；关闭同时清掉两者。
    pub(crate) visible: bool,
}
