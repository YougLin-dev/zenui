use std::fmt;
use std::marker::PhantomData;

use crate::task::TaskScope;
use crate::view::MountError;

/// 一次窗口打开的 identity；它不等于目录键，也不保证窗口仍然存活。
#[derive(Clone, Copy)]
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

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WindowError {
    /// headless 边界不执行该能力；返回错误而不是空成功。
    Unsupported(&'static str),
    /// 该 activation 已关闭或被更新的一代取代。
    Stale,
    WrongRuntime,
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
            Self::Stale => f.write_str("the window activation is no longer live"),
            Self::WrongRuntime => {
                f.write_str("a WindowCtx from another ZenUI Runtime cannot access this one")
            }
            Self::Mount(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for WindowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
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

/// 目录键的稳定记录；activation 只随真正打开递增。
pub(crate) struct WindowRecord {
    pub(crate) activation: u32,
    pub(crate) owner: u32,
    pub(crate) open: bool,
}
