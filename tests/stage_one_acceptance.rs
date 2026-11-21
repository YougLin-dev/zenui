#![allow(dead_code, unused_braces)]

use zenui::prelude::*;
use zenui_demo::*;

fn runtime() -> Runtime {
    let mut app = Runtime::new();
    open_todo_window(&mut app).expect("open window");
    app
}

fn add(app: &mut Runtime, title: &str) {
    app.input("What needs to be done?", title).expect("input");
    app.submit("Add task").expect("submit");
}

#[test]
fn the_target_todo_completes_the_stage_one_workflow() {
    let mut app = runtime();
    add(&mut app, "Alpha");
    add(&mut app, "Beta");
    assert_eq!(app.for_items().len(), 2);
    assert_eq!(app.setup_count("TodoItem"), 2);

    let before = app.for_items();
    let reads = app.once_reads();
    app.change("Complete Alpha", true).expect("toggle");
    assert_eq!(
        app.setup_count("TodoItem"),
        2,
        "a field write must not rerun component setup"
    );
    assert_eq!(
        app.once_reads(),
        reads,
        "a field write must not reread a once slot"
    );
    assert_eq!(app.checked("Complete Alpha"), Some(true));

    app.click_text("Active").expect("filter active");
    assert_eq!(app.for_items().len(), 1);
    assert_eq!(app.for_items()[0].todo, before[1].todo);
    app.click_text("All").expect("filter all");
    assert_eq!(app.for_items().len(), 2);

    let before_reorder = app.for_items();
    let unmounts = app.unmounts();
    assert!(app.set_caret("Edit Alpha", 2), "Alpha has a text input");
    app.click_text("Reverse").expect("reorder");
    let after = app.for_items();
    assert_eq!(after[0].todo, before_reorder[1].todo);
    assert_eq!(after[1].todo, before_reorder[0].todo);
    let alpha = after
        .iter()
        .find(|item| item.todo == before_reorder[0].todo)
        .expect("Alpha is still mounted");
    assert_eq!(
        alpha.instance, before_reorder[0].instance,
        "business identity kept"
    );
    assert_eq!(alpha.owner, before_reorder[0].owner, "Owner kept");
    assert_eq!(
        app.caret("Edit Alpha"),
        Some(2),
        "local editing state survives reorder"
    );
    assert_eq!(app.unmounts(), unmounts, "reorder unmounted nothing");
    assert_eq!(
        app.setup_count("TodoItem"),
        3,
        "only the filter remount created a new setup"
    );

    app.click_label("Delete Beta").expect("remove");
    assert_eq!(app.for_items().len(), 1);
    assert!(!app.texts().iter().any(|text| text == "Beta"));
    assert!(
        app.texts().iter().any(|text| text == "0 remaining"),
        "the Store transaction removed the only active row"
    );
    assert_eq!(
        app.setup_count("TodoItem"),
        3,
        "business removal unmounted the row without rerunning components"
    );
}
