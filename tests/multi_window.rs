#![allow(unused_braces)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use zenui::prelude::*;
use zenui_demo::{TodoApp, TodoState};

#[test]
fn catalog_declaration_validates_and_mounts_nothing() {
    let mut app = Runtime::new();

    let error = app
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main"), |_| view! { <view /> })?;
            Ok(())
        })
        .expect_err("a catalog needs a primary window");
    assert_eq!(error, CatalogError::MissingPrimary);

    let error = app
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
            catalog.window(WindowSpec::new("main"), |_| view! { <view /> })?;
            Ok(())
        })
        .expect_err("a duplicate key is rejected at declaration");
    assert_eq!(error, CatalogError::DuplicateKey("main"));

    let error = app
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("a").primary(), |_| view! { <view /> })?;
            catalog.window(WindowSpec::new("b").primary(), |_| view! { <view /> })?;
            Ok(())
        })
        .expect_err("the catalog has one primary");
    assert_eq!(error, CatalogError::MultiplePrimaries);

    assert!(
        app.presentation().is_err(),
        "failed declarations mounted nothing"
    );

    let runs = Rc::new(Cell::new(0));
    let counter = runs.clone();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |_| {
            counter.set(counter.get() + 1);
            view! { <text>"main"</text> }
        })?;
        Ok(())
    })
    .expect("valid catalog");
    assert_eq!(app.primary_key(), Some("main"));
    assert_eq!(runs.get(), 0, "declaration does not run factories");
    assert!(
        app.presentation().is_err(),
        "declaration does not mount a tree"
    );

    let error = app
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("other").primary(), |_| view! { <view /> })?;
            Ok(())
        })
        .expect_err("a catalog is declared once");
    assert_eq!(error, CatalogError::AlreadyDeclared);

    app.show("main").expect("show the primary window");
    assert_eq!(runs.get(), 1);
    let error = app.show("missing").expect_err("unknown window");
    assert_eq!(error, WindowError::UnknownWindow("missing"));

    let mut adhoc = Runtime::new();
    adhoc
        .open_window("adhoc", |_| view! { <view /> })
        .expect("open an ad-hoc window");
    let error = adhoc
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
            Ok(())
        })
        .expect_err("the catalog must precede window factories");
    assert_eq!(error, CatalogError::DeclaredAfterWindow);
}

#[test]
fn a_declared_catalog_rejects_the_ad_hoc_open_path() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        Ok(())
    })
    .expect("catalog");
    let error = app
        .open_window("adhoc", |_| view! { <view /> })
        .expect_err("a catalog owns the open path");
    assert!(matches!(error, WindowError::State(_)));
}

#[test]
fn show_retries_after_a_factory_failure() {
    let mut app = Runtime::new();
    let attempts = Rc::new(Cell::new(0));
    let counter = attempts.clone();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |_| {
            counter.set(counter.get() + 1);
            if counter.get() == 1 {
                Err(MountError::Component("first open failed"))
            } else {
                view! { <text>"recovered"</text> }
            }
        })?;
        Ok(())
    })
    .expect("catalog");

    let error = app.show("main").expect_err("the first factory fails");
    assert_eq!(
        error,
        WindowError::Mount(MountError::Component("first open failed"))
    );
    assert!(app.presentation().is_err(), "a failed show mounted nothing");

    let window = app.show("main").expect("retry the factory");
    assert_eq!(window.activation(), 2);
    assert_eq!(attempts.get(), 2);
    let snapshot = app.presentation_of(window).expect("snapshot");
    assert!(
        snapshot
            .elements()
            .any(|element| element.text() == "recovered")
    );
}

#[test]
fn show_rejects_a_mounted_headless_root() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        Ok(())
    })
    .expect("catalog");
    app.mount(|| view! { <text>"root"</text> })
        .expect("mount the headless root");
    let error = app.show("main").expect_err("root and windows do not mix");
    assert!(matches!(error, WindowError::State(_)));
}

#[test]
fn presentation_of_rejects_a_window_from_another_runtime() {
    fn shown() -> (Runtime, WindowCtx) {
        let mut app = Runtime::new();
        app.declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
            Ok(())
        })
        .expect("catalog");
        let window = app.show("main").expect("show");
        (app, window)
    }

    let (first, _) = shown();
    let (_second, foreign) = shown();
    assert_eq!(
        first.presentation_of(foreign),
        Err(WindowError::WrongRuntime)
    );
}

#[test]
fn show_focuses_a_visible_window_and_reopen_uses_a_new_activation() {
    let mut app = Runtime::new();
    let runs = Rc::new(RefCell::new(Vec::<&'static str>::new()));
    let main_runs = runs.clone();
    let details_runs = runs.clone();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |_| {
            main_runs.borrow_mut().push("main");
            view! { <text>"main"</text> }
        })?;
        catalog.window(WindowSpec::new("details"), move |_| {
            details_runs.borrow_mut().push("details");
            view! { <text>"details"</text> }
        })?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show main");
    assert_eq!(main.activation(), 1);
    assert_eq!(
        app.focused_window().map(|window| window.key()),
        Some("main")
    );
    let again = app.show("main").expect("focus the visible window");
    assert_eq!(again.activation(), main.activation());
    assert_eq!(runs.borrow().as_slice(), ["main"]);

    let details = app.show("details").expect("show details");
    assert_eq!(details.activation(), 1);
    assert_eq!(
        app.focused_window().map(|window| window.key()),
        Some("details")
    );
    let main_again = app.show("main").expect("focus main again");
    assert_eq!(main_again.activation(), 1);
    assert_eq!(runs.borrow().as_slice(), ["main", "details"]);
    assert_eq!(
        app.focused_window().map(|window| window.key()),
        Some("main")
    );

    app.close_window(details).expect("close details");
    assert!(!app.window_alive(details));
    assert_eq!(
        app.presentation_of(details),
        Err(WindowError::Stale),
        "a closed window has no snapshot"
    );
    let reopened = app.show("details").expect("reopen details");
    assert_eq!(reopened.activation(), 2);
    assert_eq!(runs.borrow().as_slice(), ["main", "details", "details"]);
    assert_eq!(
        app.focused_window().map(|window| window.key()),
        Some("details")
    );
}

#[test]
fn two_windows_share_state_but_keep_their_own_focus_and_snapshot() {
    let mut app = Runtime::new();
    let (shared, draft_a, draft_b) = app
        .setup(|| {
            (
                signal(String::from("initial")),
                signal(String::from("draft-a")),
                signal(String::from("draft-b")),
            )
        })
        .expect("application setup");
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("a").primary(), move |_| {
            view! {
                <view>
                    <text aria:label="a-shared">{shared}</text>
                    <text-input aria:label="a-input" value={draft_a} />
                    <button
                        aria:label="a-update"
                        on:click={move || shared.set(String::from("from-a"))}
                    >"A"</button>
                </view>
            }
        })?;
        catalog.window(WindowSpec::new("b"), move |_| {
            view! {
                <view>
                    <text aria:label="b-shared">{shared}</text>
                    <text-input aria:label="b-input" value={draft_b} />
                </view>
            }
        })?;
        Ok(())
    })
    .expect("catalog");

    let a = app.show("a").expect("show a");
    let b = app.show("b").expect("show b");
    let a_snapshot = app.presentation_of(a).expect("a snapshot");
    let b_snapshot = app.presentation_of(b).expect("b snapshot");
    assert_eq!(
        a_snapshot.find_label("a-shared").expect("a text").text(),
        "initial"
    );
    assert_eq!(
        b_snapshot.find_label("b-shared").expect("b text").text(),
        "initial"
    );

    let a_input = a_snapshot.find_label("a-input").expect("a input").id();
    let b_input = b_snapshot.find_label("b-input").expect("b input").id();
    app.focus_node(a_input).expect("focus a");
    app.focus_node(b_input).expect("focus b");
    assert_eq!(
        app.presentation_of(a).expect("a snapshot").focus(),
        Some(a_input),
        "focus in b does not move focus in a"
    );
    assert_eq!(
        app.presentation_of(b).expect("b snapshot").focus(),
        Some(b_input)
    );

    app.set_caret("a-input", 2);
    assert_eq!(app.caret("a-input"), Some(2));
    let a_snapshot = app.presentation_of(a).expect("a snapshot");
    assert_eq!(
        a_snapshot
            .find_label("a-input")
            .expect("a input")
            .edit()
            .expect("a edit state")
            .caret(),
        2
    );
    let b_snapshot = app.presentation_of(b).expect("b snapshot");
    assert_eq!(
        b_snapshot
            .find_label("b-input")
            .expect("b input")
            .edit()
            .expect("b edit state")
            .caret(),
        0,
        "per-window caret state is independent"
    );

    app.click_label("a-update").expect("update from a");
    assert_eq!(
        app.presentation_of(a)
            .expect("a snapshot")
            .find_label("a-shared")
            .expect("a text")
            .text(),
        "from-a"
    );
    assert_eq!(
        app.presentation_of(b)
            .expect("b snapshot")
            .find_label("b-shared")
            .expect("b text")
            .text(),
        "from-a",
        "a shared model change reaches both windows"
    );
}

#[test]
fn two_todo_windows_share_the_application_state() {
    let mut app = Runtime::new();
    let state = app.setup(TodoState::new).expect("application setup");
    app.declare_windows(move |catalog| {
        let main_state = state;
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            view! { <TodoApp window={window} state={main_state} /> }
        })?;
        let details_state = state;
        catalog.window(WindowSpec::new("details"), move |window| {
            view! { <TodoApp window={window} state={details_state} /> }
        })?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show main");
    let details = app.show("details").expect("show details");
    app.input("What needs to be done?", "Alpha")
        .expect("type into the focused details window");
    app.submit("Add task").expect("submit from details");

    for window in [main, details] {
        let snapshot = app.presentation_of(window).expect("snapshot");
        assert!(
            snapshot.elements().any(|element| element.text() == "Alpha"),
            "both Todo windows show the shared row"
        );
        assert!(
            snapshot
                .elements()
                .any(|element| element.text() == "1 remaining"),
            "both Todo windows show the shared derived count"
        );
    }

    app.change("Complete Alpha", true)
        .expect("complete the task");
    app.click_text("Active").expect("switch the shared filter");
    for window in [main, details] {
        let snapshot = app.presentation_of(window).expect("snapshot");
        assert!(
            !snapshot.elements().any(|element| element.text() == "Alpha"),
            "the shared Active filter hides the completed row in both windows"
        );
        assert!(
            snapshot
                .elements()
                .any(|element| element.text() == "0 remaining")
        );
    }
}

#[test]
fn dispatch_and_tasks_belong_to_their_window() {
    let mut app = Runtime::new();
    let (a_result, b_result, app_result) = app
        .setup(|| {
            (
                signal(String::from("idle")),
                signal(String::from("idle")),
                signal(String::from("idle")),
            )
        })
        .expect("application setup");
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("a").primary(), move |window| {
            let window_scope = window.task_scope();
            let app_scope = window.app_task_scope();
            view! {
                <view>
                    <text aria:label="a-result">{a_result}</text>
                    <text aria:label="app-result">{app_result}</text>
                    <button aria:label="a-task" on:click={move || {
                        if let Ok(task) = window_scope.spawn(|_| String::from("a-done")) {
                            let _ = task.on_done(move |value| {
                                a_result.set(value);
                                Ok(())
                            });
                        }
                    }}>"A task"</button>
                    <button aria:label="a-app-task" on:click={move || {
                        if let Ok(task) = app_scope.spawn(|_| String::from("app-done")) {
                            let _ = task.on_done(move |value| {
                                app_result.set(value);
                                Ok(())
                            });
                        }
                    }}>"App task"</button>
                </view>
            }
        })?;
        catalog.window(WindowSpec::new("b"), move |window| {
            let window_scope = window.task_scope();
            view! {
                <view>
                    <text aria:label="b-result">{b_result}</text>
                    <button aria:label="b-slow" on:click={move || {
                        if let Ok(task) = window_scope.spawn(|token| {
                            while !token.is_cancelled() {
                                std::thread::sleep(std::time::Duration::from_millis(1));
                            }
                            String::from("late-b")
                        }) {
                            let _ = task.on_done(move |value| {
                                b_result.set(value);
                                Ok(())
                            });
                        }
                    }}>"B slow"</button>
                </view>
            }
        })?;
        Ok(())
    })
    .expect("catalog");

    let a = app.show("a").expect("show a");
    let b = app.show("b").expect("show b");
    let a_snapshot = app.presentation_of(a).expect("a snapshot");
    let a_task = a_snapshot.find_label("a-task").expect("a task").id();
    let app_task = a_snapshot.find_label("a-app-task").expect("app task").id();
    let b_snapshot = app.presentation_of(b).expect("b snapshot");
    let b_slow = b_snapshot.find_label("b-slow").expect("b slow").id();

    app.click_node(a_task).expect("start a window task");
    app.settle_tasks().expect("deliver a window task");
    assert_eq!(
        app.presentation_of(a)
            .expect("a snapshot")
            .find_label("a-result")
            .expect("a result")
            .text(),
        "a-done"
    );

    app.click_node(app_task).expect("start an app task");
    app.click_node(b_slow).expect("start a slow b task");
    app.close_window(b)
        .expect("close b before its task finishes");
    assert_eq!(app.node_status(b_slow), NodeStatus::Stale);
    assert_eq!(
        app.click_node(b_slow),
        Err(DispatchError::StaleNode(b_slow))
    );
    app.close_window(a)
        .expect("close a while its app task runs");

    let report = app.settle_tasks().expect("settle remaining tasks");
    assert_eq!(report.delivered, 1, "the cancelled b result is discarded");
    let reopened = app.show("a").expect("reopen a");
    let snapshot = app.presentation_of(reopened).expect("a snapshot");
    assert_eq!(
        snapshot.find_label("a-result").expect("a result").text(),
        "a-done",
        "application state survives the window"
    );
    assert_eq!(
        snapshot
            .find_label("app-result")
            .expect("app result")
            .text(),
        "app-done",
        "the app-owned task survives the window that started it"
    );
}
