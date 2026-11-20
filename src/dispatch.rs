//! 按 Node ID 派发命令的公开错误类型。

use std::fmt;

use crate::presentation::NodeId;
use crate::view::MountError;

/// 一次按 Node ID 派发所执行的操作。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeOperation {
    Click,
    KeyDown,
    Change,
    Submit,
    Focus,
    Blur,
    CompositionStart,
    CompositionUpdate,
    CompositionCommit,
    CompositionCancel,
}

impl NodeOperation {
    /// 用于错误文本的稳定操作名。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::KeyDown => "keydown",
            Self::Change => "change",
            Self::Submit => "submit",
            Self::Focus => "focus",
            Self::Blur => "blur",
            Self::CompositionStart => "composition start",
            Self::CompositionUpdate => "composition update",
            Self::CompositionCommit => "composition commit",
            Self::CompositionCancel => "composition cancel",
        }
    }
}

/// 按 Node ID 派发失败的原因。
///
/// 派发只使用给出的身份：不会回退到 label、内部下标或相邻元素。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DispatchError {
    /// 本 Runtime 从未分配过该 Node ID。
    UnknownNode(NodeId),
    /// 该 Node ID 曾被分配，但对应元素已不在已提交界面中。
    StaleNode(NodeId),
    /// 元素仍然存在，但不接受该操作：缺少事件 handler、不是文本输入或不可聚焦。
    WrongKind {
        node: NodeId,
        operation: NodeOperation,
    },
    /// 操作与控件本地状态冲突，例如组合中的输入或尚未开始的组合。
    Rejected {
        node: NodeId,
        operation: NodeOperation,
        reason: &'static str,
    },
    /// handler 或随后的界面刷新失败。
    Command(MountError),
}

impl fmt::Display for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownNode(node) => write!(f, "unknown node {node:?}"),
            Self::StaleNode(node) => write!(f, "stale node {node:?}"),
            Self::WrongKind { node, operation } => {
                write!(f, "node {node:?} does not accept {}", operation.as_str())
            }
            Self::Rejected {
                node,
                operation,
                reason,
            } => write!(f, "node {node:?} rejected {}: {reason}", operation.as_str()),
            Self::Command(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for DispatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Command(error) => Some(error),
            _ => None,
        }
    }
}
