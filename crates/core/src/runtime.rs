use std::fmt;

use crate::graph::MemoGraph;
use crate::handle::RuntimeId;
use crate::layout::LayoutMap;
use crate::owner::OwnerTree;
use crate::query::{Filter, QueryCache, RowSet};
use crate::store::Store;
use crate::txn::{Fatal, Nodes, Publication, Tx, Txn};
use crate::view::ViewTree;

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
    pub fn new() -> Self {
        let id = RuntimeId::allocate();
        let store = Store::with_runtime(id);
        let mut graph = MemoGraph::new();
        let nodes = Nodes::build(&mut graph);
        let owners = OwnerTree::new();
        let views = ViewTree::new(id, owners.root());
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

    pub const fn committed(&self) -> &Store {
        &self.store
    }

    pub const fn txn(&self) -> &Txn {
        &self.txn
    }

    pub fn row_views(&self) -> Vec<(crate::handle::ViewId, crate::handle::RowKey)> {
        self.views.row_views()
    }

    /// 只读查询共享已提交 Store 的行集缓存；没有事务时也可用。
    pub fn rows(&mut self, filter: Filter) -> RowSet {
        self.query.rows_at(&mut self.store, filter)
    }

    pub const fn focused(&self) -> Option<crate::handle::ViewId> {
        self.views.focused()
    }

    pub fn caret(&self, view: crate::handle::ViewId) -> Option<usize> {
        self.views
            .require(view)
            .ok()
            .and_then(|()| self.views.caret(view))
    }

    pub fn view_owner(&self, view: crate::handle::ViewId) -> Option<crate::handle::OwnerId> {
        self.views.owner(view)
    }

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

    /// Owner 节点存储量；释放后应回到存活 Owner 的规模。
    pub fn owner_storage_len(&self) -> usize {
        self.owners.storage_len()
    }

    pub const fn cleanup_runs(&self) -> u64 {
        self.owners.cleanup_runs()
    }

    pub const fn next_view(&self) -> u32 {
        self.views.next_id()
    }

    pub const fn next_owner(&self) -> u32 {
        self.owners.next_id()
    }

    pub fn layout_pairs(&self) -> Vec<(crate::handle::ViewId, crate::handle::LayoutNodeId)> {
        self.layout.pairs()
    }

    pub fn layout_of(&self, view: crate::handle::ViewId) -> Option<crate::handle::LayoutNodeId> {
        self.views
            .require(view)
            .ok()
            .and_then(|()| self.layout.get(view))
    }

    pub fn live_owner_ids(&self) -> Vec<crate::handle::OwnerId> {
        self.owners.live_ids()
    }

    pub fn layout_len(&self) -> usize {
        self.layout.len()
    }

    /// 一个事务只接收一组模型与视图修改；闭包返回 Err 时丢弃全部暂存状态。
    pub fn frame<R>(
        &mut self,
        f: impl FnOnce(&mut Tx<'_>) -> Result<R, Fatal>,
    ) -> Result<R, Fatal> {
        let next_view = self.views.next_id();
        let next_owner = self.owners.next_id();
        self.txn
            .begin_frame(&self.store, next_view, next_owner, self.layout.next_id());
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
        f.debug_struct("Runtime")
            .field("id", &self.id)
            .field("store", &self.store)
            .field("views", &self.views)
            .field("txn", &self.txn)
            .finish()
    }
}

/// 最近 [`crate::txn::PUBLICATION_HISTORY`] 次提交的发布快照；消费者不能假设历史无界。
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

    #[test]
    fn a_write_during_compute_is_a_domain_error() {
        use crate::txn::{Phase, PhaseViolation, ValueError, WriteKind};

        let mut runtime = Runtime::new();
        let row = runtime.frame(|tx| tx.insert("A")).expect("insert");
        let commits_before = runtime.txn().commits();

        let result = runtime.frame(|tx| {
            tx.force_phase(Phase::Topo);
            let error = tx
                .set_completed(row, true)
                .expect_err("a write during Topo must fail");
            assert!(
                matches!(
                    error,
                    ValueError::Phase(PhaseViolation {
                        phase: Phase::Topo,
                        ..
                    })
                ),
                "expected a typed phase error, got {error:?}"
            );

            assert!(matches!(
                tx.set_completed(row, false),
                Err(ValueError::Aborted)
            ));
            Ok(())
        });

        assert_eq!(
            result,
            Err(Fatal::PhaseViolation(PhaseViolation {
                phase: Phase::Topo,
                attempted: WriteKind::ModelWrite,
            })),
            "the frame must return the latched fatal"
        );
        assert_eq!(runtime.txn().commits(), commits_before, "nothing committed");
        assert!(!runtime.committed().rows().next().unwrap().completed());

        runtime
            .frame(|tx| {
                tx.insert("B")?;
                Ok(())
            })
            .expect("recovery frame");
        assert_eq!(
            runtime.committed().titles().collect::<Vec<_>>(),
            vec!["A", "B"]
        );
    }

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
                matches!(error, ValueError::Phase(_)),
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
    fn a_write_in_a_closed_phase_is_a_typed_error_not_a_panic() {
        use crate::txn::{Phase, ValueError};

        let mut runtime = Runtime::new();
        let row = runtime.frame(|tx| tx.insert("A")).expect("insert");

        let result = runtime.frame(|tx| {
            tx.force_phase(Phase::Commit);
            let error = tx
                .set_completed(row, true)
                .expect_err("a Commit-phase write must fail");
            assert!(
                matches!(error, ValueError::Phase(_)),
                "expected PhaseViolation, got {error:?}"
            );
            Ok(())
        });

        assert!(result.is_err(), "the frame must not publish");
        assert_eq!(runtime.committed().len(), 1);
        assert!(!runtime.committed().rows().next().unwrap().completed());
    }

    #[test]
    fn a_memo_cycle_is_fatal_and_recoverable() {
        let mut runtime = Runtime::new();
        let commits_before = runtime.txn().commits();

        let result = runtime.frame(|tx| {
            tx.insert("A")?;
            assert!(tx.probe_cycle(), "the probe must observe a cycle");
            Ok(())
        });

        assert!(
            matches!(result, Err(Fatal::GraphCycle { .. })),
            "expected GraphCycle, got {result:?}"
        );
        assert_eq!(runtime.txn().commits(), commits_before);
        assert_eq!(runtime.committed().len(), 0, "the stage was discarded");

        runtime
            .frame(|tx| {
                tx.insert("B")?;
                Ok(())
            })
            .expect("recovery frame");
        assert_eq!(runtime.committed().titles().collect::<Vec<_>>(), vec!["B"]);
    }

    #[test]
    fn owner_cleanup_runs_after_the_swap_in_the_cleanup_stage() {
        use crate::txn::Phase;

        let mut runtime = Runtime::new();
        runtime
            .frame(|tx| {
                tx.insert("A")?;
                tx.insert("B")?;
                Ok(())
            })
            .expect("seed");
        let keys: Vec<_> = runtime.committed().keys().collect();

        let views = runtime
            .frame(|tx| {
                let mut mounted = Vec::new();
                for key in &keys {
                    mounted.push(tx.mount_row(*key, 1)?);
                }
                tx.reorder_views(&mounted)?;
                Ok(mounted)
            })
            .expect("mount");

        runtime
            .frame(|tx| {
                tx.unmount(views[0])?;
                Ok(())
            })
            .expect("unmount");

        let log = runtime.txn().cleanup_log();
        assert_eq!(log.len(), 1, "exactly one owner was released");
        let (_, phase, visible_at_cleanup) = log[0];
        assert_eq!(phase, Phase::Cleanup, "cleanup must run in its own stage");
        assert_eq!(
            visible_at_cleanup, 1,
            "the surviving view is already published when cleanup runs"
        );
    }

    #[test]
    fn a_view_write_in_cleanup_is_rejected_and_the_next_frame_succeeds() {
        use crate::txn::{Phase, PhaseViolation};

        let mut runtime = Runtime::new();
        let row = runtime.frame(|tx| tx.insert("A")).expect("insert");
        let key = runtime.committed().keys().next().unwrap();
        let view = runtime
            .frame(|tx| {
                let view = tx.mount_row(key, 1)?;
                tx.reorder_views(&[view])?;
                Ok(view)
            })
            .expect("mount");

        let result = runtime.frame(|tx| {
            tx.force_phase(Phase::Cleanup);
            let error = tx.unmount(view).expect_err("cleanup cannot unmount");
            assert!(
                matches!(
                    error,
                    Fatal::PhaseViolation(PhaseViolation {
                        phase: Phase::Cleanup,
                        ..
                    })
                ),
                "expected a typed phase error, got {error:?}"
            );
            Ok(())
        });
        assert!(result.is_err(), "the poisoned frame must not publish");
        assert_eq!(runtime.row_views().len(), 1, "the view is still mounted");
        let _ = row;

        runtime
            .frame(|tx| {
                tx.unmount(view)?;
                Ok(())
            })
            .expect("next frame unmounts");
        assert_eq!(runtime.row_views().len(), 0);
    }

    #[test]
    fn exhausted_transaction_mount_reserves_no_identity_or_layout() {
        let mut runtime = Runtime::new();
        runtime
            .frame(|tx| {
                tx.insert("A")?;
                Ok(())
            })
            .expect("seed");
        let key = runtime.committed().keys().next().expect("seeded key");
        runtime.views.set_next(u32::MAX);
        let next_owner = runtime.next_owner();
        let next_layout = runtime.layout.next_id();

        let result = runtime.frame(|tx| tx.mount_row(key, 1));

        assert!(
            matches!(
                result,
                Err(Fatal::InvalidViewHandle(crate::handle::InvalidHandle {
                    reason: crate::handle::HandleError::Exhausted { .. },
                }))
            ),
            "allocation must stop before wrapping: {result:?}"
        );
        assert_eq!(runtime.next_view(), u32::MAX);
        assert_eq!(runtime.next_owner(), next_owner);
        assert_eq!(runtime.layout.next_id(), next_layout);
        assert!(runtime.row_views().is_empty());
        assert!(runtime.layout_pairs().is_empty());
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
