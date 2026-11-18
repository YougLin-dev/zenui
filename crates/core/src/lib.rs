// crates/core/src/lib.rs
//! Headless runtime primitives for ZenUI Gate A0.
//!
//! This crate exists to make `SPEC-GATE-A0-01` checkable: it holds the data
//! structures the gate names — generational identity, a Store of rows, an owner
//! scope tree, a minimal memo graph, query caching and a retained view tree —
//! plus the counters the headless trace reports.
//!
//! Boundaries:
//!
//! - **No third-party dependencies.** `[dependencies]` stays empty; the crate is
//!   pure `std`.
//! - **No `unsafe`.**
//! - Semantics that belong to `SPEC-GATE-A0-02`…`-11` are deliberately naive.
//!   Every such place carries a `TODO(A0-xx)` comment naming the gate that owns
//!   it, so P2/P3 can pick them up without guessing. Nothing in this crate
//!   claims those gates pass.
//!
//! There is no proc macro, no windowing, no GPU and no platform backend here;
//! those are M1+ (`docs/product/roadmap.md`).

#![forbid(unsafe_code)]

mod arena;
mod transaction;

pub mod graph;
pub mod handle;
pub mod metrics;
pub mod owner;
pub mod query;
pub mod runtime;
pub mod store;
pub mod view;

pub use graph::{DepReader, GraphError, MemoGraph, NodeId, Value};
pub use handle::{
    Generation, Handle, HandleError, InvalidHandle, LayoutNodeId, MemoId, OwnerId, QueryId, RowKey,
    RuntimeId, SignalId, SlotIndex, ViewId,
};
pub use metrics::Counters;
pub use owner::{OwnerError, OwnerTree};
pub use query::{Filter, QueryCache, RowSet};
pub use runtime::Runtime;
pub use store::{Row, RowId, Store, StoreError};
pub use view::{FocusIntent, ViewKind, ViewNode, ViewTree};
