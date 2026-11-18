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
//!
//! Identity domains do not convert into each other (`SPEC-RT-002`):
//!
//! `SPEC-GATE-A0-02`: a handle stays `Copy`, so it cannot carry the title.
//! These two failures are the contract, not decoration — the first is the
//! language rule that forces `read_once` to return an owned `String`, and the
//! second is the absence of an owned `get` on the authority.
//!
//! ```compile_fail
//! // E0204: `Copy` cannot be implemented for a type that owns a `String`.
//! #[derive(Clone, Copy)]
//! struct CopyRow {
//!     title: String,
//! }
//! ```
//!
//! ```compile_fail
//! use zenui_core::{RowId, Runtime};
//!
//! fn owned_get(runtime: &Runtime, row: RowId) {
//!     let _ = runtime.get(row); // E0599: no owned `get`; use `read_once`
//! }
//! ```
//!
//! ```compile_fail
//! use zenui_core::{RowKey, ViewId};
//!
//! fn takes_row_key(_key: RowKey) {}
//!
//! fn pass_a_view(view: ViewId) {
//!     takes_row_key(view); // E0308: ViewId is not RowKey
//! }
//! ```
//!
//! ```compile_fail
//! use zenui_core::{LayoutNodeId, RowKey};
//!
//! fn takes_row_key(_key: RowKey) {}
//!
//! fn pass_a_layout_node(node: LayoutNodeId) {
//!     takes_row_key(node); // E0308: LayoutNodeId is not RowKey
//! }
//! ```
//!
//! The separation is mutual (`SPEC-GATE-A0-03`):
//!
//! ```compile_fail
//! use zenui_core::{RowKey, ViewId};
//!
//! fn takes_a_view(_view: ViewId) {}
//!
//! fn pass_a_key(key: RowKey) {
//!     takes_a_view(key); // E0308: RowKey is not ViewId
//! }
//! ```
//!
//! ```compile_fail
//! use zenui_core::{LayoutNodeId, RowKey};
//!
//! fn takes_a_layout_node(_node: LayoutNodeId) {}
//!
//! fn pass_a_key(key: RowKey) {
//!     takes_a_layout_node(key); // E0308: RowKey is not LayoutNodeId
//! }
//! ```
//!
//! ```compile_fail
//! use zenui_core::{LayoutNodeId, ViewId};
//!
//! fn takes_a_layout_node(_node: LayoutNodeId) {}
//!
//! fn pass_a_view(view: ViewId) {
//!     takes_a_layout_node(view); // E0308: ViewId is not LayoutNodeId
//! }
//! ```
//!
//! ```compile_fail
//! use zenui_core::{LayoutNodeId, ViewId};
//!
//! fn takes_a_view(_view: ViewId) {}
//!
//! fn pass_a_layout_node(node: LayoutNodeId) {
//!     takes_a_view(node); // E0308: LayoutNodeId is not ViewId
//! }
//! ```

#![forbid(unsafe_code)]

mod arena;

pub mod graph;
pub mod handle;
mod layout;
pub mod metrics;
pub mod owner;
pub mod query;
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
pub use owner::{OwnerError, OwnerTree};
pub use query::{Filter, QueryCache, RowSet};
pub use runtime::Runtime;
pub use store::{Row, RowId, Store, StoreError};
pub use txn::{Fatal, Nodes, Phase, PhaseViolation, Publication, Tx, Txn, ValueError, WriteKind};
pub use view::{FocusIntent, ViewKind, ViewNode, ViewTree};
