#![allow(unused_braces)]

use zenui::prelude::*;
use zenui_demo::{CLOSE_CONFIRMATION_WINDOW, open_todo_window, seed};

fn open_seeded() -> (Runtime, WindowCtx) {
    let mut app = Runtime::new();
    let main = open_todo_window(&mut app).expect("open the demo windows");
    seed(&mut app).expect("seed the todo rows");
    (app, main)
}

fn confirmation_window(app: &Runtime) -> Option<WindowCtx> {
    app.windows()
        .into_iter()
        .find(|info| info.spec.key() == CLOSE_CONFIRMATION_WINDOW)
        .map(|info| info.window)
}

fn draft_value(app: &Runtime, window: WindowCtx) -> Option<String> {
    app.presentation_of(window)
        .expect("window snapshot")
        .find_label("What needs to be done?")
        .expect("draft input")
        .text_value()
        .map(str::to_owned)
}

fn draft_input(app: &Runtime, window: WindowCtx) -> NodeId {
    app.presentation_of(window)
        .expect("window snapshot")
        .find_label("What needs to be done?")
        .expect("draft input")
        .id()
}

#[test]
fn closing_with_a_draft_prompts_and_deny_preserves_editing() {
    let (mut app, main) = open_seeded();
    app.input("What needs to be done?", "unfinished draft")
        .expect("type the main draft");
    app.focus("What needs to be done?")
        .expect("focus the draft");
    assert!(app.set_caret("What needs to be done?", 3));

    app.request_close(main).expect("request the close");
    let confirmation = confirmation_window(&app).expect("the close confirmation is live");
    assert!(app.window_alive(main));
    assert!(app.window_visible(confirmation));
    assert!(
        !app.window_accepts_input(main),
        "the pending parent suspends input"
    );
    assert_eq!(app.focused_window(), Some(confirmation));
    assert_eq!(
        app.input("What needs to be done?", "blocked"),
        Err(MountError::Component("target window input is suspended"))
    );
    let prompt = app
        .presentation_of(confirmation)
        .expect("confirmation snapshot");
    assert!(
        prompt
            .elements()
            .any(|element| element.text() == "Continue editing")
    );
    assert!(
        prompt
            .elements()
            .any(|element| element.text() == "Discard draft")
    );

    app.click_label("Continue editing")
        .expect("click Continue editing");
    app.apply_window_requests().expect("settle the denial");
    assert!(!app.window_alive(confirmation));
    assert!(app.window_alive(main));
    assert!(app.window_accepts_input(main));
    assert_eq!(app.focused_window(), Some(main));
    assert_eq!(
        app.presentation_of(main).expect("main snapshot").focus(),
        Some(draft_input(&app, main)),
        "the draft keeps focus"
    );
    assert_eq!(app.caret("What needs to be done?"), Some(3));
    assert_eq!(
        draft_value(&app, main).as_deref(),
        Some("unfinished draft"),
        "the draft survives the denied close"
    );
}

#[test]
fn closing_the_prompt_itself_denies_the_close() {
    let (mut app, main) = open_seeded();
    app.input("What needs to be done?", "keep me")
        .expect("type the main draft");
    let input = draft_input(&app, main);
    app.focus("What needs to be done?")
        .expect("focus the draft");
    assert!(app.set_caret("What needs to be done?", 4));

    app.request_close(main).expect("request the close");
    let confirmation = confirmation_window(&app).expect("the close confirmation is live");
    app.request_close(confirmation)
        .expect("close the confirmation window");
    assert!(!app.window_alive(confirmation));
    assert!(app.window_alive(main));
    assert!(app.window_accepts_input(main));
    assert_eq!(app.focused_window(), Some(main));
    assert_eq!(
        app.presentation_of(main).expect("main snapshot").focus(),
        Some(input)
    );
    assert_eq!(app.caret("What needs to be done?"), Some(4));
    assert_eq!(draft_value(&app, main).as_deref(), Some("keep me"));
}

#[test]
fn discarding_the_draft_closes_the_parent_and_confirmation_group() {
    let (mut app, main) = open_seeded();
    app.input("What needs to be done?", "throw away")
        .expect("type the main draft");
    app.request_close(main).expect("request the close");
    let confirmation = confirmation_window(&app).expect("the close confirmation is live");

    app.click_label("Discard draft")
        .expect("click Discard draft");
    app.apply_window_requests().expect("settle the allow");
    assert!(!app.window_alive(confirmation));
    assert!(!app.window_alive(main));
    assert!(app.windows().is_empty());

    let reopened = app.show("main").expect("reopen the main window");
    assert_eq!(
        draft_value(&app, reopened).as_deref(),
        Some(""),
        "Discard clears the app-owned draft"
    );
}

#[test]
fn an_empty_draft_closes_synchronously_without_a_prompt() {
    let (mut app, main) = open_seeded();
    assert_eq!(draft_value(&app, main).as_deref(), Some(""));

    app.request_close(main).expect("request the close");
    assert!(!app.window_alive(main));
    assert!(confirmation_window(&app).is_none());
    assert!(app.windows().is_empty());
}

#[test]
fn a_whitespace_draft_still_prompts() {
    let (mut app, main) = open_seeded();
    app.input("What needs to be done?", "   ")
        .expect("type only whitespace");

    app.request_close(main).expect("request the close");
    let confirmation = confirmation_window(&app).expect("whitespace is a non-empty draft");
    assert!(app.window_visible(confirmation));
    assert!(app.window_alive(main));
    assert!(!app.window_accepts_input(main));
}

#[test]
fn a_repeated_close_request_reuses_the_pending_confirmation() {
    let (mut app, main) = open_seeded();
    app.input("What needs to be done?", "pending")
        .expect("type the main draft");
    app.request_close(main).expect("request the close");
    let confirmation = confirmation_window(&app).expect("the close confirmation is live");
    let activation = confirmation.activation();

    app.request_close(main)
        .expect("request the close again while pending");
    assert_eq!(
        confirmation_window(&app),
        Some(confirmation),
        "the pending confirmation is reused"
    );
    assert_eq!(confirmation.activation(), activation);
    assert_eq!(app.focused_window(), Some(confirmation));
    assert!(app.window_alive(main));
    assert!(!app.window_accepts_input(main));
}
