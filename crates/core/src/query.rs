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
/// TODO(A0-07): a field write must only mark the cache dirty and register the
/// `(row, completed)` reverse edge it actually read; P1 invalidates on the whole
/// store epoch, so an unrelated write re-materializes every filter.
/// TODO(A0-08): `is_empty(filter)` currently derives from this materialized set.
/// The zero-materialization empty state needs the counter/membership index and
/// is therefore *not* satisfied by P1.
pub struct QueryCache {
    filter: Filter,
    epoch: Option<u64>,
    cached: Option<RowSet>,
    evals: u64,
    materializations: u64,
}

impl QueryCache {
    pub fn new() -> Self {
        Self {
            filter: Filter::All,
            epoch: None,
            cached: None,
            evals: 0,
            materializations: 0,
        }
    }

    /// Rows matching `filter` for the current store epoch.
    ///
    /// The first consumer of an epoch pays one evaluation and one
    /// materialization; later consumers share the same `Arc<[RowKey]>`.
    pub fn rows(&mut self, store: &Store, filter: Filter) -> RowSet {
        if self.epoch == Some(store.epoch())
            && self.filter == filter
            && let Some(cached) = &self.cached
        {
            return cached.clone();
        }

        self.evals += 1;
        self.materializations += 1;
        let keys: Vec<RowKey> = store
            .rows()
            .filter(|row| filter.matches(row.completed()))
            .map(|row| row.key())
            .collect();
        let rows = RowSet::from_keys(keys);
        self.filter = filter;
        self.epoch = Some(store.epoch());
        self.cached = Some(rows.clone());
        rows
    }

    /// True when a cached set exists for exactly this epoch and filter.
    pub fn is_fresh(&self, store: &Store, filter: Filter) -> bool {
        self.epoch == Some(store.epoch()) && self.filter == filter && self.cached.is_some()
    }

    pub fn counters(&self) -> Counters {
        Counters {
            query_evals: self.evals,
            id_materializations: self.materializations,
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
            .field("filter", &self.filter)
            .field("epoch", &self.epoch)
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
        let mut store = Store::new();
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
    fn a_write_forces_one_reevaluation_for_all_consumers() {
        let mut store = Store::new();
        let row = store.insert("A");
        let mut cache = QueryCache::new();
        let _ = cache.rows(&store, Filter::All);

        store.toggle(row).expect("live row");

        let active = cache.rows(&store, Filter::Active);
        assert!(active.is_empty());
        assert_eq!(cache.counters().query_evals, 2);
        assert_eq!(cache.counters().id_materializations, 2);
    }

    #[test]
    fn filter_membership_is_exclusive() {
        assert!(Filter::All.matches(true) && Filter::All.matches(false));
        assert!(Filter::Active.matches(false) && !Filter::Active.matches(true));
        assert!(Filter::Completed.matches(true) && !Filter::Completed.matches(false));
    }
}
