// crates/core/tests/queries.rs
//!
//! `SPEC-GATE-A0-07` and `SPEC-GATE-A0-08`.
//!
//! The counters are the criterion, not telemetry: `query_evals`,
//! `cache_builds`, `id_materializations` and `row_visits` are asserted
//! exactly, and the empty-state fixture is red for any `is_empty` that scans.

use zenui_core::{Filter, QueryCache, Runtime};

fn seed(runtime: &mut Runtime, rows: &[&str]) {
    runtime
        .frame(|tx| {
            for title in rows {
                tx.insert(*title)?;
            }
            Ok(())
        })
        .expect("seed");
}

/// `SPEC-GATE-A0-07`: one evaluation, one build and one materialization per
/// `(epoch, filter)`, and a second consumer shares the allocation without
/// cloning a `String`.
#[test]
fn a0_07_one_eval_build_and_materialization_per_epoch_filter() {
    let mut runtime = Runtime::new();
    seed(&mut runtime, &["A", "B"]);

    let mut cache = QueryCache::new();
    let store = runtime.committed();

    let first = cache.rows(store, Filter::All);
    let counters = cache.counters();
    assert_eq!(counters.query_evals, 1);
    assert_eq!(counters.cache_builds, 1);
    assert_eq!(counters.id_materializations, 1);
    assert_eq!(counters.row_visits, 2, "two rows scanned");

    let second = cache.rows(store, Filter::All);
    let after = cache.counters();
    assert_eq!(after.query_evals, 1, "a hit does not evaluate");
    assert_eq!(after.cache_builds, 1);
    assert_eq!(after.id_materializations, 1);
    assert!(first.shares_with(&second), "consumers share the snapshot");
    assert_eq!(
        runtime.committed().counters().string_clones,
        0,
        "no consumer cloned a title"
    );
    assert_eq!(first.len(), 2);
}

/// `SPEC-GATE-A0-07`: the reverse edge is `(row, completed)`, so a flag write
/// invalidates `Active`/`Completed` but not `All`.
#[test]
fn a0_07_a_completed_write_moves_only_the_filters_that_read_it() {
    let mut runtime = Runtime::new();
    seed(&mut runtime, &["A", "B"]);
    runtime
        .frame(|tx| {
            let key = runtime_keys(tx)[0];
            let handle = tx.handle_for(key).expect("row");
            tx.set_completed(handle, true).expect("write");
            Ok(())
        })
        .expect("frame");

    let mut cache = QueryCache::new();
    let store = runtime.committed();
    let _ = cache.rows(store, Filter::All);
    let _ = cache.rows(store, Filter::Active);
    let _ = cache.rows(store, Filter::Completed);
    assert_eq!(cache.counters().query_evals, 3);

    // Another write: `A` is already completed, so a second toggle moves it.
    runtime
        .frame(|tx| {
            let key = runtime_keys(tx)[0];
            let handle = tx.handle_for(key).expect("row");
            tx.toggle(handle).map_err(|_| zenui_core::Fatal::Aborted)?;
            Ok(())
        })
        .expect("toggle");

    let store = runtime.committed();
    let _ = cache.rows(store, Filter::All);
    assert_eq!(
        cache.counters().query_evals,
        3,
        "All membership cannot move on a flag write"
    );
    let _ = cache.rows(store, Filter::Active);
    let _ = cache.rows(store, Filter::Completed);
    assert_eq!(
        cache.counters().query_evals,
        5,
        "each moved filter re-evaluated"
    );
}

/// `SPEC-GATE-A0-08`: an empty result costs nothing. A `collect()`-based
/// `is_empty` would move `row_visits` and `id_materializations`, so this
/// fixture is red for that implementation.
#[test]
fn a0_08_an_empty_filter_materializes_nothing() {
    let mut runtime = Runtime::new();
    seed(&mut runtime, &["A"]);
    // Every row is active: the Completed filter is empty.
    let mut cache = QueryCache::new();
    let store = runtime.committed();

    let rows = cache.rows(store, Filter::Completed);
    assert!(rows.is_empty());
    let counters = cache.counters();
    assert_eq!(counters.query_evals, 1);
    assert_eq!(counters.cache_builds, 0, "no set was built");
    assert_eq!(counters.id_materializations, 0);
    assert_eq!(counters.row_visits, 0, "no row was visited");

    assert!(cache.is_empty(store, Filter::Completed));
    assert_eq!(cache.counters().row_visits, 0);
}

/// `SPEC-GATE-A0-08`: 1000 rows and three filter switches cost one build per
/// filter, and repeating the cycle costs nothing.
#[test]
fn a0_08_a_thousand_rows_and_three_filters_build_once_each() {
    let mut runtime = Runtime::new();
    let titles: Vec<String> = (0..1000).map(|index| format!("row-{index}")).collect();
    runtime
        .frame(|tx| {
            for title in &titles {
                tx.insert(title.clone())?;
            }
            Ok(())
        })
        .expect("seed 1000");

    let mut cache = QueryCache::new();
    let store = runtime.committed();
    for filter in [Filter::All, Filter::Active, Filter::Completed] {
        let rows = cache.rows(store, filter);
        assert_eq!(
            rows.len(),
            if filter == Filter::Completed { 0 } else { 1000 }
        );
    }
    let first = cache.counters();
    assert_eq!(first.query_evals, 3);
    assert_eq!(first.cache_builds, 2, "Completed is empty: no build");
    assert_eq!(first.id_materializations, 2);
    assert_eq!(first.row_visits, 2000);

    for filter in [Filter::All, Filter::Active, Filter::Completed] {
        let _ = cache.rows(store, filter);
    }
    assert_eq!(cache.counters(), first, "the second cycle is all hits");
}

fn runtime_keys(tx: &mut zenui_core::Tx<'_>) -> Vec<zenui_core::RowKey> {
    tx.rows(Filter::All).iter().collect()
}
