// crates/core/src/store.rs
//! The model authority: rows and their fields (`SPEC-RT-040`).
//!
//! The store owns model entities only. It never renders, never diffs a second
//! UI tree, and never creates views.

#[cfg(test)]
use std::collections::BTreeSet;
use std::fmt;

use crate::arena::Slots;
use crate::handle::{Generation, Handle, HandleError, InvalidHandle, RowKey, RuntimeId, SlotIndex};
use crate::metrics::Counters;
use crate::query::{Filter, RowView, filter_index};

/// Generation-checked handle to one store row (`SPEC-RT-005`).
pub type RowId = Handle<Row>;

/// One model entity. Titles are owned by the store, never copied into a
/// `Copy` row struct (`WITHDRAWN-RT-04`, `SPEC-RT-045`).
#[derive(Debug)]
pub struct Row {
    key: RowKey,
    title: String,
    completed: bool,
}

impl Row {
    pub const fn key(&self) -> RowKey {
        self.key
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub const fn completed(&self) -> bool {
        self.completed
    }
}

/// Store operation that needs more context than a bare handle failure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StoreError {
    /// A validated row operation failed.
    Handle(InvalidHandle),
    /// Reorder referenced a key that no longer exists.
    UnknownRow(RowKey),
    /// Reorder listed the same key twice (`SPEC-RT-024`: reject before patching).
    DuplicateKey(RowKey),
    /// Reorder did not list every live row.
    IncompleteOrder,
}

impl From<InvalidHandle> for StoreError {
    fn from(error: InvalidHandle) -> Self {
        Self::Handle(error)
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Handle(error) => write!(f, "{error}"),
            StoreError::UnknownRow(key) => write!(f, "unknown row key {}", key.raw()),
            StoreError::DuplicateKey(key) => write!(f, "duplicate row key {}", key.raw()),
            StoreError::IncompleteOrder => f.write_str("reorder must list every live row"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Row storage plus the display order.
///
/// P1 keeps the display order inside the store because the scenario reorders
/// rows as a model operation. Whether ordering belongs to the store or to the
/// `For` adapter is a P3 question (`SPEC-GATE-A0-09`) and is not settled here.
pub struct Store {
    runtime: RuntimeId,
    rows: Slots<Row>,
    order: Vec<SlotIndex>,
    next_key: u64,
    epoch: u64,
    /// Per-filter membership epochs (`SPEC-GATE-A0-07`). A write that cannot
    /// change a filter's membership set must not move its epoch; the set, not
    /// the display order, is what this counter tracks.
    filter_epoch: [u64; 3],
    /// Display-order epoch. Independent of membership: a `reorder` moves the
    /// order and nothing else, so a cache keyed on membership alone would
    /// serve the old order forever.
    order_epoch: u64,
    completed: usize,
    io_reads: u64,
    io_writes: u64,
    string_clones: u64,
    /// Rows enumerated through [`RowView::rows_for`]. Not part of `Counters`:
    /// it is the view's own scan count, which the query cache turns into
    /// `row_visits` (`ADR-0022` decision 2).
    scanned: u64,
}

/// The whole model as it will look once this commit lands (`SPEC-RT-052`).
///
/// The draft records target values, never inverse operations (`SPEC-RT-051`).
/// Building it is where every fallible slot lookup happens; `Store::install`
/// then performs a single swap.
#[derive(Debug, Default)]
pub(crate) struct StoreDraft {
    fills: Vec<(SlotIndex, RowKey, String, bool)>,
    updates: Vec<(SlotIndex, Option<String>, Option<bool>)>,
    removals: Vec<SlotIndex>,
    /// Reservations cancelled before they were ever filled.
    releases: Vec<SlotIndex>,
    order: Vec<SlotIndex>,
    completed: usize,
    next_key: u64,
    epoch: u64,
    io_writes: u64,
    /// Absolute cache revisions this commit installs (`SPEC-GATE-A0-07`).
    /// Target values, not deltas: the transaction advanced them while it was
    /// staging, and the swap adopts the transaction's last state, which is the
    /// state the frame's cache was already keyed on.
    filter_epochs: [u64; 3],
    order_epoch: u64,
}

impl StoreDraft {
    pub(crate) fn new(next_key: u64, epoch: u64, io_writes: u64) -> Self {
        Self {
            next_key,
            epoch,
            io_writes,
            ..Self::default()
        }
    }

    pub(crate) fn fill(&mut self, slot: SlotIndex, key: RowKey, title: String, completed: bool) {
        self.fills.push((slot, key, title, completed));
    }

    pub(crate) fn update(
        &mut self,
        slot: SlotIndex,
        title: Option<String>,
        completed: Option<bool>,
    ) {
        self.updates.push((slot, title, completed));
    }

    pub(crate) fn remove(&mut self, slot: SlotIndex) {
        self.removals.push(slot);
    }

    pub(crate) fn release(&mut self, slot: SlotIndex) {
        self.releases.push(slot);
    }

    pub(crate) fn set_order(&mut self, order: Vec<SlotIndex>) {
        self.order = order;
    }

    /// The cache revisions this commit installs.
    pub(crate) fn set_revisions(&mut self, filter_epochs: [u64; 3], order_epoch: u64) {
        self.filter_epochs = filter_epochs;
        self.order_epoch = order_epoch;
    }

    pub(crate) fn set_completed_count(&mut self, completed: usize) {
        self.completed = completed;
    }
}

impl Store {
    /// Build a store owned by `id`.
    ///
    /// `pub(crate)` on purpose: a public constructor taking a `Copy` id would
    /// let two stores share one id and defeat cross-runtime rejection. The only
    /// public minting point is `Runtime::new` (`SPEC-RT-005`).
    pub(crate) fn with_runtime(id: RuntimeId) -> Self {
        Self {
            runtime: id,
            rows: Slots::new(),
            order: Vec::new(),
            next_key: 1,
            epoch: 0,
            filter_epoch: [0; 3],
            order_epoch: 0,
            completed: 0,
            io_reads: 0,
            io_writes: 0,
            string_clones: 0,
            scanned: 0,
        }
    }

    /// Insert a row at the end of the display order.
    #[cfg(test)]
    pub(crate) fn insert(&mut self, title: impl Into<String>) -> RowId {
        let key = RowKey::from_raw(self.next_key);
        self.next_key += 1;
        let row = Row {
            key,
            title: title.into(),
            completed: false,
        };
        let (slot, generation) = self.rows.reserve();
        assert!(self.rows.fill(slot, row), "fresh reservation must be empty");
        self.order.push(slot);
        self.record_write();
        self.touch_filters([true, true, true]);
        self.order_epoch += 1;
        Handle::new(self.runtime, slot, generation)
    }

    /// Flip `completed` and return the new value.
    #[cfg(test)]
    pub(crate) fn toggle(&mut self, row: RowId) -> Result<bool, InvalidHandle> {
        let completed = {
            let row = self.entry_mut(row)?;
            row.completed = !row.completed;
            row.completed
        };
        if completed {
            self.completed += 1;
        } else {
            self.completed -= 1;
        }
        self.record_write();
        self.touch_filters([false, true, true]);
        Ok(completed)
    }

    /// Set `completed` explicitly.
    #[cfg(test)]
    pub(crate) fn set_completed(&mut self, row: RowId, value: bool) -> Result<(), InvalidHandle> {
        let was_completed = {
            let row = self.entry_mut(row)?;
            let previous = row.completed;
            row.completed = value;
            previous
        };
        if was_completed == value {
            // No model change: not a write, and the epoch stays put
            // (`metrics::Counters::io_writes`).
            return Ok(());
        }
        if value {
            self.completed += 1;
        } else {
            self.completed -= 1;
        }
        self.record_write();
        self.touch_filters([false, true, true]);
        Ok(())
    }

    /// Remove one row and return it.
    #[cfg(test)]
    pub(crate) fn remove(&mut self, row: RowId) -> Result<Row, InvalidHandle> {
        let slot = row.slot();
        let was_completed = self.entry(row)?.completed;
        let removed = self.rows.free(slot).ok_or(HandleError::Vacant { slot })?;
        self.order.retain(|candidate| *candidate != slot);
        if was_completed {
            self.completed -= 1;
        }
        self.record_write();
        self.touch_filters([true, true, true]);
        self.order_epoch += 1;
        Ok(removed)
    }

    /// Remove every completed row in one write.
    ///
    /// Focus is not this helper's business: `Tx::clear_completed` records
    /// exactly one `FocusRemovalIntent` before the rows go, and the commit
    /// settles it once (`SPEC-GATE-A0-10`). This is the slot-table half, used
    /// by the store's own tests.
    #[cfg(test)]
    pub(crate) fn clear_completed(&mut self) -> Vec<RowKey> {
        let doomed: Vec<SlotIndex> = self
            .order
            .iter()
            .copied()
            .filter(|slot| self.rows.get_ref(*slot).is_some_and(|row| row.completed))
            .collect();

        let mut removed = Vec::with_capacity(doomed.len());
        for slot in doomed {
            if let Some(row) = self.rows.free(slot) {
                removed.push(row.key);
                self.completed -= 1;
            }
        }
        self.order.retain(|slot| self.rows.get_ref(*slot).is_some());
        if !removed.is_empty() {
            // One `clear_completed` call is one write, however many rows it
            // removed (`metrics::Counters::io_writes`).
            self.record_write();
            self.touch_filters([true, true, true]);
            self.order_epoch += 1;
        }
        removed
    }

    /// Reorder the display list. Every live row must appear exactly once.
    #[cfg(test)]
    pub(crate) fn reorder(&mut self, keys: &[RowKey]) -> Result<(), StoreError> {
        let mut seen = BTreeSet::new();
        let mut slots = Vec::with_capacity(keys.len());
        for key in keys {
            if !seen.insert(*key) {
                return Err(StoreError::DuplicateKey(*key));
            }
            let handle = self.handle_for(*key).ok_or(StoreError::UnknownRow(*key))?;
            slots.push(handle.slot());
        }
        if slots.len() != self.order.len() {
            return Err(StoreError::IncompleteOrder);
        }
        if slots == self.order {
            // No model change, so no write and no epoch advance.
            return Ok(());
        }
        self.order = slots;
        self.record_write();
        // Membership is untouched: only the display order moved.
        self.order_epoch += 1;
        Ok(())
    }

    /// Borrow a title without an owned read. Does not bump `io_reads`.
    pub fn with_title<R>(&self, row: RowId, f: impl FnOnce(&str) -> R) -> Result<R, InvalidHandle> {
        Ok(f(self.entry(row)?.title()))
    }

    /// Owned title read over the committed store (`SPEC-RT-045`).
    ///
    /// The runtime entry point is `Tx::read_once`, returning
    /// `Result<String, InvalidHandle>`, and the mount executor cancels the
    /// mount when it fails (`zenui-app::mount_row`, pinned in
    /// `crates/app/tests/mount_path.rs`). This helper is the store-level
    /// equivalent the unit tests use, accounting the same I/O and clone.
    #[cfg(test)]
    pub(crate) fn read_owned_title(&mut self, row: RowId) -> Result<String, InvalidHandle> {
        let title = self.entry(row)?.title.clone();
        self.io_reads += 1;
        self.string_clones += 1;
        Ok(title)
    }

    // ---- transaction support (`pub(crate)`, `SPEC-RT-051`) ----

    /// Reserve a slot for a row that a transaction is about to insert.
    pub(crate) fn reserve_slot(&mut self) -> (SlotIndex, Generation) {
        self.rows.reserve()
    }

    pub(crate) const fn next_key(&self) -> u64 {
        self.next_key
    }

    /// Drop a reservation and burn its generation (`SPEC-RT-053`).
    pub(crate) fn release_reserved(&mut self, slot: SlotIndex) {
        self.rows.release(slot);
    }

    /// Slot lookup that does not depend on the order vector, so it works while
    /// a commit is filling freshly reserved slots.
    pub(crate) fn slot_for(&self, key: RowKey) -> Option<(SlotIndex, Generation)> {
        let slot = self.rows.find(|row| row.key() == key)?;
        Some((slot, self.rows.generation(slot)?))
    }

    /// Swap a finished model draft in (`SPEC-RT-052`).
    ///
    /// Infallible by construction: every lookup that can fail happened while
    /// the draft was being built, so a commit cannot stop half way.
    pub(crate) fn install(&mut self, draft: StoreDraft) {
        for (slot, key, title, completed) in draft.fills {
            let _ = self.rows.fill(
                slot,
                Row {
                    key,
                    title,
                    completed,
                },
            );
        }
        for (slot, title, completed) in draft.updates {
            if let Some(generation) = self.rows.generation(slot)
                && let Ok(row) = self.rows.get_mut(slot, generation)
            {
                if let Some(title) = title {
                    row.title = title;
                }
                if let Some(completed) = completed {
                    row.completed = completed;
                }
            }
        }
        for slot in draft.removals {
            let _ = self.rows.free(slot);
        }
        for slot in draft.releases {
            self.rows.release(slot);
        }
        self.order = draft.order;
        self.completed = draft.completed;
        self.next_key = draft.next_key;
        self.epoch = draft.epoch;
        self.io_writes = draft.io_writes;
        self.filter_epoch = draft.filter_epochs;
        self.order_epoch = draft.order_epoch;
    }

    /// The membership epoch of one filter (`SPEC-GATE-A0-07`).
    pub const fn epoch_for(&self, filter: Filter) -> u64 {
        self.filter_epoch[filter_index(filter)]
    }

    /// The display-order epoch (`SPEC-GATE-A0-07`: the reverse edge is
    /// `(row, completed)` for membership and the row list for order, so a
    /// reorder has to move a revision of its own).
    pub(crate) const fn order_epoch(&self) -> u64 {
        self.order_epoch
    }

    /// True when the filter matches no row, answered from the membership
    /// counters without scanning (`SPEC-GATE-A0-08`).
    pub const fn is_empty_for(&self, filter: Filter) -> bool {
        match filter {
            Filter::All => self.order.is_empty(),
            Filter::Active => self.completed == self.order.len(),
            Filter::Completed => self.completed == 0,
        }
    }

    /// Advance one membership epoch per changed filter. The committed path
    /// installs absolute revisions instead (`StoreDraft::set_revisions`); this
    /// helper exists for the direct mutations the unit tests drive.
    #[cfg(test)]
    pub(crate) fn touch_filters(&mut self, mask: [bool; 3]) {
        for (index, dirty) in mask.iter().enumerate() {
            if *dirty {
                self.filter_epoch[index] += 1;
            }
        }
    }

    /// Committed `completed` flag at a slot, without going through a handle.
    pub(crate) fn completed_at(&self, slot: SlotIndex) -> Option<bool> {
        self.rows.get_ref(slot).map(Row::completed)
    }

    pub(crate) fn key_of(&self, row: Handle<Row>) -> Option<RowKey> {
        self.entry(row).ok().map(Row::key)
    }

    pub(crate) fn completed_of(&self, row: Handle<Row>) -> Result<bool, InvalidHandle> {
        Ok(self.entry(row)?.completed())
    }

    /// Count one owned title read. Staged rows do not live in the slot table,
    /// so the transaction has to account for their reads here.
    pub(crate) fn note_read(&mut self) {
        self.io_reads += 1;
        self.string_clones += 1;
    }

    /// Resolve a stable key back to a live handle.
    ///
    /// Still a linear scan, and not on the empty-state path: those answers come
    /// from the membership counters (`is_empty_for`, and the staged view's
    /// `live_counts`), which is what `SPEC-RT-044` requires. A key index would
    /// be a lookup optimisation, not a gate requirement.
    pub fn handle_for(&self, key: RowKey) -> Option<RowId> {
        self.order.iter().find_map(|slot| {
            let row = self.rows.get_ref(*slot)?;
            if row.key == key {
                let generation = self.rows.generation(*slot)?;
                Some(Handle::new(self.runtime, *slot, generation))
            } else {
                None
            }
        })
    }

    /// Live rows in display order.
    pub fn rows(&self) -> impl Iterator<Item = &Row> + '_ {
        self.order
            .iter()
            .filter_map(|slot| self.rows.get_ref(*slot))
    }

    /// Stable keys in display order.
    pub fn keys(&self) -> impl Iterator<Item = RowKey> + '_ {
        self.rows().map(Row::key)
    }

    /// Titles in display order, for the trace's state line.
    pub fn titles(&self) -> impl Iterator<Item = &str> + '_ {
        self.rows().map(Row::title)
    }

    pub const fn len(&self) -> usize {
        self.order.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// `len - completed`, maintained as a counter rather than recomputed by a
    /// scan (`SPEC-RT-044`, `SPEC-RT-046`).
    pub const fn remaining(&self) -> usize {
        self.order.len() - self.completed
    }

    pub const fn has_completed(&self) -> bool {
        self.completed > 0
    }

    /// Identity of the runtime that owns this store.
    pub const fn runtime_id(&self) -> RuntimeId {
        self.runtime
    }

    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn counters(&self) -> Counters {
        Counters {
            io_reads: self.io_reads,
            io_writes: self.io_writes,
            string_clones: self.string_clones,
            ..Counters::default()
        }
    }

    /// Record one committed model mutation: exactly one epoch advance and one
    /// write for one successful mutating call (`metrics::Counters::io_writes`).
    #[cfg(test)]
    fn record_write(&mut self) {
        self.epoch += 1;
        self.io_writes += 1;
    }

    fn entry(&self, row: RowId) -> Result<&Row, InvalidHandle> {
        self.check_runtime(row)?;
        self.rows
            .get(row.slot(), row.generation())
            .map_err(InvalidHandle::from)
    }

    #[cfg(test)]
    fn entry_mut(&mut self, row: RowId) -> Result<&mut Row, InvalidHandle> {
        self.check_runtime(row)?;
        self.rows
            .get_mut(row.slot(), row.generation())
            .map_err(InvalidHandle::from)
    }

    /// Reject a handle minted by another runtime.
    ///
    /// One slice of `SPEC-GATE-A0-03`. The rest is pinned elsewhere: a stale
    /// handle dispatched after slot reuse and a cross-runtime access are
    /// fixtures in `crates/core/tests/gates.rs`, and the identity domains
    /// cannot be passed to each other by construction — six `E0308` cases in
    /// `crates/core/tests/ui/`.
    fn check_runtime(&self, row: RowId) -> Result<(), InvalidHandle> {
        if row.runtime() != self.runtime {
            return Err(InvalidHandle::from(HandleError::ForeignRuntime {
                expected: self.runtime,
                found: row.runtime(),
            }));
        }
        Ok(())
    }
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store")
            .field("rows", &self.len())
            .field("completed", &self.completed)
            .field("epoch", &self.epoch)
            .finish()
    }
}

/// The committed model as a query sees it.
///
/// The frame-side overlay is the other implementation. Both expose the same
/// two revisions, so the cache cannot tell a committed read from a staged one —
/// and, more importantly, it cannot quietly serve one for the other unless the
/// revisions really are equal.
impl RowView for Store {
    fn rows_for(&mut self, filter: Filter) -> Vec<(RowKey, bool)> {
        let rows: Vec<(RowKey, bool)> = self
            .rows()
            .map(|row| (row.key(), row.completed()))
            .collect();
        // The filter is applied after the enumeration, so the scan cost is the
        // whole model, not the size of the answer.
        self.scanned += rows.len() as u64;
        rows.into_iter()
            .filter(|(_, completed)| filter.matches(*completed))
            .collect()
    }

    fn is_empty_for(&mut self, filter: Filter) -> bool {
        Store::is_empty_for(self, filter)
    }

    fn membership_revision(&self, filter: Filter) -> u64 {
        self.epoch_for(filter)
    }

    fn order_revision(&self) -> u64 {
        self.order_epoch
    }

    fn scan_cost(&self) -> u64 {
        self.scanned
    }
}

#[cfg(test)]
impl Store {
    /// Test-only store with its own runtime identity.
    pub(crate) fn for_test() -> Self {
        Self::with_runtime(RuntimeId::allocate())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_row_rejects_its_old_handle() {
        let mut store = Store::for_test();
        let row = store.insert("A");
        assert!(store.with_title(row, str::len).is_ok());

        store.remove(row).expect("row exists");
        let error = store.with_title(row, str::len).expect_err("stale handle");
        assert!(matches!(
            error.reason,
            HandleError::Vacant { .. } | HandleError::StaleGeneration { .. }
        ));
    }

    #[test]
    fn remaining_and_has_completed_follow_toggle() {
        let mut store = Store::for_test();
        let a = store.insert("A");
        let b = store.insert("B");
        assert_eq!(store.remaining(), 2);

        store.toggle(b).expect("live row");
        assert_eq!(store.remaining(), 1);
        assert!(store.has_completed());

        store.toggle(b).expect("live row");
        assert_eq!(store.remaining(), 2);
        assert!(!store.has_completed());
        assert_eq!(store.len(), 2);
        let _ = a;
    }

    #[test]
    fn reorder_rejects_duplicate_keys_before_mutating() {
        let mut store = Store::for_test();
        store.insert("A");
        store.insert("B");
        let keys: Vec<RowKey> = store.keys().collect();

        let error = store
            .reorder(&[keys[1], keys[1]])
            .expect_err("duplicate key");
        assert!(matches!(error, StoreError::DuplicateKey(_)));
        assert_eq!(store.keys().collect::<Vec<_>>(), keys);

        store.reorder(&[keys[1], keys[0]]).expect("valid order");
        assert_eq!(store.keys().collect::<Vec<_>>(), vec![keys[1], keys[0]]);
    }

    #[test]
    fn clear_completed_removes_only_completed_rows() {
        let mut store = Store::for_test();
        let a = store.insert("A");
        let b = store.insert("B");
        store.toggle(b).expect("live row");

        let removed = store.clear_completed();
        assert_eq!(removed.len(), 1);
        assert_eq!(store.len(), 1);
        assert_eq!(store.titles().collect::<Vec<_>>(), vec!["A"]);
        assert!(store.with_title(a, str::len).is_ok());
    }

    #[test]
    fn handle_from_another_store_is_rejected() {
        let mut first = Store::for_test();
        let second = Store::for_test();
        let row = first.insert("A");

        let error = second
            .with_title(row, str::len)
            .expect_err("a handle from another runtime must not resolve");
        assert!(matches!(error.reason, HandleError::ForeignRuntime { .. }));
        assert_eq!(error.reason_kind(), "foreign_runtime");

        // The owning store still accepts the same handle.
        assert_eq!(first.with_title(row, str::len).expect("own handle"), 1);
    }

    #[test]
    fn one_mutation_is_one_write() {
        let mut store = Store::for_test();
        let a = store.insert("A");
        let b = store.insert("B");
        assert_eq!(store.counters().io_writes, 2);

        store.toggle(b).expect("live row");
        assert_eq!(store.counters().io_writes, 3);

        // Setting a field to the value it already holds is not a change.
        store.set_completed(a, false).expect("live row");
        assert_eq!(store.counters().io_writes, 3, "no-op set_completed");

        // Reordering a non-empty store to its current order changes nothing.
        let keys: Vec<RowKey> = store.keys().collect();
        store.reorder(&keys).expect("same order");
        assert_eq!(store.counters().io_writes, 3, "unchanged order");

        // A real reorder is one write.
        store.reorder(&[keys[1], keys[0]]).expect("swap two rows");
        assert_eq!(store.counters().io_writes, 4);

        // Clearing two completed rows is still one write.
        store.toggle(a).expect("live row");
        let before = store.counters().io_writes;
        assert_eq!(store.clear_completed().len(), 2);
        assert_eq!(store.counters().io_writes, before + 1);
    }

    #[test]
    fn set_completed_updates_counters_in_both_directions() {
        let mut store = Store::for_test();
        let row = store.insert("A");
        assert_eq!(store.remaining(), 1);
        assert!(!store.has_completed());

        store.set_completed(row, true).expect("live row");
        assert_eq!(store.remaining(), 0);
        assert!(store.has_completed());
        assert_eq!(store.counters().io_writes, 2);

        // Repeating the same value is not a change.
        store.set_completed(row, true).expect("live row");
        assert_eq!(store.counters().io_writes, 2);

        store.set_completed(row, false).expect("live row");
        assert_eq!(store.remaining(), 1);
        assert!(!store.has_completed());
        assert_eq!(store.counters().io_writes, 3);
    }

    #[test]
    fn write_through_a_foreign_handle_is_rejected() {
        let mut first = Store::for_test();
        let mut second = Store::for_test();
        let row = first.insert("A");

        let error = second
            .toggle(row)
            .expect_err("a foreign handle must not write");
        assert!(matches!(error.reason, HandleError::ForeignRuntime { .. }));
        assert_eq!(second.len(), 0, "the foreign store is untouched");
        assert_eq!(first.remaining(), 1, "the owning store is unchanged");

        assert!(first.toggle(row).expect("own handle"));
    }

    #[test]
    fn removing_a_completed_row_decrements_the_counter() {
        let mut store = Store::for_test();
        let a = store.insert("A");
        let b = store.insert("B");
        store.set_completed(a, true).expect("live row");
        assert!(store.has_completed());
        assert_eq!(store.remaining(), 1);

        store.remove(a).expect("live row");
        assert!(!store.has_completed());
        assert_eq!(store.remaining(), 1, "B is still active");
        assert_eq!(store.len(), 1);
        let _ = b;
    }

    #[test]
    fn owned_title_read_counts_once() {
        let mut store = Store::for_test();
        let row = store.insert("A");
        let title = store.read_owned_title(row).expect("live row");
        assert_eq!(title, "A");

        let counters = store.counters();
        assert_eq!(counters.io_reads, 1);
        assert_eq!(counters.string_clones, 1);
    }
}
