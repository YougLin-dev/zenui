use zenui_app::{TodoApp, render};
use zenui_core::{Fatal, Filter, PUBLICATION_HISTORY, ViewId};

fn seed(rows: usize) -> TodoApp {
    let mut app = TodoApp::new();
    app.frame(|tx, ui| {
        for index in 0..rows {
            tx.insert(format!("row-{index:03}"))?;
        }
        render(tx, ui)
    })
    .expect("seed");
    app
}

#[test]
fn unchanged_reconciliation_does_no_structural_work_and_stays_bounded() {
    let mut app = seed(200);
    let mounts = app.runtime().counters().mounts;
    let unmounts = app.runtime().counters().unmounts;
    let views = app.runtime().row_views();
    let owners = app.runtime().live_owners();
    let storage = app.runtime().owner_storage_len();
    let commits = app.runtime().txn().commits();

    for _ in 0..40 {
        app.render().expect("unchanged render");
    }

    assert_eq!(app.runtime().counters().mounts, mounts, "no remount");
    assert_eq!(app.runtime().counters().unmounts, unmounts, "no unmount");
    assert_eq!(
        app.runtime().row_views(),
        views,
        "row identity and order stay"
    );
    assert_eq!(app.runtime().live_owners(), owners);
    assert_eq!(app.runtime().owner_storage_len(), storage);
    assert_eq!(
        app.runtime().txn().commits(),
        commits + 40,
        "each idle frame is still one commit"
    );
    let publications = app.runtime().txn().publications();
    assert_eq!(publications.len(), PUBLICATION_HISTORY);
    assert!(publications.last().expect("publication").keys.is_empty());
    assert_eq!(app.snapshot().visible_rows, 200);
}

#[test]
fn focus_only_frames_keep_a_bounded_publication_history() {
    let mut app = seed(50);
    let views: Vec<ViewId> = app
        .runtime()
        .row_views()
        .into_iter()
        .map(|(view, _)| view)
        .collect();

    for round in 0..PUBLICATION_HISTORY * 3 {
        let view = views[round % views.len()];
        app.frame(|tx, _| tx.focus(view)).expect("focus frame");
    }

    let commits = app.runtime().txn().commits();
    let publications = app.runtime().txn().publications();
    assert_eq!(publications.len(), PUBLICATION_HISTORY);
    let last = publications.last().expect("publication");
    assert_eq!(last.commit_seq, commits - 1);
    assert!(last.keys.is_empty(), "focus changes no model keys");
    assert_eq!(
        app.runtime().live_owners(),
        1 + views.len(),
        "focus does not churn owners"
    );
}

#[test]
fn filter_churn_returns_owner_storage_to_the_live_set() {
    const ROWS: usize = 80;
    let mut app = TodoApp::new();
    app.frame(|tx, ui| {
        for index in 0..ROWS {
            let row = tx.insert(format!("row-{index:03}"))?;
            tx.set_completed(row, true).map_err(|_| Fatal::Aborted)?;
        }
        render(tx, ui)
    })
    .expect("seed completed rows");

    for _ in 0..25 {
        app.frame(|tx, ui| {
            ui.filter = Filter::Active;
            render(tx, ui)
        })
        .expect("hide completed rows");
        assert_eq!(app.runtime().live_owners(), 1, "only the root survives");
        assert_eq!(
            app.runtime().owner_storage_len(),
            app.runtime().live_owners(),
            "disposed Owner storage is released"
        );

        app.frame(|tx, ui| {
            ui.filter = Filter::All;
            render(tx, ui)
        })
        .expect("show completed rows again");
        assert_eq!(app.runtime().live_owners(), 1 + ROWS);
        assert_eq!(
            app.runtime().owner_storage_len(),
            app.runtime().live_owners(),
            "remount storage tracks the live set"
        );
        assert_eq!(app.runtime().row_views().len(), ROWS);
    }

    assert_eq!(app.runtime().counters().unmounts, ROWS as u64 * 25);
    assert!(
        app.runtime().txn().publications().len() <= PUBLICATION_HISTORY,
        "repeated churn does not retain publication history"
    );
}
