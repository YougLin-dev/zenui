//! 目标 SDK 的最小 stage-one 实现：Copy 句柄、安装的执行上下文、
//! mount-once 组件、`view!` 数据位和 retain 的行键结构。
//!
//! 本 crate 只覆盖 headless Todo 所需的行为；原生窗口、平台 IME 适配与平台能力不在其中。

#![forbid(unsafe_code)]

extern crate self as zenui;

mod context;
pub mod dispatch;
pub mod file_dialog;
pub mod menu;
mod node;
#[cfg(test)]
mod oracle;
pub mod presentation;
pub mod runtime;
pub mod session;
pub mod signal;
pub mod store;
pub mod task;
mod text_edit;
pub mod tray;
pub mod view;
pub mod window;

pub use context::Phase;
pub use dispatch::{DispatchError, NodeOperation};
pub use file_dialog::{
    FileDialogError, FileDialogKind, FileDialogOptions, FileDialogOutcome, FileDialogRequest,
    FileDialogRequestError, FileFilter,
};
pub use menu::{
    CommandError, CommandEvent, CommandSource, MenuCommand, MenuCommandState, MenuItem,
    MenuShortcut, MenuSpec, MenuStateError, ShortcutError, ShortcutKey, Submenu,
};
pub use node::PresentationKey;
pub use presentation::{
    Appearance, NodeId, NodeStatus, PresentationDelta, PresentationSnapshot, PresentationUpdate,
    SnapshotAction, SnapshotActions, SnapshotAttr, SnapshotElement, SnapshotNode, SnapshotRowBand,
    SnapshotSemantics, SnapshotText, SnapshotValue, TextEditState,
};
pub use runtime::{
    EventSource, EventTarget, FocusInfo, Intent, IntentPayload, MemoryReport, Modifiers,
    NON_WINDOWED_FOR_WARN_ROWS, RefreshReport, RowWindow, Runtime, RuntimeDiagnostic,
    StorageReport, TurnError, TurnOutcome, TurnReport,
};
pub use session::{SessionCtx, SessionError, SessionPolicy};
pub use signal::{Memo, MemoCompute, Once, ReactiveValue, Signal, memo, on_cleanup, once, signal};
pub use store::{
    Field, FieldError, RowStoreField, Store, TodoId, TodoRow, Transaction, WriteError, WriteOp,
};
pub use task::{DeliveryReport, Task, TaskError, TaskScope, TaskStatus, TaskToken};
pub use tray::{TrayError, TraySpec};
pub use view::{
    BlurEvent, Children, ChildrenFactory, CompositionEvent, CompositionPhase, FocusCause,
    FocusEvent, For, ForSource, ForStore, HandlerError, HandlerOutcome, IntoForSource,
    IntoHandlerOutcome, Key, KeyEvent, MountError, MountPlan, MountResult, Read, ReadFault,
    ReadValue, RowView, SubmitEvent,
};
pub use window::{
    ActivationId, CatalogError, CloseError, CloseEvent, CloseOutcome, ClosePolicy, CloseRequest,
    CloseSource, DropError, DropEvent, WindowCatalog, WindowCtx, WindowError, WindowInfo,
    WindowRole, WindowSpec,
};
pub use zenui_core::PanicInfo;
pub use zenui_core::model;
pub use zenui_core::model::{
    FieldId, FilterId, Row, RowId, RowKey, RowSet, Store as RowStore, StoreId,
};
pub use zenui_macro::{component, view};
pub use zenui_style as style;
pub use zenui_style::{
    Color, ColorRole, Condition, Length, LengthUnit, Opacity, Property, StyleDeclaration,
    StyleOrigin, StyleValue, em, px,
};

pub mod prelude {
    pub use crate::{
        ActivationId, Appearance, BlurEvent, CatalogError, Children, ChildrenFactory, CloseError,
        CloseEvent, CloseOutcome, ClosePolicy, CloseRequest, CloseSource, Color, ColorRole,
        CommandError, CommandEvent, CommandSource, CompositionEvent, CompositionPhase, Condition,
        DeliveryReport, DispatchError, DropError, DropEvent, EventSource, EventTarget, Field,
        FieldError, FileDialogError, FileDialogKind, FileDialogOptions, FileDialogOutcome,
        FileDialogRequest, FileDialogRequestError, FileFilter, FocusCause, FocusEvent, FocusInfo,
        For, ForSource, ForStore, HandlerError, HandlerOutcome, Intent, IntentPayload,
        IntoForSource, IntoHandlerOutcome, Key, KeyEvent, Length, LengthUnit, Memo, MemoCompute,
        MemoryReport, MenuCommand, MenuCommandState, MenuItem, MenuShortcut, MenuSpec,
        MenuStateError, Modifiers, MountError, MountPlan, MountResult, NodeId, NodeOperation,
        NodeStatus, Once, Opacity, PanicInfo, PresentationDelta, PresentationKey,
        PresentationSnapshot, PresentationUpdate, Property, ReactiveValue, Read, ReadFault,
        ReadValue, RefreshReport, Row, RowId, RowKey, RowSet, RowStore, RowStoreField, RowView,
        RowWindow, Runtime, SessionCtx, SessionError, SessionPolicy, ShortcutError, ShortcutKey,
        Signal, SnapshotAction, SnapshotActions, SnapshotAttr, SnapshotElement, SnapshotNode,
        SnapshotRowBand, SnapshotSemantics, SnapshotText, SnapshotValue, StorageReport, Store,
        StyleDeclaration, StyleOrigin, StyleValue, Submenu, SubmitEvent, Task, TaskError,
        TaskScope, TaskStatus, TaskToken, TextEditState, TodoId, TodoRow, Transaction, TrayError,
        TraySpec, TurnError, TurnOutcome, TurnReport, WindowCatalog, WindowCtx, WindowError,
        WindowInfo, WindowRole, WindowSpec, WriteError, WriteOp, component, em, memo, on_cleanup,
        once, px, signal, view,
    };
    pub use zenui_core::Filter;
}
