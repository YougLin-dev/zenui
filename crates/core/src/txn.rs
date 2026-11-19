// crates/core/src/txn.rs
//! Frame transactions: value staging plus a single publish barrier.
//!
//! `SPEC-RT-033` conflates two different things unless they are separated:
//!
//! - reading your own uncommitted writes **inside** the frame is required
//!   (`SPEC-RT-050` runs Topo/Binding in-frame), and
//! - **publishing** a value to outside observers is allowed exactly once, at the
//!   outermost exit.
//!
//! `EVID-VAL-08` violated the second: closing an inner batch emitted a patch
//! carrying `b = true`, and the outer scope later changed it to `false`, so
//! observers saw two patches. The fix is a publish barrier, not a ban on
//! in-frame visibility.
//!
//! Model changes go to `ModelJournal`, memo results to `GraphScratch`, view
//! changes to a draft. Commit applies them with one infallible swap and
//! publishes once. Rollback is a drop, so equality with the previous commit is
//! structural rather than promised.
//!
//! The one deliberate exception is slot/generation reservation: a handle minted
//! in a frame must stay valid after commit and must never alias a later row
//! after rollback, so the slot is reserved eagerly and its generation is burned
//! on discard (`SPEC-RT-053`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::graph::{GraphScratch, MemoGraph, NodeId, Value};
use crate::handle::{
    Generation, Handle, InvalidHandle, OwnerId, RowKey, RuntimeId, SlotIndex, ViewId,
};
use crate::layout::LayoutMap;
use crate::owner::OwnerTree;
use crate::query::{Filter, QueryCache, RowSet, RowView, filter_index};
use crate::store::{Row, Store, StoreDraft, StoreError};
use crate::view::{FocusIntent, ViewTree};

/// The fixed stage order of one frame (`SPEC-RT-050`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Outside,
    Mark,
    Topo,
    Binding,
    Preflight,
    Commit,
    Cleanup,
    Derived,
}

/// What a write is, for the permission table of `SPEC-RT-038`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WriteKind {
    ModelWrite,
    SourceBump,
    ViewPatch,
    FocusChange,
    OwnerMount,
    OwnerDispose,
    LayoutMap,
    Compute,
    AuthorityApply,
}

impl Phase {
    /// `SPEC-RT-038`: only events and business commands may write.
    pub const fn permits(self, kind: WriteKind) -> bool {
        use Phase::*;
        use WriteKind::*;
        match kind {
            ModelWrite | SourceBump => matches!(self, Outside | Mark),
            ViewPatch | FocusChange | OwnerMount => matches!(self, Outside),
            OwnerDispose => matches!(self, Outside | Cleanup),
            LayoutMap => matches!(self, Outside | Preflight),
            Compute => matches!(self, Topo | Binding),
            AuthorityApply => matches!(self, Commit),
        }
    }
}

/// A write attempted in a stage that does not allow it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PhaseViolation {
    pub phase: Phase,
    pub attempted: WriteKind,
}

/// Fatal: the whole transaction is discarded (`SPEC-RT-053`).
///
/// There is deliberately no `From<ValueError>`, so `?` cannot silently upgrade
/// a value error into an abort (`SPEC-RT-057`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fatal {
    PhaseViolation(PhaseViolation),
    PhaseOrder {
        expected: Phase,
        found: Phase,
    },
    BatchNotAllowed {
        phase: Phase,
    },
    Unbalanced {
        depth: u32,
    },
    /// A staged field update targets a row another staged operation removed.
    StaleStagedRow {
        key: RowKey,
    },
    /// The staged order is not a permutation of the live rows.
    OrderNotAPermutation,
    /// The layout mapping would be left inconsistent.
    LayoutMapInconsistent,
    /// The staged view order is not a permutation of the views live at commit.
    ViewOrderNotAPermutation,
    /// Two views claim the same row key: `For` must be keyed
    /// (`SPEC-GATE-A0-09`).
    DuplicateViewKey {
        key: RowKey,
    },
    /// A mount targets a row the same transaction removed.
    MountForMissingRow {
        view: ViewId,
    },
    /// The staged focus target will not be live when the commit lands.
    FocusTargetNotLive {
        view: ViewId,
    },
    /// A memo body read a node that is already computing (`SPEC-RT-038`).
    GraphCycle {
        node: NodeId,
    },
    /// The caller chose to abort the frame.
    Aborted,
}

impl fmt::Display for Fatal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fatal::PhaseViolation(violation) => write!(
                f,
                "{:?} is not allowed during {:?}",
                violation.attempted, violation.phase
            ),
            Fatal::PhaseOrder { expected, found } => {
                write!(f, "expected phase {expected:?}, found {found:?}")
            }
            Fatal::BatchNotAllowed { phase } => write!(f, "batch not allowed during {phase:?}"),
            Fatal::Unbalanced { depth } => write!(f, "unbalanced transaction end at depth {depth}"),
            Fatal::StaleStagedRow { key } => {
                write!(f, "staged operation targets removed row {}", key.raw())
            }
            Fatal::OrderNotAPermutation => f.write_str("staged order is not a permutation"),
            Fatal::LayoutMapInconsistent => f.write_str("layout map would be inconsistent"),
            Fatal::ViewOrderNotAPermutation => {
                f.write_str("staged view order is not a permutation")
            }
            Fatal::DuplicateViewKey { .. } => f.write_str("two views claim the same row key"),
            Fatal::MountForMissingRow { .. } => f.write_str("mount targets a removed row"),
            Fatal::FocusTargetNotLive { .. } => f.write_str("focus target will not be live"),
            Fatal::GraphCycle { node } => write!(f, "memo cycle at node {node}"),
            Fatal::Aborted => f.write_str("transaction aborted by the caller"),
        }
    }
}

impl std::error::Error for Fatal {}

/// A pre-journal rejection (`ADR-0019` decision 1): the write never entered
/// the journal. Whether the frame commits depends on the variant — `Phase`
/// and `Aborted` poison it; `Handle` and `Store` do not.
///
/// `golden step 08` is exactly this shape — a stale handle is refused and the
/// committed state does not move.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ValueError {
    Handle(InvalidHandle),
    Store(StoreError),
    /// A write API call made in a phase that does not permit it
    /// (`ADR-0019` decision 3). The error is reported on the value channel
    /// and the transaction is poisoned, so the frame cannot publish.
    Phase(PhaseViolation),
    /// Any write call after the transaction was already poisoned
    /// (`ADR-0019` decision 6).
    Aborted,
}

impl From<InvalidHandle> for ValueError {
    fn from(error: InvalidHandle) -> Self {
        Self::Handle(error)
    }
}

impl ValueError {
    /// True when this error also poisoned the transaction.
    pub const fn is_poisoning(&self) -> bool {
        matches!(self, Self::Phase(_))
    }
}

impl From<StoreError> for ValueError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueError::Handle(error) => write!(f, "{error}"),
            ValueError::Store(error) => write!(f, "{error}"),
            ValueError::Phase(violation) => write!(
                f,
                "write {:?} is not allowed in phase {:?}",
                violation.attempted, violation.phase
            ),
            ValueError::Aborted => f.write_str("transaction already aborted"),
        }
    }
}

impl std::error::Error for ValueError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    Live,
    Committed,
    Discarded,
}

/// One published batch. `keys` are the rows the commit touched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Publication {
    pub keys: Vec<RowKey>,
    /// The committed value of each touched row at the moment of publishing.
    pub values: Vec<(RowKey, bool)>,
    pub commit_seq: u64,
}

#[derive(Clone, Copy, Debug)]
struct SlotReservation {
    slot: SlotIndex,
}

#[derive(Debug)]
struct StagedRow {
    runtime: RuntimeId,
    slot: SlotIndex,
    generation: Generation,
    inserted: bool,
    title: Option<String>,
    completed: Option<bool>,
}

#[derive(Debug, Default)]
struct ModelJournal {
    rows: BTreeMap<RowKey, StagedRow>,
    reserved: BTreeMap<RowKey, SlotReservation>,
    inserted: Vec<RowKey>,
    removals: BTreeSet<RowKey>,
    order: Option<Vec<RowKey>>,
    next_key: u64,
    writes: u64,
}

impl ModelJournal {
    fn new(next_key: u64) -> Self {
        Self {
            rows: BTreeMap::new(),
            reserved: BTreeMap::new(),
            inserted: Vec::new(),
            removals: BTreeSet::new(),
            order: None,
            next_key,
            writes: 0,
        }
    }
}

#[derive(Debug)]
struct PlannedMount {
    id: ViewId,
    row: RowKey,
    owner: OwnerId,
    title_len: u32,
}

#[derive(Debug, Default)]
struct ViewDraft {
    next_view: u32,
    next_owner: u32,
    mounts: Vec<PlannedMount>,
    owners: Vec<(OwnerId, OwnerId)>,
    unmounts: Vec<ViewId>,
    order: Option<Vec<ViewId>>,
    focus: Option<Option<ViewId>>,
    caret: Option<(ViewId, usize)>,
}

/// Test-only: why a focus intent was recorded (`SPEC-GATE-A0-10`).
///
/// The settlement is observable only to this module: `FocusIntent` stays a
/// two-neighbour struct, so the public shape does not grow an A0-only field.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FocusSettlementCause {
    Unmount,
    BulkClear,
}

/// Test-only: which fallback rule settled a recorded intent.
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FocusSettlementRule {
    Next,
    Previous,
    Cleared,
}

/// Transaction state owned by the runtime.
pub struct Txn {
    depth: u32,
    phase: Phase,
    outcome: Outcome,
    model: ModelJournal,
    graph: GraphScratch,
    view: ViewDraft,
    staged_rev: u64,
    /// Cache revisions this frame has reached: one per filter membership set
    /// and one for the display order.
    ///
    /// Seeded from the store at `begin_frame`, advanced only where a staged
    /// write really changes the set, and adopted verbatim by `Store::install`.
    /// One lineage for committed and staged state is what lets a frame's cache
    /// entry survive its own commit, and it is why there is no second place
    /// where a revision could be "reported".
    staged_membership: [u64; 3],
    staged_order: u64,
    publishes: Vec<Publication>,
    commits: u64,
    /// Focus intents recorded before a structural change, settled once at the
    /// end of the commit (`SPEC-RT-026`, `SPEC-GATE-A0-10`).
    focus_intents: Vec<FocusIntent>,
    /// Set when the frame moves focus explicitly; cancels pending intents.
    user_focused: bool,
    /// Test-only: what the Cleanup stage observed when it released an owner.
    #[cfg(test)]
    cleanup_log: Vec<(crate::handle::OwnerId, Phase, usize)>,
    /// Test-only: `(cause, rule, target)` for every settled focus intent.
    #[cfg(test)]
    focus_settlements: Vec<(FocusSettlementCause, FocusSettlementRule, Option<ViewId>)>,
    /// Test-only: the cause of each recorded intent, aligned with
    /// `focus_intents` (both are cleared together).
    #[cfg(test)]
    intent_causes: Vec<FocusSettlementCause>,
    /// Latched fatal error: set by a phase violation or a nested failure.
    /// `commit` returns it even when the frame body returned `Ok`
    /// (`ADR-0019` decision 6).
    poison: Option<Fatal>,
}

impl Txn {
    pub(crate) fn new(next_key: u64) -> Self {
        Self {
            depth: 0,
            phase: Phase::Outside,
            outcome: Outcome::Committed,
            model: ModelJournal::new(next_key),
            graph: GraphScratch::new(),
            view: ViewDraft::default(),
            staged_rev: 0,
            staged_membership: [0; 3],
            staged_order: 0,
            publishes: Vec::new(),
            commits: 0,
            poison: None,
            focus_intents: Vec::new(),
            user_focused: false,
            #[cfg(test)]
            cleanup_log: Vec::new(),
            #[cfg(test)]
            focus_settlements: Vec::new(),
            #[cfg(test)]
            intent_causes: Vec::new(),
        }
    }

    pub(crate) fn begin_frame(&mut self, store: &Store, next_view: u32, next_owner: u32) {
        self.depth = 1;
        self.phase = Phase::Outside;
        self.outcome = Outcome::Live;
        self.poison = None;
        // Revisions continue the committed lineage rather than restarting, so
        // the key a frame builds under is the key the commit installs.
        self.staged_membership = [
            store.epoch_for(Filter::All),
            store.epoch_for(Filter::Active),
            store.epoch_for(Filter::Completed),
        ];
        self.staged_order = store.order_epoch();
        self.view.next_view = next_view;
        self.view.next_owner = next_owner;
    }

    pub const fn phase(&self) -> Phase {
        self.phase
    }

    pub const fn commits(&self) -> u64 {
        self.commits
    }

    pub const fn staged_rev(&self) -> u64 {
        self.staged_rev
    }

    #[cfg(test)]
    pub(crate) fn cleanup_log(&self) -> &[(crate::handle::OwnerId, Phase, usize)] {
        &self.cleanup_log
    }

    /// Test-only: the settlement log `SPEC-GATE-A0-10` asks for.
    #[cfg(test)]
    pub(crate) fn focus_settlements(
        &self,
    ) -> &[(FocusSettlementCause, FocusSettlementRule, Option<ViewId>)] {
        &self.focus_settlements
    }

    pub fn publications(&self) -> &[Publication] {
        &self.publishes
    }

    pub const fn staged_write_count(&self) -> u64 {
        self.model.writes
    }

    fn reset(&mut self) {
        self.depth = 0;
        self.phase = Phase::Outside;
        self.outcome = Outcome::Committed;
        self.focus_intents.clear();
        #[cfg(test)]
        self.intent_causes.clear();
        self.user_focused = false;
        self.model = ModelJournal::new(self.model.next_key);
        self.graph = GraphScratch::new();
        self.view = ViewDraft::default();
    }

    /// Test-only: force a phase so the write guard can be exercised directly.
    #[cfg(test)]
    pub(crate) fn force_phase(&mut self, phase: Phase) {
        self.phase = phase;
    }
}

impl fmt::Debug for Txn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Txn")
            .field("depth", &self.depth)
            .field("phase", &self.phase)
            .field("staged_rev", &self.staged_rev)
            .field("commits", &self.commits)
            .finish()
    }
}

/// Graph nodes the runtime declares for the Todo-shaped model.
#[derive(Clone, Copy, Debug)]
pub struct Nodes {
    pub store: NodeId,
    pub remaining: NodeId,
    pub has_completed: NodeId,
}

impl Nodes {
    /// The runtime declares no memo for a row set: the query cache slot is that
    /// memo, keyed on the revisions a memo would otherwise have to track
    /// (`SPEC-GATE-A0-07`). A graph node on top of it would be a second
    /// invalidation authority for the same value — the exact shape of defect
    /// this gate is about.
    pub(crate) fn build(graph: &mut MemoGraph) -> Self {
        Self {
            store: graph.source("store"),
            remaining: graph.memo("remaining"),
            has_completed: graph.memo("has-completed"),
        }
    }
}

/// A borrow bundle over the authorities, valid for one frame.
pub struct Tx<'a> {
    txn: &'a mut Txn,
    store: &'a mut Store,
    graph: &'a mut MemoGraph,
    views: &'a mut ViewTree,
    owners: &'a mut OwnerTree,
    query: &'a mut QueryCache,
    layout: &'a mut LayoutMap,
    nodes: Nodes,
    level: u32,
}

impl<'a> Tx<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        txn: &'a mut Txn,
        store: &'a mut Store,
        graph: &'a mut MemoGraph,
        views: &'a mut ViewTree,
        owners: &'a mut OwnerTree,
        query: &'a mut QueryCache,
        layout: &'a mut LayoutMap,
        nodes: Nodes,
    ) -> Self {
        Self {
            txn,
            store,
            graph,
            views,
            owners,
            query,
            layout,
            nodes,
            level: 0,
        }
    }

    // ---- phase ----

    fn require(&mut self, kind: WriteKind) -> Result<(), Fatal> {
        if let Some(fatal) = self.txn.poison {
            return Err(fatal);
        }
        if self.txn.phase.permits(kind) {
            return Ok(());
        }
        let violation = PhaseViolation {
            phase: self.txn.phase,
            attempted: kind,
        };
        self.txn.poison = Some(Fatal::PhaseViolation(violation));
        Err(Fatal::PhaseViolation(violation))
    }

    /// Phase guard for a stager whose error channel is `ValueError`.
    ///
    /// A phase violation is a domain error that poisons the transaction: the
    /// caller sees a typed error at the call site, and `commit` refuses to
    /// publish (`SPEC-RT-038`, `ADR-0017` decision 7).
    /// Test-only: force the phase a frame runs in.
    #[cfg(test)]
    pub(crate) fn force_phase(&mut self, phase: Phase) {
        self.txn.force_phase(phase);
    }

    /// Test-only: run a staged compute whose body reads its own node, which is
    /// exactly the cycle a recursive memo would produce.
    #[cfg(test)]
    pub(crate) fn probe_cycle(&mut self) -> bool {
        let node = self.nodes.store;
        let revision = self.txn.staged_rev;
        self.txn.graph.mark_dirty(node);
        let previous = self.txn.phase;
        self.txn.phase = Phase::Binding;
        let outcome = {
            let txn = &mut *self.txn;
            self.graph
                .compute_staged(node, &mut txn.graph, revision, |reader| {
                    let _ = reader.read(node);
                    Value::Count(0)
                })
        };
        self.txn.phase = previous;
        self.note_graph_error();
        matches!(outcome, Err(crate::graph::GraphError::Cycle { .. }))
    }

    fn require_staged(&mut self, kind: WriteKind) -> Result<(), ValueError> {
        if self.txn.poison.is_some() {
            return Err(ValueError::Aborted);
        }
        if self.txn.phase.permits(kind) {
            return Ok(());
        }
        let violation = PhaseViolation {
            phase: self.txn.phase,
            attempted: kind,
        };
        self.txn.poison = Some(Fatal::PhaseViolation(violation));
        Err(ValueError::Phase(violation))
    }

    /// A cycle found during a staged compute is fatal: latch it so the
    /// outermost exit returns `Fatal::GraphCycle` and discards the stage.
    ///
    /// Only the cycle probe reaches this. The runtime's own memo bodies read
    /// the `store` source node and cannot re-enter, so no production path can
    /// produce a cycle today; the latch stays because the probe is what pins
    /// that the failure would be fatal (`SPEC-GATE-A0-05`).
    #[cfg(test)]
    fn note_graph_error(&mut self) {
        if let Some(node) = self.txn.graph.cycle() {
            self.txn.poison = Some(Fatal::GraphCycle { node });
        }
    }

    fn bump(&mut self) {
        self.txn.staged_rev = self.txn.staged_rev.saturating_add(1);
        self.graph
            .bump_source_staged(&mut self.txn.graph, self.nodes.store);
    }

    /// Advance the membership revisions a staged write really moves.
    ///
    /// Called at the staging site that knows the answer — never by a caller
    /// claiming work it did not do. `Store::install` adopts the final values,
    /// so the committed lineage and the staged one are the same numbers.
    fn note_membership(&mut self, filters: &[Filter]) {
        for filter in filters {
            self.txn.staged_membership[filter_index(*filter)] += 1;
        }
    }

    /// Advance the display-order revision.
    fn note_order(&mut self) {
        self.txn.staged_order += 1;
    }

    // ---- model writes ----

    pub fn insert(&mut self, title: impl Into<String>) -> Result<Handle<Row>, Fatal> {
        self.require(WriteKind::ModelWrite)?;
        let key = RowKey::from_raw(self.txn.model.next_key);
        self.txn.model.next_key += 1;
        let (slot, generation) = self.store.reserve_slot();
        self.txn
            .model
            .reserved
            .insert(key, SlotReservation { slot });
        self.txn.model.rows.insert(
            key,
            StagedRow {
                runtime: self.store.runtime_id(),
                slot,
                generation,
                inserted: true,
                title: Some(title.into()),
                completed: Some(false),
            },
        );
        self.txn.model.inserted.push(key);
        self.txn.model.writes += 1;
        self.bump();
        // Insert/remove/clear move every membership revision, whichever set
        // the row is actually in: the reverse edge is collapsed per filter, and
        // over-invalidation is redundant but harmless (`ADR-0022` decision 3).
        self.note_membership(&[Filter::All, Filter::Active, Filter::Completed]);
        self.note_order();
        Ok(Handle::new(self.store.runtime_id(), slot, generation))
    }

    pub fn toggle(&mut self, row: Handle<Row>) -> Result<bool, ValueError> {
        let next = !self.effective_completed(row)?;
        self.stage_completed(row, next)?;
        Ok(next)
    }

    pub fn set_completed(&mut self, row: Handle<Row>, value: bool) -> Result<(), ValueError> {
        self.stage_completed(row, value)
    }

    pub fn remove(&mut self, row: Handle<Row>) -> Result<RowKey, ValueError> {
        self.require_staged(WriteKind::ModelWrite)?;
        let key = self.effective_key(row)?;
        if self.txn.model.removals.contains(&key) {
            // Already staged for removal in this frame: no new change, and the
            // effective flag is no longer readable (the removal wins).
            return Ok(key);
        }
        self.txn.model.removals.insert(key);
        self.txn.model.writes += 1;
        self.bump();
        self.note_membership(&[Filter::All, Filter::Active, Filter::Completed]);
        self.note_order();
        Ok(key)
    }

    pub fn clear_completed(&mut self) -> Result<Vec<RowKey>, Fatal> {
        self.require(WriteKind::ModelWrite)?;
        let mut removed = Vec::new();
        for (key, completed) in self.effective_rows() {
            if completed && self.txn.model.removals.insert(key) {
                removed.push(key);
            }
        }
        if !removed.is_empty() {
            // A bulk clear settles one intent, not one per removed row.
            if self.txn.focus_intents.is_empty()
                && let Some(focused) = self.views.focused()
                && self
                    .views
                    .node(focused)
                    .and_then(|node| node.row)
                    .is_some_and(|row| removed.contains(&row))
                && let Some(intent) = self.views.focus_intent_for(focused)
            {
                self.txn.focus_intents.push(intent);
                #[cfg(test)]
                self.txn.intent_causes.push(FocusSettlementCause::BulkClear);
            }
            self.txn.model.writes += 1;
            self.bump();
            self.note_membership(&[Filter::All, Filter::Active, Filter::Completed]);
            self.note_order();
        }
        Ok(removed)
    }

    pub fn reorder(&mut self, keys: &[RowKey]) -> Result<(), ValueError> {
        self.require_staged(WriteKind::ModelWrite)?;
        let mut seen = BTreeSet::new();
        let rows = self.effective_rows();
        let live: BTreeSet<RowKey> = rows.iter().map(|(key, _)| *key).collect();
        for key in keys {
            if !seen.insert(*key) {
                return Err(StoreError::DuplicateKey(*key).into());
            }
            if !live.contains(key) {
                return Err(StoreError::UnknownRow(*key).into());
            }
        }
        if seen != live {
            return Err(StoreError::IncompleteOrder.into());
        }
        let current: Vec<RowKey> = rows.into_iter().map(|(key, _)| key).collect();
        if current != keys {
            self.txn.model.order = Some(keys.to_vec());
            self.txn.model.writes += 1;
            self.bump();
            // Membership is untouched: only the display order moves.
            self.note_order();
        }
        Ok(())
    }

    fn stage_completed(&mut self, row: Handle<Row>, value: bool) -> Result<(), ValueError> {
        self.require_staged(WriteKind::ModelWrite)?;
        let key = self.effective_key(row)?;
        // Compare against the *effective* value, not just the staged one: a
        // no-op write must not advance `io_writes` or the epoch
        // (`metrics::Counters::io_writes`).
        if self.effective_completed(row)? == value {
            return Ok(());
        }
        let entry = self.txn.model.rows.entry(key).or_insert(StagedRow {
            runtime: self.store.runtime_id(),
            slot: row.slot(),
            generation: row.generation(),
            inserted: false,
            title: None,
            completed: None,
        });
        if entry.completed != Some(value) {
            entry.completed = Some(value);
            self.txn.model.writes += 1;
            self.bump();
            // The flag moves the row between `Active` and `Completed`; `All`
            // membership and the display order are untouched (`SPEC-GATE-A0-07`
            // reverse edge is `(row, completed)`).
            self.note_membership(&[Filter::Active, Filter::Completed]);
        }
        Ok(())
    }

    // ---- model reads (overlay first, committed second) ----

    pub fn with_title<R>(
        &self,
        row: Handle<Row>,
        f: impl FnOnce(&str) -> R,
    ) -> Result<R, ValueError> {
        if row.runtime() != self.store.runtime_id() {
            return Err(ValueError::Handle(InvalidHandle::from(
                crate::handle::HandleError::ForeignRuntime {
                    expected: self.store.runtime_id(),
                    found: row.runtime(),
                },
            )));
        }
        if self.removed_key(row).is_some() {
            return Err(ValueError::Handle(InvalidHandle::from(
                crate::handle::HandleError::Vacant { slot: row.slot() },
            )));
        }
        if let Some(staged) = self.staged_by_handle(row)
            && let Some(title) = &staged.title
        {
            return Ok(f(title));
        }
        self.store.with_title(row, f).map_err(ValueError::from)
    }

    /// Read the effective `completed` flag, staged value first.
    pub fn completed(&self, row: Handle<Row>) -> Result<bool, ValueError> {
        self.effective_completed(row)
    }

    /// Owned title read (`SPEC-RT-006`, `ADR-0017` decision 3).
    ///
    /// The receiver is the authority, the failure type is `InvalidHandle`, and
    /// a failed read does not count as I/O. Cancelling the mount is the mount
    /// executor's job (`SPEC-RT-013`), not this method's return semantics.
    pub fn read_once(&mut self, row: Handle<Row>) -> Result<String, InvalidHandle> {
        if row.runtime() != self.store.runtime_id() {
            return Err(InvalidHandle::from(
                crate::handle::HandleError::ForeignRuntime {
                    expected: self.store.runtime_id(),
                    found: row.runtime(),
                },
            ));
        }
        if self.removed_key(row).is_some() {
            return Err(InvalidHandle::from(crate::handle::HandleError::Vacant {
                slot: row.slot(),
            }));
        }
        let staged_title = self
            .staged_by_handle(row)
            .and_then(|staged| staged.title.clone());
        if let Some(title) = staged_title {
            self.store.note_read();
            return Ok(title);
        }
        let title = self.store.with_title(row, str::to_owned)?;
        self.store.note_read();
        Ok(title)
    }

    /// Rows matching `filter`, honoring this frame's staged writes
    /// (`SPEC-RT-050`: read your own writes).
    ///
    /// Goes through the query cache, keyed on
    /// `(membership revision[filter], order revision)`. Both revisions are the
    /// counters the commit installs, so a snapshot built here is still valid —
    /// and still shared — after the frame lands.
    pub fn rows(&mut self, filter: Filter) -> RowSet {
        let mut view = StagedView::new(self.store, self.txn);
        self.query.rows_at(&mut view, filter)
    }

    /// Rows still active, from the membership counters.
    ///
    /// Deliberately not `effective_rows().iter().filter(..).count()`: that
    /// scans and allocates the whole model on *every* call, including the calls
    /// where the memo below answers from cache, and no counter reports it. Work
    /// the counters cannot see is the defect this gate is about, so the number
    /// comes from the same `live_counts` the query layer uses (`SPEC-RT-044`).
    pub fn remaining(&mut self) -> u32 {
        let (live, completed) = StagedView::new(self.store, self.txn).live_counts();
        let remaining = u32::try_from((live - completed).max(0)).unwrap_or(u32::MAX);
        let node = self.nodes.remaining;
        let store_node = self.nodes.store;
        let revision = self.txn.staged_rev;

        let previous = self.txn.phase;
        self.txn.phase = Phase::Binding;
        let outcome = {
            let txn = &mut *self.txn;
            self.graph
                .compute_staged(node, &mut txn.graph, revision, |reader| {
                    reader.read(store_node);
                    Value::Count(remaining)
                })
        };
        self.txn.phase = previous;

        match outcome {
            Ok((Value::Count(value), _)) => value,
            _ => 0,
        }
    }

    /// True when any live row is completed, from the membership counters — the
    /// same rule as [`Tx::remaining`].
    pub fn has_completed(&mut self) -> bool {
        let (_, completed) = StagedView::new(self.store, self.txn).live_counts();
        let flag = completed > 0;
        let node = self.nodes.has_completed;
        let store_node = self.nodes.store;
        let revision = self.txn.staged_rev;

        let previous = self.txn.phase;
        self.txn.phase = Phase::Binding;
        let outcome = {
            let txn = &mut *self.txn;
            self.graph
                .compute_staged(node, &mut txn.graph, revision, |reader| {
                    reader.read(store_node);
                    Value::Flag(flag)
                })
        };
        self.txn.phase = previous;

        match outcome {
            Ok((Value::Flag(value), _)) => value,
            _ => false,
        }
    }

    // ---- view reads ----

    pub fn root_owner(&self) -> OwnerId {
        self.owners.root()
    }

    /// Mounted row views in display order, as this frame sees them: the
    /// committed tree with this frame's unmounts removed and its staged mounts
    /// included (`ADR-0017` decision 4 — read your own writes applies to the
    /// view tree too).
    pub fn row_views(&self) -> Vec<(ViewId, RowKey)> {
        let mut rows: Vec<(ViewId, RowKey)> = self
            .views
            .row_views()
            .into_iter()
            .filter(|(view, _)| !self.txn.view.unmounts.contains(view))
            .collect();
        for mount in &self.txn.view.mounts {
            if !rows.iter().any(|(view, _)| *view == mount.id) {
                rows.push((mount.id, mount.row));
            }
        }
        if let Some(order) = &self.txn.view.order {
            let mut ordered = Vec::with_capacity(rows.len());
            for view in order {
                if let Some(entry) = rows.iter().find(|(candidate, _)| candidate == view) {
                    ordered.push(*entry);
                }
            }
            for entry in &rows {
                if !ordered.iter().any(|(view, _)| view == &entry.0) {
                    ordered.push(*entry);
                }
            }
            return ordered;
        }
        rows
    }

    pub fn view_of_row(&self, row: RowKey) -> Option<ViewId> {
        self.views.view_of_row(row)
    }

    /// Live for the purposes of this frame: mounted in the committed tree and
    /// not already staged for unmount.
    pub fn is_live(&self, view: ViewId) -> bool {
        self.views.is_live(view) && !self.txn.view.unmounts.contains(&view)
    }

    /// Resolve a stable key to a committed handle (staged inserts included).
    pub fn handle_for(&self, key: RowKey) -> Option<Handle<Row>> {
        if let Some(staged) = self.txn.model.rows.get(&key)
            && staged.inserted
            && !self.txn.model.removals.contains(&key)
        {
            return Some(Handle::new(staged.runtime, staged.slot, staged.generation));
        }
        if self.txn.model.removals.contains(&key) {
            return None;
        }
        self.store.handle_for(key)
    }

    pub fn focus_intent_for(&self, view: ViewId) -> Option<FocusIntent> {
        self.views.focus_intent_for(view)
    }

    pub fn focused(&self) -> Option<ViewId> {
        self.txn.view.focus.unwrap_or_else(|| self.views.focused())
    }

    pub fn caret(&self, view: ViewId) -> Option<usize> {
        self.txn
            .view
            .caret
            .filter(|(planned, _)| *planned == view)
            .map(|(_, offset)| offset)
            .or_else(|| self.views.caret(view))
    }

    // ---- view writes ----

    pub fn mount_row(&mut self, row: RowKey, title_len: u32) -> Result<ViewId, Fatal> {
        self.require(WriteKind::OwnerMount)?;
        // `For` is keyed: two live views for one row would alias its identity,
        // so the duplicate is refused before any patch is staged
        // (`SPEC-RT-024`, `SPEC-GATE-A0-09`).
        //
        // The scan asks the node table, not `row_views()`: a view mounted
        // without a staged reorder is live but absent from the display order,
        // and an order-based scan would let a second view claim its key.
        let already_mounted = self
            .views
            .claiming_view(row)
            .is_some_and(|view| !self.txn.view.unmounts.contains(&view));
        if already_mounted || self.txn.view.mounts.iter().any(|mount| mount.row == row) {
            return Err(Fatal::DuplicateViewKey { key: row });
        }
        let view_id = ViewId::from_index(self.txn.view.next_view);
        let owner_id = OwnerId::from_index(self.txn.view.next_owner);
        let parent = self.owners.root();

        let draft = &mut self.txn.view;
        draft.next_view += 1;
        draft.next_owner += 1;
        draft.owners.push((owner_id, parent));
        draft.mounts.push(PlannedMount {
            id: view_id,
            row,
            owner: owner_id,
            title_len,
        });
        self.layout.plan(view_id);
        Ok(view_id)
    }

    /// Stage an unmount. The focus intent is recorded here — *before* the
    /// structure changes — and settled once at the end of the commit
    /// (`SPEC-GATE-A0-10`).
    pub fn unmount(&mut self, view: ViewId) -> Result<(), Fatal> {
        self.require(WriteKind::ViewPatch)?;
        if !self.txn.view.unmounts.contains(&view) {
            self.txn.view.unmounts.push(view);
            if self.views.focused() == Some(view)
                && self.txn.focus_intents.is_empty()
                && let Some(intent) = self.views.focus_intent_for(view)
            {
                self.txn.focus_intents.push(intent);
                #[cfg(test)]
                self.txn.intent_causes.push(FocusSettlementCause::Unmount);
            }
        }
        Ok(())
    }

    pub fn reorder_views(&mut self, order: &[ViewId]) -> Result<(), Fatal> {
        self.require(WriteKind::ViewPatch)?;
        self.txn.view.order = Some(order.to_vec());
        Ok(())
    }

    pub fn focus(&mut self, view: ViewId) -> Result<(), Fatal> {
        self.require(WriteKind::FocusChange)?;
        // A user focus move mid-frame cancels any recorded removal intent.
        self.txn.user_focused = true;
        let caret = self
            .views
            .node(view)
            .map(|node| node.title_len as usize)
            .or_else(|| {
                self.txn
                    .view
                    .mounts
                    .iter()
                    .find(|mount| mount.id == view)
                    .map(|mount| mount.title_len as usize)
            })
            .unwrap_or(0);
        self.txn.view.focus = Some(Some(view));
        self.txn.view.caret = Some((view, caret));
        Ok(())
    }

    pub fn clear_focus(&mut self) -> Result<(), Fatal> {
        self.require(WriteKind::FocusChange)?;
        self.txn.user_focused = true;
        self.txn.view.focus = Some(None);
        Ok(())
    }

    // ---- commit ----

    /// Apply the staged state with one swap and publish exactly once.
    pub fn commit(mut self) -> Result<(), Fatal> {
        if self.txn.depth != 1 {
            return Err(Fatal::Unbalanced {
                depth: self.txn.depth,
            });
        }
        if let Some(fatal) = self.txn.poison {
            // A phase violation or a nested failure left the transaction
            // poisoned. The frame body may still have returned `Ok`; the
            // latched fatal is what the outermost exit returns, and it wins
            // over the phase-order check (`ADR-0019` decision 6).
            self.discard();
            return Err(fatal);
        }
        if self.txn.phase != Phase::Outside && self.txn.phase != Phase::Mark {
            return Err(Fatal::PhaseOrder {
                expected: Phase::Mark,
                found: self.txn.phase,
            });
        }

        if let Err(fatal) = self.preflight() {
            self.discard();
            return Err(fatal);
        }

        // Capture the touched keys before `apply_model` consumes the journal.
        let keys = self.touched_keys();

        // Mark the transaction settled *before* the swap. From here on the
        // frame has succeeded; if `apply_*` panics, `Drop` must not run
        // `discard` against a half-applied store (`ADR-0017` decision 9).
        self.txn.outcome = Outcome::Committed;
        self.txn.phase = Phase::Commit;
        self.apply_model();
        self.apply_views();
        let revision = self.txn.staged_rev;
        let scratch = std::mem::take(&mut self.txn.graph);
        self.graph.commit_scratch(scratch, revision);

        let values = self.effective_rows();
        self.txn.publishes.push(Publication {
            keys,
            values,
            commit_seq: self.txn.commits,
        });
        self.txn.commits += 1;
        self.txn.phase = Phase::Outside;
        self.txn.reset();
        self.txn.outcome = Outcome::Committed;
        Ok(())
    }

    /// Everything that can fail happens here, before the swap.
    fn preflight(&mut self) -> Result<(), Fatal> {
        self.txn.phase = Phase::Preflight;

        for key in &self.txn.model.removals {
            if let Some(staged) = self.txn.model.rows.get(key)
                && !staged.inserted
                && (staged.completed.is_some() || staged.title.is_some())
            {
                return Err(Fatal::StaleStagedRow { key: *key });
            }
        }

        if let Some(order) = &self.txn.model.order {
            let live: BTreeSet<RowKey> = self
                .effective_rows()
                .into_iter()
                .map(|(key, _)| key)
                .collect();
            let inserted: BTreeSet<RowKey> = self
                .txn
                .model
                .inserted
                .iter()
                .filter(|key| !self.txn.model.removals.contains(*key))
                .copied()
                .collect();
            let listed: BTreeSet<RowKey> = order.iter().copied().collect();
            let missing: BTreeSet<RowKey> = live.difference(&listed).copied().collect();
            // A staged order must name every live row except the ones this
            // frame inserts: those are appended in insertion order. Anything
            // else — a dropped row, a duplicate, a ghost — is rejected before
            // the swap (`SPEC-RT-052`).
            if listed.len() != order.len()
                || !listed.is_subset(&live)
                || !missing.is_subset(&inserted)
            {
                return Err(Fatal::OrderNotAPermutation);
            }
        }

        let live_rows: BTreeSet<RowKey> = self
            .effective_rows()
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        let mut seen_keys: BTreeSet<RowKey> = BTreeSet::new();
        for mount in &self.txn.view.mounts {
            if !live_rows.contains(&mount.row) {
                return Err(Fatal::MountForMissingRow { view: mount.id });
            }
            if !seen_keys.insert(mount.row) {
                return Err(Fatal::DuplicateViewKey { key: mount.row });
            }
            // `seen_keys` only covers this frame's draft. A committed view the
            // frame does not unmount also claims the key — and it may be absent
            // from the display order, so this asks the node table
            // (`SPEC-RT-024`: the key preflight is complete, not
            // order-dependent).
            if let Some(existing) = self.views.claiming_view(mount.row)
                && !self.txn.view.unmounts.contains(&existing)
            {
                return Err(Fatal::DuplicateViewKey { key: mount.row });
            }
        }

        let mut live_views: BTreeSet<ViewId> = self
            .views
            .row_views()
            .into_iter()
            .map(|(view, _)| view)
            .collect();
        for view in &self.txn.view.unmounts {
            live_views.remove(view);
        }
        for mount in &self.txn.view.mounts {
            live_views.insert(mount.id);
        }
        if let Some(order) = &self.txn.view.order {
            let listed: BTreeSet<ViewId> = order.iter().copied().collect();
            if listed != live_views || listed.len() != order.len() {
                return Err(Fatal::ViewOrderNotAPermutation);
            }
        }
        if let Some(Some(view)) = self.txn.view.focus
            && !live_views.contains(&view)
        {
            return Err(Fatal::FocusTargetNotLive { view });
        }

        let mounts: Vec<ViewId> = self.txn.view.mounts.iter().map(|m| m.id).collect();
        self.layout
            .preflight(&mounts, &self.txn.view.unmounts)
            .map_err(|_| Fatal::LayoutMapInconsistent)?;

        Ok(())
    }

    /// Build the model draft, then swap it in with one call (`SPEC-RT-052`).
    ///
    /// Nothing is mutated while the draft is built: it records target values
    /// (`SPEC-RT-051`), and `Store::install` cannot fail afterwards.
    fn apply_model(&mut self) {
        let base_writes = self.store.counters().io_writes;
        let base_epoch = self.store.epoch();
        let final_rows = self.effective_rows();
        let journal = std::mem::take(&mut self.txn.model);

        let epoch = if journal.writes > 0 {
            base_epoch + 1
        } else {
            base_epoch
        };
        let mut draft = StoreDraft::new(journal.next_key, epoch, base_writes + journal.writes);

        for key in &journal.inserted {
            if let Some(staged) = journal.rows.get(key)
                && let Some(reservation) = journal.reserved.get(key)
            {
                if journal.removals.contains(key) {
                    // Inserted and removed in the same frame: the reservation
                    // is released, never filled, so no slot leaks and the
                    // generation is burned (`SPEC-RT-051`).
                    draft.release(reservation.slot);
                    continue;
                }
                draft.fill(
                    reservation.slot,
                    *key,
                    staged.title.clone().unwrap_or_default(),
                    staged.completed.unwrap_or(false),
                );
            }
        }

        for (key, staged) in &journal.rows {
            if staged.inserted || journal.removals.contains(key) {
                continue;
            }
            if staged.completed.is_none() && staged.title.is_none() {
                continue;
            }
            if let Some((slot, _)) = self.store.slot_for(*key) {
                draft.update(slot, staged.title.clone(), staged.completed);
            }
        }

        for key in &journal.removals {
            if let Some((slot, _)) = self.store.slot_for(*key) {
                draft.remove(slot);
            }
        }

        let mut order = Vec::with_capacity(final_rows.len());
        for (key, _) in &final_rows {
            let slot = journal
                .reserved
                .get(key)
                .map(|reservation| reservation.slot)
                .or_else(|| self.store.slot_for(*key).map(|(slot, _)| slot));
            if let Some(slot) = slot {
                order.push(slot);
            }
        }
        draft.set_order(order);
        draft.set_completed_count(final_rows.iter().filter(|(_, done)| *done).count());
        // The revisions the frame reached become the committed ones: the cache
        // keys a frame used are the keys the store now answers under.
        draft.set_revisions(self.txn.staged_membership, self.txn.staged_order);

        self.store.install(draft);
        self.txn.model = ModelJournal::new(journal.next_key);
    }

    fn apply_views(&mut self) {
        let draft = std::mem::take(&mut self.txn.view);

        // Unmount the views first, but do not run owner cleanup yet: the
        // Cleanup stage comes after the swap (`SPEC-RT-050`), so a cleanup
        // closure never observes a half-built tree.
        let mut disposals = Vec::new();
        for view in &draft.unmounts {
            if let Some(node) = self.views.unmount(*view) {
                disposals.push(node.owner);
            }
        }
        for (owner, parent) in &draft.owners {
            // Owner ids are minted by the runtime high-water mark, so a
            // collision is structurally impossible; never assert after a swap.
            let _ = self.owners.insert_staged(*owner, *parent);
        }
        for mount in &draft.mounts {
            self.views
                .mount_row_with_id(mount.id, mount.row, mount.owner, mount.title_len);
            self.owners.on_cleanup(mount.owner, Box::new(|| {}));
        }
        if let Some(order) = &draft.order {
            self.views.reorder(order);
        }
        let focused = match draft.focus {
            Some(Some(view)) => {
                self.views.focus(view);
                Some(view)
            }
            Some(None) => {
                self.views.clear_focus();
                None
            }
            None => None,
        };
        if let Some((view, offset)) = draft.caret
            && focused != Some(view)
        {
            // `focus` already placed the caret at the title length; applying it
            // twice would double-count `caret_ops`.
            self.views.set_caret(view, offset);
        }
        self.views.set_next(draft.next_view);
        self.owners.set_next(draft.next_owner);
        self.layout.commit(&draft.unmounts);

        // Two-phase focus settlement (`SPEC-GATE-A0-10`): the intent was
        // recorded before the tree changed, and it is settled here — after the
        // swap, once — only when the frame did not move focus explicitly.
        // Fallback order is next -> previous; a focusable input and the
        // toolbar do not exist in A0 (they are A1 nodes).
        if !self.txn.user_focused
            && focused.is_none()
            && let Some(intent) = self.txn.focus_intents.last().copied()
        {
            let next = intent.next.filter(|view| self.views.is_live(*view));
            let previous = intent.previous.filter(|view| self.views.is_live(*view));
            let target = next.or(previous);
            // Test-only observation point: the cause recorded with the intent,
            // the rule that actually won, and the settled target. `FocusIntent`
            // keeps its public shape.
            #[cfg(test)]
            self.txn.focus_settlements.push((
                self.txn
                    .intent_causes
                    .last()
                    .copied()
                    .unwrap_or(FocusSettlementCause::Unmount),
                match (next, target) {
                    (Some(_), _) => FocusSettlementRule::Next,
                    (None, Some(_)) => FocusSettlementRule::Previous,
                    (None, None) => FocusSettlementRule::Cleared,
                },
                target,
            ));
            match target {
                Some(view) => self.views.focus(view),
                None => self.views.clear_focus(),
            };
        }
        // A focus pointer left on an unmounted view is resolved once, after
        // the settlement, and counts as a move.
        if self
            .views
            .focused()
            .is_some_and(|view| !self.views.is_live(view))
        {
            self.views.clear_focus();
        }

        // Cleanup: release the owners the unmounts orphaned. No mount, no
        // flush, no user write — the transaction is already published
        // (`SPEC-RT-050`, `SPEC-RT-054`).
        self.txn.phase = Phase::Cleanup;
        for owner in disposals {
            // Test-only observation point: the phase and the *already
            // published* view count at the moment cleanup runs.
            #[cfg(test)]
            self.txn
                .cleanup_log
                .push((owner, self.txn.phase, self.views.row_views().len()));
            self.owners.dispose(owner);
        }
        self.txn.phase = Phase::Commit;
    }

    fn discard(&mut self) {
        // No user code, no panic: drop the stage and burn the reservations.
        let next_key = self.txn.model.next_key;
        let journal = std::mem::take(&mut self.txn.model);
        for reservation in journal.reserved.values() {
            self.store.release_reserved(reservation.slot);
        }
        self.query.invalidate();
        self.layout.discard();
        // Re-seed the journal from the high-water mark: a discarded frame must
        // not rewind the key allocator, or the same `RowKey` would be minted
        // twice and alias two rows (`SPEC-RT-002`).
        self.txn.model = ModelJournal::new(next_key);
        self.txn.reset();
    }

    fn touched_keys(&self) -> Vec<RowKey> {
        let mut keys: Vec<RowKey> = self
            .txn
            .model
            .rows
            .keys()
            .copied()
            .chain(self.txn.model.removals.iter().copied())
            .collect();
        keys.sort_unstable();
        keys.dedup();
        keys
    }

    // ---- helpers ----

    fn staged_by_handle(&self, row: Handle<Row>) -> Option<&StagedRow> {
        self.txn.model.rows.values().find(|staged| {
            staged.runtime == row.runtime()
                && staged.slot == row.slot()
                && staged.generation == row.generation()
        })
    }

    fn staged_key(&self, row: Handle<Row>) -> Option<RowKey> {
        self.txn
            .model
            .rows
            .iter()
            .find(|(_, staged)| {
                staged.runtime == row.runtime()
                    && staged.slot == row.slot()
                    && staged.generation == row.generation()
            })
            .map(|(key, _)| *key)
    }

    /// The key this handle resolves to in the merged overlay, if the row has
    /// been staged for removal in any open level. A removal always wins over a
    /// staged value (`ADR-0017` decision 4).
    fn removed_key(&self, row: Handle<Row>) -> Option<RowKey> {
        let key = self.staged_key(row).or_else(|| self.store.key_of(row))?;
        self.txn.model.removals.contains(&key).then_some(key)
    }

    fn effective_key(&self, row: Handle<Row>) -> Result<RowKey, ValueError> {
        if row.runtime() != self.store.runtime_id() {
            return Err(ValueError::Handle(InvalidHandle::from(
                crate::handle::HandleError::ForeignRuntime {
                    expected: self.store.runtime_id(),
                    found: row.runtime(),
                },
            )));
        }
        if let Some(key) = self.staged_key(row) {
            return Ok(key);
        }
        self.store.key_of(row).ok_or_else(|| {
            ValueError::Handle(InvalidHandle::from(crate::handle::HandleError::Vacant {
                slot: row.slot(),
            }))
        })
    }

    fn effective_completed(&self, row: Handle<Row>) -> Result<bool, ValueError> {
        if self.removed_key(row).is_some() {
            return Err(ValueError::Handle(InvalidHandle::from(
                crate::handle::HandleError::Vacant { slot: row.slot() },
            )));
        }
        if let Some(key) = self.staged_key(row)
            && let Some(staged) = self.txn.model.rows.get(&key)
            && let Some(completed) = staged.completed
        {
            return Ok(completed);
        }
        self.store.completed_of(row).map_err(ValueError::from)
    }

    /// Final row set, as this frame sees it. The merge itself lives on
    /// [`StagedView`] so the cache and the commit cannot diverge.
    fn effective_rows(&self) -> Vec<(RowKey, bool)> {
        StagedView::new(self.store, self.txn).effective_rows()
    }

    /// Discard the frame explicitly. Used by `Runtime::frame` when the closure
    /// returns an error, so the stage is dropped before the borrow ends.
    pub(crate) fn abort(&mut self) {
        self.discard();
        self.txn.outcome = Outcome::Discarded;
    }

    /// Nested batch. An inner failure poisons the whole transaction: there are
    /// no savepoints, so the outer frame cannot commit either.
    pub fn batch<R>(
        &mut self,
        f: impl FnOnce(&mut Tx<'_>) -> Result<R, Fatal>,
    ) -> Result<R, Fatal> {
        if self.txn.phase != Phase::Outside && self.txn.phase != Phase::Mark {
            return Err(Fatal::BatchNotAllowed {
                phase: self.txn.phase,
            });
        }
        self.txn.depth += 1;
        let result = {
            let mut inner = Tx {
                txn: &mut *self.txn,
                store: &mut *self.store,
                graph: &mut *self.graph,
                views: &mut *self.views,
                owners: &mut *self.owners,
                query: &mut *self.query,
                layout: &mut *self.layout,
                nodes: self.nodes,
                level: self.level + 1,
            };
            f(&mut inner)
        };
        self.txn.depth = self.txn.depth.saturating_sub(1);
        if result.is_err() {
            // No savepoints: the failure belongs to the whole transaction.
            // The frame body may still swallow the error, so the fatal is
            // latched and returned by `commit` (`ADR-0019` decision 6).
            self.txn.poison = Some(Fatal::Aborted);
        }
        result
    }
}

/// The committed model with one frame's journal applied on top.
///
/// This is the second `RowView` implementation (`ADR-0022` decision 2). The
/// cache cannot tell it apart from `Store`, and nothing outside this crate can
/// produce a third view: a fixture that wants row counts has to drive a real
/// frame.
struct StagedView<'a> {
    store: &'a Store,
    journal: &'a ModelJournal,
    membership: [u64; 3],
    order: u64,
    /// Rows this view has enumerated. The cache turns the difference across one
    /// evaluation into `row_visits`, so no enumeration is free — including one
    /// performed to answer `is_empty_for`.
    scanned: u64,
}

impl<'a> StagedView<'a> {
    fn new(store: &'a Store, txn: &'a Txn) -> Self {
        Self {
            store,
            journal: &txn.model,
            membership: txn.staged_membership,
            order: txn.staged_order,
            scanned: 0,
        }
    }

    /// Final row set: committed rows with staged inserts, updates and removals
    /// applied, in the staged order when one exists.
    ///
    /// This is the commit path's view of the model (preflight, publication); it
    /// does not count scans, because it is not a query evaluation.
    fn effective_rows(&self) -> Vec<(RowKey, bool)> {
        let mut rows: Vec<(RowKey, bool)> = self
            .store
            .rows()
            .map(|row| {
                let key = row.key();
                let completed = self
                    .journal
                    .rows
                    .get(&key)
                    .and_then(|staged| staged.completed)
                    .unwrap_or_else(|| row.completed());
                (key, completed)
            })
            .filter(|(key, _)| !self.journal.removals.contains(key))
            .collect();

        for key in &self.journal.inserted {
            if self.journal.removals.contains(key) {
                continue;
            }
            let completed = self
                .journal
                .rows
                .get(key)
                .and_then(|staged| staged.completed)
                .unwrap_or(false);
            rows.push((*key, completed));
        }

        if let Some(order) = &self.journal.order {
            // The staged order is authoritative for the rows it names. A row
            // it does not name — typically one inserted after the reorder —
            // is appended in insertion order rather than silently dropped:
            // dropping it would leave a filled slot that no read path can
            // reach (`SPEC-RT-052`).
            let mut ordered = Vec::with_capacity(rows.len());
            for key in order {
                if let Some(entry) = rows.iter().find(|(candidate, _)| candidate == key) {
                    ordered.push(*entry);
                }
            }
            for entry in &rows {
                if !ordered.iter().any(|(key, _)| key == &entry.0) {
                    ordered.push(*entry);
                }
            }
            return ordered;
        }
        rows
    }

    /// Live and completed row counts: the store's counters plus the journal's
    /// own entries, never a scan of the row list (`SPEC-RT-044`).
    ///
    /// The journal is as long as the frame is wide, not as the store is; a
    /// frame that writes one row costs one step here whatever `len()` is.
    fn live_counts(&self) -> (i64, i64) {
        let mut live = self.store.len() as i64;
        let mut completed = (self.store.len() - self.store.remaining()) as i64;

        for key in &self.journal.inserted {
            if self.journal.removals.contains(key) {
                continue;
            }
            live += 1;
            if self
                .journal
                .rows
                .get(key)
                .and_then(|staged| staged.completed)
                .unwrap_or(false)
            {
                completed += 1;
            }
        }

        for (key, staged) in &self.journal.rows {
            if staged.inserted || self.journal.removals.contains(key) {
                continue;
            }
            let Some(value) = staged.completed else {
                continue;
            };
            if self.store_completed(*key) != Some(value) {
                completed += if value { 1 } else { -1 };
            }
        }

        for key in &self.journal.removals {
            if self
                .journal
                .rows
                .get(key)
                .is_some_and(|staged| staged.inserted)
            {
                continue;
            }
            if let Some(was_completed) = self.store_completed(*key) {
                live -= 1;
                if was_completed {
                    completed -= 1;
                }
            }
        }

        (live, completed)
    }

    fn store_completed(&self, key: RowKey) -> Option<bool> {
        let (slot, _) = self.store.slot_for(key)?;
        self.store.completed_at(slot)
    }

    /// Every live row in display order, counting each row it enumerates.
    fn each_live(&mut self, visit: &mut dyn FnMut(RowKey, bool)) {
        if self.journal.order.is_some() {
            let rows = self.effective_rows();
            self.scanned += rows.len() as u64;
            for (key, completed) in rows {
                visit(key, completed);
            }
            return;
        }

        let store = self.store;
        let journal = self.journal;
        for row in store.rows() {
            let key = row.key();
            if journal.removals.contains(&key) {
                continue;
            }
            self.scanned += 1;
            let completed = journal
                .rows
                .get(&key)
                .and_then(|staged| staged.completed)
                .unwrap_or_else(|| row.completed());
            visit(key, completed);
        }
        for key in &journal.inserted {
            if journal.removals.contains(key) {
                continue;
            }
            self.scanned += 1;
            let completed = journal
                .rows
                .get(key)
                .and_then(|staged| staged.completed)
                .unwrap_or(false);
            visit(*key, completed);
        }
    }
}

impl RowView for StagedView<'_> {
    fn rows_for(&mut self, filter: Filter) -> Vec<(RowKey, bool)> {
        let mut rows = Vec::new();
        self.each_live(&mut |key, completed| {
            if filter.matches(completed) {
                rows.push((key, completed));
            }
        });
        rows
    }

    fn is_empty_for(&mut self, filter: Filter) -> bool {
        let (live, completed) = self.live_counts();
        match filter {
            Filter::All => live == 0,
            Filter::Active => completed == live,
            Filter::Completed => completed == 0,
        }
    }

    fn membership_revision(&self, filter: Filter) -> u64 {
        self.membership[filter_index(filter)]
    }

    fn order_revision(&self) -> u64 {
        self.order
    }

    fn scan_cost(&self) -> u64 {
        self.scanned
    }
}

impl Drop for Tx<'_> {
    /// Never commits. A `Tx` that leaves a frame without `commit` discards its
    /// stage: no user code, no panic, and the guard is reset.
    fn drop(&mut self) {
        if self.txn.outcome == Outcome::Live && self.level == 0 {
            self.discard();
            self.txn.outcome = Outcome::Discarded;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Runtime;

    /// The order revision this frame's cache reads are keyed under.
    ///
    /// The fixture is a unit test on purpose: the field is private to the
    /// transaction, and integration tests under `crates/core/tests/` are
    /// compiled against a library built without `cfg(test)`, so only a module
    /// inside this file can observe it without widening the public surface.
    fn order_revision(tx: &Tx<'_>) -> u64 {
        tx.txn.staged_order
    }

    /// `ADR-0023` decision 2: an insert that adds a live row moves the display
    /// order, and the commit adopts the staged value.
    ///
    /// Red-ability: delete the `note_order()` call in `Tx::insert` and the
    /// `end == start + 1` assertion fails.
    #[test]
    fn an_insert_advances_the_order_revision() {
        let mut runtime = Runtime::new();

        let (start, end) = runtime
            .frame(|tx| {
                let start = order_revision(tx);
                tx.insert("A").expect("insert must be accepted");
                Ok((start, order_revision(tx)))
            })
            .expect("the frame must commit");

        assert_eq!(
            end,
            start + 1,
            "an insert that adds a live row must advance the order revision"
        );
        assert_eq!(
            runtime.committed().order_epoch(),
            end,
            "the commit must adopt the staged order revision"
        );
    }

    /// Red-ability: delete the `note_order()` call in `Tx::remove` and the
    /// `end == start + 1` assertion fails.
    #[test]
    fn a_remove_advances_the_order_revision() {
        let mut runtime = Runtime::new();
        let row = runtime
            .frame(|tx| tx.insert("A"))
            .expect("the setup frame must commit");

        let (start, end) = runtime
            .frame(|tx| {
                let start = order_revision(tx);
                tx.remove(row).expect("remove must be accepted");
                Ok((start, order_revision(tx)))
            })
            .expect("the frame must commit");

        assert_eq!(
            end,
            start + 1,
            "a remove that drops a live row must advance the order revision"
        );
        assert_eq!(
            runtime.committed().order_epoch(),
            end,
            "the commit must adopt the staged order revision"
        );
        assert_eq!(runtime.committed().len(), 0, "the row must really be gone");
    }

    /// Red-ability: delete the `note_order()` call in `Tx::clear_completed`
    /// (or move it out of the `removed.is_empty()` guard) and this fixture
    /// fails.
    #[test]
    fn a_clear_completed_advances_the_order_revision_once() {
        let mut runtime = Runtime::new();
        runtime
            .frame(|tx| {
                let a = tx.insert("A")?;
                let b = tx.insert("B")?;
                tx.set_completed(a, true).expect("complete A");
                tx.set_completed(b, true).expect("complete B");
                Ok(())
            })
            .expect("the setup frame must commit");

        let (start, end) = runtime
            .frame(|tx| {
                let start = order_revision(tx);
                let removed = tx.clear_completed().expect("clear must be accepted");
                assert_eq!(removed.len(), 2, "both rows are completed");
                Ok((start, order_revision(tx)))
            })
            .expect("the frame must commit");

        assert_eq!(
            end,
            start + 1,
            "a clear moves the order once for the call, not once per removed row"
        );
        assert_eq!(
            runtime.committed().order_epoch(),
            end,
            "the commit must adopt the staged order revision"
        );
    }

    /// Red-ability: delete the `note_order()` call in `Tx::reorder` and the
    /// `end == start + 1` assertion fails.
    #[test]
    fn a_reorder_advances_the_order_revision() {
        let mut runtime = Runtime::new();
        runtime
            .frame(|tx| {
                tx.insert("A")?;
                tx.insert("B")?;
                Ok(())
            })
            .expect("the setup frame must commit");

        let (start, end) = runtime
            .frame(|tx| {
                let start = order_revision(tx);
                let mut keys = tx.rows(Filter::All).as_slice().to_vec();
                keys.reverse();
                tx.reorder(&keys).expect("reorder must be accepted");
                Ok((start, order_revision(tx)))
            })
            .expect("the frame must commit");

        assert_eq!(
            end,
            start + 1,
            "a reorder that moves a row must advance the order revision"
        );
        assert_eq!(
            runtime.committed().order_epoch(),
            end,
            "the commit must adopt the staged order revision"
        );
        assert_eq!(
            runtime.committed().titles().collect::<Vec<_>>(),
            vec!["B", "A"],
            "the fixture must reorder for real, not stage a no-op"
        );
    }

    /// `ADR-0023` decision 2, the other half: a `completed` write moves only
    /// the `Active` / `Completed` membership sets. It must not move the order.
    ///
    /// Red-ability: add a `note_order()` call to `stage_completed` and the
    /// `end == start` assertion fails. The `epoch_for(Filter::Active)`
    /// assertion is what keeps the fixture honest when the write stops
    /// happening at all.
    #[test]
    fn a_completed_write_does_not_advance_the_order_revision() {
        let mut runtime = Runtime::new();
        let row = runtime
            .frame(|tx| tx.insert("A"))
            .expect("the setup frame must commit");

        let active_before = runtime.committed().epoch_for(Filter::Active);
        let order_before = runtime.committed().order_epoch();

        let (start, end) = runtime
            .frame(|tx| {
                let start = order_revision(tx);
                tx.set_completed(row, true)
                    .expect("set_completed must be accepted");
                Ok((start, order_revision(tx)))
            })
            .expect("the frame must commit");

        assert_eq!(
            end, start,
            "a completed write changes membership, not the display order"
        );
        assert_eq!(
            runtime.committed().order_epoch(),
            order_before,
            "the commit must not move the order revision"
        );
        assert_eq!(
            runtime.committed().epoch_for(Filter::Active),
            active_before + 1,
            "the write must really have moved the Active membership"
        );
    }

    /// No-op calls are not writes: re-removing a key already staged for
    /// removal returns early, before any revision is touched.
    ///
    /// Red-ability: move the `note_order()` call in `Tx::remove` above the
    /// `removals.contains` early return and the `end == start + 1` assertion
    /// fails on the second remove.
    #[test]
    fn a_remove_of_an_already_removed_key_does_not_advance_the_order_revision() {
        let mut runtime = Runtime::new();
        let row = runtime
            .frame(|tx| tx.insert("A"))
            .expect("the setup frame must commit");

        let (start, end) = runtime
            .frame(|tx| {
                let start = order_revision(tx);
                tx.remove(row).expect("the first remove is effective");
                tx.remove(row)
                    .expect("re-removing a staged key is accepted as a no-op");
                Ok((start, order_revision(tx)))
            })
            .expect("the frame must commit");

        assert_eq!(
            end,
            start + 1,
            "the second remove changed nothing and must not advance the revision"
        );
    }

    /// Red-ability: remove the `removed.is_empty()` guard around the staging
    /// block in `Tx::clear_completed` and the `end == start` assertion fails.
    #[test]
    fn a_clear_with_no_completed_rows_does_not_advance_the_order_revision() {
        let mut runtime = Runtime::new();
        runtime
            .frame(|tx| {
                tx.insert("A")?;
                Ok(())
            })
            .expect("the setup frame must commit");

        let (start, end) = runtime
            .frame(|tx| {
                let start = order_revision(tx);
                let removed = tx
                    .clear_completed()
                    .expect("a clear with nothing to clear must be accepted");
                assert!(removed.is_empty(), "no row is completed");
                Ok((start, order_revision(tx)))
            })
            .expect("the frame must commit");

        assert_eq!(
            end, start,
            "a clear that removes nothing must not advance the order revision"
        );
    }

    /// Red-ability: delete the `current != keys` guard in `Tx::reorder` and
    /// the `end == start` assertion fails.
    #[test]
    fn a_reorder_to_the_current_order_does_not_advance_the_order_revision() {
        let mut runtime = Runtime::new();
        runtime
            .frame(|tx| {
                tx.insert("A")?;
                tx.insert("B")?;
                Ok(())
            })
            .expect("the setup frame must commit");

        let (start, end) = runtime
            .frame(|tx| {
                let start = order_revision(tx);
                let keys = tx.rows(Filter::All).as_slice().to_vec();
                tx.reorder(&keys)
                    .expect("the current order is a valid order");
                Ok((start, order_revision(tx)))
            })
            .expect("the frame must commit");

        assert_eq!(
            end, start,
            "a reorder that moves nothing must not advance the order revision"
        );
    }

    // ---- SPEC-GATE-A0-10: the settlement log ------------------------------

    /// Mount every committed row and return the views in display order.
    fn mount_all(runtime: &mut Runtime) -> Vec<ViewId> {
        runtime
            .frame(|tx| {
                let keys: Vec<RowKey> = tx.rows(Filter::All).iter().collect();
                let mut views = Vec::with_capacity(keys.len());
                for key in keys {
                    views.push(tx.mount_row(key, 1)?);
                }
                tx.reorder_views(&views)?;
                Ok(views)
            })
            .expect("mount every row")
    }

    fn last_settlement(
        runtime: &Runtime,
    ) -> (FocusSettlementCause, FocusSettlementRule, Option<ViewId>) {
        *runtime
            .txn()
            .focus_settlements()
            .last()
            .expect("a settlement must have been recorded")
    }

    /// `SPEC-GATE-A0-10`: deleting the focused row with a live successor
    /// settles through `Unmount` + `Next` and names the successor.
    ///
    /// Red-ability: drop the `intent.next` arm in `Tx::commit` and the settled
    /// target moves to the previous sibling or none.
    #[test]
    fn a0_10_settlement_records_next_for_a_live_successor() {
        let mut runtime = Runtime::new();
        let (a, _b, _c) = runtime
            .frame(|tx| Ok((tx.insert("A")?, tx.insert("B")?, tx.insert("C")?)))
            .expect("seed");
        let views = mount_all(&mut runtime);
        runtime
            .frame(|tx| {
                tx.focus(views[0])?;
                Ok(())
            })
            .expect("focus A");

        let before = runtime.txn().focus_settlements().len();
        runtime
            .frame(|tx| {
                tx.remove(a).map_err(|_| Fatal::Aborted)?;
                tx.unmount(views[0])?;
                Ok(())
            })
            .expect("delete the focused row");

        assert_eq!(
            runtime.txn().focus_settlements().len(),
            before + 1,
            "exactly one settlement"
        );
        assert_eq!(
            last_settlement(&runtime),
            (
                FocusSettlementCause::Unmount,
                FocusSettlementRule::Next,
                Some(views[1])
            ),
            "a live next sibling wins and the cause is the unmount"
        );
    }

    /// `SPEC-GATE-A0-10`: deleting the last row takes `Previous`.
    ///
    /// Red-ability: hard-code `FocusSettlementRule::Next` in `Tx::commit` (or
    /// return `Some` from the next arm unconditionally) and this fails.
    #[test]
    fn a0_10_settlement_records_previous_at_the_end() {
        let mut runtime = Runtime::new();
        let (_a, _b, c) = runtime
            .frame(|tx| Ok((tx.insert("A")?, tx.insert("B")?, tx.insert("C")?)))
            .expect("seed");
        let views = mount_all(&mut runtime);
        runtime
            .frame(|tx| {
                tx.focus(views[2])?;
                Ok(())
            })
            .expect("focus C");

        runtime
            .frame(|tx| {
                tx.remove(c).map_err(|_| Fatal::Aborted)?;
                tx.unmount(views[2])?;
                Ok(())
            })
            .expect("delete the last focused row");

        assert_eq!(
            last_settlement(&runtime),
            (
                FocusSettlementCause::Unmount,
                FocusSettlementRule::Previous,
                Some(views[1])
            ),
            "no next sibling exists, so the previous sibling settles"
        );
    }

    /// `SPEC-GATE-A0-10`: a bulk clear records **one** intent with cause
    /// `BulkClear`, not one per removed row.
    ///
    /// Red-ability: push a settlement inside the removal loop (or per removed
    /// row) and the `before + 1` assertion fails.
    #[test]
    fn a0_10_bulk_clear_settles_one_intent() {
        let mut runtime = Runtime::new();
        let (a, b, _c) = runtime
            .frame(|tx| Ok((tx.insert("A")?, tx.insert("B")?, tx.insert("C")?)))
            .expect("seed");
        let views = mount_all(&mut runtime);
        runtime
            .frame(|tx| {
                tx.set_completed(a, true).map_err(|_| Fatal::Aborted)?;
                tx.set_completed(b, true).map_err(|_| Fatal::Aborted)?;
                Ok(())
            })
            .expect("complete A and B");
        runtime
            .frame(|tx| {
                tx.focus(views[1])?;
                Ok(())
            })
            .expect("focus B");

        let before = runtime.txn().focus_settlements().len();
        runtime
            .frame(|tx| {
                tx.clear_completed()?;
                for view in &views[..2] {
                    tx.unmount(*view)?;
                }
                Ok(())
            })
            .expect("bulk clear");

        let log = runtime.txn().focus_settlements();
        assert_eq!(
            log.len(),
            before + 1,
            "one intent for the whole clear, not one per removed row"
        );
        assert_eq!(
            *log.last().expect("settlement"),
            (
                FocusSettlementCause::BulkClear,
                FocusSettlementRule::Next,
                Some(views[2])
            ),
            "the clear is one intent and a surviving sibling settles it"
        );
    }

    /// `SPEC-GATE-A0-10`: a user focus move in the same frame cancels the
    /// recorded intent, so no settlement is appended and the user's target
    /// stays.
    ///
    /// Red-ability: drop the `focused.is_none()` guard and the recorded intent
    /// settles over the user's target.
    #[test]
    fn a0_10_a_user_focus_move_cancels_the_settlement() {
        let mut runtime = Runtime::new();
        let (a, _b, _c) = runtime
            .frame(|tx| Ok((tx.insert("A")?, tx.insert("B")?, tx.insert("C")?)))
            .expect("seed");
        let views = mount_all(&mut runtime);
        runtime
            .frame(|tx| {
                tx.focus(views[0])?;
                Ok(())
            })
            .expect("focus A");

        let before = runtime.txn().focus_settlements().len();
        runtime
            .frame(|tx| {
                tx.remove(a).map_err(|_| Fatal::Aborted)?;
                tx.unmount(views[0])?;
                tx.focus(views[2])?;
                Ok(())
            })
            .expect("user moved focus");

        assert_eq!(
            runtime.txn().focus_settlements().len(),
            before,
            "a user focus move must cancel the recorded intent"
        );
        assert_eq!(runtime.focused(), Some(views[2]));
    }

    /// `SPEC-GATE-A0-10`: clearing focus is a user choice too; the recorded
    /// intent must not pull focus back after the user cleared it.
    ///
    /// Red-ability: drop `!self.txn.user_focused` from the settlement guard and
    /// this test fails — the intent settles on the next sibling instead of
    /// leaving focus cleared.
    #[test]
    fn a0_10_a_user_clear_cancels_the_settlement() {
        let mut runtime = Runtime::new();
        let (a, _b, _c) = runtime
            .frame(|tx| Ok((tx.insert("A")?, tx.insert("B")?, tx.insert("C")?)))
            .expect("seed");
        let views = mount_all(&mut runtime);
        runtime
            .frame(|tx| {
                tx.focus(views[0])?;
                Ok(())
            })
            .expect("focus A");

        let before = runtime.txn().focus_settlements().len();
        runtime
            .frame(|tx| {
                tx.remove(a).map_err(|_| Fatal::Aborted)?;
                tx.unmount(views[0])?;
                tx.clear_focus()?;
                Ok(())
            })
            .expect("user cleared focus");

        assert_eq!(
            runtime.txn().focus_settlements().len(),
            before,
            "a user clear must cancel the recorded intent"
        );
        assert_eq!(runtime.focused(), None);
    }

    // ---- SPEC-RT-024: key uniqueness is a property of the node table --------

    /// `SPEC-RT-024` / `SPEC-GATE-A0-09`: a view mounted **without** a staged
    /// reorder enters the tree but not the display order, and it still claims
    /// its key.
    ///
    /// `Tx::mount_row` used to scan `row_views()`, which is order-driven, so
    /// such a view was invisible to the duplicate check and a second mount for
    /// the same row was accepted: two views and two owners claiming one
    /// `RowKey`, with `row_views()` showing neither. The check now asks the
    /// node table.
    ///
    /// The refusal is asserted **at the call site** and the frame is then
    /// aborted on purpose: the commit preflight refuses the same duplicate with
    /// the same variant, so a fixture that only inspects the frame's result
    /// cannot tell the two barriers apart. The preflight half has its own
    /// fixture below.
    ///
    /// Red-ability: revert `Tx::mount_row`'s `already_mounted` scan to
    /// `row_views()` and the second mount is accepted — the in-frame assertion
    /// fails.
    #[test]
    fn a0_09_an_unordered_view_still_claims_its_key() {
        let mut runtime = Runtime::new();
        runtime
            .frame(|tx| {
                tx.insert("A")?;
                Ok(())
            })
            .expect("seed");
        let key = runtime.committed().keys().next().expect("the seeded row");

        runtime
            .frame(|tx| {
                tx.mount_row(key, 1)?;
                Ok(())
            })
            .expect("a mount without a reorder");
        assert_eq!(
            runtime.row_views().len(),
            0,
            "the fixture needs a view outside the display order"
        );
        assert_eq!(
            runtime.live_owner_ids().len(),
            2,
            "root plus one mount owner"
        );

        let result: Result<(), Fatal> = runtime.frame(|tx| {
            let second = tx.mount_row(key, 1);
            assert!(
                matches!(second, Err(Fatal::DuplicateViewKey { key: claimed }) if claimed == key),
                "the unordered view must still own its key, got {second:?}"
            );
            Err(Fatal::Aborted)
        });
        assert!(result.is_err(), "the frame was discarded");
        assert_eq!(
            runtime.live_owner_ids().len(),
            2,
            "the aborted frame minted no second owner"
        );
    }

    /// Stage one mount the way `Tx::mount_row` would, without going through it.
    ///
    /// This exists only for the fixture below: `Tx::mount_row` refuses a
    /// duplicate key at the call site, so the commit-time key preflight cannot
    /// be reached with a duplicate through the public API. Staging the draft
    /// directly exercises the barrier.
    fn stage_mount(tx: &mut Tx<'_>, row: RowKey, title_len: u32) -> ViewId {
        let view = ViewId::from_index(tx.txn.view.next_view);
        let owner = OwnerId::from_index(tx.txn.view.next_owner);
        let parent = tx.owners.root();
        tx.txn.view.next_view += 1;
        tx.txn.view.next_owner += 1;
        tx.txn.view.owners.push((owner, parent));
        tx.txn.view.mounts.push(PlannedMount {
            id: view,
            row,
            owner,
            title_len,
        });
        tx.layout.plan(view);
        view
    }

    /// `SPEC-RT-024`: the commit-time key preflight compares a staged mount
    /// against the live nodes, not only against the frame's own draft.
    ///
    /// A single staged mount whose key a committed (unordered) view already
    /// claims passes the `seen_keys` check — the draft has one mount — so this
    /// is the branch the barrier needs, and it is the only way the key check
    /// survives a call-site hole.
    ///
    /// Red-ability: delete the `claiming_view` branch in `Tx::preflight` and the
    /// frame commits: two views claim one row key, so
    /// `live_owner_ids().len() == 3` and the authority comparison fails.
    #[test]
    fn a0_06_the_commit_preflight_sees_a_committed_view_outside_the_order() {
        let mut runtime = Runtime::new();
        runtime
            .frame(|tx| {
                tx.insert("A")?;
                Ok(())
            })
            .expect("seed");
        let key = runtime.committed().keys().next().expect("the seeded row");
        runtime
            .frame(|tx| {
                tx.mount_row(key, 1)?;
                Ok(())
            })
            .expect("a mount without a reorder");

        let owners_before = runtime.live_owner_ids();
        let layout_before = runtime.layout_pairs();
        let commits_before = runtime.txn().commits();
        let publications_before = runtime.txn().publications().len();

        let result = runtime.frame(|tx| {
            let staged = stage_mount(tx, key, 1);
            tx.txn.view.order = Some(vec![staged]);
            Ok(())
        });

        assert!(
            matches!(result, Err(Fatal::DuplicateViewKey { key: claimed }) if claimed == key),
            "expected the preflight to see the committed claim, got {result:?}"
        );
        assert_eq!(runtime.live_owner_ids(), owners_before, "no owner survived");
        assert_eq!(runtime.layout_pairs(), layout_before, "layout is untouched");
        assert_eq!(runtime.txn().commits(), commits_before, "nothing committed");
        assert_eq!(
            runtime.txn().publications().len(),
            publications_before,
            "a refused commit publishes nothing"
        );
    }

    // ---- SPEC-GATE-A0-10: the two settlement branches without a witness -----

    /// `SPEC-GATE-A0-10`: "a next sibling that is no longer affected must not be
    /// settled onto" — with the next sibling unmounted in the same frame, the
    /// settlement must fall through to `Previous`.
    ///
    /// Red-ability: turn `let next = intent.next.filter(|view| self.views.is_live(*view));`
    /// into `let next = intent.next;` and the settlement records `Next` with a
    /// dead target; the pointer then lands nowhere, the trailing dead-pointer
    /// guard clears focus, and both the tuple and the `focused` assertion fail.
    #[test]
    fn a0_10_settlement_skips_a_next_sibling_unmounted_in_the_same_frame() {
        let mut runtime = Runtime::new();
        runtime
            .frame(|tx| {
                tx.insert("A")?;
                tx.insert("B")?;
                tx.insert("C")?;
                Ok(())
            })
            .expect("seed");
        let views = mount_all(&mut runtime);
        runtime
            .frame(|tx| {
                tx.focus(views[1])?;
                Ok(())
            })
            .expect("focus B");

        let before = runtime.txn().focus_settlements().len();
        runtime
            .frame(|tx| {
                // B and its next sibling C both leave the tree.
                tx.unmount(views[1])?;
                tx.unmount(views[2])?;
                Ok(())
            })
            .expect("unmount B and C");

        assert_eq!(
            runtime.txn().focus_settlements().len(),
            before + 1,
            "exactly one settlement"
        );
        assert_eq!(
            last_settlement(&runtime),
            (
                FocusSettlementCause::Unmount,
                FocusSettlementRule::Previous,
                Some(views[0])
            ),
            "a dead next sibling must be skipped, not reported"
        );
        assert_eq!(
            runtime.focused(),
            Some(views[0]),
            "focus lands on the surviving previous sibling"
        );
    }

    /// `SPEC-GATE-A0-10`: a bulk clear that removes every candidate settles to
    /// `Cleared` with no target, once, and names the clear as the cause.
    ///
    /// Red-ability: map `(None, None)` to any other rule (`Next` or `Previous`)
    /// and the exact-slice assertion fails; drop the settlement push and the
    /// count assertion fails. The `focused().is_none()` half is a result
    /// assertion, not the witness for the arm's own `clear_focus()`: the
    /// trailing dead-pointer guard would clear the pointer even if the arm
    /// omitted it, so this fixture does not depend on that redundancy.
    #[test]
    fn a0_10_a_bulk_clear_with_no_live_candidate_records_cleared() {
        let mut runtime = Runtime::new();
        let (a, b, _c) = runtime
            .frame(|tx| Ok((tx.insert("A")?, tx.insert("B")?, tx.insert("C")?)))
            .expect("seed");
        let views = mount_all(&mut runtime);
        for row in [a, b] {
            runtime
                .frame(|tx| {
                    tx.set_completed(row, true).map_err(|_| Fatal::Aborted)?;
                    Ok(())
                })
                .expect("complete a row");
        }
        runtime
            .frame(|tx| {
                tx.focus(views[0])?;
                Ok(())
            })
            .expect("focus A");

        let before = runtime.txn().focus_settlements().len();
        runtime
            .frame(|tx| {
                tx.clear_completed()?;
                for view in &views[..2] {
                    tx.unmount(*view)?;
                }
                Ok(())
            })
            .expect("clear");

        assert_eq!(
            runtime.txn().focus_settlements().len(),
            before + 1,
            "one settlement for the clear, not one per removed row"
        );
        assert_eq!(
            last_settlement(&runtime),
            (
                FocusSettlementCause::BulkClear,
                FocusSettlementRule::Cleared,
                None
            ),
            "every candidate is gone, so the rule is Cleared and the target is none"
        );
        assert!(
            runtime.focused().is_none(),
            "no candidate survives the clear"
        );
    }
}
