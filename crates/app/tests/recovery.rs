use zenui_app::{TodoApp, render};
use zenui_core::{Fatal, Filter, ValueError};

#[test]
fn an_aborted_filter_frame_restores_ui_and_visible_views() {
    let mut app = TodoApp::new();
    app.frame(|tx, ui| {
        tx.insert("A")?;
        tx.insert("B")?;
        tx.insert("C")?;
        render(tx, ui)
    })
    .expect("seed");
    let (_, b_row) = app.row_by_title("B").expect("B row");
    app.frame(|tx, ui| {
        tx.set_completed(b_row, true).map_err(|_| Fatal::Aborted)?;
        render(tx, ui)
    })
    .expect("complete B");

    let ui_before = app.ui().clone();
    let visible_before = app.runtime().row_views();
    let commits_before = app.runtime().txn().commits();
    let result: Result<(), Fatal> = app.frame(|tx, ui| {
        ui.filter = Filter::Active;
        ui.draft = String::from("abandoned");
        ui.sink.push(String::from("abandoned frame output"));
        tx.insert("D")?;
        render(tx, ui)?;
        Err(Fatal::Aborted)
    });

    assert_eq!(result, Err(Fatal::Aborted));
    assert_eq!(app.ui(), &ui_before, "all UI values are rolled back");
    assert_eq!(app.runtime().row_views(), visible_before);
    assert_eq!(
        app.runtime().txn().commits(),
        commits_before,
        "the aborted frame publishes nothing"
    );
    assert_eq!(
        app.snapshot().store_titles,
        "A,B,C",
        "the staged insert did not publish"
    );
}

#[test]
fn a_rejected_stale_handle_is_not_a_frame_abort() {
    let mut app = TodoApp::new();
    let row = app.frame(|tx, _ui| tx.insert("A")).expect("insert A");
    app.frame(|tx, _ui| {
        tx.remove(row).map_err(|_| Fatal::Aborted)?;
        Ok(())
    })
    .expect("remove the row but keep its View mounted");
    let commits_before = app.runtime().txn().commits();

    let result: Result<Result<bool, ValueError>, Fatal> = app.frame(|tx, ui| {
        let rejected = tx.toggle(row);
        render(tx, ui)?;
        Ok(rejected)
    });

    let Ok(Err(ValueError::Handle(error))) = result else {
        panic!("the stale handle must be rejected through the value channel: {result:?}");
    };
    assert_eq!(error.reason_kind(), "vacant");
    assert_eq!(
        app.runtime().txn().commits(),
        commits_before + 1,
        "the accepted frame still commits its UI cleanup"
    );
    assert_eq!(app.snapshot().store_rows, 0);
    assert_eq!(app.snapshot().visible_rows, 0);
}

#[test]
fn add_todo_commits_one_row_renders_it_and_clears_the_draft() {
    let mut app = TodoApp::new();
    app.set_draft("Write tests");

    app.add_todo().expect("add succeeds");

    assert_eq!(app.snapshot().store_titles, "Write tests");
    assert_eq!(app.snapshot().visible_titles, "Write tests");
    assert_eq!(app.draft(), "");
}

#[test]
fn an_empty_draft_is_a_noop_without_a_commit() {
    let mut app = TodoApp::new();
    let commits_before = app.runtime().txn().commits();

    app.add_todo().expect("an empty add is accepted");

    assert_eq!(app.runtime().txn().commits(), commits_before);
    assert_eq!(app.snapshot().store_rows, 0);
    assert_eq!(app.draft(), "");
}
