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
use crate::query::{Filter, QueryCache, RowSet};
use crate::store::{Row, Store, StoreError};
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
    WriteDuringCompute {
        node: NodeId,
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
            Fatal::WriteDuringCompute { node } => write!(f, "write during compute of node {node}"),
            Fatal::Unbalanced { depth } => write!(f, "unbalanced transaction end at depth {depth}"),
            Fatal::StaleStagedRow { key } => {
                write!(f, "staged operation targets removed row {}", key.raw())
            }
            Fatal::OrderNotAPermutation => f.write_str("staged order is not a permutation"),
            Fatal::LayoutMapInconsistent => f.write_str("layout map would be inconsistent"),
            Fatal::Aborted => f.write_str("transaction aborted by the caller"),
        }
    }
}

impl std::error::Error for Fatal {}

/// Value-level failure: the write is rejected, the transaction still commits.
///
/// `golden step 08` is exactly this shape — a stale handle is refused and the
/// committed state does not move.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ValueError {
    Handle(InvalidHandle),
    Store(StoreError),
}

impl From<InvalidHandle> for ValueError {
    fn from(error: InvalidHandle) -> Self {
        Self::Handle(error)
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

/// Transaction state owned by the runtime.
pub struct Txn {
    depth: u32,
    phase: Phase,
    outcome: Outcome,
    model: ModelJournal,
    graph: GraphScratch,
    view: ViewDraft,
    staged_rev: u64,
    publishes: Vec<Publication>,
    commits: u64,
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
            publishes: Vec::new(),
            commits: 0,
        }
    }

    pub(crate) fn begin_frame(&mut self, next_view: u32, next_owner: u32) {
        self.depth = 1;
        self.phase = Phase::Outside;
        self.outcome = Outcome::Live;
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
    pub all: NodeId,
    pub active: NodeId,
    pub completed: NodeId,
    pub remaining: NodeId,
    pub has_completed: NodeId,
}

impl Nodes {
    pub(crate) fn build(graph: &mut MemoGraph) -> Self {
        Self {
            store: graph.source("store"),
            all: graph.memo("rows/all"),
            active: graph.memo("rows/active"),
            completed: graph.memo("rows/completed"),
            remaining: graph.memo("remaining"),
            has_completed: graph.memo("has-completed"),
        }
    }

    pub const fn for_filter(self, filter: Filter) -> NodeId {
        match filter {
            Filter::All => self.all,
            Filter::Active => self.active,
            Filter::Completed => self.completed,
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

    fn require(&self, kind: WriteKind) -> Result<(), Fatal> {
        if self.txn.phase.permits(kind) {
            Ok(())
        } else {
            Err(Fatal::PhaseViolation(PhaseViolation {
                phase: self.txn.phase,
                attempted: kind,
            }))
        }
    }

    fn bump(&mut self) {
        self.txn.staged_rev = self.txn.staged_rev.saturating_add(1);
        self.graph
            .bump_source_staged(&mut self.txn.graph, self.nodes.store);
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
        debug_assert!(self.txn.phase.permits(WriteKind::ModelWrite));
        let key = self.effective_key(row)?;
        if self.txn.model.removals.insert(key) {
            self.txn.model.writes += 1;
            self.bump();
        }
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
            self.txn.model.writes += 1;
            self.bump();
        }
        Ok(removed)
    }

    pub fn reorder(&mut self, keys: &[RowKey]) -> Result<(), ValueError> {
        debug_assert!(self.txn.phase.permits(WriteKind::ModelWrite));
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
        }
        Ok(())
    }

    fn stage_completed(&mut self, row: Handle<Row>, value: bool) -> Result<(), ValueError> {
        debug_assert!(self.txn.phase.permits(WriteKind::ModelWrite));
        let key = self.effective_key(row)?;
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
        if let Some(staged) = self.staged_by_handle(row) {
            if let Some(title) = &staged.title {
                return Ok(f(title));
            }
            if self
                .staged_key(row)
                .is_some_and(|key| self.txn.model.removals.contains(&key))
            {
                return Err(ValueError::Handle(InvalidHandle::from(
                    crate::handle::HandleError::Vacant { slot: row.slot() },
                )));
            }
        }
        self.store.with_title(row, f).map_err(ValueError::from)
    }

    /// Read the effective `completed` flag, staged value first.
    pub fn completed(&self, row: Handle<Row>) -> Result<bool, ValueError> {
        self.effective_completed(row)
    }

    pub fn read_once(&mut self, row: Handle<Row>) -> Result<String, ValueError> {
        let title = self.with_title(row, str::to_owned)?;
        self.store.note_read();
        Ok(title)
    }

    pub fn rows(&mut self, filter: Filter) -> RowSet {
        let keys: Vec<RowKey> = self
            .effective_rows()
            .into_iter()
            .filter(|(_, completed)| filter.matches(*completed))
            .map(|(key, _)| key)
            .collect();
        let node = self.nodes.for_filter(filter);
        let store_node = self.nodes.store;
        let revision = self.txn.staged_rev;

        let previous = self.txn.phase;
        self.txn.phase = Phase::Binding;
        let outcome = {
            let txn = &mut *self.txn;
            self.graph
                .compute_staged(node, &mut txn.graph, revision, |reader| {
                    reader.read(store_node);
                    Value::Rows(RowSet::from_keys(keys))
                })
        };
        self.txn.phase = previous;

        match outcome {
            Ok((Value::Rows(rows), recomputed)) => {
                if recomputed {
                    self.query.note_eval();
                }
                rows
            }
            _ => RowSet::empty(),
        }
    }

    pub fn remaining(&mut self) -> u32 {
        let remaining = self.effective_rows().iter().filter(|(_, c)| !c).count() as u32;
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

    pub fn has_completed(&mut self) -> bool {
        let flag = self
            .effective_rows()
            .iter()
            .any(|(_, completed)| *completed);
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

    pub fn row_views(&self) -> Vec<(ViewId, RowKey)> {
        self.views.row_views()
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

    pub fn unmount(&mut self, view: ViewId) {
        if self.txn.phase != Phase::Outside {
            return;
        }
        if !self.txn.view.unmounts.contains(&view) {
            self.txn.view.unmounts.push(view);
        }
    }

    pub fn reorder_views(&mut self, order: &[ViewId]) {
        if self.txn.phase == Phase::Outside {
            self.txn.view.order = Some(order.to_vec());
        }
    }

    pub fn focus(&mut self, view: ViewId) {
        if self.txn.phase != Phase::Outside {
            return;
        }
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
    }

    pub fn clear_focus(&mut self) {
        if self.txn.phase == Phase::Outside {
            self.txn.view.focus = Some(None);
        }
    }

    // ---- commit ----

    /// Apply the staged state with one swap and publish exactly once.
    pub fn commit(mut self) -> Result<(), Fatal> {
        if self.txn.depth != 1 {
            return Err(Fatal::Unbalanced {
                depth: self.txn.depth,
            });
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

        self.txn.phase = Phase::Commit;
        self.apply_model();
        self.apply_views();
        let revision = self.txn.staged_rev;
        let scratch = std::mem::take(&mut self.txn.graph);
        self.graph.commit_scratch(scratch, revision);

        self.txn.publishes.push(Publication {
            keys,
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
            let listed: BTreeSet<RowKey> = order.iter().copied().collect();
            if listed != live || listed.len() != order.len() {
                return Err(Fatal::OrderNotAPermutation);
            }
        }

        let mounts: Vec<ViewId> = self.txn.view.mounts.iter().map(|m| m.id).collect();
        self.layout
            .preflight(&mounts, &self.txn.view.unmounts)
            .map_err(|_| Fatal::LayoutMapInconsistent)?;

        Ok(())
    }

    fn apply_model(&mut self) {
        let base_writes = self.store.counters().io_writes;
        let base_epoch = self.store.epoch();

        // The final order is authoritative and already overlay-aware: it is the
        // committed order with staged inserts, removals and reorder applied.
        let final_order: Vec<RowKey> = self
            .effective_rows()
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        let journal = std::mem::take(&mut self.txn.model);

        for key in &journal.inserted {
            if let Some(staged) = journal.rows.get(key)
                && let Some(reservation) = journal.reserved.get(key)
            {
                let title = staged.title.clone().unwrap_or_default();
                let completed = staged.completed.unwrap_or(false);
                let _ = self
                    .store
                    .fill_reserved(reservation.slot, *key, title, completed);
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
                self.store
                    .apply_set(slot, staged.title.as_deref(), staged.completed);
            }
        }

        for key in &journal.removals {
            if let Some((slot, _)) = self.store.slot_for(*key) {
                self.store.apply_remove(slot);
            }
        }

        let slots: Vec<SlotIndex> = final_order
            .iter()
            .filter_map(|key| self.store.slot_for(*key).map(|(slot, _)| slot))
            .collect();
        self.store.apply_order(slots);

        let completed = self.store.rows().filter(|row| row.completed()).count();
        let epoch = if journal.writes > 0 {
            base_epoch + 1
        } else {
            base_epoch
        };
        self.store
            .apply_counters(completed, epoch, base_writes + journal.writes);
        self.store.set_next_key(journal.next_key);
        self.txn.model = ModelJournal::new(journal.next_key);
    }

    fn apply_views(&mut self) {
        let draft = std::mem::take(&mut self.txn.view);

        for view in &draft.unmounts {
            if let Some(node) = self.views.unmount(*view) {
                self.owners.dispose(node.owner);
            }
        }
        for (owner, parent) in &draft.owners {
            let inserted = self.owners.insert_staged(*owner, *parent);
            debug_assert!(inserted, "planned owner id must be free");
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
    }

    fn discard(&mut self) {
        // No user code, no panic: drop the stage and burn the reservations.
        let journal = std::mem::take(&mut self.txn.model);
        for reservation in journal.reserved.values() {
            self.store.release_reserved(reservation.slot);
        }
        self.query.invalidate();
        self.layout.discard();
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
        if let Some(key) = self.staged_key(row) {
            if self.txn.model.removals.contains(&key) {
                return Err(ValueError::Handle(InvalidHandle::from(
                    crate::handle::HandleError::Vacant { slot: row.slot() },
                )));
            }
            if let Some(staged) = self.txn.model.rows.get(&key)
                && let Some(completed) = staged.completed
            {
                return Ok(completed);
            }
        }
        self.store.completed_of(row).map_err(ValueError::from)
    }

    /// Final row set: committed rows with staged inserts, updates and removals
    /// applied, in the staged order when one exists.
    fn effective_rows(&self) -> Vec<(RowKey, bool)> {
        let mut rows: Vec<(RowKey, bool)> = self
            .store
            .rows()
            .map(|row| {
                let key = row.key();
                let completed = self
                    .txn
                    .model
                    .rows
                    .get(&key)
                    .and_then(|staged| staged.completed)
                    .unwrap_or_else(|| row.completed());
                (key, completed)
            })
            .filter(|(key, _)| !self.txn.model.removals.contains(key))
            .collect();

        for key in &self.txn.model.inserted {
            if self.txn.model.removals.contains(key) {
                continue;
            }
            let completed = self
                .txn
                .model
                .rows
                .get(key)
                .and_then(|staged| staged.completed)
                .unwrap_or(false);
            rows.push((*key, completed));
        }

        if let Some(order) = &self.txn.model.order {
            let mut ordered = Vec::with_capacity(rows.len());
            for key in order {
                if let Some(entry) = rows.iter().find(|(candidate, _)| candidate == key) {
                    ordered.push(*entry);
                }
            }
            return ordered;
        }
        rows
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
        result
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
