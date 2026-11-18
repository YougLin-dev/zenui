// crates/core/src/query.rs
//! Query cache and the shared `RowSet` snapshot (`SPEC-RT-041` … `-045`).
//!
//! The cache is a pure function of a [`RowView`] and a revision pair. There is
//! deliberately no way for a caller to *report* that it evaluated something:
//! the counters move only inside this file, on the code path that does the
//! work. A "bookkeeping API" would make the numbers a second, declarable
//! authority and let a fixture that never touches the runtime stay green.

use std::fmt;
use std::sync::Arc;

use crate::handle::RowKey;
use crate::metrics::Counters;

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

/// The model as one query sees it: the committed store, or a frame's staged
/// overlay on top of it.
///
/// The contract is the five members fixed by `ADR-0022` decision 2. Two
/// implementations exist — `Store` and `StagedView` — and the cache knows
/// nothing about either.
pub(crate) trait RowView {
    /// The rows `filter` matches, in display order, paired with the effective
    /// `completed` flag.
    fn rows_for(&mut self, filter: Filter) -> Vec<(RowKey, bool)>;

    /// True when `filter` matches nothing, answered from counters rather than a
    /// scan (`SPEC-RT-044`).
    fn is_empty_for(&mut self, filter: Filter) -> bool;

    /// Revision of `filter`'s *membership set*. A write that cannot change the
    /// set must not move it (`SPEC-GATE-A0-07`).
    fn membership_revision(&self, filter: Filter) -> u64;

    /// Revision of the display order.
    ///
    /// Membership and order move independently: a `reorder` moves no
    /// membership, a `completed` write moves no order. Keying on membership
    /// alone would serve a stale order after a reorder.
    fn order_revision(&self) -> u64;

    /// Rows this view has enumerated since it was created.
    ///
    /// The cache charges `row_visits` with the difference across one
    /// evaluation, so a row the view enumerates *anywhere* — including inside
    /// `is_empty_for` — is counted. That is what makes the `ADR-0022`
    /// decision-6 mutation (answer emptiness by counting `rows_for(All)`)
    /// visible to a fixture instead of silently free.
    fn scan_cost(&self) -> u64;
}

pub(crate) const fn filter_index(filter: Filter) -> usize {
    match filter {
        Filter::All => 0,
        Filter::Active => 1,
        Filter::Completed => 2,
    }
}

/// The pair of revisions one cached snapshot is valid under.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Revision {
    membership: u64,
    order: u64,
}

#[derive(Clone, Default)]
struct Slot {
    revision: Option<Revision>,
    rows: Option<RowSet>,
}

/// Per-`(membership revision, order revision, filter)` query cache.
///
/// Each filter owns one slot. A title write moves neither revision, a
/// `completed` write moves only `Active`/`Completed`, a `reorder` moves only
/// the order, and an insert/remove moves all three slots' membership.
pub(crate) struct QueryCache {
    slots: [Slot; 3],
    evals: u64,
    builds: u64,
    materializations: u64,
    visits: u64,
}

impl QueryCache {
    pub(crate) fn new() -> Self {
        Self {
            slots: [Slot::default(), Slot::default(), Slot::default()],
            evals: 0,
            builds: 0,
            materializations: 0,
            visits: 0,
        }
    }

    /// Rows matching `filter` as `view` currently sees them.
    ///
    /// The first consumer of a `(membership, order, filter)` triple pays one
    /// evaluation; a non-empty result also pays one build, one materialization
    /// and one visit per scanned row. An empty result is answered from the
    /// membership counters and pays none of the three (`SPEC-GATE-A0-08`).
    pub(crate) fn rows<V: RowView + ?Sized>(&mut self, view: &mut V, filter: Filter) -> RowSet {
        let index = filter_index(filter);
        let revision = Revision {
            membership: view.membership_revision(filter),
            order: view.order_revision(),
        };
        if self.slots[index].revision == Some(revision)
            && let Some(cached) = &self.slots[index].rows
        {
            return cached.clone();
        }

        self.evals += 1;

        // The view owns the scan counter; the cache charges the difference, so
        // no enumeration performed to answer this evaluation can be free.
        let scanned = view.scan_cost();
        let rows = if view.is_empty_for(filter) {
            // Zero materialization: the membership counters already answer it.
            RowSet::empty()
        } else {
            self.builds += 1;
            self.materializations += 1;
            let keys: Vec<RowKey> = view
                .rows_for(filter)
                .into_iter()
                .map(|(key, _)| key)
                .collect();
            RowSet::from_keys(keys)
        };
        self.visits += view.scan_cost() - scanned;

        self.slots[index] = Slot {
            revision: Some(revision),
            rows: Some(rows.clone()),
        };
        rows
    }

    /// Drop the cached snapshots. Revisions are monotonic, so this is only
    /// needed when a frame is discarded: revisions it reached but never
    /// committed can be reached again by a later frame, and those snapshots
    /// describe a state that no longer exists.
    pub(crate) fn invalidate(&mut self) {
        self.slots = [Slot::default(), Slot::default(), Slot::default()];
    }

    pub(crate) fn counters(&self) -> Counters {
        Counters {
            query_evals: self.evals,
            id_materializations: self.materializations,
            cache_builds: self.builds,
            row_visits: self.visits,
            ..Counters::default()
        }
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

    /// A view whose revisions the test drives directly.
    struct TestView {
        membership: [u64; 3],
        order: u64,
        rows: Vec<(RowKey, bool)>,
        scanned: u64,
    }

    impl TestView {
        fn new(rows: &[(u64, bool)]) -> Self {
            Self {
                membership: [0; 3],
                order: 0,
                rows: rows
                    .iter()
                    .map(|(raw, completed)| (RowKey::from_raw(*raw), *completed))
                    .collect(),
                scanned: 0,
            }
        }
    }

    impl RowView for TestView {
        fn rows_for(&mut self, filter: Filter) -> Vec<(RowKey, bool)> {
            self.scanned += self.rows.len() as u64;
            self.rows
                .iter()
                .filter(|(_, completed)| filter.matches(*completed))
                .copied()
                .collect()
        }

        fn is_empty_for(&mut self, filter: Filter) -> bool {
            !self
                .rows
                .iter()
                .any(|(_, completed)| filter.matches(*completed))
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

    #[test]
    fn second_consumer_shares_the_snapshot() {
        let mut view = TestView::new(&[(1, false), (2, false)]);

        let mut cache = QueryCache::new();
        let first = cache.rows(&mut view, Filter::All);
        let second = cache.rows(&mut view, Filter::All);

        assert!(first.shares_with(&second));
        assert_eq!(cache.counters().query_evals, 1);
        assert_eq!(cache.counters().cache_builds, 1);
        assert_eq!(cache.counters().id_materializations, 1);
        assert_eq!(cache.counters().row_visits, 2);
    }

    #[test]
    fn a_membership_move_only_invalidates_the_filters_it_touches() {
        let mut view = TestView::new(&[(1, false), (2, true)]);
        let mut cache = QueryCache::new();

        let _ = cache.rows(&mut view, Filter::All);
        let _ = cache.rows(&mut view, Filter::Active);
        let _ = cache.rows(&mut view, Filter::Completed);
        assert_eq!(cache.counters().query_evals, 3);

        // A completed write cannot move `All`'s membership set.
        view.membership[filter_index(Filter::Active)] += 1;
        view.membership[filter_index(Filter::Completed)] += 1;

        let _ = cache.rows(&mut view, Filter::All);
        assert_eq!(cache.counters().query_evals, 3, "All stayed fresh");
        let _ = cache.rows(&mut view, Filter::Active);
        let _ = cache.rows(&mut view, Filter::Completed);
        assert_eq!(cache.counters().query_evals, 5);
    }

    /// `SPEC-GATE-A0-07`: an order change is invisible to membership, so the
    /// order revision has to be part of the key or the stale order survives.
    #[test]
    fn an_order_move_rebuilds_without_touching_membership() {
        let mut view = TestView::new(&[(1, false), (2, false)]);
        let mut cache = QueryCache::new();

        let before = cache.rows(&mut view, Filter::All);
        assert_eq!(
            before.as_slice(),
            &[RowKey::from_raw(1), RowKey::from_raw(2)]
        );

        view.rows.reverse();
        view.order += 1;

        let after = cache.rows(&mut view, Filter::All);
        assert_eq!(
            after.as_slice(),
            &[RowKey::from_raw(2), RowKey::from_raw(1)]
        );
        assert_eq!(cache.counters().query_evals, 2);
        assert_eq!(cache.counters().cache_builds, 2);
    }

    #[test]
    fn an_empty_filter_costs_no_materialization_and_no_visit() {
        let mut view = TestView::new(&[(1, false)]);
        let mut cache = QueryCache::new();

        let rows = cache.rows(&mut view, Filter::Completed);
        assert!(rows.is_empty());
        assert_eq!(cache.counters().query_evals, 1);
        assert_eq!(cache.counters().cache_builds, 0);
        assert_eq!(cache.counters().id_materializations, 0);
        assert_eq!(cache.counters().row_visits, 0);
    }

    /// The revision pair is the whole key: a view that moves both revisions
    /// must not be served from the slot of a state it merely resembles.
    #[test]
    fn a_different_revision_pair_is_a_different_state() {
        let mut view = TestView::new(&[(1, false)]);
        let mut cache = QueryCache::new();

        let _ = cache.rows(&mut view, Filter::Active);

        // Same rows, both revisions moved: the cached slot must not answer.
        view.membership = [1, 1, 1];
        view.order = 1;
        assert_eq!(cache.rows(&mut view, Filter::Active).len(), 1);
        assert_eq!(cache.counters().query_evals, 2, "a moved pair re-evaluates");
    }

    /// The two implementations must answer alike for the same model: the cache
    /// must not mean one thing for committed state and another for a frame.
    ///
    /// This drives the *committed* implementation directly. The runtime uses
    /// the staged one, and no external fixture can construct either view — so
    /// this is where `Store`'s side of the trait is exercised.
    #[test]
    fn the_committed_view_answers_like_the_staged_one() {
        use crate::store::Store;

        let mut store = Store::for_test();
        let a = store.insert("A");
        store.insert("B");
        let mut cache = QueryCache::new();

        let all = cache.rows(&mut store, Filter::All);
        assert_eq!(all.len(), 2);
        assert_eq!(cache.counters().row_visits, 2);

        let completed = cache.rows(&mut store, Filter::Completed);
        assert!(completed.is_empty());
        assert_eq!(
            cache.counters().row_visits,
            2,
            "the empty answer costs no scan on the committed side either"
        );

        store.toggle(a).expect("live row");
        let completed = cache.rows(&mut store, Filter::Completed);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed.as_slice(), &[store.key_of(a).expect("key")]);
        assert!(cache.rows(&mut store, Filter::Active).len() == 1);

        // An order move is a new state for the committed view too.
        let keys: Vec<RowKey> = store.keys().collect();
        store.reorder(&[keys[1], keys[0]]).expect("reorder");
        assert_eq!(
            cache.rows(&mut store, Filter::All).as_slice(),
            &[keys[1], keys[0]]
        );
    }

    /// `SPEC-RT-043`: two snapshots are equal only when their keys are, in
    /// order — the display order is part of the value, so a reordered set is a
    /// different set.
    #[test]
    fn row_sets_compare_by_content_and_order() {
        let forward = RowSet::from_keys(vec![RowKey::from_raw(1), RowKey::from_raw(2)]);
        let again = RowSet::from_keys(vec![RowKey::from_raw(1), RowKey::from_raw(2)]);
        let reversed = RowSet::from_keys(vec![RowKey::from_raw(2), RowKey::from_raw(1)]);

        assert_eq!(forward, again);
        assert_ne!(forward, reversed);
        assert!(
            !forward.shares_with(&again),
            "equal values, separate buffers"
        );
    }

    #[test]
    fn filter_membership_is_exclusive() {
        assert!(Filter::All.matches(true) && Filter::All.matches(false));
        assert!(Filter::Active.matches(false) && !Filter::Active.matches(true));
        assert!(Filter::Completed.matches(true) && !Filter::Completed.matches(false));
    }
}
