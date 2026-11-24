#![allow(unused_braces)]

use zenui::prelude::*;

#[test]
fn shortcut_text_is_parsed_and_normalized() {
    let shortcut = MenuShortcut::parse("shift+ctrl+s").expect("a valid shortcut");
    assert_eq!(shortcut.to_string(), "Ctrl+Shift+S");
    assert_eq!(shortcut.key(), ShortcutKey::Character('S'));
    assert!(shortcut.control());
    assert!(!shortcut.alt());
    assert!(shortcut.shift());

    let function = MenuShortcut::parse("alt+f12").expect("a function key shortcut");
    assert_eq!(function.to_string(), "Alt+F12");
    assert_eq!(function.key(), ShortcutKey::Function(12));
    assert!(!function.control());
    assert!(function.alt());

    let digit = MenuShortcut::parse("control+1").expect("a digit shortcut");
    assert_eq!(digit.to_string(), "Ctrl+1");
    assert_eq!(digit.key(), ShortcutKey::Character('1'));
}

#[test]
fn invalid_shortcut_text_reports_the_declaration() {
    for text in [
        "",
        "S",
        "Shift+S",
        "Ctrl+",
        "Ctrl+Escape",
        "Ctrl+Enter",
        "Ctrl+Tab",
        "Ctrl+Win+S",
        "Ctrl+Ctrl+S",
        "Ctrl+F0",
        "Ctrl+F25",
        "Ctrl+!",
    ] {
        let error = MenuShortcut::parse(text).expect_err(text);
        assert_eq!(error.text(), text);
        assert!(!error.reason().is_empty(), "{text}");
    }
}

#[test]
fn a_command_exposes_its_shortcut_binding() {
    let command = MenuCommand::new("file.save", "Save")
        .bind_shortcut("Ctrl+S")
        .expect("bind the shortcut");
    assert_eq!(
        command.shortcut().map(|shortcut| shortcut.to_string()),
        Some(String::from("Ctrl+S"))
    );
    assert_eq!(MenuCommand::new("file.open", "Open").shortcut(), None);
}

#[test]
fn duplicate_normalized_shortcuts_are_a_catalog_error() {
    let mut app = Runtime::new();
    let result = app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        catalog.menu(
            "main",
            MenuSpec::new()
                .item(MenuCommand::new("file.save", "Save").bind_shortcut("Ctrl+S")?)
                .item(MenuCommand::new("file.save-as", "Save as").bind_shortcut("ctrl+s")?),
        )?;
        Ok(())
    });
    match result {
        Err(CatalogError::DuplicateMenuShortcut {
            window,
            id,
            shortcut,
        }) => {
            assert_eq!(window, "main");
            assert_eq!(id, "file.save-as");
            assert_eq!(shortcut.to_string(), "Ctrl+S");
        }
        other => panic!("expected a duplicate shortcut error, got {other:?}"),
    }
}

#[test]
fn duplicate_shortcuts_across_submenus_are_rejected() {
    let mut app = Runtime::new();
    let result = app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        catalog.menu(
            "main",
            MenuSpec::new()
                .item(MenuCommand::new("file.save", "Save").bind_shortcut("Ctrl+S")?)
                .submenu(
                    "Edit",
                    MenuSpec::new().item(
                        MenuCommand::new("edit.save-copy", "Save copy").bind_shortcut("ctrl+s")?,
                    ),
                ),
        )?;
        Ok(())
    });
    assert!(matches!(
        result,
        Err(CatalogError::DuplicateMenuShortcut {
            window: "main",
            id: "edit.save-copy",
            ..
        })
    ));
}

#[test]
fn invalid_shortcut_text_becomes_a_catalog_error() {
    let mut app = Runtime::new();
    let result = app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        catalog.menu(
            "main",
            MenuSpec::new().item(MenuCommand::new("file.save", "Save").bind_shortcut("Shift+S")?),
        )?;
        Ok(())
    });
    match result {
        Err(CatalogError::InvalidMenuShortcut(error)) => {
            assert_eq!(error.text(), "Shift+S");
        }
        other => panic!("expected an invalid shortcut error, got {other:?}"),
    }
}

#[test]
fn the_same_shortcut_is_allowed_in_different_windows() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| view! { <view /> })?;
        catalog.window(WindowSpec::new("details"), |_| view! { <view /> })?;
        let menu = MenuSpec::new().item(MenuCommand::new("save", "Save").bind_shortcut("Ctrl+S")?);
        catalog.menu("main", menu.clone())?;
        catalog.menu("details", menu)?;
        Ok(())
    })
    .expect("the shortcut is scoped to one window's accelerator table");
}
