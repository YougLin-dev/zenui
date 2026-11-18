// crates/core/src/runtime.rs
//! The runtime authority.
//!
//! A `Runtime` is the only place that mints a `RuntimeId`, and therefore the
//! only owner of the store, the reactive graph, the retained view tree, the
//! owner scopes, the query cache, the layout mapping and the frame transaction.
//!
//! `SPEC-RT-005`'s cross-runtime rejection compares a handle's `runtime_id`
//! with the *resolver's* identity, so resolution lives on the authority rather
//! than on the handle.

use std::fmt;

use crate::graph::MemoGraph;
use crate::handle::RuntimeId;
use crate::layout::LayoutMap;
use crate::owner::OwnerTree;
use crate::query::QueryCache;
use crate::store::Store;
use crate::txn::{Fatal, Nodes, Publication, Tx, Txn};
use crate::view::ViewTree;

/// Runtime identity plus the authorities it owns.
pub struct Runtime {
    id: RuntimeId,
    store: Store,
    graph: MemoGraph,
    views: ViewTree,
    owners: OwnerTree,
    query: QueryCache,
    layout: LayoutMap,
    txn: Txn,
    nodes: Nodes,
}

impl Runtime {
    /// Mint a fresh runtime identity and build the authorities around it.
    pub fn new() -> Self {
        let id = RuntimeId::allocate();
        let store = Store::with_runtime(id);
        let mut graph = MemoGraph::new();
        let nodes = Nodes::build(&mut graph);
        let owners = OwnerTree::new();
        let views = ViewTree::new(owners.root());
        let txn = Txn::new(store.next_key());

        Self {
            id,
            store,
            graph,
            views,
            owners,
            query: QueryCache::new(),
            layout: LayoutMap::new(),
            txn,
            nodes,
        }
    }

    pub const fn id(&self) -> RuntimeId {
        self.id
    }

    /// Committed model state. Never contains staged values.
    pub const fn committed(&self) -> &Store {
        &self.store
    }

    pub const fn txn(&self) -> &Txn {
        &self.txn
    }

    /// Mounted row views in display order (committed state).
    pub fn row_views(&self) -> Vec<(crate::handle::ViewId, crate::handle::RowKey)> {
        self.views.row_views()
    }

    pub const fn focused(&self) -> Option<crate::handle::ViewId> {
        self.views.focused()
    }

    pub fn caret(&self, view: crate::handle::ViewId) -> Option<usize> {
        self.views.caret(view)
    }

    /// Diagnostics for the trace. Monotonic counters may advance on a frame
    /// that is later discarded; authority state does not (`SPEC-RT-053`).
    pub fn counters(&self) -> crate::metrics::Counters {
        let mut total = self.store.counters();
        total += self.views.counters();
        total += self.query.counters();
        total.memo_recomputes = self.graph.counters().memo_recomputes;
        total.commits = self.txn.commits();
        total
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    pub fn active_source_count(&self) -> usize {
        self.graph.active_source_count()
    }

    pub const fn geometry_epoch(&self) -> u64 {
        self.views.geometry_epoch()
    }

    pub const fn live_owners(&self) -> usize {
        self.owners.live()
    }

    pub const fn cleanup_runs(&self) -> u64 {
        self.owners.cleanup_runs()
    }

    /// Next view id the frame will hand out (`SPEC-RT-053` high-water mark).
    pub const fn next_view(&self) -> u32 {
        self.views.next_id()
    }

    /// Next owner id the frame will hand out (`SPEC-RT-053` high-water mark).
    pub const fn next_owner(&self) -> u32 {
        self.owners.next_id()
    }

    /// The committed `ViewId -> LayoutNodeId` pairs, in view order.
    pub fn layout_pairs(&self) -> Vec<(crate::handle::ViewId, crate::handle::LayoutNodeId)> {
        self.layout.pairs()
    }

    /// The committed layout node assigned to a view.
    pub fn layout_of(&self, view: crate::handle::ViewId) -> Option<crate::handle::LayoutNodeId> {
        self.layout.get(view)
    }

    /// Live owner ids, in creation order. The *set* matters for rollback
    /// comparison (`SPEC-RT-053`), not just the count.
    pub fn live_owner_ids(&self) -> Vec<crate::handle::OwnerId> {
        self.owners.live_ids()
    }

    pub fn layout_len(&self) -> usize {
        self.layout.len()
    }

    /// Run one frame.
    ///
    /// The closure sees staged writes and may read its own uncommitted values
    /// (`SPEC-RT-050`). Publishing happens once, in `commit`, after the closure
    /// returns `Ok`. An `Err`, a `?`, or a panic discards the whole stage.
    pub fn frame<R>(
        &mut self,
        f: impl FnOnce(&mut Tx<'_>) -> Result<R, Fatal>,
    ) -> Result<R, Fatal> {
        let next_view = self.views.next_id();
        let next_owner = self.owners.next_id();
        self.txn.begin_frame(next_view, next_owner);
        let mut tx = self.tx();
        match f(&mut tx) {
            Ok(value) => {
                tx.commit()?;
                Ok(value)
            }
            Err(fatal) => {
                tx.abort();
                Err(fatal)
            }
        }
    }

    /// Test-only: open a transaction in a forced phase so the write guard can
    /// be exercised without reaching into the graph internals.
    #[cfg(test)]
    pub(crate) fn probe_tx_in_phase(&mut self, phase: crate::txn::Phase) -> Tx<'_> {
        self.txn.force_phase(phase);
        self.tx()
    }

    fn tx(&mut self) -> Tx<'_> {
        Tx::new(
            &mut self.txn,
            &mut self.store,
            &mut self.graph,
            &mut self.views,
            &mut self.owners,
            &mut self.query,
            &mut self.layout,
            self.nodes,
        )
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `RuntimeId` renders as `RuntimeId(..)`; the value never leaves here.
        f.debug_struct("Runtime")
            .field("id", &self.id)
            .field("store", &self.store)
            .field("views", &self.views)
            .field("txn", &self.txn)
            .finish()
    }
}

/// Convenience for fixtures that want to count publications.
pub fn publications(runtime: &Runtime) -> &[Publication] {
    runtime.txn.publications()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_runtime_mints_its_own_id() {
        let a = Runtime::new();
        let b = Runtime::new();
        assert_ne!(a.id(), b.id());
        assert_eq!(a.committed().runtime_id(), a.id());
    }

    #[test]
    fn a_frame_commits_once_and_publishes_once() {
        let mut runtime = Runtime::new();
        let before = runtime.txn().commits();

        runtime
            .frame(|tx| {
                tx.insert("A")?;
                Ok(())
            })
            .expect("frame");

        assert_eq!(runtime.txn().commits(), before + 1);
        assert_eq!(runtime.txn().publications().len(), 1);
        assert_eq!(runtime.committed().len(), 1);
    }

    /// `SPEC-GATE-A0-05`: writing from inside a compute stage is a domain
    /// error, and the runtime recovers on the next clean frame.
    #[test]
    fn a_write_during_compute_is_a_domain_error() {
        use crate::txn::{Phase, WriteKind};

        let mut runtime = Runtime::new();
        {
            let mut tx = runtime.probe_tx_in_phase(Phase::Topo);
            let error = tx.insert("A").expect_err("write during Topo must fail");
            assert!(matches!(
                error,
                Fatal::PhaseViolation(violation)
                    if violation.attempted == WriteKind::ModelWrite && violation.phase == Phase::Topo
            ));
        }

        // Recovery: the next frame starts Outside and commits.
        runtime
            .frame(|tx| {
                tx.insert("B")?;
                Ok(())
            })
            .expect("recovery frame");
        assert_eq!(runtime.committed().titles().collect::<Vec<_>>(), vec!["B"]);
    }

    /// `SPEC-RT-038`: a model write outside the write phases is a typed domain
    /// error, and the frame cannot publish afterwards.
    #[test]
    fn a_write_outside_the_write_phase_is_typed_and_never_commits() {
        use crate::txn::{Phase, ValueError};

        let mut runtime = Runtime::new();
        let row = runtime.frame(|tx| tx.insert("A")).expect("insert");
        let commits_before = runtime.txn().commits();

        let result = runtime.frame(|tx| {
            tx.force_phase(Phase::Binding);
            let error = tx.remove(row).expect_err("a Binding-phase write must fail");
            assert!(
                matches!(error, ValueError::Phase(violation) if violation.phase == Phase::Binding),
                "expected a typed phase error, got {error:?}"
            );
            Ok(())
        });

        assert!(
            result.is_err(),
            "a poisoned frame must not publish, got {result:?}"
        );
        assert_eq!(runtime.txn().commits(), commits_before);
        assert_eq!(runtime.committed().len(), 1, "the row survives");
    }

    #[test]
    fn a_failed_frame_leaves_the_committed_state_untouched() {
        let mut runtime = Runtime::new();
        runtime
            .frame(|tx| {
                tx.insert("A")?;
                Ok(())
            })
            .expect("frame");

        let rows_before = runtime.committed().len();
        let epoch_before = runtime.committed().epoch();

        let result: Result<(), Fatal> = runtime.frame(|tx| {
            tx.insert("B")?;
            Err(Fatal::PhaseOrder {
                expected: crate::txn::Phase::Mark,
                found: crate::txn::Phase::Outside,
            })
        });

        assert!(result.is_err());
        assert_eq!(runtime.committed().len(), rows_before);
        assert_eq!(runtime.committed().epoch(), epoch_before);
    }
}
