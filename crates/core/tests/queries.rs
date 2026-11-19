use zenui_core::{Counters, Fatal, Filter, RowKey, Runtime};

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

fn key_of(runtime: &Runtime, title: &str) -> RowKey {
    runtime
        .committed()
        .rows()
        .find(|row| row.title() == title)
        .unwrap_or_else(|| panic!("row {title} exists"))
        .key()
}

fn delta(runtime: &Runtime, earlier: Counters) -> Counters {
    runtime.counters().delta(earlier)
}

#[test]
fn one_eval_build_and_materialization_per_epoch_filter() {
    let mut runtime = Runtime::new();
    seed(&mut runtime, &["A", "B"]);

    let before = runtime.counters();
    let first = runtime
        .frame(|tx| Ok(tx.rows(Filter::All)))
        .expect("first read");
    let after_first = runtime.counters();
    let counters = delta(&runtime, before);
    assert_eq!(counters.query_evals, 1);
    assert_eq!(counters.cache_builds, 1);
    assert_eq!(counters.id_materializations, 1);
    assert_eq!(counters.row_visits, 2, "two rows scanned");
    assert_eq!(first.len(), 2);

    let second = runtime
        .frame(|tx| Ok(tx.rows(Filter::All)))
        .expect("second read");
    let after_second = delta(&runtime, after_first);
    assert_eq!(after_second.query_evals, 0, "a hit does not evaluate");
    assert_eq!(after_second.cache_builds, 0);
    assert_eq!(after_second.id_materializations, 0);
    assert_eq!(after_second.row_visits, 0);
    assert!(first.shares_with(&second), "consumers share the snapshot");

    assert_eq!(
        runtime.committed().counters().string_clones,
        0,
        "no consumer cloned a title"
    );
}

#[test]
fn a_completed_write_moves_only_the_filters_that_read_it() {
    let mut runtime = Runtime::new();
    seed(&mut runtime, &["A", "B"]);
    let a = key_of(&runtime, "A");

    runtime
        .frame(|tx| {
            for filter in [Filter::All, Filter::Active, Filter::Completed] {
                let _ = tx.rows(filter);
            }
            Ok(())
        })
        .expect("warm");

    let before = runtime.counters();
    let (all, active, completed) = runtime
        .frame(|tx| {
            let handle = tx.handle_for(a).expect("row A");
            tx.set_completed(handle, true).map_err(|_| Fatal::Aborted)?;
            Ok((
                tx.rows(Filter::All),
                tx.rows(Filter::Active),
                tx.rows(Filter::Completed),
            ))
        })
        .expect("toggle");

    let counters = delta(&runtime, before);
    assert_eq!(
        counters.query_evals, 2,
        "All membership cannot move on a flag write"
    );
    assert_eq!(counters.cache_builds, 2);
    assert_eq!(counters.row_visits, 4, "two rows per moved filter");
    assert_eq!(all.len(), 2);
    assert_eq!(active.len(), 1);
    assert_eq!(completed.len(), 1);
}

#[test]
fn an_empty_filter_materializes_nothing() {
    let mut runtime = Runtime::new();
    seed(&mut runtime, &["A"]);

    let before = runtime.counters();
    let rows = runtime
        .frame(|tx| Ok(tx.rows(Filter::Completed)))
        .expect("read");
    assert!(rows.is_empty());

    let counters = delta(&runtime, before);
    assert_eq!(counters.query_evals, 1);
    assert_eq!(counters.cache_builds, 0, "no set was built");
    assert_eq!(counters.id_materializations, 0);
    assert_eq!(counters.row_visits, 0, "no row was visited");
}

#[test]
fn a_frame_that_empties_a_filter_visits_nothing() {
    let mut runtime = Runtime::new();
    seed(&mut runtime, &["A", "B"]);
    let a = key_of(&runtime, "A");
    runtime
        .frame(|tx| {
            let handle = tx.handle_for(a).expect("row A");
            tx.set_completed(handle, true).map_err(|_| Fatal::Aborted)?;
            Ok(())
        })
        .expect("complete A");

    let before = runtime.counters();
    let rows = runtime
        .frame(|tx| {
            let handle = tx.handle_for(a).expect("row A");
            tx.remove(handle).map_err(|_| Fatal::Aborted)?;
            Ok(tx.rows(Filter::Completed))
        })
        .expect("remove");

    assert!(rows.is_empty());
    let counters = delta(&runtime, before);
    assert_eq!(
        counters.query_evals, 1,
        "the staged empty state is a new key"
    );
    assert_eq!(counters.cache_builds, 0);
    assert_eq!(counters.id_materializations, 0);
    assert_eq!(
        counters.row_visits, 0,
        "a collect()-based emptiness check would visit the surviving row"
    );
}

#[test]
fn a_thousand_rows_and_three_filters_build_once_each() {
    let mut runtime = Runtime::new();
    let titles: Vec<String> = (0..1000).map(|index| format!("row-{index}")).collect();
    runtime
        .frame(|tx| {
            for title in &titles {
                tx.insert(title.as_str())?;
            }
            Ok(())
        })
        .expect("seed 1000");

    let before = runtime.counters();
    runtime
        .frame(|tx| {
            for filter in [Filter::All, Filter::Active, Filter::Completed] {
                let rows = tx.rows(filter);
                assert_eq!(
                    rows.len(),
                    if filter == Filter::Completed { 0 } else { 1000 }
                );
            }
            Ok(())
        })
        .expect("first cycle");

    let first = delta(&runtime, before);
    assert_eq!(first.query_evals, 3);
    assert_eq!(first.cache_builds, 2, "Completed is empty: no build");
    assert_eq!(first.id_materializations, 2);
    assert_eq!(first.row_visits, 2000);

    let before = runtime.counters();
    runtime
        .frame(|tx| {
            for filter in [Filter::All, Filter::Active, Filter::Completed] {
                let _ = tx.rows(filter);
            }
            Ok(())
        })
        .expect("second cycle");
    assert_eq!(
        delta(&runtime, before),
        Counters {
            commits: 1,
            ..Counters::default()
        },
        "the second cycle is all hits"
    );
}

#[test]
fn a_reorder_moves_the_order_and_the_next_read_sees_it() {
    let mut runtime = Runtime::new();
    seed(&mut runtime, &["A", "B"]);
    let forward: Vec<RowKey> = runtime.committed().keys().collect();

    let warm = runtime.frame(|tx| Ok(tx.rows(Filter::All))).expect("warm");
    assert_eq!(warm.as_slice(), forward.as_slice());

    let reversed: Vec<RowKey> = forward.iter().rev().copied().collect();
    let before = runtime.counters();
    let in_frame = runtime
        .frame(|tx| {
            tx.reorder(&reversed)
                .map_err(|_| Fatal::OrderNotAPermutation)?;
            Ok(tx.rows(Filter::All))
        })
        .expect("reorder");

    assert_eq!(
        in_frame.as_slice(),
        reversed.as_slice(),
        "the staged order must be what the query sees"
    );
    let counters = delta(&runtime, before);
    assert_eq!(counters.query_evals, 1, "an order move invalidates the set");
    assert_eq!(counters.cache_builds, 1);
    assert_eq!(counters.row_visits, 2);

    let committed: Vec<RowKey> = runtime.committed().keys().collect();
    assert_eq!(committed, reversed);
    let before = runtime.counters();
    let after = runtime.frame(|tx| Ok(tx.rows(Filter::All))).expect("read");
    assert_eq!(after.as_slice(), reversed.as_slice());
    assert_eq!(
        delta(&runtime, before).query_evals,
        0,
        "the in-frame build is the post-commit answer"
    );
    assert!(after.shares_with(&in_frame), "one allocation, not two");
}

#[test]
fn a_frame_reads_its_own_staged_writes() {
    let mut runtime = Runtime::new();
    seed(&mut runtime, &["A", "B"]);
    let a = key_of(&runtime, "A");

    let (all, active) = runtime
        .frame(|tx| {
            tx.insert("C")?;
            let handle = tx.handle_for(a).expect("row A");
            tx.remove(handle).map_err(|_| Fatal::Aborted)?;
            Ok((tx.rows(Filter::All), tx.rows(Filter::Active)))
        })
        .expect("staged writes");

    assert_eq!(all.len(), 2, "B and the staged C; A is staged for removal");
    assert!(!all.iter().any(|key| key == a), "A is gone in this frame");
    assert_eq!(active.len(), 2);

    assert_eq!(
        runtime.committed().titles().collect::<Vec<_>>(),
        vec!["B", "C"]
    );
}

#[test]
fn the_committed_query_cache_is_readable_outside_a_frame() {
    let mut runtime = Runtime::new();
    seed(&mut runtime, &["A", "B", "C"]);
    let b = key_of(&runtime, "B");

    let before = runtime.counters();
    let first = runtime.rows(Filter::All);
    let second = runtime.rows(Filter::All);
    assert!(first.shares_with(&second), "consumers share one projection");
    assert_eq!(delta(&runtime, before).query_evals, 1);

    let active = runtime.rows(Filter::Active);
    runtime
        .frame(|tx| {
            let handle = tx.handle_for(b).expect("row B");
            tx.set_completed(handle, true).map_err(|_| Fatal::Aborted)?;
            Ok(())
        })
        .expect("complete B");
    let after_hiding = runtime.rows(Filter::Active);
    assert_eq!(active.len(), 3);
    assert_eq!(after_hiding.len(), 2, "B leaves the active filter");

    runtime
        .frame(|tx| {
            let handle = tx.handle_for(b).expect("row B");
            tx.set_completed(handle, false)
                .map_err(|_| Fatal::Aborted)?;
            Ok(())
        })
        .expect("reactivate B");
    let returned = runtime.rows(Filter::Active);
    assert_eq!(returned.len(), 3);
    assert!(
        returned.iter().any(|key| key == b),
        "the candidate row returns to the active filter"
    );
    assert_eq!(
        delta(&runtime, before).query_evals,
        4,
        "All builds once and Active rebuilds on each membership move"
    );
}
