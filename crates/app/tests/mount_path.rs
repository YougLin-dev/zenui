use zenui_app::TodoApp;
use zenui_core::Filter;

#[test]
fn each_new_view_costs_exactly_one_owned_read() {
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

#[test]
fn binding_a_filter_clones_no_titles() {
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

    assert_eq!(
        app.runtime().counters().string_clones - before,
        3,
        "three mount reads and nothing else"
    );
}

#[test]
fn two_consumers_of_one_filter_clone_no_titles() {
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
