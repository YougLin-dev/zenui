//! 应用会话的最后窗口策略与显式退出。
//!
//! 会话不属于某个 activation：窗口可以关闭、重新打开，模型与托盘仍由同一个
//! Runtime 持有。宿主按声明的策略决定最后一个窗口关闭后是否继续消息循环；
//! 显式退出由命令处理器捕获 [`SessionCtx`] 后排队，结算时关闭全部 activation。

use std::fmt;
use std::marker::PhantomData;

use crate::window::WindowError;

/// 最后一个窗口关闭后应用会话的行为。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionPolicy {
    /// 结束宿主会话；这是没有声明时的默认行为。
    ///
    /// Runtime 仍可由嵌入方重新打开窗口，但原生消息循环已经返回。
    #[default]
    QuitOnLastClose,
    /// 保持会话与托盘图标，允许宿主在零窗口时继续接收托盘和 worker 消息。
    ///
    /// 原生宿主要求同时声明托盘，否则零窗口时会话没有可达的 UI 或退出入口。
    StayResident,
}

/// 应用会话的 Copy handle；可以在命令处理器中捕获并排队会话动作。
///
/// 动作只接受 `Event` / `Delivery` 阶段，由
/// [`Runtime::apply_session_requests`](crate::Runtime::apply_session_requests)
/// 在窗口请求之后结算。`request_show` 只打开目录中已声明的窗口；显式退出
/// 关闭全部 activation，不运行逐窗口的关闭确认。
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SessionCtx {
    pub(crate) runtime: u32,
    _not_send: PhantomData<*const ()>,
}

impl SessionCtx {
    pub(crate) const fn new(runtime: u32) -> Self {
        Self {
            runtime,
            _not_send: PhantomData,
        }
    }

    /// 请求打开或聚焦目录窗口；窗口已关闭时重新执行 factory。
    pub fn request_show(self, key: &'static str) -> Result<(), SessionError> {
        self.request(SessionAction::Show(key))
    }

    /// 请求结束应用会话；结算后关闭全部窗口并让宿主消息循环返回。
    pub fn request_quit(self) -> Result<(), SessionError> {
        self.request(SessionAction::Quit)
    }

    fn request(self, action: SessionAction) -> Result<(), SessionError> {
        crate::context::with_state_of(self.runtime, |state| state.queue_session_request(action))
    }
}

/// Runtime 队列里的一个会话动作。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SessionAction {
    /// 打开或聚焦目录窗口。
    Show(&'static str),
    /// 结束会话并关闭全部窗口。
    Quit,
}

/// 会话声明、队列或结算失败的原因。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SessionError {
    /// 一个 Runtime 只能声明一次会话策略。
    AlreadyDeclared,
    /// 会话策略必须在首个窗口打开或根树挂载之前声明。
    RegistrationClosed,
    /// 当前生命周期状态不允许该会话动作。
    State(&'static str),
    /// 会话已显式结束，不再接受新的窗口或会话动作。
    Ended,
    /// 目录中没有这个窗口键。
    UnknownWindow(&'static str),
    /// 常驻会话的原生宿主需要声明托盘，才能在零窗口时继续接收消息。
    TrayRequired,
    /// 会话动作中的窗口操作失败。
    Window(WindowError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyDeclared => {
                f.write_str("the application already declares a session policy")
            }
            Self::RegistrationClosed => f.write_str(
                "the session policy must be declared before the first window or root mount",
            ),
            Self::State(message) => f.write_str(message),
            Self::Ended => f.write_str("the application session has ended"),
            Self::UnknownWindow(key) => write!(f, "window key {key} is not in the catalog"),
            Self::TrayRequired => {
                f.write_str("a resident application session requires a declared tray icon")
            }
            Self::Window(error) => write!(f, "{error}"),
        }
    }
}

impl fmt::Debug for SessionCtx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionCtx")
            .field("runtime", &self.runtime)
            .finish_non_exhaustive()
    }
}

impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Window(error) => Some(error),
            _ => None,
        }
    }
}

impl From<WindowError> for SessionError {
    fn from(error: WindowError) -> Self {
        Self::Window(error)
    }
}
