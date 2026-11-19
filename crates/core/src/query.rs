use std::fmt;
use std::sync::Arc;

use crate::handle::RowKey;
use crate::metrics::Counters;

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

#[derive(Clone, PartialEq, Eq)]
/// 共享有序 ID 投影。多个消费者共享 identity，不各自复制标题集合。
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

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = RowKey> + '_ {
        self.ids.iter().copied()
    }

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

pub(crate) trait RowView {
    fn rows_for(&mut self, filter: Filter) -> Vec<(RowKey, bool)>;

    fn is_empty_for(&mut self, filter: Filter) -> bool;

    fn membership_revision(&self, filter: Filter) -> u64;

    fn order_revision(&self) -> u64;

    fn scan_cost(&self) -> u64;
}

pub(crate) const fn filter_index(filter: Filter) -> usize {
    match filter {
        Filter::All => 0,
        Filter::Active => 1,
        Filter::Completed => 2,
    }
}

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

    /// 同一个过滤和排序版本只构造一次行集；空查询不产生 ID 物化。
    pub(crate) fn rows_at<V: RowView + ?Sized>(&mut self, view: &mut V, filter: Filter) -> RowSet {
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

        let scanned = view.scan_cost();
        let rows = if view.is_empty_for(filter) {
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
        let first = cache.rows_at(&mut view, Filter::All);
        let second = cache.rows_at(&mut view, Filter::All);

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

        let _ = cache.rows_at(&mut view, Filter::All);
        let _ = cache.rows_at(&mut view, Filter::Active);
        let _ = cache.rows_at(&mut view, Filter::Completed);
        assert_eq!(cache.counters().query_evals, 3);

        view.membership[filter_index(Filter::Active)] += 1;
        view.membership[filter_index(Filter::Completed)] += 1;

        let _ = cache.rows_at(&mut view, Filter::All);
        assert_eq!(cache.counters().query_evals, 3, "All stayed fresh");
        let _ = cache.rows_at(&mut view, Filter::Active);
        let _ = cache.rows_at(&mut view, Filter::Completed);
        assert_eq!(cache.counters().query_evals, 5);
    }

    #[test]
    fn an_order_move_rebuilds_without_touching_membership() {
        let mut view = TestView::new(&[(1, false), (2, false)]);
        let mut cache = QueryCache::new();

        let before = cache.rows_at(&mut view, Filter::All);
        assert_eq!(
            before.as_slice(),
            &[RowKey::from_raw(1), RowKey::from_raw(2)]
        );

        view.rows.reverse();
        view.order += 1;

        let after = cache.rows_at(&mut view, Filter::All);
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

        let rows = cache.rows_at(&mut view, Filter::Completed);
        assert!(rows.is_empty());
        assert_eq!(cache.counters().query_evals, 1);
        assert_eq!(cache.counters().cache_builds, 0);
        assert_eq!(cache.counters().id_materializations, 0);
        assert_eq!(cache.counters().row_visits, 0);
    }

    #[test]
    fn a_different_revision_pair_is_a_different_state() {
        let mut view = TestView::new(&[(1, false)]);
        let mut cache = QueryCache::new();

        let _ = cache.rows_at(&mut view, Filter::Active);

        view.membership = [1, 1, 1];
        view.order = 1;
        assert_eq!(cache.rows_at(&mut view, Filter::Active).len(), 1);
        assert_eq!(cache.counters().query_evals, 2, "a moved pair re-evaluates");
    }

    #[test]
    fn the_committed_view_answers_like_the_staged_one() {
        use crate::store::Store;

        let mut store = Store::for_test();
        let a = store.insert("A");
        store.insert("B");
        let mut cache = QueryCache::new();

        let all = cache.rows_at(&mut store, Filter::All);
        assert_eq!(all.len(), 2);
        assert_eq!(cache.counters().row_visits, 2);

        let completed = cache.rows_at(&mut store, Filter::Completed);
        assert!(completed.is_empty());
        assert_eq!(
            cache.counters().row_visits,
            2,
            "the empty answer costs no scan on the committed side either"
        );

        store.toggle(a).expect("live row");
        let completed = cache.rows_at(&mut store, Filter::Completed);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed.as_slice(), &[store.key_of(a).expect("key")]);
        assert!(cache.rows_at(&mut store, Filter::Active).len() == 1);

        let keys: Vec<RowKey> = store.keys().collect();
        store.reorder(&[keys[1], keys[0]]).expect("reorder");
        assert_eq!(
            cache.rows_at(&mut store, Filter::All).as_slice(),
            &[keys[1], keys[0]]
        );
    }

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
