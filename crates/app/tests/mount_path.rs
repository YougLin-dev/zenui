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
