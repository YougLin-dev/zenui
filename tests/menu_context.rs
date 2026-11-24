#![allow(unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;

fn context_declaration() -> MenuSpec {
    MenuSpec::new()
        .command("edit.copy", "Copy")
        .separator()
        .submenu(
            "More",
            MenuSpec::new().command("edit.select-all", "Select all"),
        )
}

#[test]
fn catalog_rejects_unknown_and_duplicate_context_menus() {
    let mut app = Runtime::new();
    let error = app
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
            catalog.context_menu("missing", MenuSpec::new())?;
            Ok(())
        })
        .expect_err("a context menu needs a declared window");
    assert_eq!(error, CatalogError::UnknownContextMenuWindow("missing"));

    let mut app = Runtime::new();
    let error = app
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
            catalog.context_menu("main", MenuSpec::new())?;
            catalog.context_menu("main", MenuSpec::new())?;
            Ok(())
        })
        .expect_err("each window declares at most one context menu");
    assert_eq!(error, CatalogError::DuplicateContextMenu("main"));

    assert!(
        app.presentation().is_err(),
        "a rejected catalog mounts nothing"
    );
}

#[test]
fn catalog_rejects_repeated_and_shared_context_command_ids() {
    let mut app = Runtime::new();
    let error = app
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
            catalog.context_menu(
                "main",
                MenuSpec::new()
                    .command("edit.copy", "Copy")
                    .submenu("More", MenuSpec::new().command("edit.copy", "Copy again")),
            )?;
            Ok(())
        })
        .expect_err("command ids are unique within one context menu");
    assert_eq!(
        error,
        CatalogError::DuplicateMenuCommand {
            window: "main",
            id: "edit.copy"
        }
    );

    let mut app = Runtime::new();
    let error = app
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |window| {
                window.on_command("file.new", |_| {}).expect("register");
                view! { <view /> }
            })?;
            catalog.menu("main", MenuSpec::new().command("file.new", "New"))?;
            catalog.context_menu(
                "main",
                MenuSpec::new().submenu("Edit", MenuSpec::new().command("file.new", "New")),
            )?;
            Ok(())
        })
        .expect_err("the menu bar and context menu share one command namespace");
    assert_eq!(
        error,
        CatalogError::DuplicateMenuCommand {
            window: "main",
            id: "file.new"
        }
    );
}

#[test]
fn context_menus_reject_shortcuts() {
    let mut app = Runtime::new();
    let error = app
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
            catalog.context_menu(
                "main",
                MenuSpec::new()
                    .item(MenuCommand::new("edit.copy", "Copy").bind_shortcut("Ctrl+C")?),
            )?;
            Ok(())
        })
        .expect_err("a context menu has no accelerator table");
    assert_eq!(
        error,
        CatalogError::ContextMenuShortcut {
            window: "main",
            id: "edit.copy"
        }
    );
}

#[test]
fn catalog_limits_the_context_menu_tree_to_65535_items() {
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
    let error = app
        .declare_windows(|catalog| {
            catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
            catalog.context_menu("main", menu_with(65_536))?;
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
fn the_window_context_menu_is_readable_for_a_live_window_only() {
    let menu = context_declaration();
    let declaration = menu.clone();
    let mut app = Runtime::new();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), |window| {
            for id in ["edit.copy", "edit.select-all"] {
                window.on_command(id, |_| {}).expect("register");
            }
            view! { <view /> }
        })?;
        catalog.window(WindowSpec::new("plain"), |_| view! { <view /> })?;
        catalog.context_menu("main", declaration)?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show main");
    let plain = app.show("plain").expect("show plain");
    assert_eq!(app.window_context_menu(main), Some(menu.clone()));
    assert_eq!(
        app.window_context_menu(plain),
        None,
        "a window without a context menu"
    );
    assert_eq!(
        app.window_menu(main),
        None,
        "a context menu does not create a menu bar"
    );

    app.close_window(main).expect("close main");
    assert_eq!(
        app.window_context_menu(main),
        None,
        "a closed activation has no context menu"
    );

    let mut other = Runtime::new();
    let foreign = other
        .open_window("foreign", |_| view! { <view /> })
        .expect("foreign window");
    assert_eq!(
        app.window_context_menu(foreign),
        None,
        "a foreign context cannot read this runtime"
    );

    let reopened = app.show("main").expect("reopen main");
    assert_eq!(app.window_context_menu(reopened), Some(menu));
}

#[test]
fn context_menu_commands_register_and_dispatch_as_context_menu_source() {
    let runs = Rc::new(Cell::new(0));
    let sources = Rc::new(std::cell::RefCell::new(Vec::new()));
    let handler_runs = runs.clone();
    let handler_sources = sources.clone();
    let mut app = Runtime::new();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            let handler_runs = handler_runs.clone();
            let handler_sources = handler_sources.clone();
            window
                .on_command("edit.copy", move |event| {
                    handler_runs.set(handler_runs.get() + 1);
                    handler_sources.borrow_mut().push(event.source());
                })
                .expect("register edit.copy");
            window
                .on_command("edit.select-all", |_| {})
                .expect("register edit.select-all");
            view! { <view /> }
        })?;
        catalog.context_menu("main", context_declaration())?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show main");
    app.dispatch_command(main, "edit.copy", CommandSource::ContextMenu)
        .expect("dispatch the context command");
    assert_eq!(runs.get(), 1);
    assert_eq!(
        sources.borrow().as_slice(),
        [CommandSource::ContextMenu],
        "the handler sees the context-menu source"
    );

    assert_eq!(
        app.dispatch_command(main, "edit.unknown", CommandSource::ContextMenu),
        Err(CommandError::UnknownCommand("edit.unknown")),
        "only declared context commands dispatch"
    );
    assert_eq!(runs.get(), 1, "an unknown command runs nothing");
}

#[test]
fn a_context_menu_command_without_a_handler_fails_the_open_explicitly() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |window| {
            window
                .on_command("edit.copy", |_| {})
                .expect("register copy");
            view! { <view /> }
        })?;
        catalog.context_menu("main", context_declaration())?;
        Ok(())
    })
    .expect("catalog");

    let error = app
        .show("main")
        .expect_err("a declared context item without a handler fails the open");
    assert_eq!(
        error,
        WindowError::Command(CommandError::MissingHandler("edit.select-all"))
    );
    assert!(
        app.windows().is_empty(),
        "the failed open releases the window"
    );
}

#[test]
fn context_menu_and_window_declaration_order_does_not_matter() {
    for menu_first in [true, false] {
        let runs = Rc::new(Cell::new(0));
        let handler_runs = runs.clone();
        let mut app = Runtime::new();
        app.declare_windows(move |catalog| {
            let menu = context_declaration();
            if menu_first {
                catalog.context_menu("main", menu.clone())?;
            }
            let handler_runs = handler_runs.clone();
            catalog.window(WindowSpec::new("main").primary(), move |window| {
                let handler_runs = handler_runs.clone();
                window
                    .on_command("edit.copy", move |_| {
                        handler_runs.set(handler_runs.get() + 1);
                    })
                    .expect("register edit.copy");
                window
                    .on_command("edit.select-all", |_| {})
                    .expect("register edit.select-all");
                view! { <view /> }
            })?;
            if !menu_first {
                catalog.context_menu("main", menu)?;
            }
            Ok(())
        })
        .expect("catalog");

        let main = app.show("main").expect("show the window");
        app.dispatch_command(main, "edit.copy", CommandSource::ContextMenu)
            .expect("dispatch the declared command");
        assert_eq!(runs.get(), 1, "menu_first={menu_first}");
    }
}
