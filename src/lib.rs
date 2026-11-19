//! 目标 SDK 的最小 stage-one 实现：Copy 句柄、安装的执行上下文、
//! mount-once 组件、`view!` 数据位和 retain 的行键结构。
//!
//! 本 crate 只覆盖 headless Todo 所需的行为；原生窗口、IME 与平台能力不在其中。

#![forbid(unsafe_code)]

extern crate self as zenui;

mod context;
pub mod runtime;
pub mod signal;
pub mod store;
pub mod view;

pub use context::Phase;
pub use runtime::Runtime;
pub use signal::{Memo, Once, Signal, memo, once, signal};
pub use store::{Field, FieldError, Store, TodoId, Transaction, WriteError, WriteOp};
pub use view::{
    Children, ChildrenFactory, Key, KeyEvent, MountError, MountPlan, MountResult, SubmitEvent,
};
pub use zenui_macro::{component, view};

pub mod prelude {
    pub use crate::{
        Children, ChildrenFactory, Field, FieldError, Key, KeyEvent, Memo, MountError, MountPlan,
        MountResult, Once, Runtime, Signal, Store, SubmitEvent, TodoId, Transaction, WriteError,
        WriteOp, component, memo, once, signal, view,
    };
    pub use zenui_core::Filter;
}
