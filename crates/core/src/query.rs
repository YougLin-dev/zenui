// crates/core/src/query.rs
//! Query cache and the shared `RowSet` snapshot (`SPEC-RT-041` … `-045`).
//!
//! P1 implements the shape and the sharing, not the full wiring: membership
//! candidates and reverse field edges are P3 work and are marked below.

use std::fmt;
use std::sync::Arc;

use crate::handle::RowKey;
use crate::metrics::Counters;
use crate::store::Store;

/// Which rows a list shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Filter {
    All,
    Active,
    Completed,
}

impl Filter {
    pub const fn as_str(self) -> &'static str {
        match self {
            Filter::All => "all",
            Filter::Active => "active",
            Filter::Completed => "completed",
        }
    }

    pub const fn matches(self, completed: bool) -> bool {
        match self {
            Filter::All => true,
            Filter::Active => !completed,
            Filter::Completed => completed,
        }
    }
}

/// Immutable, shared, ordered set of stable row keys.
///
/// Contains no owned titles (`SPEC-RT-045`); clones share one allocation
/// (`SPEC-RT-043`).
#[derive(Clone, PartialEq, Eq)]
pub struct RowSet {
    ids: Arc<[RowKey]>,
}

impl RowSet {
    pub fn from_keys(keys: Vec<RowKey>) -> Self {
        Self {
            ids: Arc::from(keys.into_boxed_slice()),
        }
    }

    pub fn empty() -> Self {
        Self {
            ids: Arc::from(Vec::new().into_boxed_slice()),
        }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// True emptiness from the snapshot length, never a collect-and-check.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = RowKey> + '_ {
        self.ids.iter().copied()
    }

    /// True when both snapshots share one allocation (`SPEC-RT-043`).
    pub fn shares_with(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.ids, &other.ids)
    }

    pub fn as_slice(&self) -> &[RowKey] {
        &self.ids
    }
}

impl fmt::Debug for RowSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.ids.iter()).finish()
    }
}

/// Per-`(store_epoch, filter)` query cache.
///
/// Each filter owns one slot keyed on the *membership* epoch of that filter
/// (`Store::epoch_for`), not on the global model epoch: a title write or a
/// `reorder` leaves every slot fresh, a `completed` write only touches
/// `Active`/`Completed`, and only insert/remove/clear touch all three. That is
/// the `(row, completed)` reverse edge collapsed to the filters that actually
/// read the field — every membership predicate in this crate reads exactly
/// that field and nothing else.
pub struct QueryCache {
    slots: [Slot; 3],
    evals: u64,
    builds: u64,
    materializations: u64,
    visits: u64,
}

#[derive(Clone, Default)]
struct Slot {
    epoch: Option<u64>,
    cached: Option<RowSet>,
}

pub(crate) const fn filter_index(filter: Filter) -> usize {
    match filter {
        Filter::All => 0,
        Filter::Active => 1,
        Filter::Completed => 2,
    }
}

impl QueryCache {
    pub fn new() -> Self {
        Self {
            slots: [Slot::default(), Slot::default(), Slot::default()],
            evals: 0,
            builds: 0,
            materializations: 0,
            visits: 0,
        }
    }

    /// Rows matching `filter` for the current store epoch.
    ///
    /// The first consumer of a `(membership epoch, filter)` pays one
    /// evaluation; a non-empty result also pays one build, one materialization
    /// and one visit per scanned row. An empty result is answered from the
    /// membership counters and pays none of the three (`SPEC-GATE-A0-08`).
    pub fn rows(&mut self, store: &Store, filter: Filter) -> RowSet {
        self.rows_at(store, filter, store.epoch_for(filter))
    }

    /// Rows keyed on an explicit membership revision.
    ///
    /// Inside a frame the caller passes the monotonic staged revision rather
    /// than the committed epoch: keying on `epoch` would serve the previous
    /// frame's cache while uncommitted writes are still visible.
    pub fn rows_at(&mut self, store: &Store, filter: Filter, revision: u64) -> RowSet {
        let index = filter_index(filter);
        if self.slots[index].epoch == Some(revision)
            && let Some(cached) = &self.slots[index].cached
        {
            return cached.clone();
        }

        self.evals += 1;

        if store.is_empty_for(filter) {
            // Zero materialization: the counter index already answers it.
            let empty = RowSet::empty();
            self.slots[index] = Slot {
                epoch: Some(revision),
                cached: Some(empty.clone()),
            };
            return empty;
        }

        self.builds += 1;
        self.materializations += 1;
        self.visits += store.scan_cost() as u64;
        let keys: Vec<RowKey> = store
            .rows()
            .filter(|row| filter.matches(row.completed()))
            .map(|row| row.key())
            .collect();
        let rows = RowSet::from_keys(keys);
        self.slots[index] = Slot {
            epoch: Some(revision),
            cached: Some(rows.clone()),
        };
        rows
    }

    /// True when `filter` matches nothing, from the membership counters alone.
    ///
    /// This is the only `is_empty` the runtime uses; a `collect()`-based
    /// implementation would move `row_visits` and `id_materializations`, which
    /// the A0-08 fixture pins at zero.
    pub fn is_empty(&self, store: &Store, filter: Filter) -> bool {
        store.is_empty_for(filter)
    }

    /// Account an evaluation the caller performed against a staged overlay.
    /// The caller reports whether it actually built a set.
    pub(crate) fn note_eval(&mut self, rebuilt: bool) {
        self.evals += 1;
        if rebuilt {
            self.builds += 1;
            self.materializations += 1;
        }
    }

    /// Drop the cached snapshots. The cache is not authority, so a rolled-back
    /// frame only has to forget it.
    pub fn invalidate(&mut self) {
        self.slots = [Slot::default(), Slot::default(), Slot::default()];
    }

    /// True when a cached set exists for exactly this epoch and filter.
    pub fn is_fresh(&self, store: &Store, filter: Filter) -> bool {
        let index = filter_index(filter);
        self.slots[index].epoch == Some(store.epoch_for(filter))
            && self.slots[index].cached.is_some()
    }

    pub fn counters(&self) -> Counters {
        Counters {
            query_evals: self.evals,
            id_materializations: self.materializations,
            cache_builds: self.builds,
            row_visits: self.visits,
            ..Counters::default()
        }
    }
}

impl Default for QueryCache {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for QueryCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueryCache")
            .field("slots", &self.slots.len())
            .field("evals", &self.evals)
            .field("materializations", &self.materializations)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_consumer_shares_the_snapshot() {
        let mut store = Store::for_test();
        store.insert("A");
        store.insert("B");

        let mut cache = QueryCache::new();
        let first = cache.rows(&store, Filter::All);
        let second = cache.rows(&store, Filter::All);

        assert_eq!(first.as_slice(), second.as_slice());
        assert_eq!(cache.counters().query_evals, 1);
        assert_eq!(cache.counters().id_materializations, 1);
    }

    #[test]
    fn a_completed_write_only_invalidates_the_filters_it_moves() {
        let mut store = Store::for_test();
        let row = store.insert("A");
        let mut cache = QueryCache::new();

        // Warm all three filters in one epoch.
        let _ = cache.rows(&store, Filter::All);
        let _ = cache.rows(&store, Filter::Active);
        let _ = cache.rows(&store, Filter::Completed);
        assert_eq!(cache.counters().query_evals, 3);

        store.toggle(row).expect("live row");

        // `All` membership cannot change on a flag write: still fresh.
        let _ = cache.rows(&store, Filter::All);
        assert_eq!(cache.counters().query_evals, 3, "All stayed fresh");

        // `Active` and `Completed` memberships moved: one re-evaluation each.
        let _ = cache.rows(&store, Filter::Active);
        let _ = cache.rows(&store, Filter::Completed);
        assert_eq!(cache.counters().query_evals, 5);

        // A second consumer in the same epoch shares the snapshot.
        let before = cache.counters().query_evals;
        let _ = cache.rows(&store, Filter::Active);
        assert_eq!(cache.counters().query_evals, before, "a hit is free");
    }

    #[test]
    fn an_empty_filter_costs_no_materialization_and_no_visit() {
        let store = Store::for_test();
        let mut cache = QueryCache::new();

        let rows = cache.rows(&store, Filter::Active);
        assert!(rows.is_empty());
        assert_eq!(cache.counters().query_evals, 1);
        assert_eq!(cache.counters().cache_builds, 0);
        assert_eq!(cache.counters().id_materializations, 0);
        assert_eq!(cache.counters().row_visits, 0);

        // And `is_empty` is answered from the counters, never by scanning.
        assert!(cache.is_empty(&store, Filter::Completed));
        assert_eq!(cache.counters().row_visits, 0);
    }

    #[test]
    fn filter_membership_is_exclusive() {
        assert!(Filter::All.matches(true) && Filter::All.matches(false));
        assert!(Filter::Active.matches(false) && !Filter::Active.matches(true));
        assert!(Filter::Completed.matches(true) && !Filter::Completed.matches(false));
    }
}
