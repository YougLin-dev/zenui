// crates/app/tests/mount_path.rs
//!
//! `SPEC-GATE-A0-02` and `SPEC-RT-045`: the mount executor, not a fixture that
//! calls `read_once` itself.
//!
//! `render()` is what a UI layer actually runs: it reads the current query,
//! keeps the views whose rows are still visible, and mounts the rest by reading
//! each title exactly once. The counters are the criterion, so a fixture that
//! performed the read on the executor's behalf would measure itself.

use zenui_app::TodoApp;
use zenui_core::Filter;

#[test]
fn a0_02_each_new_view_costs_exactly_one_owned_read() {
    let mut app = TodoApp::new();
    app.frame(|tx, _ui| {
        tx.insert("A")?;
        tx.insert("B")?;
        tx.insert("C")?;
        Ok(())
    })
    .expect("seed");

    let before = app.runtime().counters().io_reads;
    app.render().expect("first render");
    assert_eq!(app.runtime().row_views().len(), 3);
    assert_eq!(
        app.runtime().counters().io_reads - before,
        3,
        "one owned read per mounted row"
    );

    // Rendering again mounts nothing, so it reads nothing: the executor keeps
    // the view identity and does not re-read a title it already has.
    let before = app.runtime().counters().io_reads;
    let mounts_before = app.runtime().counters().mounts;
    app.render().expect("second render");
    assert_eq!(
        app.runtime().counters().io_reads - before,
        0,
        "an existing view is not re-read"
    );
    assert_eq!(
        app.runtime().counters().mounts,
        mounts_before,
        "an existing view is not remounted"
    );
    assert_eq!(app.runtime().row_views().len(), 3);
}

/// `SPEC-RT-045`: the snapshot carries keys, never titles, so binding a query
/// to views clones no `String` on top of the one owned read per mount.
#[test]
fn a0_07_binding_a_filter_clones_no_titles() {
    let mut app = TodoApp::new();
    app.frame(|tx, _ui| {
        for title in ["A", "B", "C"] {
            tx.insert(title)?;
        }
        Ok(())
    })
    .expect("seed");

    let before = app.runtime().counters().string_clones;
    app.render().expect("render");
    // Exactly the three owned reads of the mounts; the query itself adds none.
    assert_eq!(
        app.runtime().counters().string_clones - before,
        3,
        "three mount reads and nothing else"
    );
}

/// `SPEC-GATE-A0-07`: two consumers of one filter's row set clone no titles.
///
/// See also `a0_07_two_consumers_share_one_snapshot_and_clone_no_titles` in
/// `zenui-core`'s `tests/queries.rs`: that one runs on the runtime path and adds
/// the shared-allocation identity and the cross-commit hit, while this fixture
/// is the app-level witness next to the binding path below.
///
/// Red-ability: make `RowSet` carry owned titles (instead of `RowKey`) and
/// binding it to views clones them again; the counter moves off zero.
#[test]
fn a0_07_two_consumers_of_one_filter_clone_no_titles() {
    let mut app = TodoApp::new();
    app.frame(|tx, _ui| {
        for title in ["A", "B", "C"] {
            tx.insert(title)?;
        }
        Ok(())
    })
    .expect("seed");
    app.render().expect("first render");
    let before = app.runtime().counters();

    app.frame(|tx, ui| {
        let first = tx.rows(Filter::All);
        let second = tx.rows(Filter::All);
        assert_eq!(
            first.iter().collect::<Vec<_>>(),
            second.iter().collect::<Vec<_>>(),
            "both consumers see the same row set"
        );
        zenui_app::render(tx, ui)?;
        Ok(())
    })
    .expect("two consumers");

    let after = app.runtime().counters();
    assert_eq!(
        after.string_clones - before.string_clones,
        0,
        "two consumers of one row set clone no titles"
    );
    assert_eq!(
        after.io_reads - before.io_reads,
        0,
        "an existing view is not re-read"
    );
}
