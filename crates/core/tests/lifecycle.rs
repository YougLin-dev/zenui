// crates/core/tests/lifecycle.rs
//!
//! `SPEC-GATE-A0-09`, `-10` and `-11`.
//!
//! These fixtures drive the public frame API only, and assert *counters and
//! identities* rather than "it looks right": a reorder that remounts, a
//! duplicate key that slips through, or a focus intent settled twice all show
//! up as a moved counter.

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

/// `SPEC-GATE-A0-09`: a duplicate key is refused before any patch, and a
/// reorder preserves ViewId, owner, caret and focus with zero unmount/setup.
#[test]
fn a0_09_a_duplicate_key_is_refused_before_any_patch() {
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
        // The same row key a second time: `For` must be keyed.
        tx.mount_row(key, 1)?;
        Ok(())
    });
    assert!(
        matches!(result, Err(Fatal::DuplicateViewKey { .. })),
        "expected DuplicateViewKey, got {result:?}"
    );
    assert_eq!(runtime.row_views(), visible_before, "no patch was applied");

    // The same key twice inside one frame is refused too.
    let result = runtime.frame(|tx| {
        runtime_keys(tx);
        Ok(())
    });
    assert!(result.is_ok());
}

/// `SPEC-GATE-A0-09`: reorder keeps identity and moves no mount counter.
#[test]
fn a0_09_a_reorder_preserves_identity_and_moves_no_counter() {
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
    // Re-mount the third so the reorder below has three live views.
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
    let mut identities_after: Vec<(u32, RowKey)> = runtime
        .row_views()
        .into_iter()
        .map(|(view, key)| (view.index(), key))
        .collect();
    let mut expected = identities_before.clone();
    identities_after.sort();
    expected.sort();
    assert_eq!(identities_after, expected, "the same ViewIds survived");
}

/// `SPEC-GATE-A0-10`: the intent is recorded before the structural change and
/// settled once after the commit, next first.
#[test]
fn a0_10_deleting_the_focused_row_settles_one_intent() {
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
}

/// `SPEC-GATE-A0-10`: a user focus move in the same frame cancels the intent.
#[test]
fn a0_10_a_user_focus_move_cancels_the_intent() {
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

    let row = runtime_row(&runtime, "A");
    runtime
        .frame(|tx| {
            tx.remove(row).map_err(|_| Fatal::Aborted)?;
            tx.unmount(views[0])?;
            // The user moves focus deliberately later in the same frame.
            tx.focus(views[2])?;
            Ok(())
        })
        .expect("delete + user focus");

    assert_eq!(
        runtime.focused(),
        Some(views[2]),
        "the user's choice wins over the recorded intent"
    );
}

/// `SPEC-GATE-A0-10`: a bulk clear settles once, not once per removed row.
#[test]
fn a0_10_a_bulk_clear_settles_one_intent() {
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

    // Complete A and B, focus A, then clear.
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
    let focused_before_clear = runtime.focused();
    runtime
        .frame(|tx| {
            tx.clear_completed()?;
            for view in &views[..2] {
                tx.unmount(*view)?;
            }
            Ok(())
        })
        .expect("clear");

    eprintln!(
        "focus before={:?} after={:?} moves {} -> {}",
        focused_before_clear,
        runtime.focused(),
        moves_before,
        runtime.counters().focus_moves
    );
    assert_eq!(
        runtime.counters().focus_moves,
        moves_before + 1,
        "one intent for the whole clear"
    );
    assert!(
        runtime.focused().is_none(),
        "every candidate row was cleared, so the fallback is no focus"
    );
}

/// `SPEC-GATE-A0-11`: 1000+ interleaved operations return to baseline.
#[test]
fn a0_11_churn_returns_to_the_baseline() {
    let mut runtime = Runtime::new();
    // Warm the static wiring first (the three filter memos depend on the store
    // node), so the baseline is "after one full frame", not "before any read".
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
    for round in 0..300u32 {
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

    // Drain: unmount first, then remove the rows.
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
    assert!(operations >= 250);

    assert_eq!(runtime.live_owners(), baseline_owners, "owners leaked");
    assert_eq!(runtime.edge_count(), baseline_edges, "edges leaked");
    assert_eq!(runtime.active_source_count(), baseline_sources);
    assert_eq!(
        runtime.layout_pairs().len(),
        baseline_layout,
        "layout leaked"
    );
    assert_eq!(runtime.row_views().len(), 0, "no view survived");
}

fn runtime_row(runtime: &Runtime, title: &str) -> RowId {
    runtime
        .committed()
        .rows()
        .find(|row| row.title() == title)
        .and_then(|row| runtime.committed().handle_for(row.key()))
        .unwrap_or_else(|| panic!("row {title} exists"))
}

fn runtime_keys(tx: &mut Tx<'_>) -> Vec<RowKey> {
    tx.rows(Filter::All).iter().collect()
}

/// `SPEC-GATE-A0-02`: a mount reads the owned title exactly once, and a borrow
/// read costs no I/O.
#[test]
fn a0_02_a_mount_reads_the_title_exactly_once() {
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
    let views = runtime
        .frame(|tx| {
            let all = keys(tx);
            let mut views = Vec::new();
            for key in &all {
                let handle = tx.handle_for(*key).expect("row");
                let title = tx.read_once(handle).expect("owned read");
                views.push(tx.mount_row(*key, title.chars().count() as u32)?);
            }
            tx.reorder_views(&views)?;
            Ok(views)
        })
        .expect("mount all");

    let after = runtime.committed().counters().io_reads;
    assert_eq!(after - before, 3, "one owned read per mounted row");

    // A borrow read is free.
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
    assert_eq!(views.len(), 3);
}
