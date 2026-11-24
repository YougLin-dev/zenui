#![allow(unused_braces)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use zenui::prelude::*;

fn text_of(app: &Runtime, window: WindowCtx, label: &str) -> String {
    app.presentation_of(window)
        .expect("snapshot")
        .find_label(label)
        .unwrap_or_else(|| panic!("no element labelled {label}"))
        .text()
}

#[test]
fn a_tray_app_command_runs_with_zero_windows_and_commits_state() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let handler_seen = seen.clone();
    let mut app = Runtime::new();
    let (status, store, rows) = app
        .setup(|| {
            let store = Store::new();
            let status = signal(String::from("idle"));
            let rows = memo(move || store.rows(Filter::All).len().to_string());
            (status, store, rows)
        })
        .expect("application setup");
    app.on_app_command("app.seed", move |event| {
        handler_seen.borrow_mut().push(event);
        status.set(String::from("tray"));
        let mut transaction = Transaction::new();
        transaction = transaction.write(store.insert("from-tray"));
        transaction.finish().expect("commit the tray row");
    })
    .expect("register the app command");

    assert!(app.windows().is_empty(), "the app starts with zero windows");
    app.dispatch_app_command("app.seed", CommandSource::Tray, None)
        .expect("run the tray command without a window");
    assert!(app.windows().is_empty(), "dispatch opens no window");

    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |_| {
            view! {
                <view>
                    <text aria:label="status">{status}</text>
                    <text aria:label="rows">{rows}</text>
                </view>
            }
        })?;
        Ok(())
    })
    .expect("declare the window");
    let main = app.show("main").expect("show the first window");
    assert_eq!(text_of(&app, main, "status"), "tray");
    assert_eq!(text_of(&app, main, "rows"), "1");

    let events = seen.borrow();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source(), CommandSource::Tray);
    assert_eq!(events[0].window(), None);
    assert_eq!(events[0].target(), None);
}

#[test]
fn an_app_command_exposes_a_live_target_and_settles_requests() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let reveal_seen = seen.clone();
    let mut app = Runtime::new();
    app.on_app_command("app.reveal", move |event| {
        reveal_seen.borrow_mut().push(event);
        event
            .window()
            .expect("a live target")
            .request_show("details")
            .expect("queue the show");
    })
    .expect("register reveal");
    app.on_app_command("app.close", |event| {
        event
            .window()
            .expect("a live target")
            .request_close()
            .expect("queue the close");
    })
    .expect("register close");
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        catalog.window(WindowSpec::new("details"), |_| view! { <view /> })?;
        Ok(())
    })
    .expect("declare the windows");
    let main = app.show("main").expect("show main");

    app.dispatch_app_command("app.reveal", CommandSource::Tray, Some(main))
        .expect("run the reveal");
    assert_eq!(
        app.windows().len(),
        1,
        "the show request waits for settlement"
    );
    assert_eq!(app.apply_window_requests().expect("settle"), 1);
    let details = app
        .windows()
        .into_iter()
        .find(|info| info.spec.key() == "details")
        .expect("details opened")
        .window;
    assert!(app.window_alive(details));

    let event = seen.borrow()[0];
    assert_eq!(event.window(), Some(main));
    assert_eq!(event.source(), CommandSource::Tray);

    app.dispatch_app_command("app.close", CommandSource::Tray, Some(main))
        .expect("run the close");
    assert!(
        app.window_alive(main),
        "the close request waits for settlement"
    );
    assert_eq!(app.apply_window_requests().expect("settle"), 1);
    assert!(!app.window_alive(main));
}

#[test]
fn window_dispatch_falls_back_to_the_app_and_tray_skips_window_handlers() {
    let window_runs = Rc::new(Cell::new(0));
    let window_only_runs = Rc::new(Cell::new(0));
    let app_runs = Rc::new(Cell::new(0));
    let app_windows = Rc::new(RefCell::new(Vec::new()));
    let app_targets = Rc::new(RefCell::new(Vec::new()));
    let handler_window_runs = window_runs.clone();
    let window_only_handler_runs = window_only_runs.clone();
    let local_app_runs = app_runs.clone();
    let shared_app_runs = app_runs.clone();
    let shared_app_windows = app_windows.clone();
    let shared_app_targets = app_targets.clone();
    let mut app = Runtime::new();
    app.on_app_command("local.act", move |_| {
        local_app_runs.set(local_app_runs.get() + 1)
    })
    .expect("register the app fallback for a local id");
    app.on_app_command("shared.act", move |event| {
        shared_app_runs.set(shared_app_runs.get() + 1);
        shared_app_windows.borrow_mut().push(event.window());
        shared_app_targets.borrow_mut().push(event.target());
    })
    .expect("register the shared app command");
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            let handler_window_runs = handler_window_runs.clone();
            window
                .on_command("local.act", move |_| {
                    handler_window_runs.set(handler_window_runs.get() + 1);
                })
                .expect("register the window handler");
            let window_only_handler_runs = window_only_handler_runs.clone();
            window
                .on_command("window.only", move |_| {
                    window_only_handler_runs.set(window_only_handler_runs.get() + 1);
                })
                .expect("register the window-only handler");
            view! { <view><button aria:label="target">"Target"</button></view> }
        })?;
        catalog.menu(
            "main",
            MenuSpec::new()
                .command("local.act", "Local")
                .command("window.only", "Window only"),
        )?;
        Ok(())
    })
    .expect("declare the window");
    let main = app.show("main").expect("show main");
    let node = app
        .presentation_of(main)
        .expect("snapshot")
        .find_label("target")
        .expect("target button")
        .id();

    app.dispatch_command(main, "local.act", CommandSource::MenuBar)
        .expect("the window handler wins for a window source");
    assert_eq!(window_runs.get(), 1);
    assert_eq!(app_runs.get(), 0);

    app.dispatch_command(main, "local.act", CommandSource::Tray)
        .expect("tray selects the app handler");
    assert_eq!(window_runs.get(), 1, "tray never runs a window handler");
    assert_eq!(app_runs.get(), 1);

    assert_eq!(
        app.dispatch_command(main, "window.only", CommandSource::Tray),
        Err(CommandError::MissingHandler("window.only")),
        "tray does not fall back to a window handler"
    );
    assert_eq!(window_only_runs.get(), 0);
    app.dispatch_command(main, "window.only", CommandSource::MenuBar)
        .expect("the window-only handler still runs for window sources");
    assert_eq!(window_only_runs.get(), 1);

    app.dispatch_command(main, "shared.act", CommandSource::MenuBar)
        .expect("an undeclared window command falls back to the app");
    assert_eq!(app_runs.get(), 2);
    app.dispatch_command_on(main, "shared.act", CommandSource::ContextMenu, Some(node))
        .expect("the fallback preserves the node target");
    assert_eq!(app_runs.get(), 3);
    assert_eq!(
        app_targets.borrow().as_slice(),
        [None, Some(node)],
        "the fallback keeps its optional node target"
    );

    app.dispatch_app_command("shared.act", CommandSource::Tray, Some(main))
        .expect("app dispatch runs the app handler");
    assert_eq!(app_runs.get(), 4);
    assert_eq!(
        app_windows.borrow().as_slice(),
        [Some(main), Some(main), Some(main)],
        "the fallback and app dispatch expose the live target"
    );

    app.dispatch_app_command("shared.act", CommandSource::Tray, None)
        .expect("an app command runs without a target");
    assert_eq!(app_windows.borrow().last().copied(), Some(None));

    assert_eq!(
        app.dispatch_app_command("missing.act", CommandSource::Tray, None),
        Err(CommandError::MissingHandler("missing.act"))
    );
    assert_eq!(
        app.dispatch_command(main, "missing.act", CommandSource::MenuBar),
        Err(CommandError::UnknownCommand("missing.act"))
    );
}

#[test]
fn declared_window_menu_commands_may_be_served_by_app_handlers() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut app = Runtime::new();
    for (id, label) in [("shared.save", "save"), ("shared.copy", "copy")] {
        let seen = seen.clone();
        app.on_app_command(id, move |event| {
            seen.borrow_mut().push((label, event));
        })
        .expect("register the shared app command");
    }
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        catalog.menu("main", MenuSpec::new().command("shared.save", "Save"))?;
        catalog.context_menu("main", MenuSpec::new().command("shared.copy", "Copy"))?;
        Ok(())
    })
    .expect("app handlers satisfy the declared menu and context commands");
    let main = app
        .show("main")
        .expect("the window opens without window handlers");

    app.dispatch_command(main, "shared.save", CommandSource::MenuBar)
        .expect("the menu bar dispatches the app handler");
    app.dispatch_command(main, "shared.copy", CommandSource::ContextMenu)
        .expect("the context menu dispatches the app handler");

    let seen = seen.borrow();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].0, "save");
    assert_eq!(seen[0].1.source(), CommandSource::MenuBar);
    assert_eq!(seen[0].1.window(), Some(main));
    assert_eq!(seen[1].0, "copy");
    assert_eq!(seen[1].1.source(), CommandSource::ContextMenu);
    assert_eq!(seen[1].1.window(), Some(main));
}

#[test]
fn stale_targets_are_typed_errors_and_never_reach_a_reopened_activation() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let handler_seen = seen.clone();
    let mut app = Runtime::new();
    app.on_app_command("app.ping", move |event| {
        handler_seen.borrow_mut().push(event);
    })
    .expect("register ping");
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        Ok(())
    })
    .expect("declare the window");
    let first = app.show("main").expect("show the first activation");
    app.close_window(first).expect("close the first activation");
    let second = app.show("main").expect("reopen the same key");
    assert_ne!(first.activation(), second.activation());

    assert_eq!(
        app.dispatch_app_command("app.ping", CommandSource::Tray, Some(first)),
        Err(CommandError::Stale)
    );
    assert!(seen.borrow().is_empty(), "a stale target runs nothing");

    app.dispatch_app_command("app.ping", CommandSource::Tray, Some(second))
        .expect("the current activation dispatches");
    assert_eq!(seen.borrow().len(), 1);
    assert_eq!(seen.borrow()[0].window(), Some(second));

    let mut other = Runtime::new();
    let foreign = other
        .open_window("foreign", |_| view! { <view /> })
        .expect("open a foreign window");
    assert_eq!(
        app.dispatch_app_command("app.ping", CommandSource::Tray, Some(foreign)),
        Err(CommandError::WrongRuntime)
    );
    assert_eq!(seen.borrow().len(), 1);
}

#[test]
fn registration_errors_and_panics_leave_the_app_usable() {
    let mut app = Runtime::new();
    let (signal, store) = app
        .setup(|| (signal(String::from("idle")), Store::new()))
        .expect("application setup");
    app.on_app_command("app.ok", move |_| {
        signal.set(String::from("ran"));
        let mut transaction = Transaction::new();
        transaction = transaction.write(store.insert("row"));
        transaction.finish().expect("commit");
    })
    .expect("register ok");
    assert_eq!(
        app.on_app_command("app.ok", |_| {}),
        Err(CommandError::AlreadyRegistered("app.ok"))
    );
    app.on_app_command("app.boom", |_| panic!("the app command failed"))
        .expect("register boom");

    assert_eq!(
        app.dispatch_app_command("app.boom", CommandSource::Tray, None),
        Err(CommandError::HandlerPanicked("app.boom"))
    );
    app.dispatch_app_command("app.ok", CommandSource::Tray, None)
        .expect("the app is usable after a contained panic");

    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |_| {
            let rows = memo(move || store.rows(Filter::All).len().to_string());
            view! { <view><text aria:label="rows">{rows}</text></view> }
        })?;
        Ok(())
    })
    .expect("declare the window");
    let main = app.show("main").expect("show main");
    assert_eq!(text_of(&app, main, "rows"), "1");
    assert_eq!(
        app.on_app_command("app.late", |_| {}),
        Err(CommandError::RegistrationClosed)
    );
}

#[test]
fn app_registration_may_follow_window_declaration_but_not_the_first_window() {
    let runs = Rc::new(Cell::new(0));
    let handler_runs = runs.clone();
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        Ok(())
    })
    .expect("declare the window");
    app.on_app_command("app.late", move |_| {
        handler_runs.set(handler_runs.get() + 1);
    })
    .expect("declaration does not start the runtime");
    let main = app.show("main").expect("show the first window");
    app.dispatch_app_command("app.late", CommandSource::Tray, Some(main))
        .expect("dispatch after registration");
    assert_eq!(runs.get(), 1);
}

#[test]
fn registration_is_closed_after_a_headless_root_mounts() {
    let mut app = Runtime::new();
    app.mount(|| view! { <view /> })
        .expect("mount the headless root");
    assert_eq!(
        app.on_app_command("app.late", |_| {}),
        Err(CommandError::RegistrationClosed)
    );
}

#[test]
fn app_handlers_survive_window_close_and_reopen() {
    let runs = Rc::new(Cell::new(0));
    let handler_runs = runs.clone();
    let mut app = Runtime::new();
    app.on_app_command("app.count", move |_| {
        handler_runs.set(handler_runs.get() + 1);
    })
    .expect("register the app command");
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        Ok(())
    })
    .expect("declare the window");

    let first = app.show("main").expect("show");
    app.dispatch_app_command("app.count", CommandSource::Tray, Some(first))
        .expect("dispatch with a window");
    app.close_window(first).expect("close the only window");
    assert!(app.windows().is_empty());
    app.dispatch_app_command("app.count", CommandSource::Tray, None)
        .expect("dispatch with zero windows");
    let second = app.show("main").expect("reopen");
    app.dispatch_command(second, "app.count", CommandSource::Tray)
        .expect("tray dispatch through a window");
    assert_eq!(runs.get(), 3);

    let mut other = Runtime::new();
    other
        .on_app_command("app.count", |_| {})
        .expect("another runtime registers the same id");
}

#[test]
fn hidden_and_suspended_targets_are_valid_for_app_commands() {
    let runs = Rc::new(Cell::new(0));
    let handler_runs = runs.clone();
    let mut app = Runtime::new();
    app.on_app_command("app.ping", move |_| {
        handler_runs.set(handler_runs.get() + 1)
    })
    .expect("register ping");
    app.declare_windows(|catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .close_policy(ClosePolicy::Confirm),
            |window| {
                window
                    .on_close(move |_| {
                        window
                            .request_show("confirm")
                            .expect("show the confirmation");
                    })
                    .expect("register the close handler");
                view! { <view /> }
            },
        )?;
        catalog.window(
            WindowSpec::new("confirm").confirmation_for("main"),
            |window| {
                window
                    .close_request()
                    .expect("read the request")
                    .expect("a request exists");
                view! { <view /> }
            },
        )?;
        Ok(())
    })
    .expect("declare the windows");
    let main = app.show("main").expect("show main");
    app.hide(main).expect("hide the target");
    app.dispatch_app_command("app.ping", CommandSource::Tray, Some(main))
        .expect("a hidden target is still valid");
    app.show("main").expect("restore the target");

    app.request_close(main).expect("begin the close epoch");
    app.dispatch_app_command("app.ping", CommandSource::Tray, Some(main))
        .expect("a suspended target is still valid");
    assert_eq!(runs.get(), 2);
}
