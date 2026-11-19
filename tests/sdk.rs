#![allow(unused_braces)]

use zenui::prelude::*;

#[component]
fn Counter() -> MountResult<MountPlan> {
    let count = signal(0);
    let label = memo(move || format!("count={}", count.get()));

    view! {
        <view>
            <text>{label}</text>
            <button on:click={move || count.set(count.get() + 1)}>"Add"</button>
        </view>
    }
}

#[test]
fn a_component_mounts_once_and_updates_its_binding() {
    let mut app = Runtime::new();
    app.mount(|| view! { <Counter /> }).expect("mount");

    assert_eq!(app.texts(), vec!["count=0", "Add"]);
    assert_eq!(app.setup_count("Counter"), 1);

    app.click_text("Add").expect("click");
    assert_eq!(app.texts(), vec!["count=1", "Add"]);
    assert_eq!(
        app.setup_count("Counter"),
        1,
        "a field update does not rerun component setup"
    );
}

#[component]
fn RowCount() -> MountResult<MountPlan> {
    let store = Store::new();
    let count = memo(move || store.rows(Filter::All).len());
    let label = memo(move || format!("n={}", count.get()));

    view! {
        <view>
            <text>{label}</text>
            <button on:click={move || {
                let _ = Transaction::new().write(store.insert("x")).finish();
            }}>"Insert"</button>
        </view>
    }
}

#[test]
fn a_rows_memo_sees_a_committed_insert() {
    let mut app = Runtime::new();
    app.mount(|| view! { <RowCount /> }).expect("mount");
    assert_eq!(app.texts(), vec!["n=0", "Insert"]);
    app.click_text("Insert").expect("insert");
    println!("after insert: {:?}", app.texts());
    assert_eq!(app.texts(), vec!["n=1", "Insert"]);
}
