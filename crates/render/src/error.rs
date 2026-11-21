//! 场景构建错误。

use std::error::Error;
use std::fmt;

use zenui::NodeId;
use zenui_style::{Property, StyleValue};

/// 从 presentation snapshot 构建场景时的显式失败。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneError {
    /// viewport 不是有限的逻辑尺寸，或 scale 不是正数。
    InvalidViewport {
        /// 具体拒绝原因。
        reason: &'static str,
    },
    /// 快照中有渲染器没有内建视觉的元素。
    UnsupportedTag {
        /// 该元素的 Node ID。
        node: NodeId,
        /// 未知的 intrinsic 名称。
        tag: &'static str,
    },
    /// 快照中有场景 seam 不理解的属性；不会静默丢弃。
    UnsupportedAttribute {
        /// 携带该属性的元素。
        node: NodeId,
        /// 未知的属性名。
        name: &'static str,
    },
    /// 快照中有场景 seam 不理解的样式声明；不会静默丢弃。
    UnsupportedStyle {
        /// 携带该声明的元素。
        node: NodeId,
        /// 声明影响的属性。
        property: Property,
        /// 声明给出的取值。
        value: StyleValue,
    },
    /// leaf intrinsic（`checkbox` / `text-input`）带有子节点。
    UnsupportedContent {
        /// 携带子节点的元素。
        node: NodeId,
        /// 为什么这些内容不受支持。
        reason: &'static str,
    },
    /// 快照顶层直接包含裸文本；它没有 Node ID，不能成为场景元素。
    BareRootText,
    /// Taffy 无法完成布局。
    Layout {
        /// Taffy 给出的错误描述。
        message: String,
    },
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidViewport { reason } => {
                write!(f, "invalid render viewport: {reason}")
            }
            Self::UnsupportedTag { node, tag } => {
                write!(f, "{node:?} has unsupported tag {tag:?}")
            }
            Self::UnsupportedAttribute { node, name } => {
                write!(f, "{node:?} has unsupported attribute {name:?}")
            }
            Self::UnsupportedStyle {
                node,
                property,
                value,
            } => {
                write!(f, "{node:?} has unsupported style {property:?} = {value:?}")
            }
            Self::UnsupportedContent { node, reason } => {
                write!(f, "{node:?} has unsupported content: {reason}")
            }
            Self::BareRootText => {
                f.write_str("a top-level text node has no Node ID and cannot enter a scene")
            }
            Self::Layout { message } => write!(f, "scene layout failed: {message}"),
        }
    }
}

impl Error for SceneError {}
