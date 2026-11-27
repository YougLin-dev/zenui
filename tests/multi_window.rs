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
fn catalog_windows_get_runtime_global_activation_ids() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| {
            view! {
                <view>
                    <button aria:label="mine">"Mine"</button>
                </view>
            }
        })?;
        catalog.window(WindowSpec::new("details"), |_| view! { <view /> })?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show main");
    let details = app.show("details").expect("show details");
    assert_eq!(main.activation(), 1);
    assert_eq!(
        details.activation(),
        1,
        "both windows are the first open of their catalog key"
    );
    assert_ne!(
        main.activation_id(),
        details.activation_id(),
        "activation ids are runtime-global, not catalog-local"
    );
    assert_eq!(main.runtime_id(), details.runtime_id());
    assert_eq!(
        ActivationId::from_bits(main.activation_id().to_bits()),
        main.activation_id(),
        "the provider can round-trip the packed identity"
    );
    assert_eq!(main.activation_id().generation(), 1);
    let node = app
        .presentation_of(main)
        .expect("snapshot")
        .find_label("mine")
        .expect("the button")
        .id();
    assert_eq!(
        node.runtime_id(),
        main.runtime_id(),
        "a NodeId exposes the runtime that minted it"
    );

    app.close_window(main).expect("close main");
    let reopened = app.show("main").expect("reopen main");
    assert_eq!(
        reopened.activation(),
        2,
        "the key's open generation advances"
    );
    assert_ne!(
        reopened.activation_id().slot(),
        main.activation_id().slot(),
        "a reopened window gets a fresh runtime slot"
    );
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

#[test]
fn specs_carry_titles_and_sizes_and_live_windows_are_enumerable() {
    let defaults = WindowSpec::new("plain");
    assert_eq!(defaults.key(), "plain");
    assert_eq!(defaults.title(), "plain");
    assert_eq!(defaults.size(), (900, 700));
    assert!(!defaults.is_primary());

    let spec = WindowSpec::new("main")
        .primary()
        .titled("Main window")
        .sized(640, 480);
    assert_eq!(spec.title(), "Main window");
    assert_eq!(spec.size(), (640, 480));
    assert!(spec.is_primary());

    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(spec, |_| view! { <text>"main"</text> })?;
        catalog.window(
            WindowSpec::new("second").titled("Details").sized(320, 240),
            |_| view! { <text>"details"</text> },
        )?;
        Ok(())
    })
    .expect("catalog");

    assert!(app.windows().is_empty(), "declaration mounts nothing");
    let main = app.show("main").expect("show main");
    let second = app.show("second").expect("show second");
    let windows = app.windows();
    assert_eq!(windows.len(), 2, "both windows are live");
    assert_eq!(windows[0].window, main);
    assert_eq!(windows[0].spec.title(), "Main window");
    assert_eq!(windows[0].spec.size(), (640, 480));
    assert!(windows[0].visible);
    assert_eq!(windows[1].window, second);
    assert_eq!(windows[1].spec.title(), "Details");
    assert_eq!(windows[1].spec.size(), (320, 240));
    assert!(windows[1].visible);

    app.hide(second).expect("hide second");
    let windows = app.windows();
    assert_eq!(windows.len(), 2, "hide keeps the activation");
    assert!(windows[1].window == second);
    assert!(!windows[1].visible);

    app.close_window(main).expect("close main");
    let windows = app.windows();
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].window, second);
    assert!(!windows[0].visible, "main was closed, second stays hidden");
}

#[test]
fn window_revision_tracks_open_hide_show_and_close() {
    let mut app = Runtime::new();
    assert_eq!(app.window_revision(), 0);

    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| {
            view! { <view /> }
        })?;
        Ok(())
    })
    .expect("catalog");
    assert_eq!(
        app.window_revision(),
        0,
        "declaration changes no native state"
    );

    let main = app.show("main").expect("show main");
    let after_open = app.window_revision();
    assert!(after_open > 0, "opening a window bumps the revision");

    let focused = app.show("main").expect("show focuses the same window");
    assert_eq!(focused, main);
    assert_eq!(
        app.window_revision(),
        after_open,
        "focusing an already visible window changes nothing"
    );

    app.hide(main).expect("hide");
    let after_hide = app.window_revision();
    assert!(after_hide > after_open, "hiding bumps the revision");
    app.hide(main).expect("hide is idempotent");
    assert_eq!(
        app.window_revision(),
        after_hide,
        "hiding twice changes nothing"
    );

    app.show("main").expect("restore");
    let after_show = app.window_revision();
    assert!(after_show > after_hide, "showing bumps the revision");
    app.close_window(main).expect("close");
    assert!(
        app.window_revision() > after_show,
        "closing bumps the revision"
    );
    assert!(app.windows().is_empty(), "a closed window is not live");
}

#[test]
fn focus_traversal_and_blur_are_scoped_to_the_window() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| {
            view! {
                <view>
                    <text-input aria:label="main-a" />
                    <text-input aria:label="main-b" />
                </view>
            }
        })?;
        catalog.window(WindowSpec::new("second"), |_| {
            view! {
                <view>
                    <text-input aria:label="second-a" />
                    <text-input aria:label="second-b" />
                </view>
            }
        })?;
        catalog.window(
            WindowSpec::new("empty"),
            |_| view! { <text>"no controls"</text> },
        )?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show main");
    let second = app.show("second").expect("show second");
    let empty = app.show("empty").expect("show empty");
    let main_snapshot = app.presentation_of(main).expect("main snapshot");
    let main_a = main_snapshot.find_label("main-a").expect("main a").id();
    let main_b = main_snapshot.find_label("main-b").expect("main b").id();
    let second_snapshot = app.presentation_of(second).expect("second snapshot");
    let second_a = second_snapshot
        .find_label("second-a")
        .expect("second a")
        .id();
    let second_b = second_snapshot
        .find_label("second-b")
        .expect("second b")
        .id();

    app.focus_node(main_a).expect("focus main a");
    app.focus_node(second_a).expect("focus second a");
    assert_eq!(
        app.presentation_of(main).expect("snapshot").focus(),
        Some(main_a)
    );
    assert_eq!(
        app.presentation_of(second).expect("snapshot").focus(),
        Some(second_a)
    );

    app.focus_next_in(main).expect("tab inside main");
    assert_eq!(
        app.presentation_of(main).expect("snapshot").focus(),
        Some(main_b),
        "main advances its own focus order"
    );
    assert_eq!(
        app.presentation_of(second).expect("snapshot").focus(),
        Some(second_a),
        "the other window keeps its focus"
    );

    app.blur_in(main).expect("blur main");
    assert_eq!(app.presentation_of(main).expect("snapshot").focus(), None);
    assert_eq!(
        app.presentation_of(second).expect("snapshot").focus(),
        Some(second_a),
        "blurring main does not touch second"
    );

    app.focus_previous_in(second)
        .expect("shift-tab inside second");
    assert_eq!(
        app.presentation_of(second).expect("snapshot").focus(),
        Some(second_b),
        "second wraps within its own focus order"
    );

    app.focus_next_in(empty)
        .expect("a window without focusable controls ignores Tab");
    assert_eq!(
        app.presentation_of(empty).expect("empty snapshot").focus(),
        None
    );

    app.close_window(main).expect("close main");
    assert_eq!(app.focus_next_in(main), Err(WindowError::Stale));

    let mut other = Runtime::new();
    let foreign = other
        .open_window("foreign", |_| view! { <view /> })
        .expect("open a foreign window");
    assert_eq!(app.focus_next_in(foreign), Err(WindowError::WrongRuntime));
}

#[test]
fn window_requests_open_focus_hide_and_close_from_handlers() {
    let mut app = Runtime::new();
    let runs = Rc::new(Cell::new(0));
    let details_ctx = Rc::new(Cell::new(None));
    let counter = runs.clone();
    let seen = details_ctx.clone();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), |window| {
            view! {
                <view>
                    <button
                        aria:label="open-details"
                        on:click={move || {
                            window.request_show("details").expect("queue show");
                        }}
                    >"Open"</button>
                    <button
                        aria:label="close-main"
                        on:click={move || {
                            window.request_close().expect("queue close");
                        }}
                    >"Close"</button>
                </view>
            }
        })?;
        catalog.window(WindowSpec::new("details"), move |window| {
            counter.set(counter.get() + 1);
            seen.set(Some(window));
            view! {
                <view>
                    <text aria:label="details-body">"details"</text>
                    <button
                        aria:label="hide-details"
                        on:click={move || {
                            window.request_hide().expect("queue hide");
                        }}
                    >"Hide"</button>
                    <button
                        aria:label="close-details"
                        on:click={move || {
                            window.request_close().expect("queue close");
                        }}
                    >"Close"</button>
                </view>
            }
        })?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show main");
    app.click_label("open-details").expect("queue details");
    assert_eq!(
        app.windows().len(),
        1,
        "a request waits for the settle step"
    );
    app.apply_window_requests().expect("apply the show request");
    let details = details_ctx.get().expect("the details factory ran");
    assert_eq!(app.windows().len(), 2);
    assert!(app.window_alive(details) && app.window_visible(details));
    assert_eq!(app.focused_window(), Some(details));

    app.focus("open-details").expect("focus the main window");
    assert_eq!(app.focused_window(), Some(main));
    app.click_label("open-details")
        .expect("focus the open details");
    app.apply_window_requests()
        .expect("apply the focus request");
    assert_eq!(runs.get(), 1, "a visible target is focused, not remounted");
    assert_eq!(details_ctx.get(), Some(details));
    assert_eq!(
        app.focused_window(),
        Some(details),
        "the request refocuses the visible target"
    );

    app.click_label("hide-details").expect("queue hide");
    app.apply_window_requests().expect("apply the hide request");
    assert!(app.window_alive(details));
    assert!(!app.window_visible(details));
    assert_eq!(app.focused_window(), Some(main));

    app.click_label("open-details")
        .expect("show the hidden details");
    app.apply_window_requests().expect("apply the show request");
    assert!(app.window_visible(details));
    assert_eq!(app.focused_window(), Some(details));
    assert_eq!(
        runs.get(),
        1,
        "showing a hidden window keeps its activation"
    );

    app.click_label("close-details")
        .expect("queue the details close");
    app.apply_window_requests()
        .expect("apply the close request");
    assert!(!app.window_alive(details));
    assert_eq!(app.windows().len(), 1);
    assert_eq!(app.focused_window(), Some(main));

    app.click_label("open-details").expect("reopen details");
    app.apply_window_requests().expect("apply the reopen");
    let reopened = details_ctx.get().expect("the factory ran again");
    assert_ne!(reopened, details);
    assert!(reopened.activation() > details.activation());
    assert_eq!(runs.get(), 2);

    app.click_label("close-details")
        .expect("queue closing the details");
    app.apply_window_requests().expect("apply the close");
    assert_eq!(app.windows().len(), 1);
    app.click_label("close-main")
        .expect("queue closing the main window");
    app.apply_window_requests().expect("apply the close");
    assert!(app.windows().is_empty());
}

#[test]
fn window_requests_check_phase_catalog_and_source_liveness() {
    let mut app = Runtime::new();
    let setup_probe = Rc::new(RefCell::new(None));
    let setup_outcome = setup_probe.clone();
    let handler_probe = Rc::new(RefCell::new(None));
    let handler_outcome = handler_probe.clone();
    let handler_click = handler_probe.clone();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            *setup_outcome.borrow_mut() = Some(window.request_hide());
            let handler_click = handler_click.clone();
            view! {
                <view>
                    <button
                        aria:label="bad"
                        on:click={move || {
                            *handler_click.borrow_mut() = Some(window.request_show("missing"));
                        }}
                    >"Bad"</button>
                    <button
                        aria:label="queue"
                        on:click={move || {
                            window.request_show("details").expect("queue");
                        }}
                    >"Queue"</button>
                </view>
            }
        })?;
        catalog.window(
            WindowSpec::new("details"),
            |_| view! { <text>"details"</text> },
        )?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show main");
    assert!(
        matches!(
            setup_probe.borrow().as_ref(),
            Some(Err(WindowError::State(_)))
        ),
        "a factory runs in Setup and cannot queue window requests"
    );

    app.click_label("bad").expect("click the bad action");
    assert_eq!(
        handler_outcome.borrow().as_ref(),
        Some(&Err(WindowError::UnknownWindow("missing"))),
        "the catalog key is checked before the request is queued"
    );
    app.apply_window_requests()
        .expect("a rejected request queues nothing");

    app.click_label("queue").expect("queue showing details");
    app.close_window(main)
        .expect("close the source before settling");
    app.apply_window_requests()
        .expect("a request from a closed source is dropped");
    assert!(
        app.windows().is_empty(),
        "a stale source cannot open another window"
    );
}
