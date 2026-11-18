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
//! - Where the A0 implementation is narrower than a later gate, a `TODO(A0-xx)`
//!   comment names the gate that owns the gap. The fixtures that decide the A0
//!   gates live in `crates/core/tests/`; nothing in this crate claims a gate
//!   passes — `docs/spec/08-gates.md` is the status ledger.
//!
//! There is no proc macro, no windowing, no GPU and no platform backend here;
//! those are M1+ (`docs/product/roadmap.md`).
//!
//! Identity domains do not convert into each other (`SPEC-RT-002`), and the
//! type-level contracts are carried by `tests/ui/` under `ADR-0021`: trybuild
//! compiles each case and matches the diagnostic — error code, span and
//! message — against a checked-in `.stderr` snapshot.
//!
//! - `SPEC-GATE-A0-02`: a handle stays `Copy`, so it cannot carry the title
//!   (`E0204` on a `Copy` type owning a `String`); the authority has no owned
//!   `get` (`E0599`), because an owned title read is `Tx::read_once` returning
//!   `Result<String, InvalidHandle>`.
//! - `SPEC-GATE-A0-03`: `RowKey`, `ViewId` and `LayoutNodeId` do not accept
//!   each other in either direction (`E0308`, six cases).
//! - `ADR-0022` decision 5: the query cache is not constructible from outside
//!   this crate (`E0603`: the module is private, `E0624`: the constructor is).
//!
//! A `compile_fail` doctest cannot carry these: it only checks that the
//! snippet does not compile, so a typo keeps it green. See `ADR-0021`.

#![forbid(unsafe_code)]

mod arena;

pub mod graph;
pub mod handle;
mod layout;
pub mod metrics;
pub mod owner;
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
pub use owner::{OwnerError, OwnerTree};
pub use query::{Filter, RowSet};
pub use runtime::Runtime;
pub use store::{Row, RowId, Store, StoreError};
pub use txn::{Fatal, Nodes, Phase, PhaseViolation, Publication, Tx, Txn, ValueError, WriteKind};
pub use view::{FocusIntent, ViewKind, ViewNode, ViewTree};
