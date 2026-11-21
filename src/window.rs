use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

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
    pub fn request_hide(self) -> Result<(), WindowError> {
        self.request(WindowAction::Hide)
    }

    /// 请求关闭自身窗口；释放 activation 的树、资源与窗口任务。
    pub fn request_close(self) -> Result<(), WindowError> {
        self.request(WindowAction::Close)
    }

    fn request(self, action: WindowAction) -> Result<(), WindowError> {
        crate::context::with_state_of(self.runtime, |state| {
            state.queue_window_request(WindowRequest { from: self, action })
        })
    }
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

/// 声明期的窗口描述；登记身份、原生标题/尺寸与 primary 标记，不挂载界面。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowSpec {
    key: &'static str,
    title: &'static str,
    width: u32,
    height: u32,
    primary: bool,
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
}

/// 目录中的一个窗口：静态描述与每次打开都执行的 factory。
pub(crate) struct CatalogEntry {
    pub(crate) spec: WindowSpec,
    pub(crate) factory: WindowFactory,
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
}

impl WindowCatalog {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
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

    pub(crate) fn finish(
        self,
    ) -> Result<(BTreeMap<&'static str, CatalogEntry>, &'static str), CatalogError> {
        let mut primary = None;
        let mut entries = BTreeMap::new();
        for (spec, factory) in self.entries {
            if spec.primary && primary.replace(spec.key).is_some() {
                return Err(CatalogError::MultiplePrimaries);
            }
            entries.insert(spec.key, CatalogEntry { spec, factory });
        }
        let Some(primary) = primary else {
            return Err(CatalogError::MissingPrimary);
        };
        Ok((entries, primary))
    }
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
            Self::Mount(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for WindowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog(error) => Some(error),
            Self::Mount(error) => Some(error),
            _ => None,
        }
    }
}

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
