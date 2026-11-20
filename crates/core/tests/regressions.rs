use zenui_core::{Fatal, Filter, LayoutNodeId, RowId, RowKey, Runtime, ValueError, ViewId};

fn key_of(runtime: &Runtime, title: &str) -> RowKey {
    runtime
        .committed()
        .rows()
        .find(|row| row.title() == title)
        .map(|row| row.key())
        .unwrap_or_else(|| panic!("row {title} exists"))
}

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

    let row = runtime.frame(|tx| tx.insert("real")).expect("insert");
    assert_eq!(runtime.committed().len(), 1);
    assert_eq!(
        runtime.committed().with_title(row, str::to_owned).unwrap(),
        "real"
    );
}

#[test]
fn the_key_index_follows_removal_and_reinsertion() {
    let mut runtime = Runtime::new();
    let first = runtime.frame(|tx| tx.insert("A")).expect("insert A");
    let first_key = runtime.committed().keys().next().expect("A key");
    assert_eq!(runtime.committed().handle_for(first_key), Some(first));

    runtime
        .frame(|tx| tx.remove(first).map_err(|_| Fatal::Aborted))
        .expect("remove A");
    assert!(runtime.committed().handle_for(first_key).is_none());

    let second = runtime.frame(|tx| tx.insert("B")).expect("insert B");
    let second_key = runtime.committed().keys().next().expect("B key");
    assert_ne!(second_key, first_key);
    assert_eq!(runtime.committed().handle_for(second_key), Some(second));
}

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

        Ok(())
    });

    assert!(
        matches!(result, Err(Fatal::Aborted)),
        "the frame must refuse to publish, got {result:?}"
    );
    assert_eq!(runtime.committed().len(), 0);
    assert_eq!(runtime.txn().commits(), commits_before, "nothing committed");
}

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

#[test]
fn an_unordered_mount_remains_available_for_a_later_render() {
    let mut runtime = Runtime::new();
    runtime
        .frame(|tx| {
            tx.insert("A")?;
            Ok(())
        })
        .expect("seed");
    let key = key_of(&runtime, "A");

    let (ordered, view) = runtime
        .frame(|tx| {
            tx.mount_row(key, 1)?;
            Ok((tx.row_views(), tx.view_of_row(key)))
        })
        .expect("mount without ordering");
    let view = view.expect("the row lookup finds a staged mount");
    assert_eq!(
        ordered,
        vec![(view, key)],
        "the row can immediately be focused"
    );

    runtime
        .frame(|tx| {
            assert_eq!(tx.row_views(), ordered, "the committed row is visible");
            assert_eq!(tx.view_of_row(key), Some(view), "the row lookup is stable");
            tx.reorder_views(&[view])?;
            tx.focus(view)?;
            Ok(())
        })
        .expect("the next render can adopt the mounted row");
    assert_eq!(runtime.row_views(), ordered);
}

#[test]
fn projection_includes_ordered_unordered_and_staged_views_exactly_once() {
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
    let view_a = runtime
        .frame(|tx| tx.mount_row(a, 1))
        .expect("mount without order");
    let view_b = runtime
        .frame(|tx| {
            let view = tx.mount_row(b, 2)?;
            let order: Vec<_> = tx.row_views().into_iter().map(|(view, _)| view).collect();
            tx.reorder_views(&order)?;
            Ok(view)
        })
        .expect("ordered mount");
    assert_eq!(runtime.row_views(), vec![(view_a, a), (view_b, b)]);

    runtime
        .frame(|tx| {
            assert!(tx.is_live(view_a), "a committed unordered view is live");
            tx.unmount(view_a)?;
            assert!(!tx.is_live(view_a), "a staged unmount is no longer live");
            assert_eq!(tx.view_of_row(a), None, "a removed row has no lookup");
            assert_eq!(tx.row_views(), vec![(view_b, b)]);
            assert_eq!(tx.view_of_row(b), Some(view_b));
            Ok(())
        })
        .expect("unmount an already ordered row");

    let view_c = runtime
        .frame(|tx| {
            let view = tx.mount_row(a, 3)?;
            assert!(tx.is_live(view), "a staged view is live");
            assert_eq!(tx.view_of_row(a), Some(view), "staged row lookup matches");
            let order: Vec<_> = tx.row_views().into_iter().map(|(view, _)| view).collect();
            tx.reorder_views(&order)?;
            Ok(view)
        })
        .expect("remount after removal");
    assert_eq!(runtime.row_views(), vec![(view_b, b), (view_c, a)]);

    runtime
        .frame(|tx| {
            tx.unmount(view_c)?;
            assert!(
                !tx.is_live(view_c),
                "a same-frame unmount is no longer live"
            );
            assert_eq!(tx.view_of_row(a), None);
            tx.reorder_views(&[view_b])?;
            Ok(())
        })
        .expect("unmount a staged row");
    assert_eq!(runtime.row_views(), vec![(view_b, b)]);
}

#[test]
fn a_view_created_and_removed_in_one_frame_is_never_published() {
    let mut runtime = Runtime::new();
    runtime
        .frame(|tx| {
            tx.insert("A")?;
            Ok(())
        })
        .expect("seed");
    let key = key_of(&runtime, "A");

    runtime
        .frame(|tx| {
            let view = tx.mount_row(key, 1)?;
            tx.unmount(view)?;
            assert!(!tx.is_live(view));
            assert_eq!(tx.view_of_row(key), None);
            assert_eq!(tx.row_views(), Vec::new());
            Ok(())
        })
        .expect("cancel a never-published View");
    assert_eq!(runtime.row_views(), Vec::new());
    assert_eq!(runtime.live_owner_ids().len(), 1);
    assert_eq!(runtime.layout_pairs(), Vec::new());
}

#[test]
fn cancelling_one_planned_view_does_not_reuse_another_views_layout_id() {
    let mut runtime = Runtime::new();
    runtime
        .frame(|tx| {
            tx.insert("A")?;
            tx.insert("B")?;
            tx.insert("C")?;
            Ok(())
        })
        .expect("seed");
    let a = key_of(&runtime, "A");
    let b = key_of(&runtime, "B");
    let c = key_of(&runtime, "C");
    let pair = runtime
        .frame(|tx| {
            let first = tx.mount_row(a, 1)?;
            let second = tx.mount_row(b, 2)?;
            tx.unmount(first)?;
            tx.reorder_views(&[second])?;
            Ok((first, second))
        })
        .expect("mixed cancelled and published mounts");
    assert_eq!(runtime.layout_of(pair.1).map(LayoutNodeId::index), Some(1));
    let third = runtime
        .frame(|tx| tx.mount_row(c, 3))
        .expect("a later mount");
    assert_ne!(
        runtime.layout_of(pair.1),
        runtime.layout_of(third),
        "a cancelled plan must not make two live Views share one layout ID"
    );
}

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
