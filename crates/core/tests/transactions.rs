use zenui_core::{Fatal, RowId, Runtime, StoreError, Tx, ValueError, ViewId};

fn fatal<T>(result: Result<T, ValueError>) -> Result<T, Fatal> {
    result.map_err(|_| Fatal::Aborted)
}

fn runtime_keys(tx: &mut Tx<'_>) -> Vec<zenui_core::RowKey> {
    tx.rows(zenui_core::Filter::All).iter().collect()
}

type Published = (Vec<u64>, Vec<(u64, bool)>);

#[derive(Debug, PartialEq, Eq)]
struct Authority {
    titles: Vec<String>,

    completed: Vec<bool>,
    order: Vec<u64>,
    epoch: u64,
    remaining: usize,
    has_completed: bool,
    views: Vec<(u32, u64)>,
    focused: Option<u32>,
    caret: Option<usize>,
    geometry_epoch: u64,
    live_owners: usize,
    layout_nodes: usize,

    layout_pairs: Vec<(u32, u32)>,

    owner_ids: Vec<u32>,

    next_view: u32,
    next_owner: u32,

    io_writes: u64,
    commits: u64,

    publications: usize,

    published: Option<Published>,
}

fn authority(runtime: &Runtime) -> Authority {
    let store = runtime.committed();
    let views = runtime
        .row_views()
        .into_iter()
        .map(|(view, key)| (view.index(), key.raw()))
        .collect();
    let focused = runtime.focused();
    Authority {
        titles: store.titles().map(str::to_owned).collect(),
        completed: store.rows().map(|row| row.completed()).collect(),
        order: store.keys().map(|key| key.raw()).collect(),
        epoch: store.epoch(),
        remaining: store.remaining(),
        has_completed: store.has_completed(),
        views,
        focused: focused.map(ViewId::index),
        caret: focused.and_then(|view| runtime.caret(view)),
        geometry_epoch: runtime.geometry_epoch(),
        live_owners: runtime.live_owners(),
        layout_nodes: runtime.layout_len(),
        layout_pairs: runtime
            .layout_pairs()
            .into_iter()
            .map(|(view, node)| (view.index(), node.index()))
            .collect(),
        owner_ids: runtime
            .live_owner_ids()
            .into_iter()
            .map(|owner| owner.index())
            .collect(),
        next_view: runtime.next_view(),
        next_owner: runtime.next_owner(),
        io_writes: store.counters().io_writes,
        commits: runtime.txn().commits(),
        publications: runtime.txn().publications().len(),
        published: runtime.txn().publications().last().map(|publication| {
            (
                publication.keys.iter().map(|key| key.raw()).collect(),
                publication
                    .values
                    .iter()
                    .map(|(key, done)| (key.raw(), *done))
                    .collect(),
            )
        }),
    }
}

#[test]
fn nested_batch_publishes_once_with_the_outer_value() {
    let mut runtime = Runtime::new();

    let (a_row, b_row) = runtime
        .frame(|tx| {
            let a = tx.insert("a")?;
            let b = tx.insert("b")?;
            Ok((a, b))
        })
        .expect("seed");

    let commits_before = runtime.txn().commits();
    let published_before = runtime.txn().publications().len();

    runtime
        .frame(|tx| {
            fatal(tx.set_completed(a_row, true))?;

            tx.batch(|tx| {
                fatal(tx.set_completed(b_row, true))?;
                assert!(
                    fatal(tx.completed(b_row))?,
                    "an in-frame reader must see its own uncommitted write"
                );
                Ok(())
            })?;

            fatal(tx.set_completed(b_row, false))?;
            assert!(!fatal(tx.completed(b_row))?);
            Ok(())
        })
        .expect("frame");

    let publications = runtime.txn().publications();
    assert_eq!(
        publications.len(),
        published_before + 1,
        "one interaction publishes exactly once"
    );
    assert_eq!(runtime.txn().commits(), commits_before + 1);

    let published = publications.last().expect("publication");
    assert_eq!(published.keys.len(), 2, "both rows are in the batch");
    let store = runtime.committed();
    let b_title = store.titles().nth(1).expect("b");
    assert_eq!(b_title, "b");
    assert!(
        !store.rows().nth(1).expect("b row").completed(),
        "the published value is the outer one, not the intermediate `true`"
    );

    assert_eq!(
        published.values.iter().filter(|(_, done)| *done).count(),
        1,
        "the published snapshot must contain no intermediate value: {:?}",
        published.values
    );
}

#[test]
fn rejected_commit_restores_the_previous_commit() {
    let mut runtime = Runtime::new();

    runtime
        .frame(|tx| {
            tx.insert("A")?;
            tx.insert("B")?;
            Ok(())
        })
        .expect("seed");

    let a_row: RowId = runtime.frame(|tx| tx.insert("C")).expect("insert C");

    let doomed_row: RowId = runtime.frame(|tx| tx.insert("Z")).expect("insert Z");
    runtime
        .frame(|tx| {
            fatal(tx.remove(doomed_row))?;
            Ok(())
        })
        .expect("remove Z");
    let before_stale = authority(&runtime);
    runtime
        .frame(|tx| {
            let rejected = tx.toggle(doomed_row);
            assert!(
                matches!(rejected, Err(ValueError::Handle(_))),
                "a stale generation must be rejected, got {rejected:?}"
            );
            Ok(())
        })
        .expect("the frame still commits");
    let mut after_stale = authority(&runtime);

    assert_eq!(
        after_stale
            .published
            .as_ref()
            .map(|snapshot| snapshot.1.clone()),
        before_stale
            .published
            .as_ref()
            .map(|snapshot| snapshot.1.clone()),
        "the published snapshot values must not move"
    );
    assert_eq!(
        after_stale
            .published
            .as_ref()
            .map(|snapshot| snapshot.0.len()),
        Some(0),
        "a rejected write publishes no changed keys"
    );
    after_stale.commits = before_stale.commits;
    after_stale.publications = before_stale.publications;
    after_stale.published = before_stale.published.clone();
    assert_eq!(
        before_stale, after_stale,
        "a rejected stale generation must not move any authority field"
    );
    assert_eq!(
        runtime.committed().len(),
        3,
        "the rejected write wrote nothing"
    );

    runtime
        .frame(|tx| {
            let keys: Vec<_> = runtime_keys(tx);
            let mut views = Vec::with_capacity(keys.len());
            for key in &keys {
                views.push(tx.mount_row(*key, 1)?);
            }
            tx.reorder_views(&views)?;
            tx.focus(views[0])?;
            Ok(())
        })
        .expect("mount all");
    assert_eq!(runtime.row_views().len(), 3);
    assert!(runtime.layout_pairs().len() == 3);

    let before = authority(&runtime);

    let existing: Vec<ViewId> = runtime.row_views().into_iter().map(|(v, _)| v).collect();
    let result = runtime.frame(|tx| {
        let known: Vec<_> = runtime_keys(tx);
        let new_row = tx.insert("D")?;
        let new_key = runtime_keys(tx)
            .into_iter()
            .find(|key| !known.contains(key) && tx.handle_for(*key) == Some(new_row))
            .expect("staged key");
        let new_view = tx.mount_row(new_key, 1)?;
        tx.reorder_views(&[new_view, existing[1], existing[2]])?;
        tx.focus(new_view)?;
        tx.unmount(existing[0])?;
        let keys: Vec<_> = runtime_keys(tx);
        tx.reorder(&keys).map_err(|_| Fatal::OrderNotAPermutation)?;
        fatal(tx.set_completed(a_row, true))?;
        fatal(tx.remove(a_row))?;
        Ok(())
    });

    assert!(
        matches!(result, Err(Fatal::StaleStagedRow { .. })),
        "expected the preflight to reject the staged conflict, got {result:?}"
    );

    let after = authority(&runtime);
    assert_eq!(before, after, "authority state must be identical");

    runtime
        .frame(|tx| {
            tx.insert("E")?;
            Ok(())
        })
        .expect("recovery frame");
    assert!(runtime.committed().titles().any(|title| title == "E"));
}

#[test]
fn a_duplicate_key_is_a_value_error_that_keeps_the_frame_alive() {
    let mut runtime = Runtime::new();

    runtime
        .frame(|tx| {
            tx.insert("A")?;
            tx.insert("B")?;
            Ok(())
        })
        .expect("seed");

    let before = authority(&runtime);

    runtime
        .frame(|tx| {
            let keys: Vec<_> = runtime_keys(tx);
            let duplicate = tx.reorder(&[keys[0], keys[0]]);
            assert!(
                matches!(
                    duplicate,
                    Err(ValueError::Store(StoreError::DuplicateKey(_)))
                ),
                "a repeated key must be rejected as a value error, got {duplicate:?}"
            );

            tx.insert("C")?;
            Ok(())
        })
        .expect("the frame must still commit");

    let after = authority(&runtime);
    assert_eq!(
        after.titles,
        vec!["A".to_owned(), "B".to_owned(), "C".to_owned()]
    );
    assert_eq!(after.commits, before.commits + 1, "the frame committed");
    assert_eq!(
        after.io_writes,
        before.io_writes + 1,
        "only the insert wrote"
    );
    assert_eq!(after.publications, before.publications + 1);
}

#[test]
fn a_duplicate_key_leaves_every_authority_field_identical() {
    let mut runtime = Runtime::new();

    runtime
        .frame(|tx| {
            tx.insert("A")?;
            tx.insert("B")?;
            Ok(())
        })
        .expect("seed");

    runtime
        .frame(|tx| {
            let keys: Vec<_> = runtime_keys(tx);
            let mut views = Vec::with_capacity(keys.len());
            for key in &keys {
                views.push(tx.mount_row(*key, 1)?);
            }
            tx.reorder_views(&views)?;
            tx.focus(views[0])?;
            Ok(())
        })
        .expect("mount");

    let before = authority(&runtime);

    runtime
        .frame(|tx| {
            let keys: Vec<_> = runtime_keys(tx);
            let duplicate = tx.reorder(&[keys[0], keys[0]]);
            assert!(
                matches!(
                    duplicate,
                    Err(ValueError::Store(StoreError::DuplicateKey(_)))
                ),
                "a repeated key must be rejected as a value error, got {duplicate:?}"
            );
            Ok(())
        })
        .expect("the frame still commits");

    let mut after = authority(&runtime);

    after.commits = before.commits;
    after.publications = before.publications;
    assert_eq!(
        before, after,
        "a rejected duplicate key must not move any authority field"
    );
}

#[test]
fn a_failed_read_does_not_advance_io_reads() {
    let mut runtime = Runtime::new();

    let stale: RowId = runtime.frame(|tx| tx.insert("A")).expect("insert A");
    runtime
        .frame(|tx| {
            fatal(tx.remove(stale))?;
            Ok(())
        })
        .expect("remove A");

    let reads_before = runtime.committed().counters().io_reads;
    runtime
        .frame(|tx| {
            let result = tx.read_once(stale);
            assert!(result.is_err(), "a stale handle must not read");
            Ok(())
        })
        .expect("frame");

    assert_eq!(
        runtime.committed().counters().io_reads,
        reads_before,
        "a rejected read must not be counted as an owned read"
    );
}

#[test]
fn stale_handle_does_not_alias_the_reused_slot() {
    let mut runtime = Runtime::new();

    let old: RowId = runtime.frame(|tx| tx.insert("A")).expect("insert A");

    runtime
        .frame(|tx| {
            fatal(tx.remove(old))?;
            Ok(())
        })
        .expect("remove A");

    let new: RowId = runtime.frame(|tx| tx.insert("B")).expect("insert B");
    assert_ne!(old, new, "a recycled slot must mint a new generation");

    runtime
        .frame(|tx| {
            assert!(
                matches!(
                    tx.with_title(old, str::to_owned),
                    Err(ValueError::Handle(_))
                ),
                "a stale handle must not read the recycled slot"
            );
            assert!(
                matches!(tx.completed(old), Err(ValueError::Handle(_))),
                "a stale handle must not read a field"
            );
            assert!(
                matches!(tx.toggle(old), Err(ValueError::Handle(_))),
                "a stale handle must not write"
            );
            assert_eq!(tx.with_title(new, str::to_owned).expect("live row"), "B");
            Ok(())
        })
        .expect("frame");

    assert_eq!(runtime.committed().titles().collect::<Vec<_>>(), vec!["B"]);
}

#[test]
fn a_view_from_an_equal_index_runtime_cannot_be_focused() {
    let mut owner = Runtime::new();
    let mut other = Runtime::new();
    let foreign_view: ViewId = owner
        .frame(|tx| {
            tx.insert("owner")?;
            let key = runtime_keys(tx)[0];
            let view = tx.mount_row(key, 3)?;
            Ok(view)
        })
        .expect("mount owner View");
    let own_view = other
        .frame(|tx| {
            tx.insert("other")?;
            let key = runtime_keys(tx)[0];
            let view = tx.mount_row(key, 4)?;
            tx.reorder_views(&[view])?;
            Ok(view)
        })
        .expect("mount other View");

    assert_eq!(foreign_view.index(), own_view.index());

    let result = other.frame(|tx| tx.focus(foreign_view));

    assert!(
        matches!(result, Err(Fatal::InvalidViewHandle(_))),
        "the foreign View must be rejected, got {result:?}"
    );
    assert_eq!(other.focused(), None);
    assert_eq!(other.caret(foreign_view), None);
}

#[test]
fn a_foreign_view_cannot_change_structure_or_existing_focus() {
    let mut owner = Runtime::new();
    let mut other = Runtime::new();
    let foreign = owner
        .frame(|tx| {
            tx.insert("owner")?;
            let key = runtime_keys(tx)[0];
            tx.mount_row(key, 3)
        })
        .expect("foreign view");
    let owned = other
        .frame(|tx| {
            tx.insert("other")?;
            let key = runtime_keys(tx)[0];
            let view = tx.mount_row(key, 4)?;
            tx.focus(view)?;
            Ok(view)
        })
        .expect("own view");
    assert_eq!(foreign.index(), owned.index());
    assert_eq!(other.caret(foreign), None);
    assert_eq!(other.layout_of(foreign), None);

    let unmount = other.frame(|tx| {
        assert!(!tx.is_live(foreign));
        assert_eq!(tx.caret(foreign), None);
        tx.unmount(foreign)
    });
    assert!(matches!(unmount, Err(Fatal::InvalidViewHandle(_))));
    let reorder = other.frame(|tx| tx.reorder_views(&[foreign]));
    assert!(matches!(reorder, Err(Fatal::InvalidViewHandle(_))));
    assert_eq!(other.row_views().len(), 1);
    assert_eq!(other.live_owners(), 2);
    assert_eq!(other.focused(), Some(owned));
    assert_eq!(other.caret(owned), Some(4));
    assert_eq!(
        owner.row_views(),
        vec![(foreign, owner.committed().keys().next().unwrap())]
    );
}

#[test]
fn a_disposed_view_cannot_target_its_replacement() {
    let mut runtime = Runtime::new();
    let key = runtime
        .frame(|tx| {
            tx.insert("A")?;
            Ok(runtime_keys(tx)[0])
        })
        .expect("key");
    let retired = runtime
        .frame(|tx| {
            let view = tx.mount_row(key, 2)?;
            tx.reorder_views(&[view])?;
            Ok(view)
        })
        .expect("mount to retire");
    runtime
        .frame(|tx| tx.unmount(retired))
        .expect("retire view");
    let replacement = runtime
        .frame(|tx| tx.mount_row(key, 3))
        .expect("replacement");
    assert_ne!(retired, replacement);

    for result in [
        runtime.frame(|tx| tx.focus(retired)),
        runtime.frame(|tx| tx.unmount(retired)),
    ] {
        assert!(matches!(result, Err(Fatal::InvalidViewHandle(_))));
    }
    runtime
        .frame(|tx| {
            tx.reorder_views(&[replacement])?;
            tx.focus(replacement)
        })
        .expect("the replacement accepts legal focus");
    assert_eq!(runtime.focused(), Some(replacement));
    assert_eq!(runtime.caret(retired), None);
    assert_eq!(runtime.layout_of(retired), None);
}

#[test]
fn an_unordered_staged_view_has_no_neighbors_without_being_invalid() {
    let mut runtime = Runtime::new();
    let key = runtime
        .frame(|tx| {
            tx.insert("A")?;
            Ok(runtime_keys(tx)[0])
        })
        .expect("key");

    runtime
        .frame(|tx| {
            let view = tx.mount_row(key, 1)?;
            tx.reorder_views(&[view])?;
            assert!(tx.is_live(view), "staged view identity should be known");
            assert_eq!(tx.focus_intent_for(view), None);
            Ok(())
        })
        .expect("staged reorder");
}

#[test]
fn cross_runtime_access_is_rejected() {
    let mut first = Runtime::new();
    let mut second = Runtime::new();

    let row: RowId = first.frame(|tx| tx.insert("A")).expect("insert");

    second
        .frame(|tx| {
            assert!(
                matches!(tx.with_title(row, |_| ()), Err(ValueError::Handle(_))),
                "a foreign handle must not resolve"
            );
            assert!(tx.read_once(row).is_err());
            assert!(matches!(tx.completed(row), Err(ValueError::Handle(_))));
            assert!(matches!(tx.toggle(row), Err(ValueError::Handle(_))));
            Ok(())
        })
        .expect("frame");

    assert_eq!(second.committed().len(), 0, "the foreign store stays empty");
    assert_eq!(
        first.committed().titles().collect::<Vec<_>>(),
        vec!["A"],
        "the owning store is untouched"
    );
}
