// crates/core/tests/gates.rs
//!
//! Gate fixtures for `SPEC-GATE-A0-03`, `-04` and `-06`.
//!
//! Each one is written so that deleting the implementation it covers makes it
//! fail: a fixture that stays green when the feature is removed proves nothing.

use zenui_core::{Fatal, RowId, Runtime, StoreError, Tx, ValueError, ViewId};

/// A value error never aborts a transaction; a fixture that wants to abort on
/// one has to say so explicitly.
fn fatal<T>(result: Result<T, ValueError>) -> Result<T, Fatal> {
    result.map_err(|_| Fatal::Aborted)
}

/// Every row key visible through the frame's own overlay.
fn runtime_keys(tx: &mut Tx<'_>) -> Vec<zenui_core::RowKey> {
    tx.rows(zenui_core::Filter::All).iter().collect()
}

/// Authority state that a rollback must leave untouched (`SPEC-RT-053`).
///
/// Monotonic diagnostics are deliberately excluded: a failed frame's reads
/// really happened, and `SPEC-RT-056`'s rule for external I/O applies to them.
/// Keys and values that went out with one publication.
type Published = (Vec<u64>, Vec<(u64, bool)>);

#[derive(Debug, PartialEq, Eq)]
struct Authority {
    titles: Vec<String>,
    /// Model content includes the per-row flag, not just the title.
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
    /// The mapping itself, item by item (`SPEC-RT-053`).
    layout_pairs: Vec<(u32, u32)>,
    /// The live owner set, not its size.
    owner_ids: Vec<u32>,
    /// Id high-water marks: a discarded frame must not leave a gap that shows
    /// up as `focus=view#N` later (`SPEC-RT-053`).
    next_view: u32,
    next_owner: u32,
    /// `io_writes` and `commits` advance only on commit.
    io_writes: u64,
    commits: u64,
    /// Pending snapshots. A failed frame publishes nothing.
    publications: usize,
    /// The last published snapshot itself: keys and the committed values that
    /// went out with it (`SPEC-RT-053` names it; a count is not a snapshot).
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

/// `SPEC-GATE-A0-04`: nested batch publishes once, and an inner reader still
/// sees its own uncommitted write (EVID-VAL-08's scenario).
#[test]
fn a0_04_nested_batch_publishes_once_with_the_outer_value() {
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
            // Outer scope: a = true.
            fatal(tx.set_completed(a_row, true))?;

            tx.batch(|tx| {
                // Inner scope: b = true, and the inner reader must see it.
                fatal(tx.set_completed(b_row, true))?;
                assert!(
                    fatal(tx.completed(b_row))?,
                    "an in-frame reader must see its own uncommitted write"
                );
                Ok(())
            })?;

            // Outer scope closes: b = false.
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
    // The publish barrier itself must carry the outer value: exactly one row
    // (a) is completed in the snapshot, never a `true` for b.
    assert_eq!(
        published.values.iter().filter(|(_, done)| *done).count(),
        1,
        "the published snapshot must contain no intermediate value: {:?}",
        published.values
    );
}

/// `SPEC-GATE-A0-06`: a failure **after** staging must leave every authority
/// field exactly as the previous commit left it, and the next frame must work.
#[test]
fn a0_06_rejected_commit_restores_the_previous_commit() {
    let mut runtime = Runtime::new();

    runtime
        .frame(|tx| {
            tx.insert("A")?;
            tx.insert("B")?;
            Ok(())
        })
        .expect("seed");

    let a_row: RowId = runtime.frame(|tx| tx.insert("C")).expect("insert C");

    // `SPEC-GATE-A0-06` asks for an `InvalidGeneration` injection. Under
    // ADR-0017 decision 7 a stale handle is a *value* error, so the frame
    // commits and the injection is pinned by "the rejected write leaves the
    // authority identical", not by a rollback.
    let doomed_row: RowId = runtime.frame(|tx| tx.insert("Z")).expect("insert Z");
    runtime
        .frame(|tx| {
            fatal(tx.remove(doomed_row))?;
            Ok(())
        })
        .expect("remove Z");
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
    assert_eq!(
        runtime.committed().len(),
        3,
        "the rejected write wrote nothing"
    );

    // Mount every row and focus the first one, so the frame below has real
    // view, owner, layout and focus state to roll back.
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

    // Stage a non-empty batch — a row insert, a view mount, a layout plan, a
    // focus move, an unmount, a legal reorder, an update on `C`, then a
    // removal of `C`. Preflight must reject the update/removal conflict after
    // all of that is staged; otherwise the rollback comparison is vacuous.
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

    // The phase guard is reset: the next clean frame commits.
    runtime
        .frame(|tx| {
            tx.insert("E")?;
            Ok(())
        })
        .expect("recovery frame");
    assert!(runtime.committed().titles().any(|title| title == "E"));
}

/// `SPEC-GATE-A0-06`: a duplicate key is a *value* error, not an abort
/// (`ADR-0017` decision 7). The frame continues and commits; the model does
/// not move and `io_writes` does not advance.
#[test]
fn a0_06_a_duplicate_key_is_a_value_error_that_keeps_the_frame_alive() {
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
            // The transaction is still alive: a rejected write is not fatal.
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

/// `ADR-0017` decision 3: a failed `read_once` does not count as I/O.
#[test]
fn a0_03_a_failed_read_does_not_advance_io_reads() {
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

/// `SPEC-GATE-A0-03`: after a slot is recycled, the old handle resolves to
/// nothing — it must not read the row that took its place.
#[test]
fn a0_03_stale_handle_does_not_alias_the_reused_slot() {
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

/// `SPEC-GATE-A0-03`: a handle from another runtime is rejected on read and on
/// write, and the foreign store is untouched.
#[test]
fn a0_03_cross_runtime_access_is_rejected() {
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
