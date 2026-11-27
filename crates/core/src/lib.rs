//! 无窗口的模型、响应式图、释放作用域、行集查询和事务原语。

#![forbid(unsafe_code)]

mod arena;

pub mod graph;
pub mod handle;
mod layout;
pub mod metrics;
pub mod model;
pub mod owner;
mod panic;
mod query;
pub mod runtime;
pub mod store;
pub mod txn;
pub mod view;

pub use graph::{DepReader, GraphError, MemoGraph, NodeId, Value};
pub use handle::{
    Generation, Handle, HandleError, InvalidHandle, LayoutNodeId, MemoId, OwnerId, QueryId, RowKey,
    RuntimeId, SignalId, SlotIndex, ViewId,
};
pub use metrics::Counters;
pub use model::RowSet;
pub use owner::{OwnerError, OwnerTree};
pub use panic::{PanicInfo, catch_panic};
pub use query::{Filter, RowSet as LegacyRowSet};
pub use runtime::Runtime;
pub use store::{Row, RowId, Store, StoreError};
pub use txn::{
    Fatal, Nodes, PUBLICATION_HISTORY, Phase, PhaseViolation, Publication, Tx, Txn, ValueError,
    WriteKind,
};
pub use view::{FocusIntent, ViewKind, ViewNode, ViewTree};
