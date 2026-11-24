//! 托盘声明的结构校验与应用级命令边界。

use std::cell::RefCell;
use std::rc::Rc;

use zenui::prelude::*;

#[test]
fn a_tray_declaration_validates_the_tooltip_and_menu() {
    let mut app = Runtime::new();
    let long_tooltip = "😀".repeat(64);
    assert_eq!(
        app.declare_tray(TraySpec::new(long_tooltip)),
        Err(TrayError::TooltipTooLong { units: 128 }),
        "the tooltip limit counts UTF-16 units, not characters"
    );

    let duplicate = TraySpec::new("ZenUI").with_menu(
        MenuSpec::new()
            .command("app.one", "One")
            .command("app.one", "Again"),
    );
    assert_eq!(
        app.declare_tray(duplicate),
        Err(TrayError::DuplicateCommand("app.one"))
    );

    let shortcut = TraySpec::new("ZenUI").with_menu(
        MenuSpec::new().item(
            MenuCommand::new("app.save", "Save")
                .bind_shortcut("Ctrl+S")
                .expect("parse the shortcut"),
        ),
    );
    assert_eq!(
        app.declare_tray(shortcut),
        Err(TrayError::UnsupportedShortcut { id: "app.save" }),
        "a tray menu has no accelerator table"
    );

    let tray = TraySpec::new("a".repeat(127)).with_menu(MenuSpec::new().command("app.one", "One"));
    app.declare_tray(tray.clone())
        .expect("the 127-unit tooltip is the limit");
    assert_eq!(app.tray(), Some(tray));
    assert_eq!(
        app.declare_tray(TraySpec::new("again")),
        Err(TrayError::AlreadyDeclared)
    );
}

#[test]
fn a_tray_menu_has_the_same_65535_item_limit() {
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
    assert_eq!(
        app.declare_tray(TraySpec::new("ZenUI").with_menu(menu_with(65_536))),
        Err(TrayError::MenuTooLarge { items: 65_536 })
    );
}

#[test]
fn a_tray_menu_requires_an_application_handler_for_every_command() {
    let mut app = Runtime::new();
    app.declare_tray(
        TraySpec::new("ZenUI").with_menu(
            MenuSpec::new()
                .command("app.one", "One")
                .command("app.two", "Two"),
        ),
    )
    .expect("declare the tray");
    assert_eq!(app.missing_tray_handler(), Some("app.one"));

    app.on_app_command("app.one", |_| {})
        .expect("register the first handler");
    assert_eq!(app.missing_tray_handler(), Some("app.two"));

    app.on_app_command("app.two", |_| {})
        .expect("register the second handler");
    assert_eq!(app.missing_tray_handler(), None);
}

#[test]
fn a_tray_must_be_declared_before_the_first_window_opens() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        Ok(())
    })
    .expect("declare the window");
    app.show("main").expect("open the window");

    assert_eq!(
        app.declare_tray(TraySpec::new("late")),
        Err(TrayError::RegistrationClosed)
    );
}

#[test]
fn tray_commands_run_the_application_handler_with_the_tray_source() {
    let seen = Rc::new(RefCell::new(Vec::new()));
    let handler_seen = seen.clone();
    let mut app = Runtime::new();
    app.on_app_command("app.one", move |event| {
        handler_seen
            .borrow_mut()
            .push((event.source(), event.window(), event.target()));
    })
    .expect("register the app command");
    app.declare_tray(TraySpec::new("ZenUI").with_menu(MenuSpec::new().command("app.one", "One")))
        .expect("declare the tray");

    app.dispatch_app_command("app.one", CommandSource::Tray, None)
        .expect("dispatch the tray command without a window");
    let seen = seen.borrow();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0], (CommandSource::Tray, None, None));
}
