#![allow(unused_braces)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use zenui::Phase;
use zenui::prelude::*;

#[test]
fn menu_specs_carry_stable_command_ids_and_static_defaults() {
    let menu = MenuSpec::new()
        .command("file.new", "New")
        .item(MenuCommand::new("file.open", "Open...").enabled(false))
        .separator()
        .submenu(
            "View",
            MenuSpec::new().item(MenuCommand::new("view.sidebar", "Sidebar").checked(true)),
        );

    assert_eq!(menu.items().len(), 4);

    let MenuItem::Command(new) = &menu.items()[0] else {
        panic!("the first item is a command");
    };
    assert_eq!(new.id(), "file.new");
    assert_eq!(new.label(), "New");
    assert!(new.is_enabled(), "commands default to enabled");
    assert!(!new.is_checked(), "commands default to unchecked");

    let MenuItem::Command(open) = &menu.items()[1] else {
        panic!("the second item is a command");
    };
    assert_eq!(open.id(), "file.open");
    assert_eq!(open.label(), "Open...");
    assert!(!open.is_enabled());

    assert_eq!(menu.items()[2], MenuItem::Separator);

    let MenuItem::Submenu(view) = &menu.items()[3] else {
        panic!("the last item is a submenu");
    };
    assert_eq!(view.label(), "View");
    let MenuItem::Command(sidebar) = &view.items()[0] else {
        panic!("the submenu item is a command");
    };
    assert_eq!(sidebar.id(), "view.sidebar");
    assert_eq!(sidebar.label(), "Sidebar");
    assert!(sidebar.is_checked());
    assert!(sidebar.is_enabled());
}

#[test]
fn catalog_rejects_unknown_and_duplicate_menus() {
    let mut app = Runtime::new();
    let error = app
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
            catalog.menu("missing", MenuSpec::new())?;
            Ok(())
        })
        .expect_err("a menu needs a declared window");
    assert_eq!(error, CatalogError::UnknownMenuWindow("missing"));

    let mut app = Runtime::new();
    let error = app
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
            catalog.menu("main", MenuSpec::new())?;
            catalog.menu("main", MenuSpec::new())?;
            Ok(())
        })
        .expect_err("each window declares at most one menu");
    assert_eq!(error, CatalogError::DuplicateMenu("main"));

    assert!(
        app.presentation().is_err(),
        "a rejected catalog mounts nothing"
    );
}

#[test]
fn catalog_rejects_duplicate_command_ids_in_one_menu_tree() {
    let mut app = Runtime::new();
    let error = app
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
            catalog.menu(
                "main",
                MenuSpec::new()
                    .command("file.new", "New")
                    .submenu("More", MenuSpec::new().command("file.new", "New again")),
            )?;
            Ok(())
        })
        .expect_err("command ids are unique within a window menu");
    assert_eq!(
        error,
        CatalogError::DuplicateMenuCommand {
            window: "main",
            id: "file.new"
        }
    );
}

#[test]
fn catalog_limits_the_menu_tree_to_65535_items() {
    fn menu_with(count: usize) -> MenuSpec {
        let mut menu = MenuSpec::new();
        for index in 0..count {
            let id: &'static str = Box::leak(format!("command.{index}").into_boxed_str());
            let label: &'static str = Box::leak(format!("Command {index}").into_boxed_str());
            menu = menu.command(id, label);
        }
        menu
    }

    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        catalog.menu("main", menu_with(65_535))?;
        Ok(())
    })
    .expect("65535 items fit the limit");

    let mut app = Runtime::new();
    let error = app
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
            catalog.menu("main", menu_with(65_536))?;
            Ok(())
        })
        .expect_err("one item over the limit is rejected");
    assert_eq!(
        error,
        CatalogError::MenuTooLarge {
            window: "main",
            items: 65_536
        }
    );
}

#[test]
fn menu_and_window_declaration_order_does_not_matter() {
    for menu_first in [true, false] {
        let runs = Rc::new(Cell::new(0));
        let handler_runs = runs.clone();
        let mut app = Runtime::new();
        app.declare_windows(move |catalog| {
            let menu = MenuSpec::new().command("file.new", "New");
            if menu_first {
                catalog.menu("main", menu.clone())?;
            }
            let handler_runs = handler_runs.clone();
            catalog.window(WindowSpec::new("main").primary(), move |window| {
                let handler_runs = handler_runs.clone();
                window
                    .on_command("file.new", move |_| {
                        handler_runs.set(handler_runs.get() + 1);
                    })
                    .expect("register the menu command");
                view! { <view /> }
            })?;
            if !menu_first {
                catalog.menu("main", menu)?;
            }
            Ok(())
        })
        .expect("catalog");

        let main = app.show("main").expect("show the window");
        app.dispatch_command(main, "file.new", CommandSource::MenuBar)
            .expect("dispatch the declared command");
        assert_eq!(runs.get(), 1, "menu_first={menu_first}");
    }
}

#[test]
fn the_window_menu_is_readable_for_a_live_window_only() {
    let menu = MenuSpec::new().command("file.new", "New");
    let declaration = menu.clone();
    let mut app = Runtime::new();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), |window| {
            window.on_command("file.new", |_| {}).expect("register");
            view! { <view /> }
        })?;
        catalog.window(WindowSpec::new("plain"), |_| view! { <view /> })?;
        catalog.menu("main", declaration)?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show main");
    let plain = app.show("plain").expect("show plain");
    assert_eq!(app.window_menu(main), Some(menu.clone()));
    assert_eq!(app.window_menu(plain), None, "a window without a menu");

    app.close_window(main).expect("close main");
    assert_eq!(
        app.window_menu(main),
        None,
        "a closed activation has no menu"
    );

    let mut other = Runtime::new();
    let foreign = other
        .open_window("foreign", |_| view! { <view /> })
        .expect("foreign window");
    assert_eq!(
        app.window_menu(foreign),
        None,
        "a foreign context cannot read this runtime"
    );

    let reopened = app.show("main").expect("reopen main");
    assert_eq!(app.window_menu(reopened), Some(menu));
}

#[test]
fn an_empty_menu_needs_no_handlers() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        catalog.menu("main", MenuSpec::new())?;
        Ok(())
    })
    .expect("catalog");

    let main = app
        .show("main")
        .expect("an empty menu opens without handlers");
    assert!(app.window_alive(main));
}

fn command_by_id(menu: &MenuSpec, id: &str) -> MenuCommand {
    menu.items()
        .iter()
        .find_map(|item| match item {
            MenuItem::Command(command) if command.id() == id => Some(command.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no command {id}"))
}

#[test]
fn menu_state_reads_committed_state_and_rejects_writes() {
    let mut app = Runtime::new();
    let (checked, store) = app
        .setup(|| (signal(true), Store::new()))
        .expect("application setup");
    let menu = MenuSpec::new()
        .command("state.toggle", "Toggle")
        .command("state.signal-write", "Signal write")
        .command("state.txn-write", "Store write")
        .command("state.panic", "Panic")
        .command("state.on", "On")
        .command("state.off", "Off")
        .command("state.commit", "Commit")
        .command("state.rows", "Rows");
    let declaration = menu.clone();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            window
                .on_command("state.toggle", move |_| checked.set(!checked.get()))
                .expect("register toggle");
            window
                .on_command("state.on", move |_| checked.set(true))
                .expect("register on");
            window
                .on_command("state.off", move |_| checked.set(false))
                .expect("register off");
            window
                .on_command("state.commit", move |_| {
                    let mut transaction = Transaction::new();
                    transaction = transaction.write(store.insert("row"));
                    transaction.finish().expect("commit the row");
                })
                .expect("register commit");
            for id in [
                "state.signal-write",
                "state.txn-write",
                "state.panic",
                "state.rows",
            ] {
                window.on_command(id, |_| {}).expect("register inert");
            }
            window
                .on_menu_state(move |command| match command.id() {
                    "state.toggle" => MenuCommandState::new(true, checked.get()),
                    "state.signal-write" => {
                        checked.set(false);
                        command.default_state()
                    }
                    "state.txn-write" => {
                        let mut transaction = Transaction::new();
                        transaction = transaction.write(store.insert("bad"));
                        let _ = transaction.finish();
                        command.default_state()
                    }
                    "state.panic" => panic!("the menu state callback failed"),
                    "state.rows" => MenuCommandState::new(true, store.rows(Filter::All).is_empty()),
                    _ => command.default_state(),
                })
                .expect("register menu state");
            view! { <view /> }
        })?;
        catalog.menu("main", declaration)?;
        Ok(())
    })
    .expect("catalog");
    let main = app.show("main").expect("show main");

    let toggle = command_by_id(&menu, "state.toggle");
    assert_eq!(
        app.menu_state(main, &toggle),
        Ok(Some(MenuCommandState::new(true, true)))
    );

    let error = app
        .menu_state(main, &command_by_id(&menu, "state.signal-write"))
        .expect_err("a Signal write is rejected");
    assert!(
        matches!(
            error,
            MenuStateError::Context {
                operation: "Signal::set",
                phase: Phase::Idle
            }
        ),
        "{error:?}"
    );
    assert_eq!(
        app.menu_state(main, &toggle),
        Ok(Some(MenuCommandState::new(true, true))),
        "the rejected write leaves the Signal unchanged"
    );

    let error = app
        .menu_state(main, &command_by_id(&menu, "state.txn-write"))
        .expect_err("a Store transaction is rejected");
    assert!(
        matches!(
            error,
            MenuStateError::Write(WriteError::Context {
                operation: "Transaction::finish",
                phase: Phase::Idle
            })
        ),
        "{error:?}"
    );
    let rows = command_by_id(&menu, "state.rows");
    assert_eq!(
        app.menu_state(main, &rows),
        Ok(Some(MenuCommandState::new(true, true))),
        "the rejected transaction leaves the Store unchanged"
    );

    let error = app
        .menu_state(main, &command_by_id(&menu, "state.panic"))
        .expect_err("a panicking callback is contained");
    assert_eq!(error, MenuStateError::HandlerPanicked);
    assert_eq!(
        app.menu_state(main, &toggle),
        Ok(Some(MenuCommandState::new(true, true))),
        "the runtime is still usable after a contained panic"
    );

    app.dispatch_command(main, "state.off", CommandSource::MenuBar)
        .expect("commit the off state");
    assert_eq!(
        app.menu_state(main, &toggle),
        Ok(Some(MenuCommandState::new(true, false))),
        "the callback sees the committed Signal"
    );
    app.dispatch_command(main, "state.on", CommandSource::MenuBar)
        .expect("commit the on state");
    assert_eq!(
        app.menu_state(main, &toggle),
        Ok(Some(MenuCommandState::new(true, true)))
    );
    app.dispatch_command(main, "state.commit", CommandSource::MenuBar)
        .expect("commit a row");
    assert_eq!(
        app.menu_state(main, &rows),
        Ok(Some(MenuCommandState::new(true, false))),
        "the callback reads the committed Store"
    );
}

#[test]
fn menu_state_registration_is_per_activation_and_readable() {
    let attempts = Rc::new(Cell::new(0));
    let attempts_factory = attempts.clone();
    let duplicate: Rc<RefCell<Option<Result<(), WindowError>>>> = Rc::new(RefCell::new(None));
    let duplicate_factory = duplicate.clone();
    let checked = Rc::new(Cell::new(true));
    let checked_factory = checked.clone();
    let menu = MenuSpec::new().command("act", "Act");
    let declaration = menu.clone();

    let mut app = Runtime::new();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            attempts_factory.set(attempts_factory.get() + 1);
            window.on_command("act", |_| {}).expect("register act");
            match attempts_factory.get() {
                1 => {
                    let checked = checked_factory.clone();
                    window
                        .on_menu_state(move |command| {
                            MenuCommandState::new(true, checked.get() && command.id() == "act")
                        })
                        .expect("register the first state handler");
                    let checked = checked_factory.clone();
                    *duplicate_factory.borrow_mut() = Some(window.on_menu_state(move |command| {
                        MenuCommandState::new(false, checked.get() && command.id() == "act")
                    }));
                }
                3 => {
                    let checked = checked_factory.clone();
                    window
                        .on_menu_state(move |command| {
                            MenuCommandState::new(true, checked.get() && command.id() == "act")
                        })
                        .expect("register the new state handler");
                }
                _ => {}
            }
            view! { <view /> }
        })?;
        catalog.menu("main", declaration.clone())?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show the first activation");
    assert!(
        matches!(
            duplicate.borrow().as_ref(),
            Some(Err(WindowError::State(_)))
        ),
        "a second state handler is rejected: {:?}",
        duplicate.borrow()
    );
    let act = command_by_id(&menu, "act");
    assert_eq!(
        app.menu_state(main, &act),
        Ok(Some(MenuCommandState::new(true, true)))
    );

    app.close_window(main).expect("close the first activation");
    assert_eq!(
        app.menu_state(main, &act),
        Err(MenuStateError::Stale),
        "a closed activation cannot query state"
    );
    let reopened = app
        .show("main")
        .expect("the second activation opens without a state handler");
    assert_eq!(
        app.menu_state(reopened, &act),
        Ok(None),
        "the old handler does not leak into the new activation"
    );

    app.close_window(reopened)
        .expect("close the second activation");
    let registered = app.show("main").expect("the third activation registers");
    assert_eq!(
        app.menu_state(registered, &act),
        Ok(Some(MenuCommandState::new(true, true)))
    );

    let mut other = Runtime::new();
    let foreign = other
        .open_window("foreign", |_| view! { <view /> })
        .expect("foreign window");
    assert_eq!(
        app.menu_state(foreign, &act),
        Err(MenuStateError::WrongRuntime)
    );
}

fn source_name(source: CommandSource) -> &'static str {
    match source {
        CommandSource::MenuBar => "menu-bar",
        CommandSource::Accelerator => "accelerator",
        CommandSource::ContextMenu => "context-menu",
        CommandSource::Accessibility => "accessibility",
        CommandSource::Tray => "tray",
        _ => "future",
    }
}

#[test]
fn command_sources_are_non_exhaustive_and_cover_every_entry_point() {
    assert_eq!(source_name(CommandSource::MenuBar), "menu-bar");
    assert_eq!(source_name(CommandSource::Accelerator), "accelerator");
    assert_eq!(source_name(CommandSource::ContextMenu), "context-menu");
    assert_eq!(source_name(CommandSource::Accessibility), "accessibility");
    assert_eq!(source_name(CommandSource::Tray), "tray");
}

#[component]
fn NestedRegistration(
    window: WindowCtx,
    outcome: Rc<RefCell<Option<Result<(), WindowError>>>>,
) -> MountResult<MountPlan> {
    *outcome.borrow_mut() = Some(window.on_command("file.open", |_| {}));
    view! { <view /> }
}

#[test]
fn command_handlers_register_once_during_the_window_factory_only() {
    let duplicate: Rc<RefCell<Option<Result<(), WindowError>>>> = Rc::new(RefCell::new(None));
    let unknown: Rc<RefCell<Option<Result<(), WindowError>>>> = Rc::new(RefCell::new(None));
    let late: Rc<RefCell<Option<Result<(), WindowError>>>> = Rc::new(RefCell::new(None));
    let duplicate_factory = duplicate.clone();
    let unknown_factory = unknown.clone();
    let late_factory = late.clone();
    let nested_outcome: Rc<RefCell<Option<Result<(), WindowError>>>> = Rc::new(RefCell::new(None));
    let nested_factory = nested_outcome.clone();

    let mut app = Runtime::new();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            window
                .on_command("file.new", |_| {})
                .expect("register the first command");
            *duplicate_factory.borrow_mut() = Some(window.on_command("file.new", |_| {}));
            *unknown_factory.borrow_mut() = Some(window.on_command("not.declared", |_| {}));
            window
                .on_command("file.open", |_| {})
                .expect("register every declared command");
            let late = late_factory.clone();
            let nested = nested_factory.clone();
            view! {
                <view>
                    <NestedRegistration window={window} outcome={nested} />
                    <button
                        aria:label="late"
                        on:click={move || {
                            *late.borrow_mut() = Some(window.on_command("file.new", |_| {}));
                        }}
                    >"Late"</button>
                </view>
            }
        })?;
        catalog.menu(
            "main",
            MenuSpec::new()
                .command("file.new", "New")
                .command("file.open", "Open"),
        )?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show");
    assert!(
        matches!(
            duplicate.borrow().as_ref(),
            Some(Err(WindowError::Command(CommandError::AlreadyRegistered(
                "file.new"
            ))))
        ),
        "a command id registers once per activation: {:?}",
        duplicate.borrow()
    );
    assert!(
        matches!(
            unknown.borrow().as_ref(),
            Some(Err(WindowError::Command(CommandError::UnknownCommand(
                "not.declared"
            ))))
        ),
        "only commands declared in the window menu can register"
    );
    assert!(
        matches!(
            nested_outcome.borrow().as_ref(),
            Some(Err(WindowError::State(_)))
        ),
        "nested component setup cannot register command handlers"
    );

    app.click_label("late").expect("click the late action");
    assert!(
        matches!(late.borrow().as_ref(), Some(Err(WindowError::State(_)))),
        "registration after setup is rejected"
    );

    assert!(app.window_alive(main));
}

#[test]
fn a_declared_command_without_a_handler_fails_the_open_explicitly() {
    let attempts = Rc::new(Cell::new(0));
    let attempts_probe = attempts.clone();
    let first_runs = Rc::new(Cell::new(0));
    let first_runs_probe = first_runs.clone();

    let mut app = Runtime::new();
    app.declare_windows(move |catalog| {
        let first_runs = first_runs.clone();
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            attempts.set(attempts.get() + 1);
            if attempts.get() == 1 {
                let runs = first_runs.clone();
                window
                    .on_command("file.new", move |_| runs.set(runs.get() + 1))
                    .expect("register only the first command");
            } else {
                window
                    .on_command("file.new", |_| {})
                    .expect("register the first command");
                window
                    .on_command("file.open", |_| {})
                    .expect("register the missing command");
            }
            view! { <view /> }
        })?;
        catalog.menu(
            "main",
            MenuSpec::new()
                .command("file.new", "New")
                .command("file.open", "Open"),
        )?;
        Ok(())
    })
    .expect("catalog");

    let error = app
        .show("main")
        .expect_err("a declared item without a handler fails the open");
    assert_eq!(
        error,
        WindowError::Command(CommandError::MissingHandler("file.open"))
    );
    assert!(
        app.windows().is_empty(),
        "the failed open releases the window"
    );

    let window = app.show("main").expect("the retry registers every command");
    assert_eq!(window.activation(), 2, "the retry uses a new activation");
    assert_eq!(attempts_probe.get(), 2);
    assert_eq!(
        first_runs_probe.get(),
        0,
        "the failed activation left no handler"
    );
}

struct CommandApp {
    app: Runtime,
    main: WindowCtx,
    runs: Rc<Cell<u32>>,
    events: Rc<RefCell<Vec<CommandEvent>>>,
}

fn text_of(app: &Runtime, window: WindowCtx, label: &str) -> String {
    app.presentation_of(window)
        .expect("snapshot")
        .find_label(label)
        .unwrap_or_else(|| panic!("no element labelled {label}"))
        .text()
}

fn command_app() -> CommandApp {
    let mut app = Runtime::new();
    let (status, row_count) = app
        .setup(|| {
            let store = Store::new();
            let status = signal(String::from("idle"));
            let row_count = memo(move || store.rows(Filter::All).len().to_string());
            (status, row_count)
        })
        .expect("application setup");
    let runs = Rc::new(Cell::new(0));
    let events = Rc::new(RefCell::new(Vec::new()));
    let runs_factory = runs.clone();
    let events_factory = events.clone();

    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            let update_runs = runs_factory.clone();
            let update_events = events_factory.clone();
            window
                .on_command("do.update", move |event| {
                    update_runs.set(update_runs.get() + 1);
                    update_events.borrow_mut().push(event);
                    status.set(String::from("ran"));
                    let mut transaction = Transaction::new();
                    transaction = transaction.write(Store::new().insert("from-command"));
                    transaction.finish().expect("commit the store write");
                })
                .expect("register update");
            window
                .on_command("do.open", move |_| {
                    window.request_show("details").expect("queue show");
                })
                .expect("register open");
            window
                .on_command("do.hide", move |_| {
                    window.request_hide().expect("queue hide");
                })
                .expect("register hide");
            window
                .on_command("do.close", move |_| {
                    window.request_close().expect("queue close");
                })
                .expect("register close");
            view! {
                <view>
                    <text aria:label="status">{status}</text>
                    <text aria:label="rows">{row_count}</text>
                </view>
            }
        })?;
        catalog.window(WindowSpec::new("details"), |_| {
            view! { <text aria:label="details">"details"</text> }
        })?;
        catalog.menu(
            "main",
            MenuSpec::new()
                .command("do.update", "Update")
                .command("do.open", "Open")
                .command("do.hide", "Hide")
                .command("do.close", "Close"),
        )?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show main");
    CommandApp {
        app,
        main,
        runs,
        events,
    }
}

#[test]
fn dispatch_runs_the_handler_once_in_the_event_phase_and_settles_requests() {
    let mut harness = command_app();
    let main = harness.main;
    assert_eq!(text_of(&harness.app, main, "status"), "idle");
    assert_eq!(text_of(&harness.app, main, "rows"), "0");

    harness
        .app
        .dispatch_command(main, "do.update", CommandSource::MenuBar)
        .expect("menu dispatch");
    assert_eq!(harness.runs.get(), 1, "one dispatch runs one handler");
    assert_eq!(
        text_of(&harness.app, main, "status"),
        "ran",
        "the handler wrote a Signal"
    );
    assert_eq!(
        text_of(&harness.app, main, "rows"),
        "1",
        "the handler committed a Store write"
    );
    let event = harness
        .events
        .borrow()
        .last()
        .copied()
        .expect("the handler saw an event");
    assert_eq!(event.source(), CommandSource::MenuBar);
    assert_eq!(event.target(), None);

    harness
        .app
        .dispatch_command(main, "do.update", CommandSource::Accelerator)
        .expect("accelerator dispatch");
    assert_eq!(harness.runs.get(), 2);
    assert_eq!(
        harness.events.borrow().last().copied().unwrap().source(),
        CommandSource::Accelerator
    );

    assert_eq!(
        harness
            .app
            .dispatch_command(main, "missing", CommandSource::MenuBar),
        Err(CommandError::UnknownCommand("missing"))
    );
    assert_eq!(harness.runs.get(), 2, "an unknown command runs nothing");

    harness
        .app
        .dispatch_command(main, "do.open", CommandSource::ContextMenu)
        .expect("queue the show through the existing window path");
    assert_eq!(
        harness.app.windows().len(),
        1,
        "the request waits for the settle step"
    );
    harness
        .app
        .apply_window_requests()
        .expect("apply the show request");
    assert_eq!(harness.app.windows().len(), 2);

    harness
        .app
        .dispatch_command(main, "do.hide", CommandSource::Accessibility)
        .expect("queue the hide");
    harness
        .app
        .apply_window_requests()
        .expect("apply the hide request");
    assert!(!harness.app.window_visible(main));
    assert_eq!(
        harness
            .app
            .dispatch_command(main, "do.update", CommandSource::MenuBar),
        Err(CommandError::Hidden)
    );

    harness.app.show("main").expect("restore the window");
    harness
        .app
        .dispatch_command(main, "do.close", CommandSource::Accessibility)
        .expect("queue the close");
    harness
        .app
        .apply_window_requests()
        .expect("apply the close request");
    assert!(!harness.app.window_alive(main));
    assert_eq!(
        harness
            .app
            .dispatch_command(main, "do.update", CommandSource::MenuBar),
        Err(CommandError::Stale)
    );
}

#[test]
fn a_panicking_command_handler_is_contained() {
    let mut app = Runtime::new();
    let status = app.setup(|| signal(String::from("idle"))).expect("setup");
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            window
                .on_command("boom", |_| panic!("the command handler failed"))
                .expect("register boom");
            window
                .on_command("ok", move |_| status.set(String::from("ok")))
                .expect("register ok");
            view! { <view><text aria:label="status">{status}</text></view> }
        })?;
        catalog.menu(
            "main",
            MenuSpec::new().command("boom", "Boom").command("ok", "OK"),
        )?;
        Ok(())
    })
    .expect("catalog");
    let main = app.show("main").expect("show main");

    assert_eq!(
        app.dispatch_command(main, "boom", CommandSource::MenuBar),
        Err(CommandError::HandlerPanicked("boom"))
    );
    app.dispatch_command(main, "ok", CommandSource::MenuBar)
        .expect("the runtime is still usable after a contained panic");
    assert_eq!(text_of(&app, main, "status"), "ok");
}

#[test]
fn dispatch_validates_context_targets() {
    let seen: Rc<RefCell<Vec<CommandEvent>>> = Rc::new(RefCell::new(Vec::new()));
    let a_seen = seen.clone();
    let mut app = Runtime::new();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("a").primary(), move |window| {
            let seen = a_seen.clone();
            window
                .on_command("act", move |event| seen.borrow_mut().push(event))
                .expect("register act");
            view! { <view><button aria:label="a-button">"A"</button></view> }
        })?;
        catalog.window(WindowSpec::new("b"), |_| {
            view! { <view><button aria:label="b-button">"B"</button></view> }
        })?;
        catalog.menu("a", MenuSpec::new().command("act", "Act"))?;
        Ok(())
    })
    .expect("catalog");

    let a = app.show("a").expect("show a");
    let b = app.show("b").expect("show b");
    let a_button = app
        .presentation_of(a)
        .expect("a snapshot")
        .find_label("a-button")
        .expect("a button")
        .id();
    let b_button = app
        .presentation_of(b)
        .expect("b snapshot")
        .find_label("b-button")
        .expect("b button")
        .id();

    app.dispatch_command_on(a, "act", CommandSource::ContextMenu, Some(a_button))
        .expect("dispatch with a local target");
    assert_eq!(
        seen.borrow().last().copied().unwrap().target(),
        Some(a_button)
    );

    assert_eq!(
        app.dispatch_command_on(a, "act", CommandSource::ContextMenu, Some(b_button)),
        Err(CommandError::InvalidTarget(b_button))
    );
    app.close_window(b).expect("close b");
    assert_eq!(
        app.dispatch_command_on(a, "act", CommandSource::Accessibility, Some(b_button)),
        Err(CommandError::InvalidTarget(b_button))
    );

    let mut other = Runtime::new();
    let foreign = other
        .open_window("foreign", |_| view! { <view /> })
        .expect("open a foreign window");
    assert_eq!(
        app.dispatch_command(foreign, "act", CommandSource::MenuBar),
        Err(CommandError::WrongRuntime)
    );
}

#[test]
fn dispatch_rejects_a_window_awaiting_close_confirmation() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .close_policy(ClosePolicy::Confirm),
            move |window| {
                window.on_command("act", |_| {}).expect("register act");
                window
                    .on_close(move |_| {
                        window
                            .request_show("confirm")
                            .expect("show the confirmation");
                        CloseOutcome::Pending
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
                    .expect("read the pending request")
                    .expect("a pending request exists");
                view! { <view /> }
            },
        )?;
        catalog.menu("main", MenuSpec::new().command("act", "Act"))?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show main");
    app.request_close(main).expect("begin the close epoch");
    assert_eq!(
        app.dispatch_command(main, "act", CommandSource::MenuBar),
        Err(CommandError::Suspended)
    );
}

#[test]
fn closing_and_reopening_requires_fresh_command_handlers() {
    let attempts = Rc::new(Cell::new(0));
    let attempts_probe = attempts.clone();
    let first_runs = Rc::new(Cell::new(0));
    let first_probe = first_runs.clone();
    let reopened_runs = Rc::new(Cell::new(0));
    let reopened_probe = reopened_runs.clone();

    let mut app = Runtime::new();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            attempts.set(attempts.get() + 1);
            match attempts.get() {
                1 => {
                    let runs = first_runs.clone();
                    window
                        .on_command("act", move |_| runs.set(runs.get() + 1))
                        .expect("register the first activation");
                }
                2 => {
                    // 忘记重新注册：新 activation 必须明确失败，而不是复用旧处理器。
                }
                _ => {
                    let runs = reopened_runs.clone();
                    window
                        .on_command("act", move |_| runs.set(runs.get() + 1))
                        .expect("register the new activation");
                }
            }
            view! { <view /> }
        })?;
        catalog.menu("main", MenuSpec::new().command("act", "Act"))?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("the first activation registers");
    app.dispatch_command(main, "act", CommandSource::MenuBar)
        .expect("dispatch during the first activation");
    assert_eq!(first_probe.get(), 1);

    app.close_window(main).expect("close the first activation");
    assert_eq!(
        app.dispatch_command(main, "act", CommandSource::MenuBar),
        Err(CommandError::Stale),
        "an old activation cannot dispatch after close"
    );
    let error = app
        .show("main")
        .expect_err("the reopened activation must register again");
    assert_eq!(
        error,
        WindowError::Command(CommandError::MissingHandler("act"))
    );

    let reopened = app.show("main").expect("the third activation registers");
    assert!(reopened.activation() > main.activation());
    app.dispatch_command(reopened, "act", CommandSource::MenuBar)
        .expect("dispatch during the new activation");
    assert_eq!(reopened_probe.get(), 1);
    assert_eq!(
        first_probe.get(),
        1,
        "the previous activation's handler does not leak into the new one"
    );
    assert_eq!(attempts_probe.get(), 3);
}
