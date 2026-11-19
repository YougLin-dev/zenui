use zenui_app::{TodoApp, render};
use zenui_core::{Fatal, Filter, RowKey, ViewId};

fn view_keyed(app: &TodoApp, key: RowKey) -> ViewId {
    app.runtime()
        .row_views()
        .into_iter()
        .find_map(|(view, candidate)| (candidate == key).then_some(view))
        .expect("row has a View")
}

#[test]
fn a_returning_row_gets_a_fresh_once_read_and_new_owner() {
    let mut app = TodoApp::new();
    app.frame(|tx, ui| {
        tx.insert("A")?;
        tx.insert("B")?;
        tx.insert("C")?;
        render(tx, ui)
    })
    .expect("seed");
    let (b_key, b_row) = app.row_by_title("B").expect("B row");
    let old_b = view_keyed(&app, b_key);
    let old_owner = app.runtime().view_owner(old_b).expect("B owner");
    app.frame(|tx, _ui| tx.focus(old_b)).expect("focus B");
    assert_eq!(
        app.runtime().caret(old_b),
        Some("B".chars().count()),
        "the focused View stores its mount-time title length"
    );

    app.frame(|tx, ui| {
        tx.set_completed(b_row, true).map_err(|_| Fatal::Aborted)?;
        ui.filter = Filter::Active;
        render(tx, ui)
    })
    .expect("B leaves the active view");
    assert!(
        !app.runtime()
            .row_views()
            .iter()
            .any(|(view, _)| *view == old_b)
    );
    assert_eq!(app.runtime().view_owner(old_b), None);
    assert_eq!(app.runtime().caret(old_b), None, "the old caret is gone");

    app.frame(|tx, _ui| tx.set_title(b_row, "B updated").map_err(|_| Fatal::Aborted))
        .expect("rename the hidden row");

    let reads_before = app.runtime().counters().io_reads;
    app.frame(|tx, ui| {
        ui.filter = Filter::All;
        render(tx, ui)
    })
    .expect("B returns");

    let new_b = view_keyed(&app, b_key);
    let new_owner = app.runtime().view_owner(new_b).expect("new B owner");
    assert_ne!(new_b, old_b, "a returning row gets a fresh View");
    assert_ne!(new_owner, old_owner, "a returning row gets a fresh Owner");
    assert!(
        app.snapshot().visible_titles.contains("B updated"),
        "the mount-time read must use the current title: {}",
        app.snapshot().visible_titles
    );
    assert_eq!(
        app.runtime().counters().io_reads - reads_before,
        1,
        "only the returning row performs a mount-time read"
    );
    assert_eq!(
        app.runtime().caret(new_b),
        None,
        "old caret is not inherited"
    );
    assert_ne!(
        app.runtime().focused(),
        Some(new_b),
        "old focus is not inherited"
    );

    app.frame(|tx, _ui| tx.focus(new_b))
        .expect("focus the returning row");
    assert_eq!(
        app.runtime().caret(new_b),
        Some("B updated".chars().count()),
        "the new View stores the fresh mount-time read, not the old title length"
    );
}

#[test]
fn unchanged_rows_keep_identity_and_are_not_read_again() {
    let mut app = TodoApp::new();
    app.frame(|tx, ui| {
        tx.insert("A")?;
        tx.insert("B")?;
        tx.insert("C")?;
        render(tx, ui)
    })
    .expect("seed");
    let (a_key, _) = app.row_by_title("A").expect("A row");
    let (b_key, b_row) = app.row_by_title("B").expect("B row");
    let (c_key, _) = app.row_by_title("C").expect("C row");
    let old_a = view_keyed(&app, a_key);
    let old_b = view_keyed(&app, b_key);
    let old_c = view_keyed(&app, c_key);
    let old_a_owner = app.runtime().view_owner(old_a).expect("A owner");
    let old_b_owner = app.runtime().view_owner(old_b).expect("B owner");
    let old_c_owner = app.runtime().view_owner(old_c).expect("C owner");

    let reads_before = app.runtime().counters().io_reads;
    app.frame(|tx, ui| {
        tx.set_completed(b_row, true).map_err(|_| Fatal::Aborted)?;
        ui.filter = Filter::Active;
        render(tx, ui)
    })
    .expect("filter hides B");
    assert_eq!(
        app.runtime().counters().io_reads - reads_before,
        0,
        "hiding a row must not read any title"
    );
    assert_eq!(view_keyed(&app, a_key), old_a);
    assert_eq!(view_keyed(&app, c_key), old_c);
    assert_eq!(app.runtime().view_owner(old_a), Some(old_a_owner));
    assert_eq!(app.runtime().view_owner(old_c), Some(old_c_owner));

    app.frame(|tx, ui| {
        tx.set_title(b_row, "B updated")
            .map_err(|_| Fatal::Aborted)?;
        tx.reorder(&[c_key, a_key, b_key])
            .map_err(|_| Fatal::Aborted)?;
        render(tx, ui)
    })
    .expect("rename the hidden row and reorder the model");
    assert_eq!(view_keyed(&app, a_key), old_a);
    assert_eq!(view_keyed(&app, c_key), old_c);
    assert_eq!(app.runtime().view_owner(old_a), Some(old_a_owner));
    assert_eq!(app.runtime().view_owner(old_c), Some(old_c_owner));
    assert_eq!(
        app.runtime().counters().io_reads - reads_before,
        0,
        "filtering and reordering mounted rows must not read titles"
    );

    let mounts_before = app.runtime().counters().mounts;
    app.frame(|tx, ui| {
        ui.filter = Filter::All;
        render(tx, ui)
    })
    .expect("B returns");
    let new_b = view_keyed(&app, b_key);
    let new_b_owner = app.runtime().view_owner(new_b).expect("new B owner");
    assert_eq!(view_keyed(&app, a_key), old_a, "A stays mounted");
    assert_eq!(view_keyed(&app, c_key), old_c, "C stays mounted");
    assert_eq!(app.runtime().view_owner(old_a), Some(old_a_owner));
    assert_eq!(app.runtime().view_owner(old_c), Some(old_c_owner));
    assert_ne!(new_b, old_b);
    assert_ne!(new_b_owner, old_b_owner);
    assert_eq!(app.runtime().counters().mounts, mounts_before + 1);
    assert_eq!(
        app.runtime().counters().io_reads - reads_before,
        1,
        "only the returning row reads its mounted title"
    );
    assert_eq!(app.snapshot().visible_titles, "C,A,B updated");
}
