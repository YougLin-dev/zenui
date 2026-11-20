#![allow(dead_code, unused_braces)]

#[path = "../examples/minimal_todo.rs"]
mod todo;

use todo::*;
use zenui::prelude::*;

fn runtime() -> Runtime {
    let mut app = Runtime::new();
    app.open_window("main", |window| view! { <TodoApp window={window} /> })
        .expect("open window");
    app
}

fn add(app: &mut Runtime, title: &str) {
    app.input("What needs to be done?", title).expect("input");
    app.submit("Add task").expect("submit");
}

#[test]
fn the_minimal_todo_adds_rows_and_clears_the_draft() {
    let mut app = runtime();
    assert!(app.texts().iter().any(|text| text == "Nothing here yet"));

    add(&mut app, "A");
    assert!(app.texts().iter().any(|text| text == "A"));
    assert!(app.texts().iter().any(|text| text == "1 remaining"));
    assert_eq!(app.for_items().len(), 1);

    add(&mut app, "B");
    assert_eq!(app.for_items().len(), 2);
    assert!(app.texts().iter().any(|text| text == "2 remaining"));
    assert!(
        !app.texts().iter().any(|text| text == "Nothing here yet"),
        "the empty branch is unmounted"
    );
}

#[test]
fn filtering_remounts_rows_with_a_fresh_once_read() {
    let mut app = runtime();
    add(&mut app, "A");

    let first = app.for_items()[0];
    let reads = app.once_reads();

    app.input("Edit A", "A updated").expect("rename");
    assert!(
        app.texts().iter().any(|text| text == "A"),
        "the mounted row keeps its once title"
    );

    app.change("Complete A", true).expect("complete");
    app.click_text("Active").expect("filter active");
    assert!(
        !app.texts().iter().any(|text| text == "A"),
        "the completed row is filtered out"
    );
    assert!(
        !app.for_items().iter().any(|item| item.todo == first.todo),
        "the hidden row is unmounted"
    );

    app.click_text("All").expect("filter all");
    let returned = app
        .for_items()
        .into_iter()
        .find(|item| item.todo == first.todo)
        .expect("the row returns");
    assert_ne!(
        returned.instance, first.instance,
        "fresh component instance"
    );
    assert_ne!(returned.owner, first.owner, "fresh Owner");
    assert_eq!(
        app.once_reads(),
        reads + 1,
        "only the returning row rereads"
    );
    assert!(
        app.texts().iter().any(|text| text == "A updated"),
        "the remount reads the current title: {:?}",
        app.texts()
    );

    app.change("Complete A updated", false)
        .expect("reactivate the row");
    app.click_text("Active").expect("filter active again");
    assert!(
        app.for_items().iter().any(|item| item.todo == first.todo),
        "the candidate row returns to the active filter"
    );
}

#[test]
fn deleting_a_row_removes_the_model_and_interface() {
    let mut app = runtime();
    add(&mut app, "A");
    assert_eq!(app.for_items().len(), 1);

    app.click_label("Delete A").expect("delete");

    assert!(app.for_items().is_empty(), "the row unmounts");
    assert!(!app.texts().iter().any(|text| text == "A"));
    assert!(app.texts().iter().any(|text| text == "Nothing here yet"));
}

#[test]
fn reorder_keeps_row_identity_owner_and_local_editing_state() {
    let mut app = runtime();
    add(&mut app, "A");
    add(&mut app, "B");

    let before = app.for_items();
    let first = before[0];
    let second = before[1];
    let unmounts = app.unmounts();

    assert!(app.set_caret("Edit A", 2), "row A has a text input");
    app.click_text("Reverse").expect("reorder");

    let after = app.for_items();
    assert_eq!(after[0].todo, second.todo);
    assert_eq!(after[1].todo, first.todo);
    let moved = after
        .iter()
        .find(|item| item.todo == first.todo)
        .expect("row A");
    assert_eq!(moved.instance, first.instance, "same component instance");
    assert_eq!(moved.owner, first.owner, "same Owner");
    assert_eq!(app.caret("Edit A"), Some(2), "local editing state survives");
    assert_eq!(
        app.setup_count("TodoItem"),
        2,
        "reorder does not rerun setup"
    );
    assert_eq!(app.unmounts(), unmounts, "reorder unmounts nothing");
}

#[test]
fn a_field_update_does_not_rerun_the_row_component() {
    let mut app = runtime();
    add(&mut app, "A");
    let reads = app.once_reads();
    assert_eq!(app.setup_count("TodoItem"), 1);

    app.change("Complete A", true).expect("toggle");

    assert_eq!(app.setup_count("TodoItem"), 1, "no component rerun");
    assert_eq!(app.once_reads(), reads, "no additional once read");
    assert_eq!(app.checked("Complete A"), Some(true));
}

#[test]
fn typed_data_slots_distinguish_reactive_literal_and_once() {
    #[component]
    fn Slots() -> MountResult<MountPlan> {
        let text = signal(String::from("a"));

        view! {
            <view>
                <text>{text}</text>
                <text>{"literal"}</text>
                <text>{once(text.get())}</text>
                <button on:click={move || text.set(String::from("b"))}>"Change"</button>
            </view>
        }
    }

    let mut app = Runtime::new();
    app.mount(|| view! { <Slots /> }).expect("mount");
    assert_eq!(app.texts(), vec!["a", "literal", "a", "Change"]);

    app.click_text("Change").expect("change");
    assert_eq!(app.texts(), vec!["b", "literal", "a", "Change"]);
}

#[test]
fn clearing_completed_rows_updates_the_model_and_interface() {
    let mut app = runtime();
    add(&mut app, "A");
    app.change("Complete A", true).expect("complete");

    app.click_text("Clear done").expect("clear");

    assert!(app.for_items().is_empty());
    assert!(!app.texts().iter().any(|text| text == "A"));
    assert!(app.texts().iter().any(|text| text == "Nothing here yet"));
}

#[test]
fn empty_and_no_match_states_stay_distinct() {
    let mut app = runtime();
    add(&mut app, "A");
    app.change("Complete A", true).expect("complete");
    app.click_text("Active").expect("filter");

    assert!(app.for_items().is_empty());
    assert!(app.texts().iter().any(|text| text == "Nothing matches"));
    assert!(!app.texts().iter().any(|text| text == "Nothing here yet"));

    app.click_text("All").expect("all");
    app.click_text("Clear done").expect("clear");
    assert!(app.texts().iter().any(|text| text == "Nothing here yet"));
    assert!(!app.texts().iter().any(|text| text == "Nothing matches"));
}
