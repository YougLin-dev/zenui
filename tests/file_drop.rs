#![allow(unused_braces)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use zenui::prelude::*;

fn path(value: &str) -> PathBuf {
    PathBuf::from(value)
}

fn drop_host() -> (Runtime, WindowCtx) {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |window| {
            window
                .on_drop(|drop| {
                    let _ = drop.into_paths();
                })
                .expect("register the drop handler");
            view! { <view>"main"</view> }
        })?;
        catalog.window(WindowSpec::new("other"), |_window| {
            view! { <view>"other"</view> }
        })
    })
    .expect("declare windows");
    let main = app.show("main").expect("show main");
    (app, main)
}

#[test]
fn drop_handlers_register_once_during_the_window_factory_only() {
    let duplicate = Rc::new(RefCell::new(None));
    let late = Rc::new(RefCell::new(None));
    let duplicate_factory = duplicate.clone();
    let late_factory = late.clone();

    let mut app = Runtime::new();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            window.on_drop(|_| {}).expect("register the first handler");
            *duplicate_factory.borrow_mut() = Some(window.on_drop(|_| {}));
            let late = late_factory.clone();
            view! {
                <view>
                    <button
                        aria:label="late"
                        on:click={move || {
                            *late.borrow_mut() = Some(window.on_drop(|_| {}));
                        }}
                    >"Late"</button>
                </view>
            }
        })
    })
    .expect("declare the window");

    let _main = app.show("main").expect("show main");
    assert!(
        matches!(
            duplicate.borrow().as_ref(),
            Some(Err(WindowError::State(_)))
        ),
        "a drop handler registers once per activation: {:?}",
        duplicate.borrow()
    );
    app.click_label("late")
        .expect("click the late registration");
    assert!(
        matches!(late.borrow().as_ref(), Some(Err(WindowError::State(_)))),
        "on_drop is unavailable outside the factory: {:?}",
        late.borrow()
    );
}

#[test]
fn drop_files_delivers_one_ordered_event_and_runs_in_the_event_phase() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let events_factory = events.clone();
    let done = Rc::new(RefCell::new(None));
    let done_factory = done.clone();

    let mut app = Runtime::new();
    let (status, finished) = app
        .setup(|| (signal(String::new()), signal(String::new())))
        .expect("create the app signals");
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            let events = events_factory.clone();
            let done = done_factory.clone();
            window
                .on_drop(move |drop| {
                    let received = drop.into_paths();
                    events.borrow_mut().push(received.clone());
                    status.set(
                        received
                            .iter()
                            .map(|path| path.display().to_string())
                            .collect::<Vec<_>>()
                            .join("|"),
                    );
                    let task = window
                        .app_task_scope()
                        .spawn(|_| String::from("worker done"))
                        .expect("start the app task");
                    let done = done.clone();
                    task.on_done(move |value| {
                        finished.set(value.clone());
                        *done.borrow_mut() = Some(value);
                        Ok(())
                    })
                    .expect("register the continuation");
                    window
                        .request_show("details")
                        .expect("queue the window show");
                })
                .expect("register the drop handler");
            view! { <view><text>{status}</text></view> }
        })?;
        catalog.window(WindowSpec::new("details"), |_window| {
            view! { <view>"details"</view> }
        })
    })
    .expect("declare windows");

    let main = app.show("main").expect("show main");
    let paths = vec![
        path("C:/证据/你好 🙂.txt"),
        path("duplicate.txt"),
        path("duplicate.txt"),
    ];
    app.drop_files(main, paths.clone())
        .expect("deliver the drop");
    let delivered = events.borrow();
    assert_eq!(delivered.len(), 1, "each call is one event");
    assert_eq!(delivered[0], paths, "source order is preserved");
    drop(delivered);
    assert!(
        app.presentation_of(main)
            .expect("main snapshot")
            .elements()
            .any(|element| element.text() == "C:/证据/你好 🙂.txt|duplicate.txt|duplicate.txt"),
        "the handler wrote the Signal and the refresh published it"
    );

    app.drop_files(main, vec![path("second.txt")])
        .expect("deliver a second drop");
    assert_eq!(events.borrow().len(), 2, "each call is one event");
    assert_eq!(events.borrow()[1], [path("second.txt")]);

    app.apply_window_requests()
        .expect("settle the show request");
    assert!(
        app.windows()
            .iter()
            .any(|info| info.spec.key() == "details" && app.window_alive(info.window)),
        "request_show works in the drop handler"
    );
    app.settle_tasks().expect("settle the app-owned task");
    assert_eq!(done.borrow().as_deref(), Some("worker done"));
}

#[test]
fn drop_files_rejects_foreign_closed_hidden_suspended_empty_and_unhandled_targets() {
    let (mut app, main) = drop_host();

    let mut foreign = Runtime::new();
    foreign
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("foreign").primary(), |_window| {
                view! { <view>"foreign"</view> }
            })
        })
        .expect("declare the foreign window");
    let foreign = foreign.show("foreign").expect("show the foreign window");
    assert_eq!(
        app.drop_files(foreign, vec![path("foreign.txt")]),
        Err(DropError::WrongRuntime)
    );

    assert_eq!(
        app.drop_files(main, Vec::new()),
        Err(DropError::Empty),
        "an empty drop never runs the handler"
    );
    let other = app.show("other").expect("show the other window");
    assert_eq!(
        app.drop_files(other, vec![path("other.txt")]),
        Err(DropError::NoHandler)
    );

    app.hide(main).expect("hide main");
    assert_eq!(
        app.drop_files(main, vec![path("hidden.txt")]),
        Err(DropError::Hidden)
    );
    app.show("main").expect("show main again");

    app.close_window(main).expect("close main");
    assert_eq!(
        app.drop_files(main, vec![path("closed.txt")]),
        Err(DropError::Stale),
        "a closed activation is stale"
    );
}

#[test]
fn drop_files_rejects_suspended_windows_before_the_handler() {
    let runs = Rc::new(RefCell::new(0));
    let runs_factory = runs.clone();
    let mut app = Runtime::new();
    app.declare_windows(move |catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .close_policy(ClosePolicy::Confirm),
            move |window| {
                window
                    .on_close(move |request| {
                        window
                            .request_show("confirm")
                            .expect("show the confirmation");
                        let _ = request;
                        CloseOutcome::Pending
                    })
                    .expect("register the close handler");
                let runs = runs_factory.clone();
                window
                    .on_drop(move |drop| {
                        let _ = drop.into_paths();
                        *runs.borrow_mut() += 1;
                    })
                    .expect("register the drop handler");
                view! { <view>"main"</view> }
            },
        )?;
        catalog.window(
            WindowSpec::new("confirm")
                .confirmation_for("main")
                .titled("Discard?"),
            |_window| view! { <view>"confirm"</view> },
        )
    })
    .expect("declare windows");

    let main = app.show("main").expect("show main");
    app.request_close(main).expect("request a confirmed close");
    app.apply_window_requests()
        .expect("show the confirmation window");
    assert_eq!(
        app.drop_files(main, vec![path("suspended.txt")]),
        Err(DropError::Suspended)
    );
    let confirm = app
        .windows()
        .into_iter()
        .find(|info| info.spec.key() == "confirm")
        .expect("the confirmation window is live")
        .window;
    assert_eq!(
        app.drop_files(confirm, vec![path("confirm.txt")]),
        Err(DropError::Suspended),
        "the confirmation window is also suspended"
    );
    assert_eq!(*runs.borrow(), 0, "rejected drops never run user code");
}

#[test]
fn a_panicking_drop_handler_is_contained_and_the_runtime_stays_usable() {
    let panicked = Rc::new(RefCell::new(false));
    let events = Rc::new(RefCell::new(Vec::new()));
    let panicked_factory = panicked.clone();
    let events_factory = events.clone();
    let mut app = Runtime::new();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            let panicked = panicked_factory.clone();
            let events = events_factory.clone();
            window
                .on_drop(move |drop| {
                    if !*panicked.borrow() {
                        *panicked.borrow_mut() = true;
                        panic!("the drop handler failed");
                    }
                    events.borrow_mut().push(drop.into_paths());
                })
                .expect("register the drop handler");
            view! { <view>"main"</view> }
        })
    })
    .expect("declare windows");

    let main = app.show("main").expect("show main");
    assert_eq!(
        app.drop_files(main, vec![path("panic.txt")]),
        Err(DropError::HandlerPanicked)
    );
    app.drop_files(main, vec![path("after.txt")])
        .expect("a later drop still runs");
    assert_eq!(*events.borrow(), [vec![path("after.txt")]]);
}

#[test]
fn closing_removes_the_drop_handler_and_reopening_requires_a_new_one() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let events_factory = events.clone();
    let mut app = Runtime::new();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            let events = events_factory.clone();
            window
                .on_drop(move |drop| {
                    events.borrow_mut().push(drop.into_paths());
                })
                .expect("register the drop handler");
            view! { <view>"main"</view> }
        })
    })
    .expect("declare windows");

    let first = app.show("main").expect("show main");
    app.close_window(first).expect("close main");
    assert_eq!(
        app.drop_files(first, vec![path("old.txt")]),
        Err(DropError::Stale)
    );

    let reopened = app.show("main").expect("reopen main");
    assert_ne!(reopened, first);
    app.drop_files(reopened, vec![path("new.txt")])
        .expect("the fresh factory registered a handler");
    assert_eq!(*events.borrow(), [vec![path("new.txt")]]);
    assert_eq!(
        app.drop_files(first, vec![path("old-again.txt")]),
        Err(DropError::Stale),
        "the old activation never receives a new drop"
    );
    assert_eq!(events.borrow().len(), 1);
}
