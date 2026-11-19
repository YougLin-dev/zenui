use zenui_core::{Fatal, Filter, RowId, RowKey, Runtime, ViewId};

fn mount_all(runtime: &mut Runtime) -> Vec<ViewId> {
    runtime
        .frame(|tx| {
            let keys: Vec<RowKey> = tx.rows(Filter::All).iter().collect();
            let mut views = Vec::with_capacity(keys.len());
            for key in keys {
                views.push(tx.mount_row(key, 2)?);
            }
            tx.reorder_views(&views)?;
            Ok(views)
        })
        .expect("mount all")
}

fn seed(runtime: &mut Runtime) {
    runtime
        .frame(|tx| {
            for title in ["A", "B", "C", "D"] {
                tx.insert(title)?;
            }
            Ok(())
        })
        .expect("seed");
}

#[test]
fn reordering_after_a_middle_removal_selects_the_next_final_sibling() {
    let mut runtime = Runtime::new();
    seed(&mut runtime);
    let [a, b, c, d]: [ViewId; 4] = mount_all(&mut runtime).try_into().expect("four views");

    runtime.frame(|tx| tx.focus(b)).expect("focus B");
    let moves_before = runtime.counters().focus_moves;
    let carets_before = runtime.counters().caret_ops;

    runtime
        .frame(|tx| {
            tx.unmount(b)?;
            tx.reorder_views(&[a, d, c])?;
            Ok(())
        })
        .expect("remove B and swap its old next sibling");

    assert_eq!(runtime.row_views().len(), 3, "the fixture removed B");
    assert_eq!(
        runtime.focused(),
        Some(d),
        "D occupies the released position"
    );
    assert_eq!(runtime.caret(d), Some(2));
    assert_eq!(runtime.counters().focus_moves, moves_before + 1);
    assert_eq!(runtime.counters().caret_ops, carets_before + 1);
}

#[test]
fn reordering_after_an_end_removal_falls_back_to_the_last_final_sibling() {
    let mut runtime = Runtime::new();
    seed(&mut runtime);
    let [a, b, c, d]: [ViewId; 4] = mount_all(&mut runtime).try_into().expect("four views");
    runtime.frame(|tx| tx.focus(d)).expect("focus D");
    let moves_before = runtime.counters().focus_moves;
    let carets_before = runtime.counters().caret_ops;

    runtime
        .frame(|tx| {
            tx.unmount(d)?;
            tx.reorder_views(&[b, c, a])?;
            Ok(())
        })
        .expect("remove D and move A after its old position");

    assert_eq!(runtime.row_views().len(), 3, "the fixture removed D");
    assert_eq!(
        runtime.focused(),
        Some(a),
        "there is no next after the released end"
    );
    assert_eq!(runtime.caret(a), Some(2));
    assert_eq!(runtime.counters().focus_moves, moves_before + 1);
    assert_eq!(runtime.counters().caret_ops, carets_before + 1);
}

#[test]
fn explicit_focus_in_the_same_frame_beats_the_final_automatic_candidate() {
    let mut runtime = Runtime::new();
    seed(&mut runtime);
    let [a, b, c, d]: [ViewId; 4] = mount_all(&mut runtime).try_into().expect("four views");
    runtime.frame(|tx| tx.focus(a)).expect("focus A");
    let moves_before = runtime.counters().focus_moves;
    let carets_before = runtime.counters().caret_ops;

    runtime
        .frame(|tx| {
            tx.unmount(a)?;
            tx.reorder_views(&[d, b, c])?;
            tx.focus(b)
        })
        .expect("explicitly select B instead of the automatic D");

    assert_eq!(runtime.focused(), Some(b));
    assert_eq!(runtime.caret(b), Some(2));
    assert_eq!(runtime.counters().focus_moves, moves_before + 1);
    assert_eq!(runtime.counters().caret_ops, carets_before + 1);
}

#[test]
fn explicit_clear_in_the_same_frame_cannot_recover_a_candidate() {
    let mut runtime = Runtime::new();
    seed(&mut runtime);
    let [a, b, c, d]: [ViewId; 4] = mount_all(&mut runtime).try_into().expect("four views");
    runtime.frame(|tx| tx.focus(a)).expect("focus A");
    let moves_before = runtime.counters().focus_moves;
    let carets_before = runtime.counters().caret_ops;

    runtime
        .frame(|tx| {
            tx.unmount(a)?;
            tx.reorder_views(&[d, b, c])?;
            tx.clear_focus()
        })
        .expect("explicitly clear instead of taking the automatic D");

    assert_eq!(runtime.focused(), None);
    assert_eq!(runtime.caret(d), None, "no automatic caret is created");
    assert_eq!(runtime.counters().focus_moves, moves_before + 1);
    assert_eq!(runtime.counters().caret_ops, carets_before);
}

#[test]
fn clear_before_the_focused_unmount_cancels_recovery_before_it_is_recorded() {
    let mut runtime = Runtime::new();
    seed(&mut runtime);
    let [a, b, c, d]: [ViewId; 4] = mount_all(&mut runtime).try_into().expect("four views");
    runtime.frame(|tx| tx.focus(a)).expect("focus A");

    runtime
        .frame(|tx| {
            tx.clear_focus()?;
            tx.unmount(a)?;
            tx.reorder_views(&[d, b, c])
        })
        .expect("clear must cancel a subsequently recorded deletion recovery");

    assert_eq!(runtime.focused(), None);
    assert_eq!(runtime.caret(d), None);
}

#[test]
fn removing_the_only_view_leaves_no_focus_or_caret() {
    let mut runtime = Runtime::new();
    runtime
        .frame(|tx| {
            tx.insert("A")?;
            Ok(())
        })
        .expect("seed");
    let only = mount_all(&mut runtime)[0];
    runtime.frame(|tx| tx.focus(only)).expect("focus A");

    runtime
        .frame(|tx| tx.unmount(only))
        .expect("remove the only View");

    assert_eq!(runtime.focused(), None);
    assert_eq!(runtime.caret(only), None);
}

#[test]
fn focused_user_choice_before_removal_wins_without_a_second_move() {
    let mut runtime = Runtime::new();
    seed(&mut runtime);
    let [a, b, c, d]: [ViewId; 4] = mount_all(&mut runtime).try_into().expect("four views");
    runtime.frame(|tx| tx.focus(a)).expect("focus A");
    let moves_before = runtime.counters().focus_moves;
    let carets_before = runtime.counters().caret_ops;

    runtime
        .frame(|tx| {
            tx.focus(c)?;
            tx.unmount(a)?;
            tx.reorder_views(&[b, c, d])
        })
        .expect("an explicit focus before removal suppresses automatic recovery");

    assert_eq!(runtime.focused(), Some(c));
    assert_eq!(runtime.caret(c), Some(2));
    assert_eq!(runtime.counters().focus_moves, moves_before + 1);
    assert_eq!(runtime.counters().caret_ops, carets_before + 1);
}

#[test]
fn a_model_removal_alone_does_not_settle_focus_until_the_view_unmounts() {
    let mut runtime = Runtime::new();
    runtime
        .frame(|tx| {
            tx.insert("A")?;
            tx.insert("B")?;
            Ok(())
        })
        .expect("seed");
    let [a, b]: [ViewId; 2] = mount_all(&mut runtime).try_into().expect("two views");
    runtime.frame(|tx| tx.focus(a)).expect("focus A");
    let row_a = runtime_row(&runtime, "A");
    let moves_before = runtime.counters().focus_moves;
    let carets_before = runtime.counters().caret_ops;

    runtime
        .frame(|tx| {
            tx.remove(row_a).map_err(|_| Fatal::Aborted)?;
            Ok(())
        })
        .expect("the model removes its row");

    assert_eq!(
        runtime.focused(),
        Some(a),
        "the View still exists and keeps focus"
    );
    assert_eq!(runtime.caret(a), Some(2), "its local caret is untouched");
    assert_eq!(
        runtime
            .row_views()
            .into_iter()
            .map(|(view, _)| view)
            .collect::<Vec<_>>(),
        vec![a, b],
        "row removal alone does not unmount the View"
    );
    assert_eq!(runtime.counters().focus_moves, moves_before);
    assert_eq!(runtime.counters().caret_ops, carets_before);

    runtime
        .frame(|tx| tx.unmount(a))
        .expect("the View unmount settles against the surviving sibling");

    assert_eq!(runtime.focused(), Some(b));
    assert_eq!(runtime.caret(b), Some(2));
    assert_eq!(runtime.counters().focus_moves, moves_before + 1);
    assert_eq!(runtime.counters().caret_ops, carets_before + 1);
}

#[test]
fn sibling_unmounts_in_the_same_frame_settle_on_the_final_neighbor() {
    let mut runtime = Runtime::new();
    seed(&mut runtime);
    let [_a, b, c, d]: [ViewId; 4] = mount_all(&mut runtime).try_into().expect("four views");
    runtime.frame(|tx| tx.focus(b)).expect("focus B");
    let moves_before = runtime.counters().focus_moves;
    let carets_before = runtime.counters().caret_ops;

    runtime
        .frame(|tx| {
            tx.unmount(b)?;
            tx.unmount(c)
        })
        .expect("B and its next sibling C leave together");

    assert_eq!(runtime.focused(), Some(d), "D survives after the skipped C");
    assert_eq!(runtime.caret(d), Some(2));
    assert_eq!(runtime.counters().focus_moves, moves_before + 1);
    assert_eq!(runtime.counters().caret_ops, carets_before + 1);
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
fn removing_a_focused_view_uses_the_reordered_later_sibling() {
    let mut runtime = Runtime::new();
    runtime
        .frame(|tx| {
            for title in ["A", "B", "C", "D"] {
                tx.insert(title)?;
            }
            Ok(())
        })
        .expect("seed");
    let [a, b, c, d]: [ViewId; 4] = mount_all(&mut runtime).try_into().expect("four views");
    runtime.frame(|tx| tx.focus(a)).expect("focus A");
    let moves_before = runtime.counters().focus_moves;
    let carets_before = runtime.counters().caret_ops;
    let row_a = runtime_row(&runtime, "A");

    runtime
        .frame(|tx| {
            tx.remove(row_a).map_err(|_| Fatal::Aborted)?;
            tx.unmount(a)?;
            tx.reorder_views(&[c, b, d])?;
            Ok(())
        })
        .expect("remove A and swap its former next sibling");

    assert_eq!(runtime.focused(), Some(c), "C is first in the final order");
    assert_eq!(
        runtime.caret(c),
        Some(2),
        "the settled caret has one placement"
    );
    assert_eq!(runtime.counters().focus_moves, moves_before + 1);
    assert_eq!(runtime.counters().caret_ops, carets_before + 1);
    assert_eq!(
        runtime
            .row_views()
            .into_iter()
            .map(|(view, _)| view)
            .collect::<Vec<_>>(),
        vec![c, b, d],
        "the fixture actually changes the final order"
    );
}

#[test]
fn settlement_can_focus_a_same_frame_mount_at_the_removed_position() {
    let mut runtime = Runtime::new();
    runtime
        .frame(|tx| {
            for title in ["A", "B", "C", "D"] {
                tx.insert(title)?;
            }
            Ok(())
        })
        .expect("seed");
    let keys: Vec<RowKey> = runtime.committed().keys().collect();
    let views = mount_all(&mut runtime);
    let [a, b, c, old_d]: [ViewId; 4] = views.try_into().expect("four views");
    runtime.frame(|tx| tx.focus(a)).expect("focus A");
    let moves_before = runtime.counters().focus_moves;
    let carets_before = runtime.counters().caret_ops;

    let new_d = runtime
        .frame(|tx| {
            tx.unmount(a)?;
            tx.unmount(old_d)?;
            let new_d = tx.mount_row(keys[3], 4)?;
            tx.reorder_views(&[new_d, b, c])?;
            Ok(new_d)
        })
        .expect("remove the focused View and mount its new next sibling");

    assert_eq!(runtime.focused(), Some(new_d));
    assert_eq!(
        runtime.caret(new_d),
        Some(4),
        "the mount supplies its own caret"
    );
    assert_eq!(runtime.counters().focus_moves, moves_before + 1);
    assert_eq!(runtime.counters().caret_ops, carets_before + 1);
}

#[test]
fn an_earlier_reorder_in_the_frame_already_provides_the_deletion_position() {
    let mut runtime = Runtime::new();
    seed(&mut runtime);
    let [a, b, c, d]: [ViewId; 4] = mount_all(&mut runtime).try_into().expect("four views");
    runtime.frame(|tx| tx.focus(b)).expect("focus B");

    runtime
        .frame(|tx| {
            let order: Vec<ViewId> = tx.row_views().into_iter().map(|(view, _)| view).collect();
            assert_eq!(order, vec![a, b, c, d]);
            let order: Vec<ViewId> = [a, c, b, d].into_iter().filter(|view| *view != b).collect();
            tx.reorder_views(&order)?;
            tx.unmount(b)
        })
        .expect("move B next to D before deleting it");

    assert_eq!(
        runtime.focused(),
        Some(c),
        "C is the next surviving sibling at B's staged deletion position"
    );
    assert_eq!(runtime.caret(c), Some(2));
}
