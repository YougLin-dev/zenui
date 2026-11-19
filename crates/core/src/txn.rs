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
    /// 不同阶段能接受的操作不同；用户写入在提交演算之前结束。
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PhaseViolation {
    pub phase: Phase,
    pub attempted: WriteKind,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fatal {
    PhaseViolation(PhaseViolation),
    PhaseOrder { expected: Phase, found: Phase },
    BatchNotAllowed { phase: Phase },
    Unbalanced { depth: u32 },

    StaleStagedRow { key: RowKey },

    OrderNotAPermutation,

    LayoutMapInconsistent,

    ViewOrderNotAPermutation,

    DuplicateViewKey { key: RowKey },

    MountForMissingRow { view: ViewId },

    FocusTargetNotLive { view: ViewId },

    InvalidViewHandle(InvalidHandle),

    GraphCycle { node: NodeId },

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
            Fatal::InvalidViewHandle(error) => write!(f, "invalid view handle: {error}"),
            Fatal::GraphCycle { node } => write!(f, "memo cycle at node {node}"),
            Fatal::Aborted => f.write_str("transaction aborted by the caller"),
        }
    }
}

impl std::error::Error for Fatal {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ValueError {
    Handle(InvalidHandle),
    Store(StoreError),

    Phase(PhaseViolation),

    Aborted,
}

impl From<InvalidHandle> for ValueError {
    fn from(error: InvalidHandle) -> Self {
        Self::Handle(error)
    }
}

impl ValueError {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Publication {
    pub keys: Vec<RowKey>,

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
    next_layout: u32,
    mounts: Vec<PlannedMount>,
    owners: Vec<(OwnerId, OwnerId)>,
    unmounts: Vec<ViewId>,
    order: Option<Vec<ViewId>>,
    focus: Option<Option<ViewId>>,
    caret: Option<(ViewId, usize)>,
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FocusSettlementCause {
    Unmount,
    BulkClear,
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FocusSettlementRule {
    Next,
    Previous,
    Cleared,
}

pub struct Txn {
    depth: u32,
    phase: Phase,
    outcome: Outcome,
    model: ModelJournal,
    graph: GraphScratch,
    view: ViewDraft,
    staged_rev: u64,

    staged_membership: [u64; 3],
    staged_order: u64,
    publishes: Vec<Publication>,
    commits: u64,

    focus_intents: Vec<FocusIntent>,

    user_focused: bool,

    #[cfg(test)]
    cleanup_log: Vec<(crate::handle::OwnerId, Phase, usize)>,

    #[cfg(test)]
    focus_settlements: Vec<(FocusSettlementCause, FocusSettlementRule, Option<ViewId>)>,

    #[cfg(test)]
    intent_causes: Vec<FocusSettlementCause>,

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

    pub(crate) fn begin_frame(
        &mut self,
        store: &Store,
        next_view: u32,
        next_owner: u32,
        next_layout: u32,
    ) {
        self.depth = 1;
        self.phase = Phase::Outside;
        self.outcome = Outcome::Live;
        self.poison = None;

        self.staged_membership = [
            store.epoch_for(Filter::All),
            store.epoch_for(Filter::Active),
            store.epoch_for(Filter::Completed),
        ];
        self.staged_order = store.order_epoch();
        self.view.next_view = next_view;
        self.view.next_owner = next_owner;
        self.view.next_layout = next_layout;
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

#[derive(Clone, Copy, Debug)]
pub struct Nodes {
    pub store: NodeId,
    pub remaining: NodeId,
    pub has_completed: NodeId,
}

impl Nodes {
    pub(crate) fn build(graph: &mut MemoGraph) -> Self {
        Self {
            store: graph.source("store"),
            remaining: graph.memo("remaining"),
            has_completed: graph.memo("has-completed"),
        }
    }
}

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

    #[cfg(test)]
    pub(crate) fn force_phase(&mut self, phase: Phase) {
        self.txn.force_phase(phase);
    }

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

    fn note_membership(&mut self, filters: &[Filter]) {
        for filter in filters {
            self.txn.staged_membership[filter_index(*filter)] += 1;
        }
    }

    fn note_order(&mut self) {
        self.txn.staged_order += 1;
    }

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

            self.note_order();
        }
        Ok(())
    }

    fn stage_completed(&mut self, row: Handle<Row>, value: bool) -> Result<(), ValueError> {
        self.require_staged(WriteKind::ModelWrite)?;
        let key = self.effective_key(row)?;

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

            self.note_membership(&[Filter::Active, Filter::Completed]);
        }
        Ok(())
    }

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

    pub fn completed(&self, row: Handle<Row>) -> Result<bool, ValueError> {
        self.effective_completed(row)
    }

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

    pub fn rows(&mut self, filter: Filter) -> RowSet {
        let mut view = StagedView::new(self.store, self.txn);
        self.query.rows_at(&mut view, filter)
    }

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

    pub fn root_owner(&self) -> OwnerId {
        self.owners.root()
    }

    pub fn row_views(&self) -> Vec<(ViewId, RowKey)> {
        let mut rows: Vec<(ViewId, RowKey)> = self
            .views
            .row_views()
            .into_iter()
            .filter(|(view, _)| !self.txn.view.unmounts.contains(view))
            .collect();
        for mount in &self.txn.view.mounts {
            if !self.txn.view.unmounts.contains(&mount.id)
                && !rows.iter().any(|(view, _)| *view == mount.id)
            {
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
        self.row_views()
            .into_iter()
            .find_map(|(view, key)| (key == row).then_some(view))
    }

    pub fn is_live(&self, view: ViewId) -> bool {
        self.require_view(view).is_ok()
            && self
                .row_views()
                .iter()
                .any(|(candidate, _)| *candidate == view)
    }

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

    fn require_view(&self, view: ViewId) -> Result<(), Fatal> {
        // 暂存 mount 尚不在已提交树中；丢弃事务后，未发布身份可从原 checkpoint 重用。
        if self.txn.view.mounts.iter().any(|mount| mount.id == view) {
            return Ok(());
        }
        self.views.require(view).map_err(Fatal::InvalidViewHandle)
    }

    pub fn focus_intent_for(&self, view: ViewId) -> Option<FocusIntent> {
        self.views.focus_intent_for(view)
    }

    pub fn focused(&self) -> Option<ViewId> {
        self.txn.view.focus.unwrap_or_else(|| self.views.focused())
    }

    pub fn caret(&self, view: ViewId) -> Option<usize> {
        self.require_view(view).ok()?;
        self.txn
            .view
            .caret
            .filter(|(planned, _)| *planned == view)
            .map(|(_, offset)| offset)
            .or_else(|| self.views.caret(view))
    }

    pub fn mount_row(&mut self, row: RowKey, title_len: u32) -> Result<ViewId, Fatal> {
        self.require(WriteKind::OwnerMount)?;

        let already_mounted = self
            .views
            .claiming_view(row)
            .is_some_and(|view| !self.txn.view.unmounts.contains(&view));
        if already_mounted || self.txn.view.mounts.iter().any(|mount| mount.row == row) {
            return Err(Fatal::DuplicateViewKey { key: row });
        }
        let view_id = self.views.mint(self.txn.view.next_view);
        self.txn.view.next_view = self.txn.view.next_view.checked_add(1).ok_or_else(|| {
            Fatal::InvalidViewHandle(
                crate::handle::HandleError::Exhausted {
                    slot: view_id.slot(),
                }
                .into(),
            )
        })?;
        let owner_id = OwnerId::from_index(self.txn.view.next_owner);
        let parent = self.owners.root();

        let draft = &mut self.txn.view;
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

    pub fn unmount(&mut self, view: ViewId) -> Result<(), Fatal> {
        self.require(WriteKind::ViewPatch)?;
        self.require_view(view)?;
        if let Some(position) = self
            .txn
            .view
            .mounts
            .iter()
            .position(|mount| mount.id == view)
        {
            let mount = self.txn.view.mounts.swap_remove(position);
            let owner = mount.owner;
            self.txn
                .view
                .owners
                .retain(|(candidate, _)| *candidate != owner);
            self.txn.view.unmounts.push(view);
        } else if !self.txn.view.unmounts.contains(&view) {
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
        for view in order {
            self.require_view(*view)?;
        }
        self.txn.view.order = Some(order.to_vec());
        Ok(())
    }

    pub fn focus(&mut self, view: ViewId) -> Result<(), Fatal> {
        self.require(WriteKind::FocusChange)?;
        self.require_view(view)?;
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

    /// 先在暂存视图校验全部行、焦点和布局关系，再一次性应用模型与控件树。
    pub fn commit(mut self) -> Result<(), Fatal> {
        if self.txn.depth != 1 {
            return Err(Fatal::Unbalanced {
                depth: self.txn.depth,
            });
        }
        if let Some(fatal) = self.txn.poison {
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

        let keys = self.touched_keys();

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

        draft.set_revisions(self.txn.staged_membership, self.txn.staged_order);

        self.store.install(draft);
        self.txn.model = ModelJournal::new(journal.next_key);
    }

    fn apply_views(&mut self) {
        let draft = std::mem::take(&mut self.txn.view);

        let mut disposals = Vec::new();
        for view in &draft.unmounts {
            if let Ok(Some(node)) = self.views.unmount(*view) {
                disposals.push(node.owner);
            }
        }
        for (owner, parent) in &draft.owners {
            let _ = self.owners.insert_staged(*owner, *parent);
        }
        for mount in &draft.mounts {
            self.views
                .mount_row_with_id(mount.id, mount.row, mount.owner, mount.title_len);
            self.owners
                .on_cleanup(mount.owner, Box::new(|| {}))
                .expect("staged mount requires a live owner");
        }
        if let Some(order) = &draft.order {
            self.views
                .reorder(order)
                .expect("order was validated before commit");
        }
        let focused = match draft.focus {
            Some(Some(view)) => {
                self.views
                    .focus(view)
                    .expect("focus target was validated before commit");
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
            self.views
                .set_caret(view, offset)
                .expect("caret target was validated before commit");
        }
        self.views.set_next(draft.next_view);
        self.owners.set_next(draft.next_owner);
        self.layout.commit(&draft.unmounts);

        if !self.txn.user_focused
            && focused.is_none()
            && let Some(intent) = self.txn.focus_intents.last().copied()
        {
            let next = intent.next.filter(|view| self.views.is_live(*view));
            let previous = intent.previous.filter(|view| self.views.is_live(*view));
            let target = next.or(previous);

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
                Some(view) => self.views.focus(view).expect("focused view was validated"),
                None => self.views.clear_focus(),
            };
        }

        if self
            .views
            .focused()
            .is_some_and(|view| !self.views.is_live(view))
        {
            self.views.clear_focus();
        }

        self.txn.phase = Phase::Cleanup;
        for owner in disposals {
            #[cfg(test)]
            self.txn
                .cleanup_log
                .push((owner, self.txn.phase, self.views.row_views().len()));
            self.owners.dispose(owner);
        }
        self.txn.phase = Phase::Commit;
    }

    fn discard(&mut self) {
        let next_key = self.txn.model.next_key;
        let journal = std::mem::take(&mut self.txn.model);
        for reservation in journal.reserved.values() {
            self.store.release_reserved(reservation.slot);
        }
        self.query.invalidate();
        self.layout.set_next(self.txn.view.next_layout);
        self.layout.discard();

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

    fn effective_rows(&self) -> Vec<(RowKey, bool)> {
        StagedView::new(self.store, self.txn).effective_rows()
    }

    pub(crate) fn abort(&mut self) {
        self.discard();
        self.txn.outcome = Outcome::Discarded;
    }

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
            self.txn.poison = Some(Fatal::Aborted);
        }
        result
    }
}

struct StagedView<'a> {
    store: &'a Store,
    journal: &'a ModelJournal,
    membership: [u64; 3],
    order: u64,

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

    fn order_revision(tx: &Tx<'_>) -> u64 {
        tx.txn.staged_order
    }

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

    #[test]
    fn settlement_records_next_for_a_live_successor() {
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

    #[test]
    fn settlement_records_previous_at_the_end() {
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

    #[test]
    fn bulk_clear_settles_one_intent() {
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

    #[test]
    fn a_user_focus_move_cancels_the_settlement() {
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
                assert!(tx.txn.user_focused, "explicit focus sets the frame marker");
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

    #[test]
    fn a_user_clear_cancels_the_settlement() {
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

    #[test]
    fn an_unordered_view_still_claims_its_key_without_being_unrecoverable() {
        let mut runtime = Runtime::new();
        runtime
            .frame(|tx| {
                tx.insert("A")?;
                Ok(())
            })
            .expect("seed");
        let key = runtime.committed().keys().next().expect("the seeded row");

        let view = runtime
            .frame(|tx| tx.mount_row(key, 1))
            .expect("a mount without a reorder");
        assert_eq!(
            runtime.row_views(),
            vec![(view, key)],
            "a mounted row stays observable when no explicit order is supplied"
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
                "the one visible view must still own its key, got {second:?}"
            );
            Err(Fatal::Aborted)
        });
        assert!(result.is_err(), "the frame was discarded");
        assert_eq!(
            runtime.live_owner_ids().len(),
            2,
            "the aborted frame minted no second owner"
        );

        runtime
            .frame(|tx| {
                assert!(tx.is_live(view), "the live row is staged as live");
                assert_eq!(tx.view_of_row(key), Some(view));
                tx.reorder_views(&[view])?;
                tx.focus(view)?;
                Ok(())
            })
            .expect("a subsequent reorder and focus accepts the mounted view");
        runtime
            .frame(|tx| tx.unmount(view))
            .expect("the mounted row can be removed");
        assert_eq!(runtime.row_views(), Vec::<(ViewId, RowKey)>::new());
        assert_eq!(runtime.live_owner_ids().len(), 1);
    }

    #[test]
    fn an_older_generation_cannot_use_a_live_view_through_runtime_or_tx() {
        let mut runtime = Runtime::new();
        runtime
            .frame(|tx| {
                tx.insert("A")?;
                Ok(())
            })
            .expect("seed");
        let key = runtime.committed().keys().next().expect("seeded key");
        let live = runtime.frame(|tx| tx.mount_row(key, 3)).expect("live view");
        let stale = ViewId::new(live.runtime(), live.slot(), live.generation().next());

        for result in [
            runtime.frame(|tx| tx.focus(stale)),
            runtime.frame(|tx| tx.unmount(stale)),
            runtime.frame(|tx| tx.reorder_views(&[stale])),
        ] {
            assert!(
                matches!(
                    result,
                    Err(Fatal::InvalidViewHandle(InvalidHandle {
                        reason: crate::handle::HandleError::StaleGeneration { .. },
                    }))
                ),
                "a different generation must be stale: {result:?}"
            );
        }
        assert_eq!(runtime.caret(stale), None);
        assert_eq!(runtime.layout_of(stale), None);
        assert_eq!(runtime.focused(), None);
        runtime
            .frame(|tx| tx.focus(live))
            .expect("the live identity remains usable");
        assert_eq!(runtime.caret(live), Some(3));
    }

    fn stage_mount(tx: &mut Tx<'_>, row: RowKey, title_len: u32) -> ViewId {
        let view = tx.views.mint(tx.txn.view.next_view);
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

    #[test]
    fn the_commit_preflight_sees_a_committed_view_outside_the_order() {
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

    #[test]
    fn settlement_skips_a_next_sibling_unmounted_in_the_same_frame() {
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

    #[test]
    fn a_bulk_clear_with_no_live_candidate_records_cleared() {
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
