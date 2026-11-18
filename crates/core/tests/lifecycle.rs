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

    // The same key twice inside *one* frame is refused by the *staged* check,
    // not by the preflight that would also catch it at commit. The first mount
    // is legal because the committed view is already staged for unmount.
    //
    // The refusal is asserted at the call site and the frame is then aborted on
    // purpose: a fixture that only inspects the frame's result cannot tell the
    // two checks apart, because preflight rejects the duplicate with the same
    // variant.
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
    // Compared *unsorted*: the identities must survive, and they must survive
    // in the new order. A sorted comparison would pass even if the reorder had
    // silently done nothing.
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
    // A settlement that ran twice would land on the same view (no second move)
    // but would place the caret a second time: the caret counter is what makes
    // "settled once" checkable rather than merely satisfiable.
    assert_eq!(
        runtime.counters().caret_ops,
        carets_before + 1,
        "one settlement places the caret once"
    );
}

/// `SPEC-GATE-A0-10`: `clear_focus` is a user choice too. The recorded intent
/// must not pull focus back after the user cleared it in the same frame.
#[test]
fn a0_10_a_user_clear_cancels_the_intent() {
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

/// `SPEC-GATE-A0-10`: when the clear removes every candidate, the result is
/// "no focus", and only one move is counted.
///
/// Scope, stated honestly: in this shape the recorded intent and the generic
/// "focus left on an unmounted view" cleanup produce the same observables, so
/// deleting the intent path keeps this fixture green. It pins the fallback
/// *result*; the fixture below, where a sibling survives, is the one that pins
/// the intent itself.
#[test]
fn a0_10_a_bulk_clear_with_no_candidate_leaves_no_focus() {
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
        runtime.counters().focus_moves,
        moves_before + 1,
        "one intent for the whole clear"
    );
    assert!(
        runtime.focused().is_none(),
        "every candidate row was cleared, so the fallback is no focus"
    );
}

/// `SPEC-GATE-A0-10`: the same clear, with a surviving sibling.
///
/// This is the variant that can tell one settlement from two: the fallback
/// lands on a live view, so a second settlement re-places the caret and
/// `caret_ops` moves. In the fixture above every candidate is gone and a
/// repeated settlement is indistinguishable — the reason both cases exist.
#[test]
fn a0_10_a_bulk_clear_settles_once_for_the_surviving_sibling() {
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
    // Focus the *middle* removed row: its next sibling (C) survives, so the
    // intent has a live target.
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

/// `SPEC-GATE-A0-11`: 1000+ interleaved operations return to baseline.
#[test]
fn a0_11_churn_returns_to_the_baseline() {
    let mut runtime = Runtime::new();
    // Warm the static wiring first (the two remaining memos depend on the store
    // node, and the query cache fills its slots), so the baseline is "after one
    // full frame", not "before any read".
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
    // `SPEC-GATE-A0-11` asks for 1000+ interleaved operations, not 250.
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
    assert_eq!(
        operations, 1200,
        "`SPEC-GATE-A0-11` asks for 1000+ operations"
    );

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

fn runtime_row(runtime: &Runtime, title: &str) -> RowId {
    runtime
        .committed()
        .rows()
        .find(|row| row.title() == title)
        .and_then(|row| runtime.committed().handle_for(row.key()))
        .unwrap_or_else(|| panic!("row {title} exists"))
}

/// `SPEC-GATE-A0-02`: `read_once` is the only owned read and it counts one
/// I/O; a borrow read costs none.
///
/// This is the *primitive*. The mount executor that consumes it lives in
/// `zenui-app`, and the criterion "one owned read per mount, and a mount that
/// fails is cancelled" is pinned against that real path in
/// `crates/app/tests/mount_path.rs` — a fixture that calls `read_once` itself
/// would only prove the fixture.
#[test]
fn a0_02_an_owned_read_counts_once_and_a_borrow_read_is_free() {
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
}
