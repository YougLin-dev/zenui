use zenui_core::{Fatal, Filter, RowId, RowKey, Runtime, Tx, ViewId};

fn keys(tx: &mut Tx<'_>) -> Vec<RowKey> {
    tx.rows(Filter::All).iter().collect()
}

fn mount_all(runtime: &mut Runtime) -> Vec<ViewId> {
    runtime
        .frame(|tx| {
            let all = keys(tx);
            let mut views = Vec::with_capacity(all.len());
            for key in &all {
                views.push(tx.mount_row(*key, 1)?);
            }
            tx.reorder_views(&views)?;
            Ok(views)
        })
        .expect("mount all")
}

#[test]
fn a_duplicate_key_is_refused_before_any_patch() {
    let mut runtime = Runtime::new();
    runtime
        .frame(|tx| {
            tx.insert("A")?;
            tx.insert("B")?;
            Ok(())
        })
        .expect("seed");

    let first = mount_all(&mut runtime);
    assert_eq!(first.len(), 2);
    let visible_before = runtime.row_views();
    let key = visible_before[0].1;

    let result = runtime.frame(|tx| {
        tx.mount_row(key, 1)?;
        Ok(())
    });
    assert!(
        matches!(result, Err(Fatal::DuplicateViewKey { .. })),
        "expected DuplicateViewKey, got {result:?}"
    );
    assert_eq!(runtime.row_views(), visible_before, "no patch was applied");

    let result: Result<(), Fatal> = runtime.frame(|tx| {
        tx.unmount(visible_before[0].0)?;
        tx.mount_row(key, 1)?;
        let second = tx.mount_row(key, 1);
        assert!(
            matches!(second, Err(Fatal::DuplicateViewKey { .. })),
            "the second mount must be refused before the commit, got {second:?}"
        );
        Err(Fatal::Aborted)
    });
    assert!(result.is_err(), "the frame was discarded");
    assert_eq!(
        runtime.row_views(),
        visible_before,
        "the aborted frame applied no patch"
    );
}

#[test]
fn a_reorder_preserves_identity_and_moves_no_counter() {
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
            let middle = views[1];
            tx.focus(middle)?;
            tx.unmount(views[2])?;
            Ok(())
        })
        .expect("focus and unmount");

    runtime
        .frame(|tx| {
            let all = keys(tx);
            let view = tx.mount_row(all[2], 1)?;
            let mut order: Vec<ViewId> = tx.row_views().into_iter().map(|(v, _)| v).collect();
            if !order.contains(&view) {
                order.push(view);
            }
            tx.reorder_views(&order)?;
            Ok(())
        })
        .expect("remount");

    let before = runtime.counters();
    let identities_before: Vec<(u32, RowKey)> = runtime
        .row_views()
        .into_iter()
        .map(|(view, key)| (view.index(), key))
        .collect();
    let focused_before = runtime.focused();
    let caret_before = focused_before.and_then(|view| runtime.caret(view));
    let owners_before = runtime.live_owner_ids();

    let reversed: Vec<ViewId> = runtime
        .row_views()
        .into_iter()
        .rev()
        .map(|(view, _)| view)
        .collect();
    runtime
        .frame(|tx| {
            tx.reorder_views(&reversed)?;
            Ok(())
        })
        .expect("reorder");

    let after = runtime.counters();
    assert_eq!(after.mounts, before.mounts, "reorder mounts nothing");
    assert_eq!(after.unmounts, before.unmounts, "reorder unmounts nothing");
    assert_eq!(runtime.focused(), focused_before);
    assert_eq!(
        focused_before.and_then(|view| runtime.caret(view)),
        caret_before
    );
    assert_eq!(runtime.live_owner_ids(), owners_before);

    let identities_after: Vec<(u32, RowKey)> = runtime
        .row_views()
        .into_iter()
        .map(|(view, key)| (view.index(), key))
        .collect();
    let expected: Vec<(u32, RowKey)> = identities_before.iter().rev().copied().collect();
    assert_eq!(
        identities_after, expected,
        "the same ViewIds, in the reversed display order"
    );
}

#[test]
fn deleting_the_focused_row_settles_one_intent() {
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
            tx.focus(views[0])?;
            Ok(())
        })
        .expect("focus A");

    let focus_before = runtime.focused();
    let moves_before = runtime.counters().focus_moves;
    let carets_before = runtime.counters().caret_ops;

    let row = runtime_row(&runtime, "A");
    runtime
        .frame(|tx| {
            tx.remove(row).map_err(|_| Fatal::Aborted)?;
            tx.unmount(views[0])?;
            Ok(())
        })
        .expect("delete focused");

    assert_ne!(runtime.focused(), focus_before, "focus moved away");
    assert_eq!(
        runtime.focused(),
        Some(views[1]),
        "next sibling, not previous"
    );
    assert_eq!(
        runtime.counters().focus_moves,
        moves_before + 1,
        "exactly one settlement"
    );

    assert_eq!(
        runtime.counters().caret_ops,
        carets_before + 1,
        "one settlement places the caret once"
    );
}

#[test]
fn deleting_the_last_focused_row_falls_back_to_previous() {
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
            tx.focus(views[2])?;
            Ok(())
        })
        .expect("focus C");

    let moves_before = runtime.counters().focus_moves;
    let carets_before = runtime.counters().caret_ops;

    let row = runtime_row(&runtime, "C");
    runtime
        .frame(|tx| {
            tx.remove(row).map_err(|_| Fatal::Aborted)?;
            tx.unmount(views[2])?;
            Ok(())
        })
        .expect("delete the last focused row");

    assert_eq!(
        runtime.focused(),
        Some(views[1]),
        "no next sibling exists, so the settlement must take previous"
    );
    assert_eq!(
        runtime.counters().focus_moves,
        moves_before + 1,
        "exactly one settlement"
    );
    assert_eq!(
        runtime.counters().caret_ops,
        carets_before + 1,
        "one settlement places the caret once"
    );
}

#[test]
fn a_user_clear_cancels_the_intent() {
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
            tx.focus(views[0])?;
            Ok(())
        })
        .expect("focus A");

    let moves_before = runtime.counters().focus_moves;
    let row = runtime_row(&runtime, "A");
    runtime
        .frame(|tx| {
            tx.remove(row).map_err(|_| Fatal::Aborted)?;
            tx.unmount(views[0])?;
            tx.clear_focus()?;
            Ok(())
        })
        .expect("delete + clear focus");

    assert!(
        runtime.focused().is_none(),
        "the user's clear must not be overridden by the intent"
    );
    assert_eq!(
        runtime.counters().focus_moves,
        moves_before + 1,
        "only the clear moved focus; no settlement followed it"
    );
}

#[test]
fn a_user_focus_move_cancels_the_intent() {
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
            tx.focus(views[0])?;
            Ok(())
        })
        .expect("focus A");

    let moves_before = runtime.counters().focus_moves;
    let row = runtime_row(&runtime, "A");
    runtime
        .frame(|tx| {
            tx.remove(row).map_err(|_| Fatal::Aborted)?;
            tx.unmount(views[0])?;

            tx.focus(views[2])?;
            Ok(())
        })
        .expect("delete + user focus");

    assert_eq!(
        runtime.focused(),
        Some(views[2]),
        "the user's choice wins over the recorded intent"
    );
    assert_eq!(
        runtime.counters().focus_moves,
        moves_before + 1,
        "only the explicit focus moves; automatic settlement must not also run"
    );
}

#[test]
fn a_bulk_clear_selects_the_surviving_next_sibling() {
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

    for title in ["A", "B"] {
        let row = runtime_row(&runtime, title);
        runtime
            .frame(|tx| {
                tx.set_completed(row, true).map_err(|_| Fatal::Aborted)?;
                Ok(())
            })
            .expect("complete");
    }
    runtime
        .frame(|tx| {
            tx.focus(views[0])?;
            Ok(())
        })
        .expect("focus A");

    let moves_before = runtime.counters().focus_moves;
    let carets_before = runtime.counters().caret_ops;
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
        runtime
            .row_views()
            .into_iter()
            .map(|(view, _)| view)
            .collect::<Vec<_>>(),
        vec![views[2]],
        "A and B are actually unmounted; C survives"
    );
    assert_eq!(
        runtime.counters().focus_moves,
        moves_before + 1,
        "one intent for the whole clear"
    );
    assert_eq!(
        runtime.counters().caret_ops,
        carets_before + 1,
        "one placement for the surviving next sibling"
    );
    assert_eq!(
        runtime.focused(),
        Some(views[2]),
        "C is the surviving next sibling after A and its old next B are unmounted"
    );
    assert_eq!(
        runtime.caret(views[2]),
        Some(1),
        "C receives one settled caret"
    );
}

#[test]
fn a_bulk_clear_settles_once_for_the_surviving_sibling() {
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

    for title in ["A", "B"] {
        let row = runtime_row(&runtime, title);
        runtime
            .frame(|tx| {
                tx.set_completed(row, true).map_err(|_| Fatal::Aborted)?;
                Ok(())
            })
            .expect("complete");
    }

    runtime
        .frame(|tx| {
            tx.focus(views[1])?;
            Ok(())
        })
        .expect("focus B");

    let moves_before = runtime.counters().focus_moves;
    let carets_before = runtime.counters().caret_ops;
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
        runtime.focused(),
        Some(views[2]),
        "the surviving next sibling takes focus"
    );
    assert_eq!(
        runtime.counters().focus_moves,
        moves_before + 1,
        "one intent for the whole clear, not one per removed row"
    );
    assert_eq!(
        runtime.counters().caret_ops,
        carets_before + 1,
        "the settlement ran once"
    );
}

#[test]
fn churn_returns_to_the_baseline() {
    let mut runtime = Runtime::new();

    runtime
        .frame(|tx| {
            for filter in [Filter::All, Filter::Active, Filter::Completed] {
                tx.rows(filter);
            }
            Ok(())
        })
        .expect("warm-up");
    let baseline_owners = runtime.live_owners();
    let baseline_edges = runtime.edge_count();
    let baseline_sources = runtime.active_source_count();
    let baseline_layout = runtime.layout_pairs().len();

    let mut operations = 0u32;
    let mut mounted: Vec<(ViewId, RowKey)> = Vec::new();
    let mut orphaned: Vec<RowKey> = Vec::new();

    for round in 0..1200u32 {
        let filter = match round % 3 {
            0 => Filter::All,
            1 => Filter::Active,
            _ => Filter::Completed,
        };
        runtime
            .frame(|tx| {
                tx.rows(filter);
                match round % 4 {
                    0 => {
                        let handle = tx.insert(format!("row-{round}"))?;
                        let key = keys(tx)
                            .into_iter()
                            .find(|candidate| tx.handle_for(*candidate) == Some(handle))
                            .expect("staged key");
                        let view = tx.mount_row(key, 1)?;
                        mounted.push((view, key));
                    }
                    2 => {
                        if let Some((view, key)) = mounted.pop() {
                            tx.unmount(view)?;
                            orphaned.push(key);
                        }
                    }
                    3 => {
                        if let Some(key) = orphaned.pop()
                            && let Some(handle) = tx.handle_for(key)
                        {
                            let _ = tx.remove(handle);
                        }
                    }
                    _ => {}
                }
                Ok(())
            })
            .expect("churn frame");
        operations += 1;
    }

    while let Some((view, key)) = mounted.pop() {
        runtime
            .frame(|tx| {
                tx.unmount(view)?;
                Ok(())
            })
            .expect("drain unmount");
        orphaned.push(key);
    }
    for key in orphaned {
        runtime
            .frame(|tx| {
                if let Some(handle) = tx.handle_for(key) {
                    let _ = tx.remove(handle);
                }
                Ok(())
            })
            .expect("drain remove");
    }
    assert_eq!(operations, 1200, "the fixture covers repeated churn");

    assert_eq!(runtime.live_owners(), baseline_owners, "owners leaked");
    assert_eq!(runtime.edge_count(), baseline_edges, "edges leaked");
    assert_eq!(runtime.active_source_count(), baseline_sources);
    assert_eq!(
        runtime.layout_pairs().len(),
        baseline_layout,
        "layout leaked"
    );
    assert_eq!(runtime.row_views().len(), 0, "no view survived");
    assert_eq!(
        runtime.committed().len(),
        0,
        "the model is back to empty, not just the view tree"
    );
}

fn read_titles(runtime: &mut Runtime, filter: Filter) -> Vec<String> {
    runtime
        .frame(|tx| {
            let rows = tx.rows(filter);
            let mut titles = Vec::with_capacity(rows.len());
            for key in rows.iter() {
                let handle = tx.handle_for(key).expect("the row set names a live row");
                titles.push(
                    tx.with_title(handle, |title| title.to_owned())
                        .map_err(|_| Fatal::Aborted)?,
                );
            }
            Ok(titles)
        })
        .expect("read")
}

#[test]
fn the_cache_returns_to_the_baseline_after_churn() {
    let filters = [Filter::All, Filter::Active, Filter::Completed];
    let mut runtime = Runtime::new();
    runtime
        .frame(|tx| {
            for filter in filters {
                tx.rows(filter);
            }
            Ok(())
        })
        .expect("warm-up");

    let mut mounted: Vec<(ViewId, RowKey)> = Vec::new();
    let mut orphaned: Vec<RowKey> = Vec::new();
    let mut operations = 0u32;
    for round in 0..1200u32 {
        let filter = filters[(round % 3) as usize];

        if round % 5 == 4 {
            let result: Result<(), Fatal> = runtime.frame(|tx| {
                tx.insert(format!("ghost-{round}"))?;
                tx.rows(filter);
                Err(Fatal::Aborted)
            });
            assert!(result.is_err(), "the ghost frame was discarded");
            operations += 1;
            continue;
        }
        runtime
            .frame(|tx| {
                tx.rows(filter);
                match round % 4 {
                    0 => {
                        let handle = tx.insert(format!("row-{round}"))?;
                        let key = keys(tx)
                            .into_iter()
                            .find(|candidate| tx.handle_for(*candidate) == Some(handle))
                            .expect("staged key");
                        let view = tx.mount_row(key, 1)?;
                        mounted.push((view, key));
                    }
                    2 => {
                        if let Some((view, key)) = mounted.pop() {
                            tx.unmount(view)?;
                            orphaned.push(key);
                        }
                    }
                    3 => {
                        if let Some(key) = orphaned.pop()
                            && let Some(handle) = tx.handle_for(key)
                        {
                            let _ = tx.remove(handle);
                        }
                    }
                    _ => {}
                }
                Ok(())
            })
            .expect("churn frame");
        operations += 1;
    }
    while let Some((view, key)) = mounted.pop() {
        runtime
            .frame(|tx| {
                tx.unmount(view)?;
                Ok(())
            })
            .expect("drain unmount");
        orphaned.push(key);
    }
    for key in orphaned {
        runtime
            .frame(|tx| {
                if let Some(handle) = tx.handle_for(key) {
                    let _ = tx.remove(handle);
                }
                Ok(())
            })
            .expect("drain remove");
    }
    assert_eq!(operations, 1200, "the fixture covers repeated churn");
    assert_eq!(runtime.committed().len(), 0, "the drain emptied the model");

    let ghost: Result<(), Fatal> = runtime.frame(|tx| {
        let a = tx.insert("ghost-A")?;
        let b = tx.insert("ghost-B")?;
        let c = tx.insert("ghost-C")?;
        tx.set_completed(c, true).expect("complete ghost C");
        let _ = (a, b);
        for filter in filters {
            tx.rows(filter);
        }
        Err(Fatal::Aborted)
    });
    assert!(ghost.is_err(), "the ghost frame was discarded");

    runtime
        .frame(|tx| {
            let a = tx.insert("A")?;
            let b = tx.insert("B")?;
            let c = tx.insert("C")?;
            tx.set_completed(c, true).expect("complete C");
            let _ = (a, b);
            Ok(())
        })
        .expect("seed the baseline");

    let before = runtime.counters();
    let seen: Vec<Vec<String>> = filters
        .iter()
        .map(|filter| read_titles(&mut runtime, *filter))
        .collect();
    let delta = runtime.counters().delta(before);
    assert_eq!(
        delta.query_evals, 3,
        "one evaluation per filter; a stale hit would evaluate fewer"
    );
    assert_eq!(
        delta.cache_builds, 3,
        "one build per non-empty filter; the model has three rows, one completed"
    );
    assert_eq!(
        delta.id_materializations, 3,
        "each build materializes one snapshot"
    );

    let mut fresh = Runtime::new();
    fresh
        .frame(|tx| {
            let a = tx.insert("A")?;
            let b = tx.insert("B")?;
            let c = tx.insert("C")?;
            tx.set_completed(c, true).expect("complete C");
            let _ = (a, b);
            Ok(())
        })
        .expect("fresh seed");
    let expected: Vec<Vec<String>> = filters
        .iter()
        .map(|filter| read_titles(&mut fresh, *filter))
        .collect();
    assert_eq!(
        seen, expected,
        "the churned runtime must read the same row sets as a fresh one"
    );

    let before_second = runtime.counters();
    let again: Vec<Vec<String>> = filters
        .iter()
        .map(|filter| read_titles(&mut runtime, *filter))
        .collect();
    let delta = runtime.counters().delta(before_second);
    assert_eq!(again, seen, "a cache hit returns the same snapshot");
    assert_eq!(
        delta.id_materializations, 0,
        "a cache hit must not materialize again"
    );
    assert_eq!(delta.query_evals, 0, "a cache hit is not an evaluation");
    assert_eq!(delta.cache_builds, 0, "a cache hit is not a build");
}

fn runtime_row(runtime: &Runtime, title: &str) -> RowId {
    runtime
        .committed()
        .rows()
        .find(|row| row.title() == title)
        .and_then(|row| runtime.committed().handle_for(row.key()))
        .unwrap_or_else(|| panic!("row {title} exists"))
}

#[test]
fn an_owned_read_counts_once_and_a_borrow_read_is_free() {
    let mut runtime = Runtime::new();
    runtime
        .frame(|tx| {
            tx.insert("A")?;
            tx.insert("B")?;
            tx.insert("C")?;
            Ok(())
        })
        .expect("seed");

    let before = runtime.committed().counters().io_reads;
    runtime
        .frame(|tx| {
            let all = keys(tx);
            for key in &all {
                let handle = tx.handle_for(*key).expect("row");
                let title = tx.read_once(handle).expect("owned read");
                assert!(!title.is_empty(), "read_once returns the title");
            }
            Ok(())
        })
        .expect("owned reads");

    let after = runtime.committed().counters().io_reads;
    assert_eq!(after - before, 3, "one owned read per handle");

    runtime
        .frame(|tx| {
            let all = keys(tx);
            for key in &all {
                let handle = tx.handle_for(*key).expect("row");
                let _ = tx.with_title(handle, |title| title.len()).expect("borrow");
            }
            Ok(())
        })
        .expect("borrow reads");
    assert_eq!(
        runtime.committed().counters().io_reads,
        after,
        "a borrow read is not I/O"
    );
}
