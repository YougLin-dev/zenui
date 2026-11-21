#![allow(unused_braces)]

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use zenui::prelude::*;
use zenui_demo::{TodoApp, TodoState, open_todo_window};

#[test]
fn application_setup_shares_state_with_the_window_factory() {
    let mut app = Runtime::new();
    let value = app
        .setup(|| signal(String::from("app-owned")))
        .expect("application setup");
    let factory_value = value;
    app.open_window("main", move |_| {
        assert_eq!(factory_value.get(), "app-owned");
        view! {
            <view>
                <text aria:label="value">{factory_value}</text>
                <button
                    aria:label="update"
                    on:click={move || factory_value.set(String::from("updated"))}
                >"Update"</button>
            </view>
        }
    })
    .expect("window");
    let snapshot = app.presentation().expect("snapshot");
    assert_eq!(
        snapshot.find_label("value").expect("value text").text(),
        "app-owned"
    );

    app.click_label("update").expect("update the app signal");
    let snapshot = app.presentation().expect("snapshot");
    assert_eq!(
        snapshot.find_label("value").expect("value text").text(),
        "updated"
    );
}

#[test]
fn application_setup_must_run_before_a_window_mounts() {
    let mut app = Runtime::new();
    app.open_window("main", |_| view! { <view /> })
        .expect("window");
    let error = app
        .setup(|| ())
        .expect_err("setup after a mounted window must fail");
    assert!(matches!(error, MountError::Setup(_)));
}

#[test]
fn application_setup_is_refused_after_a_window_factory_ran() {
    let mut app = Runtime::new();
    let window = app
        .open_window("main", |_| view! { <view /> })
        .expect("window");
    app.close_window(window).expect("close window");
    let error = app
        .setup(|| ())
        .expect_err("setup after a closed window must fail");
    assert!(matches!(error, MountError::Setup(_)));

    let mut app = Runtime::new();
    let _ = app.open_window("main", |_| Err(MountError::Component("rejected")));
    let error = app
        .setup(|| ())
        .expect_err("setup after a failed factory must fail");
    assert!(matches!(error, MountError::Setup(_)));
}

#[test]
fn application_setup_runs_once() {
    let mut app = Runtime::new();
    app.setup(|| ()).expect("first setup");
    let error = app.setup(|| ()).expect_err("second setup must fail");
    assert!(matches!(error, MountError::Setup(_)));
}

#[test]
fn application_setup_rejects_a_later_headless_root_mount() {
    let mut app = Runtime::new();
    app.setup(|| ()).expect("application setup");
    let error = app
        .mount(|| view! { <view /> })
        .expect_err("a headless root would dispose the application owner");
    assert!(matches!(error, MountError::Setup(_)));
}

#[test]
fn failed_application_setup_releases_its_resources_and_can_retry() {
    let leaked = Rc::new(Cell::new(None));
    let created = leaked.clone();
    let mut app = Runtime::new();
    let error = app
        .setup(move || {
            let value = signal(String::from("leaked"));
            created.set(Some(value));
            value.set(String::from("not allowed during setup"));
            value
        })
        .expect_err("a write during application setup must fault");
    assert!(matches!(error, MountError::Context { .. }));

    let retried = leaked.clone();
    let value = app
        .setup(move || {
            let disposed = catch_unwind(AssertUnwindSafe(|| {
                retried
                    .get()
                    .expect("the failed setup created a signal")
                    .get()
            }));
            assert!(disposed.is_err(), "the failed setup released its signal");
            signal(String::from("retry"))
        })
        .expect("application setup may retry after a fault");
    let text = value;
    app.open_window("main", move |_| view! { <text>{text}</text> })
        .expect("open window");
    let snapshot = app.presentation().expect("snapshot");
    assert!(
        snapshot.elements().any(|element| element.text() == "retry"),
        "the retried setup produced the window state"
    );
}

#[test]
fn application_state_survives_window_close_and_reopen() {
    let mut app = Runtime::new();
    let value = app
        .setup(|| signal(String::from("kept")))
        .expect("application setup");
    let first = value;
    let window = app
        .open_window("main", move |_| view! { <text>{first}</text> })
        .expect("open window");
    app.close_window(window).expect("close window");
    assert!(
        matches!(app.setup(|| ()), Err(MountError::Setup(_))),
        "closing the window does not end the application"
    );

    let second = value;
    app.open_window("main", move |_| view! { <text>{second}</text> })
        .expect("reopen window");
    let snapshot = app.presentation().expect("snapshot");
    assert!(
        snapshot.elements().any(|element| element.text() == "kept"),
        "the app-owned signal survives the window owner"
    );
}

#[test]
fn todo_store_draft_and_filter_belong_to_the_application() {
    let mut app = Runtime::new();
    let state = app.setup(TodoState::new).expect("application setup");
    let first = state;
    let window = app
        .open_window("main", move |window| {
            view! { <TodoApp window={window} state={first} /> }
        })
        .expect("open window");
    app.input("What needs to be done?", "Alpha").expect("draft");
    app.submit("Add task").expect("add Alpha");
    app.input("What needs to be done?", "Beta").expect("draft");
    app.submit("Add task").expect("add Beta");
    app.change("Complete Beta", true).expect("complete Beta");
    app.click_text("Active").expect("filter active");
    app.input("What needs to be done?", "unfinished draft")
        .expect("draft");
    app.close_window(window).expect("close window");

    let second = state;
    app.open_window("main", move |window| {
        view! { <TodoApp window={window} state={second} /> }
    })
    .expect("reopen window");
    let snapshot = app.presentation().expect("snapshot");
    assert!(
        snapshot.elements().any(|element| element.text() == "Alpha"),
        "the Store survived the window owner"
    );
    assert!(
        !snapshot.elements().any(|element| element.text() == "Beta"),
        "the Active filter survived the window owner"
    );
    assert_eq!(
        snapshot
            .find_label("What needs to be done?")
            .expect("draft input")
            .text_value(),
        Some("unfinished draft"),
        "the draft survived the window owner"
    );
}

#[test]
fn application_setup_and_the_window_share_the_committed_appearance() {
    let mut app = Runtime::new();
    app.set_appearance(Appearance::Dark)
        .expect("set appearance before setup");
    open_todo_window(&mut app).expect("open window");
    assert_eq!(
        app.presentation().expect("snapshot").appearance(),
        Appearance::Dark
    );

    app.click_label("Toggle theme").expect("toggle the theme");
    assert_eq!(
        app.presentation().expect("snapshot").appearance(),
        Appearance::Light,
        "the window and its app-owned state see the same appearance"
    );
}
