// crates/core/tests/gates.rs
//!
//! Gate fixtures for `SPEC-GATE-A0-03`, `-04` and `-06`.
//!
//! Each one is written so that deleting the implementation it covers makes it
//! fail: a fixture that stays green when the feature is removed proves nothing.

use zenui_core::{Fatal, RowId, Runtime, Tx, ValueError, ViewId};

/// A value error never aborts a transaction; a fixture that wants to abort on
/// one has to say so explicitly.
fn fatal<T>(result: Result<T, ValueError>) -> Result<T, Fatal> {
    result.map_err(|_| Fatal::Aborted)
}

/// Authority state that a rollback must leave untouched (`SPEC-RT-053`).
///
/// Monotonic diagnostics are deliberately excluded: a failed frame's reads
/// really happened, and `SPEC-RT-056`'s rule for external I/O applies to them.
#[derive(Debug, PartialEq, Eq)]
struct Authority {
    titles: Vec<String>,
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
    let before = authority(&runtime);

    // Stage a non-empty batch: an insert, a legal reorder, an update on `C`,
    // and then a removal of `C`. Preflight must reject the update/removal
    // conflict — not before staging, which would make the rollback vacuous.
    let result = runtime.frame(|tx| {
        tx.insert("D")?;
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

fn runtime_keys(tx: &mut Tx<'_>) -> Vec<zenui_core::RowKey> {
    let rows: Vec<_> = tx.rows(zenui_core::Filter::All).iter().collect();
    rows
}

/// `SPEC-GATE-A0-03`: after a slot is recycled, the old handle resolves to
/// nothing — it must not read the row that took its place.
#[test]
fn a0_03_stale_handle_does_not_alias_the_reused_slot() {
    let mut runtime = Runtime::new();

    let old = runtime.frame(|tx| tx.insert("A")).expect("insert A");
    let old_key = runtime.committed().keys().next().expect("key");

    runtime
        .frame(|tx| {
            fatal(tx.remove(old))?;
            Ok(())
        })
        .expect("remove A");

    let new = runtime.frame(|tx| tx.insert("B")).expect("insert B");

    assert_ne!(old, new, "a recycled slot must mint a new generation");

    runtime
        .frame(|tx| {
            assert!(
                tx.with_title(old, |_| ()).is_err(),
                "the old handle must not read the row that reused its slot"
            );
            assert!(tx.completed(old).is_err());
            assert!(
                tx.set_completed(old, true).is_err(),
                "the old handle must not write either"
            );
            assert_eq!(fatal(tx.with_title(new, str::to_owned))?, "B");
            Ok(())
        })
        .expect("probe");

    assert_eq!(
        runtime.committed().titles().collect::<Vec<_>>(),
        vec!["B"],
        "the rejected write left B untouched"
    );
    assert!(!runtime.committed().has_completed());
    let _ = old_key;
}

/// `SPEC-GATE-A0-03`: a handle from another runtime is rejected on read and on
/// write, and the foreign store is untouched.
#[test]
fn a0_03_cross_runtime_access_is_rejected() {
    let mut first = Runtime::new();
    let mut second = Runtime::new();

    let row = first.frame(|tx| tx.insert("A")).expect("insert");

    second
        .frame(|tx| {
            assert!(matches!(
                tx.with_title(row, |_| ()),
                Err(ValueError::Handle(_))
            ));
            assert!(matches!(tx.toggle(row), Err(ValueError::Handle(_))));
            Ok(())
        })
        .expect("probe");

    assert_eq!(
        second.committed().len(),
        0,
        "the foreign store is untouched"
    );
    assert_eq!(first.committed().remaining(), 1);
}
