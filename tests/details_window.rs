#![allow(unused_braces)]

use zenui::prelude::*;
use zenui_demo::{DETAILS_WINDOW, open_todo_window, seed};

fn open_seeded() -> (Runtime, WindowCtx) {
    let mut app = Runtime::new();
    let main = open_todo_window(&mut app).expect("open the demo windows");
    seed(&mut app).expect("seed the todo rows");
    (app, main)
}

fn details_window(app: &Runtime) -> Option<WindowCtx> {
    app.windows()
        .into_iter()
        .find(|info| info.spec.key() == DETAILS_WINDOW)
        .map(|info| info.window)
}

fn open_details(app: &mut Runtime) -> WindowCtx {
    app.click_label("Open details")
        .expect("click the Details action");
    app.apply_window_requests()
        .expect("settle the open request");
    details_window(app).expect("the details window is live")
}

fn label_text(app: &Runtime, window: WindowCtx, label: &str) -> String {
    app.presentation_of(window)
        .expect("window snapshot")
        .find_label(label)
        .expect("labelled element")
        .text()
}

fn has_text(app: &Runtime, window: WindowCtx, text: &str) -> bool {
    app.presentation_of(window)
        .expect("window snapshot")
        .elements()
        .any(|element| element.text() == text)
}

fn draft_value(app: &Runtime, window: WindowCtx) -> Option<String> {
    app.presentation_of(window)
        .expect("window snapshot")
        .find_label("What needs to be done?")
        .expect("draft input")
        .text_value()
        .map(str::to_owned)
}

#[test]
fn the_toolbar_opens_or_focuses_the_details_window() {
    let (mut app, main) = open_seeded();
    assert_eq!(app.windows().len(), 1);
    assert!(details_window(&app).is_none());

    let details = open_details(&mut app);
    assert_eq!(app.windows().len(), 2);
    assert!(app.window_alive(main));
    assert_eq!(app.focused_window(), Some(details));
    assert_eq!(label_text(&app, details, "Details filter"), "All");
    assert!(has_text(&app, details, "1 remaining"));
    assert!(has_text(&app, details, "Write tests"));
    assert!(has_text(&app, details, "Read docs"));

    let activation = details.activation();
    app.focus("What needs to be done?")
        .expect("focus the main window's draft");
    assert_eq!(app.focused_window(), Some(main));
    app.click_label("Open details")
        .expect("click the Details action again");
    app.apply_window_requests()
        .expect("settle the focus request");
    assert_eq!(
        details_window(&app),
        Some(details),
        "a visible window is reused"
    );
    assert_eq!(
        app.focused_window(),
        Some(details),
        "the action refocuses the visible details window"
    );
    assert_eq!(details_window(&app).expect("live").activation(), activation);
    assert!(app.window_alive(main));
}

#[test]
fn details_follows_shared_state_and_keeps_window_local_editing() {
    let (mut app, main) = open_seeded();
    let details = open_details(&mut app);

    app.input("What needs to be done?", "unfinished draft")
        .expect("type the main draft");
    app.focus("What needs to be done?")
        .expect("focus the draft");
    assert!(app.set_caret("What needs to be done?", 3));
    let main_input = app
        .presentation_of(main)
        .expect("main snapshot")
        .find_label("What needs to be done?")
        .expect("draft input")
        .id();

    let note = app
        .presentation_of(details)
        .expect("details snapshot")
        .find_label("Details note")
        .expect("note input")
        .id();
    app.focus_node(note).expect("focus the details note");
    app.insert_text_node(note, "abc")
        .expect("type in the details note");
    assert_eq!(app.caret("Details note"), Some(3));
    assert_eq!(
        app.caret("What needs to be done?"),
        Some(3),
        "typing in details does not move the main caret"
    );
    assert_eq!(
        app.presentation_of(main).expect("main snapshot").focus(),
        Some(main_input),
        "the main tree keeps its focus while details is active"
    );
    assert_eq!(app.focused_window(), Some(details));

    app.change("Complete Read docs", true)
        .expect("complete the last active task");
    assert!(has_text(&app, details, "0 remaining"));

    app.click_text("Done")
        .expect("switch the main filter to Done");
    assert_eq!(label_text(&app, details, "Details filter"), "Done");
    assert!(has_text(&app, details, "Write tests"));
    assert!(has_text(&app, details, "Read docs"));

    app.click_text("Active")
        .expect("switch the main filter to Active");
    assert_eq!(label_text(&app, details, "Details filter"), "Active");
    assert!(has_text(&app, details, "Nothing matches"));
    assert!(!has_text(&app, details, "Write tests"));

    assert_eq!(
        draft_value(&app, main).as_deref(),
        Some("unfinished draft"),
        "the main draft survives details typing and shared updates"
    );
    assert_eq!(app.caret("What needs to be done?"), Some(3));
    assert_eq!(
        app.presentation_of(main).expect("main snapshot").focus(),
        Some(main_input)
    );
}

#[test]
fn details_hides_closes_and_reopens_without_touching_the_main_window() {
    let (mut app, main) = open_seeded();
    app.input("What needs to be done?", "draft kept")
        .expect("type the main draft");
    app.focus("What needs to be done?")
        .expect("focus the draft");
    assert!(app.set_caret("What needs to be done?", 4));
    app.click_text("Load sample")
        .expect("commit app-owned rows");
    app.settle_tasks().expect("settle the sample task");
    assert!(has_text(&app, main, "Imported A"));
    let main_input = app
        .presentation_of(main)
        .expect("main snapshot")
        .find_label("What needs to be done?")
        .expect("draft input")
        .id();

    let details = open_details(&mut app);
    let note = app
        .presentation_of(details)
        .expect("details snapshot")
        .find_label("Details note")
        .expect("note input")
        .id();
    app.focus_node(note).expect("focus the details note");
    app.insert_text_node(note, "note")
        .expect("type in the details note");

    app.click_label("Hide details").expect("click Hide");
    app.apply_window_requests()
        .expect("settle the hide request");
    assert!(app.window_alive(details));
    assert!(!app.window_visible(details));
    assert_eq!(
        app.focused_window(),
        Some(main),
        "hiding details returns the active-window role to the main window"
    );
    assert_eq!(
        app.presentation_of(main).expect("main snapshot").focus(),
        Some(main_input)
    );
    assert_eq!(app.caret("What needs to be done?"), Some(4));
    assert_eq!(draft_value(&app, main).as_deref(), Some("draft kept"));

    app.click_label("Open details")
        .expect("show the hidden details window");
    app.apply_window_requests()
        .expect("settle the show request");
    assert_eq!(
        details_window(&app),
        Some(details),
        "hide keeps the activation"
    );
    assert!(app.window_visible(details));
    assert_eq!(
        app.presentation_of(details)
            .expect("details snapshot")
            .find_label("Details note")
            .expect("note input")
            .text_value(),
        Some("note"),
        "hiding preserves the details local edit state"
    );
    assert_eq!(app.caret("Details note"), Some(4));

    app.click_label("Close details").expect("click Close");
    app.apply_window_requests()
        .expect("settle the close request");
    assert!(!app.window_alive(details));
    assert!(app.window_alive(main));
    assert_eq!(app.windows().len(), 1);
    assert_eq!(app.focused_window(), Some(main));
    assert_eq!(
        app.presentation_of(main).expect("main snapshot").focus(),
        Some(main_input)
    );
    assert_eq!(app.caret("What needs to be done?"), Some(4));
    assert_eq!(draft_value(&app, main).as_deref(), Some("draft kept"));
    assert!(has_text(&app, main, "Imported A"));

    let reopened = open_details(&mut app);
    assert_ne!(reopened, details);
    assert!(reopened.activation() > details.activation());
    assert_eq!(
        app.presentation_of(reopened)
            .expect("reopened snapshot")
            .find_label("Details note")
            .expect("note input")
            .text_value(),
        Some(""),
        "reopening runs a fresh factory"
    );
    assert_eq!(
        app.presentation_of(main).expect("main snapshot").focus(),
        Some(main_input)
    );
    assert_eq!(app.caret("What needs to be done?"), Some(4));
    assert_eq!(draft_value(&app, main).as_deref(), Some("draft kept"));
    assert!(has_text(&app, main, "Imported A"));
}
