//! 目标 SDK 的最小 stage-one 实现：Copy 句柄、安装的执行上下文、
//! mount-once 组件、`view!` 数据位和 retain 的行键结构。
//!
//! 本 crate 只覆盖 headless Todo 所需的行为；原生窗口、平台 IME 适配与平台能力不在其中。

#![forbid(unsafe_code)]

extern crate self as zenui;

mod context;
pub mod dispatch;
pub mod presentation;
pub mod runtime;
pub mod signal;
pub mod store;
pub mod task;
mod text_edit;
pub mod view;
pub mod window;

pub use context::Phase;
pub use dispatch::{DispatchError, NodeOperation};
pub use presentation::{
    Appearance, NodeId, NodeStatus, PresentationSnapshot, SnapshotAttr, SnapshotElement,
    SnapshotNode, SnapshotValue, TextEditState,
};
pub use runtime::{FocusInfo, Runtime};
pub use signal::{Memo, Once, Signal, memo, once, signal};
pub use store::{Field, FieldError, Store, TodoId, Transaction, WriteError, WriteOp};
pub use task::{DeliveryReport, Task, TaskError, TaskScope, TaskStatus, TaskToken};
pub use view::{
    BlurEvent, Children, ChildrenFactory, CompositionEvent, CompositionPhase, FocusCause,
    FocusEvent, Key, KeyEvent, MountError, MountPlan, MountResult, SubmitEvent,
};
pub use window::{
    CatalogError, CloseError, ClosePolicy, CloseRequest, WindowCatalog, WindowCtx, WindowError,
    WindowInfo, WindowRole, WindowSpec,
};
pub use zenui_macro::{component, view};
pub use zenui_style as style;
pub use zenui_style::{
    Color, ColorRole, Condition, Length, LengthUnit, Opacity, Property, StyleDeclaration,
    StyleOrigin, StyleValue, em, px,
};

pub mod prelude {
    pub use crate::{
        Appearance, BlurEvent, CatalogError, Children, ChildrenFactory, CloseError, ClosePolicy,
        CloseRequest, Color, ColorRole, CompositionEvent, CompositionPhase, Condition,
        DeliveryReport, DispatchError, Field, FieldError, FocusCause, FocusEvent, FocusInfo, Key,
        KeyEvent, Length, LengthUnit, Memo, MountError, MountPlan, MountResult, NodeId,
        NodeOperation, NodeStatus, Once, Opacity, PresentationSnapshot, Property, Runtime, Signal,
        SnapshotAttr, SnapshotElement, SnapshotNode, SnapshotValue, Store, StyleDeclaration,
        StyleOrigin, StyleValue, SubmitEvent, Task, TaskError, TaskScope, TaskStatus, TaskToken,
        TextEditState, TodoId, Transaction, WindowCatalog, WindowCtx, WindowError, WindowInfo,
        WindowRole, WindowSpec, WriteError, WriteOp, component, em, memo, once, px, signal, view,
    };
    pub use zenui_core::Filter;
}
