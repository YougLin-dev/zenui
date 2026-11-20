use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::BTreeSet;
use std::fmt;

use crate::arena::Slots;
use crate::handle::{Generation, Handle, HandleError, InvalidHandle, RowKey, RuntimeId, SlotIndex};
use crate::metrics::Counters;
use crate::query::{Filter, RowView, filter_index};

pub type RowId = Handle<Row>;

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StoreError {
    Handle(InvalidHandle),

    UnknownRow(RowKey),

    DuplicateKey(RowKey),

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

pub struct Store {
    runtime: RuntimeId,
    rows: Slots<Row>,
    order: Vec<SlotIndex>,
    /// 业务键到 slot 的索引；行查找和事务提交不再线性扫描 arena。
    by_key: BTreeMap<RowKey, SlotIndex>,
    next_key: u64,
    epoch: u64,

    filter_epoch: [u64; 3],

    order_epoch: u64,
    completed: usize,
    io_reads: u64,
    io_writes: u64,
    string_clones: u64,

    scanned: u64,
}

#[derive(Debug, Default)]
pub(crate) struct StoreDraft {
    fills: Vec<(SlotIndex, RowKey, String, bool)>,
    updates: Vec<(SlotIndex, Option<String>, Option<bool>)>,
    removals: Vec<SlotIndex>,

    releases: Vec<SlotIndex>,
    order: Vec<SlotIndex>,
    completed: usize,
    next_key: u64,
    epoch: u64,
    io_writes: u64,

    filter_epochs: [u64; 3],
    order_epoch: u64,
}

impl StoreDraft {
    /// 暂存行修改、删除、排序、代际释放和版本推进，提交前不改变已发布 Store。
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

    pub(crate) fn set_revisions(&mut self, filter_epochs: [u64; 3], order_epoch: u64) {
        self.filter_epochs = filter_epochs;
        self.order_epoch = order_epoch;
    }

    pub(crate) fn set_completed_count(&mut self, completed: usize) {
        self.completed = completed;
    }
}

impl Store {
    pub(crate) fn with_runtime(id: RuntimeId) -> Self {
        Self {
            runtime: id,
            rows: Slots::new(),
            order: Vec::new(),
            by_key: BTreeMap::new(),
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
        self.by_key.insert(key, slot);
        self.record_write();
        self.touch_filters([true, true, true]);
        self.order_epoch += 1;
        Handle::new(self.runtime, slot, generation)
    }

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

    #[cfg(test)]
    pub(crate) fn set_completed(&mut self, row: RowId, value: bool) -> Result<(), InvalidHandle> {
        let was_completed = {
            let row = self.entry_mut(row)?;
            let previous = row.completed;
            row.completed = value;
            previous
        };
        if was_completed == value {
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

    #[cfg(test)]
    pub(crate) fn remove(&mut self, row: RowId) -> Result<Row, InvalidHandle> {
        let slot = row.slot();
        let was_completed = self.entry(row)?.completed;
        let removed = self.rows.free(slot).ok_or(HandleError::Vacant { slot })?;
        self.by_key.remove(&removed.key);
        self.order.retain(|candidate| *candidate != slot);
        if was_completed {
            self.completed -= 1;
        }
        self.record_write();
        self.touch_filters([true, true, true]);
        self.order_epoch += 1;
        Ok(removed)
    }

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
                self.by_key.remove(&row.key);
                removed.push(row.key);
                self.completed -= 1;
            }
        }
        self.order.retain(|slot| self.rows.get_ref(*slot).is_some());
        if !removed.is_empty() {
            self.record_write();
            self.touch_filters([true, true, true]);
            self.order_epoch += 1;
        }
        removed
    }

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
            return Ok(());
        }
        self.order = slots;
        self.record_write();

        self.order_epoch += 1;
        Ok(())
    }

    pub fn with_title<R>(&self, row: RowId, f: impl FnOnce(&str) -> R) -> Result<R, InvalidHandle> {
        Ok(f(self.entry(row)?.title()))
    }

    #[cfg(test)]
    pub(crate) fn read_owned_title(&mut self, row: RowId) -> Result<String, InvalidHandle> {
        let title = self.entry(row)?.title.clone();
        self.io_reads += 1;
        self.string_clones += 1;
        Ok(title)
    }

    pub(crate) fn reserve_slot(&mut self) -> (SlotIndex, Generation) {
        self.rows.reserve()
    }

    pub(crate) const fn next_key(&self) -> u64 {
        self.next_key
    }

    pub(crate) fn release_reserved(&mut self, slot: SlotIndex) {
        self.rows.release(slot);
    }

    pub(crate) fn slot_for(&self, key: RowKey) -> Option<(SlotIndex, Generation)> {
        let slot = *self.by_key.get(&key)?;
        Some((slot, self.rows.generation(slot)?))
    }

    pub(crate) fn install(&mut self, draft: StoreDraft) {
        for (slot, key, title, completed) in draft.fills {
            if self.rows.fill(
                slot,
                Row {
                    key,
                    title,
                    completed,
                },
            ) {
                self.by_key.insert(key, slot);
            }
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
            if let Some(row) = self.rows.free(slot) {
                self.by_key.remove(&row.key);
            }
        }
        // releases 只包含从未填充过的 reservation，因此不影响 by_key。
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

    pub const fn epoch_for(&self, filter: Filter) -> u64 {
        self.filter_epoch[filter_index(filter)]
    }

    pub(crate) const fn order_epoch(&self) -> u64 {
        self.order_epoch
    }

    pub const fn is_empty_for(&self, filter: Filter) -> bool {
        match filter {
            Filter::All => self.order.is_empty(),
            Filter::Active => self.completed == self.order.len(),
            Filter::Completed => self.completed == 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn touch_filters(&mut self, mask: [bool; 3]) {
        for (index, dirty) in mask.iter().enumerate() {
            if *dirty {
                self.filter_epoch[index] += 1;
            }
        }
    }

    pub(crate) fn completed_at(&self, slot: SlotIndex) -> Option<bool> {
        self.rows.get_ref(slot).map(Row::completed)
    }

    pub(crate) fn key_of(&self, row: Handle<Row>) -> Option<RowKey> {
        self.entry(row).ok().map(Row::key)
    }

    pub(crate) fn completed_of(&self, row: Handle<Row>) -> Result<bool, InvalidHandle> {
        Ok(self.entry(row)?.completed())
    }

    pub(crate) fn note_read(&mut self) {
        self.io_reads += 1;
        self.string_clones += 1;
    }

    pub fn handle_for(&self, key: RowKey) -> Option<RowId> {
        let slot = *self.by_key.get(&key)?;
        let generation = self.rows.generation(slot)?;
        Some(Handle::new(self.runtime, slot, generation))
    }

    pub fn rows(&self) -> impl Iterator<Item = &Row> + '_ {
        self.order
            .iter()
            .filter_map(|slot| self.rows.get_ref(*slot))
    }

    pub fn keys(&self) -> impl Iterator<Item = RowKey> + '_ {
        self.rows().map(Row::key)
    }

    pub fn titles(&self) -> impl Iterator<Item = &str> + '_ {
        self.rows().map(Row::title)
    }

    pub const fn len(&self) -> usize {
        self.order.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub const fn remaining(&self) -> usize {
        self.order.len() - self.completed
    }

    pub const fn has_completed(&self) -> bool {
        self.completed > 0
    }

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

    /// 先拒绝来自其它 Runtime 的句柄，再按 slot 和代际读取；不能误取复用位置的新行。
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

impl RowView for Store {
    fn rows_for(&mut self, filter: Filter) -> Vec<(RowKey, bool)> {
        let rows: Vec<(RowKey, bool)> = self
            .rows()
            .map(|row| (row.key(), row.completed()))
            .collect();

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

        store.set_completed(a, false).expect("live row");
        assert_eq!(store.counters().io_writes, 3, "no-op set_completed");

        let keys: Vec<RowKey> = store.keys().collect();
        store.reorder(&keys).expect("same order");
        assert_eq!(store.counters().io_writes, 3, "unchanged order");

        store.reorder(&[keys[1], keys[0]]).expect("swap two rows");
        assert_eq!(store.counters().io_writes, 4);

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
