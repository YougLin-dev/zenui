// crates/core/tests/regressions.rs
//!
//! Regressions found by the ADR-0017 conformance audit.
//!
//! Every test here was written against a real defect: each one fails on the
//! implementation that had the bug.

use zenui_core::{Fatal, Filter, LayoutNodeId, RowId, RowKey, Runtime, ValueError, ViewId};

fn key_of(runtime: &Runtime, title: &str) -> RowKey {
    runtime
        .committed()
        .rows()
        .find(|row| row.title() == title)
        .map(|row| row.key())
        .unwrap_or_else(|| panic!("row {title} exists"))
}

/// `SPEC-RT-051`: insert + remove in one frame must land as "no row", and the
/// reservation must be released instead of left filled.
#[test]
fn an_insert_removed_in_the_same_frame_leaves_no_row() {
    let mut runtime = Runtime::new();

    runtime
        .frame(|tx| {
            let handle = tx.insert("ghost")?;
            let _ = tx.remove(handle);
            Ok(())
        })
        .expect("frame");

    assert_eq!(
        runtime.committed().len(),
        0,
        "the row must not be published"
    );
    assert_eq!(runtime.row_views().len(), 0);

    // And the slot comes back: the next insert still works and is reachable.
    let row = runtime.frame(|tx| tx.insert("real")).expect("insert");
    assert_eq!(runtime.committed().len(), 1);
    assert_eq!(
        runtime.committed().with_title(row, str::to_owned).unwrap(),
        "real"
    );
}

/// `ADR-0017` decision 4: a staged removal wins over any staged value, so a
/// frame-internal read of a removed row is `InvalidHandle`.
#[test]
fn a_removal_wins_over_a_staged_value() {
    let mut runtime = Runtime::new();
    let row: RowId = runtime.frame(|tx| tx.insert("A")).expect("insert");

    runtime
        .frame(|tx| {
            let _ = tx.remove(row);
            assert!(
                matches!(
                    tx.with_title(row, str::to_owned),
                    Err(ValueError::Handle(_))
                ),
                "a removed row must not be readable in the same frame"
            );
            assert!(
                tx.read_once(row).is_err(),
                "an owned read of a removed row must fail"
            );
            assert!(
                matches!(tx.completed(row), Err(ValueError::Handle(_))),
                "a removed row has no completed value"
            );
            Ok(())
        })
        .expect("frame");

    assert_eq!(runtime.committed().len(), 0);
}

/// `ADR-0017` decision 8: a swallowed inner failure poisons the transaction.
#[test]
fn a_swallowed_nested_failure_poisons_the_frame() {
    let mut runtime = Runtime::new();
    let commits_before = runtime.txn().commits();

    let result = runtime.frame(|tx| {
        let inner: Result<(), Fatal> = tx.batch(|inner| {
            inner.insert("poisoned")?;
            Err(Fatal::Aborted)
        });
        assert!(inner.is_err(), "the inner batch failed");
        // Swallow it deliberately: the outer closure still says Ok.
        Ok(())
    });

    assert!(
        matches!(result, Err(Fatal::Aborted)),
        "the frame must refuse to publish, got {result:?}"
    );
    assert_eq!(runtime.committed().len(), 0);
    assert_eq!(runtime.txn().commits(), commits_before, "nothing committed");
}

/// `SPEC-RT-002`: a discarded frame must not rewind the row-key allocator.
#[test]
fn a_discarded_frame_does_not_rewind_row_keys() {
    let mut runtime = Runtime::new();

    let first: RowId = runtime.frame(|tx| tx.insert("A")).expect("insert A");
    let discarded: Result<(), Fatal> = runtime.frame(|tx| {
        tx.insert("B")?;
        Err(Fatal::Aborted)
    });
    assert!(discarded.is_err());

    let second: RowId = runtime.frame(|tx| tx.insert("C")).expect("insert C");
    // A second insert after the discard is what exposes a rewound allocator:
    // the first one after the reset would simply take the low key.
    let third: RowId = runtime.frame(|tx| tx.insert("D")).expect("insert D");
    let keys: Vec<RowKey> = runtime.committed().keys().collect();
    let mut unique = keys.clone();
    unique.sort_unstable();
    unique.dedup();

    assert_eq!(runtime.committed().len(), 3);
    assert_eq!(
        unique.len(),
        keys.len(),
        "row keys must stay unique: {keys:?}"
    );
    assert_ne!(first, second);
    assert_ne!(second, third);
}

/// `SPEC-RT-052`: a partial view order is rejected before the swap, not
/// silently filtered at apply time.
#[test]
fn a_partial_view_order_is_rejected() {
    let mut runtime = Runtime::new();
    runtime
        .frame(|tx| {
            tx.insert("A")?;
            tx.insert("B")?;
            Ok(())
        })
        .expect("seed");

    let a = key_of(&runtime, "A");
    let b = key_of(&runtime, "B");
    runtime
        .frame(|tx| {
            let first = tx.mount_row(a, 1)?;
            let second = tx.mount_row(b, 1)?;
            tx.reorder_views(&[first, second])?;
            Ok(())
        })
        .expect("mount");

    let views = runtime.row_views();
    let first_view = views[0].0;
    let before = runtime.row_views();

    let result = runtime.frame(|tx| {
        tx.reorder_views(&[first_view])?;
        Ok(())
    });

    assert!(result.is_err(), "a partial order must not commit");
    assert_eq!(runtime.row_views(), before, "the view order did not move");
}

/// `SPEC-RT-052`: a mount for a row the frame removed is a preflight failure.
#[test]
fn a_mount_for_a_removed_row_is_rejected() {
    let mut runtime = Runtime::new();
    let row: RowId = runtime.frame(|tx| tx.insert("A")).expect("insert");
    let key = key_of(&runtime, "A");

    let result = runtime.frame(|tx| {
        let _ = tx.remove(row);
        tx.mount_row(key, 1)?;
        Ok(())
    });

    assert!(result.is_err(), "a phantom mount must not commit");
    assert_eq!(runtime.row_views().len(), 0);
    assert_eq!(runtime.committed().len(), 1, "the removal did not commit");
}

/// `SPEC-RT-052`: the focus pointer after the swap must equal the journal.
#[test]
fn a_focus_on_an_unmounted_view_is_rejected() {
    let mut runtime = Runtime::new();
    let row: RowId = runtime.frame(|tx| tx.insert("A")).expect("insert");
    let key = key_of(&runtime, "A");
    let view = runtime.frame(|tx| tx.mount_row(key, 1)).expect("mount");

    let result = runtime.frame(|tx| {
        tx.unmount(view)?;
        tx.focus(view)?;
        Ok(())
    });

    assert!(
        result.is_err(),
        "focusing a view this frame unmounts must fail"
    );
    assert_eq!(runtime.focused(), None);
    let _ = row;
}

/// `SPEC-RT-051`: a discarded frame must not burn a layout id.
#[test]
fn a_discarded_frame_does_not_burn_layout_ids() {
    let mut runtime = Runtime::new();

    runtime
        .frame(|tx| {
            tx.insert("A")?;
            Ok(())
        })
        .expect("seed");
    let key = key_of(&runtime, "A");

    let discarded: Result<ViewId, Fatal> = runtime.frame(|tx| {
        tx.mount_row(key, 1)?;
        Err(Fatal::Aborted)
    });
    assert!(discarded.is_err());
    assert!(runtime.layout_pairs().is_empty(), "nothing mapped yet");

    let view = runtime.frame(|tx| tx.mount_row(key, 1)).expect("mount");

    assert_eq!(
        runtime.layout_of(view).map(LayoutNodeId::index),
        Some(0),
        "the discarded frame must not have advanced the layout allocator"
    );
}

/// `SPEC-RT-052`: a row inserted after a reorder must not vanish from the
/// committed store. Removing the append step in `effective_rows` makes this
/// fail: the slot is filled but no read path can reach it.
#[test]
fn a_row_inserted_after_a_reorder_is_not_dropped() {
    let mut runtime = Runtime::new();

    runtime
        .frame(|tx| {
            tx.insert("A")?;
            tx.insert("B")?;
            Ok(())
        })
        .expect("seed");

    let a = key_of(&runtime, "A");
    let b = key_of(&runtime, "B");

    runtime
        .frame(|tx| {
            tx.reorder(&[b, a])
                .map_err(|_| Fatal::OrderNotAPermutation)?;
            tx.insert("C")?;
            Ok(())
        })
        .expect("reorder then insert");

    let store = runtime.committed();
    assert_eq!(
        store.titles().collect::<Vec<_>>(),
        vec!["B", "A", "C"],
        "the new row is appended to the staged order"
    );
    assert_eq!(store.len(), 3);
    assert_eq!(store.rows().count(), 3, "every row is reachable");
    assert_eq!(store.keys().count(), 3);

    let c = key_of(&runtime, "C");
    let handle = store.handle_for(c).expect("C resolves");
    assert_eq!(store.with_title(handle, str::to_owned).unwrap(), "C");
}

/// `ADR-0017` decision 4: a frame-internal view read sees the mounts and
/// unmounts staged by the same frame, including inside a nested batch.
#[test]
fn a_frame_internal_view_read_sees_staged_mounts() {
    let mut runtime = Runtime::new();
    runtime
        .frame(|tx| {
            tx.insert("A")?;
            tx.insert("B")?;
            Ok(())
        })
        .expect("seed");

    let a = key_of(&runtime, "A");
    let b = key_of(&runtime, "B");

    runtime
        .frame(|tx| {
            let va = tx.mount_row(a, 1)?;
            tx.reorder_views(&[va])?;
            Ok(())
        })
        .expect("mount A");

    let visible = runtime.row_views();
    let va = visible[0].0;

    runtime
        .frame(|tx| {
            tx.unmount(va)?;
            let vb = tx.batch(|inner| inner.mount_row(b, 1))?;
            // Inside the same frame the re-render must see both changes.
            let staged: Vec<(ViewId, RowKey)> = tx.row_views();
            assert!(
                !staged.iter().any(|(view, _)| *view == va),
                "the staged unmount must not be visible"
            );
            assert!(
                staged.iter().any(|(view, key)| *view == vb && *key == b),
                "the staged mount must be visible"
            );
            tx.reorder_views(&[vb])?;
            Ok(())
        })
        .expect("rerender");

    let after = runtime.row_views();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].1, b);
}

/// `metrics::Counters::io_writes`: a write that changes nothing is not a
/// write, so it must not advance the epoch or the write counter. Comparing
/// against the staged value alone made this fail.
#[test]
fn a_no_op_write_does_not_advance_the_epoch() {
    let mut runtime = Runtime::new();
    let row: RowId = runtime.frame(|tx| tx.insert("A")).expect("insert");

    let before = runtime.committed().epoch();
    let writes_before = runtime.committed().counters().io_writes;

    runtime
        .frame(|tx| {
            tx.set_completed(row, false).expect("no-op write");
            Ok(())
        })
        .expect("frame");

    assert_eq!(runtime.committed().epoch(), before, "epoch must not move");
    assert_eq!(
        runtime.committed().counters().io_writes,
        writes_before,
        "a no-op is not a write"
    );
    assert!(!runtime.committed().rows().next().unwrap().completed());
}

/// `SPEC-GATE-A0-07` + `SPEC-RT-053`: a discarded frame reaches revisions the
/// store never installs, so the snapshots it built must not survive.
///
/// The cache is keyed on `(membership, order)`; a later frame reaches the same
/// order revision with a *different* permutation, and without the invalidation
/// on discard it is served the dead frame\'s row set.
#[test]
fn a_discarded_frame_does_not_leave_a_row_set_behind() {
    let mut runtime = Runtime::new();
    runtime
        .frame(|tx| {
            for title in ["A", "B", "C"] {
                tx.insert(title)?;
            }
            Ok(())
        })
        .expect("seed");

    let forward: Vec<RowKey> = runtime.committed().keys().collect();
    let reversed: Vec<RowKey> = forward.iter().rev().copied().collect();
    let rotated: Vec<RowKey> = vec![forward[1], forward[2], forward[0]];

    let result: Result<(), Fatal> = runtime.frame(|tx| {
        tx.reorder(&reversed)
            .map_err(|_| Fatal::OrderNotAPermutation)?;
        let seen = tx.rows(Filter::All);
        assert_eq!(seen.as_slice(), reversed.as_slice(), "in-frame read");
        Err(Fatal::Aborted)
    });
    assert!(result.is_err(), "the frame was discarded");
    assert_eq!(
        runtime.committed().keys().collect::<Vec<_>>(),
        forward,
        "nothing was installed"
    );

    // The next frame reaches the same order revision with another permutation.
    let seen = runtime
        .frame(|tx| {
            tx.reorder(&rotated)
                .map_err(|_| Fatal::OrderNotAPermutation)?;
            Ok(tx.rows(Filter::All))
        })
        .expect("reorder");
    assert_eq!(
        seen.as_slice(),
        rotated.as_slice(),
        "the discarded frame\'s snapshot must not answer"
    );
}
