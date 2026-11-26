#![allow(unused_braces)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use zenui::prelude::*;

type Log = Rc<RefCell<Vec<&'static str>>>;

fn record(log: &Log, entry: &'static str) {
    log.borrow_mut().push(entry);
}

#[test]
fn keydown_runs_capture_target_and_bubble_in_path_order() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = Runtime::new();
    app.mount({
        let log = log.clone();
        move || {
            let panel_capture = log.clone();
            let panel_target = log.clone();
            let panel_bubble = log.clone();
            let target_capture = log.clone();
            let target_target = log.clone();
            let target_bubble = log.clone();
            view! {
                <view
                    aria:label="panel"
                    on:keydown_capture={move |_event: &mut KeyEvent| {
                        record(&panel_capture, "panel-capture")
                    }}
                    on:keydown={move |_event: &mut KeyEvent| record(&panel_target, "panel-target")}
                    on:keydown_bubble={move |_event: &mut KeyEvent| {
                        record(&panel_bubble, "panel-bubble")
                    }}
                >
                    <button
                        aria:label="target"
                        on:keydown_capture={move |_event: &mut KeyEvent| {
                            record(&target_capture, "target-capture")
                        }}
                        on:keydown={move |_event: &mut KeyEvent| {
                            record(&target_target, "target-target")
                        }}
                        on:keydown_bubble={move |_event: &mut KeyEvent| {
                            record(&target_bubble, "target-bubble")
                        }}
                    >"Target"</button>
                </view>
            }
        }
    })
    .expect("mount");
    let target = app
        .presentation()
        .expect("snapshot")
        .find_label("target")
        .expect("target")
        .id();
    app.focus("target").expect("focus the target");

    let report = app
        .dispatch_intent(Intent::keydown(target, Key::Enter, Modifiers::default()))
        .expect("keydown turn");
    assert!(report.is_ok());
    assert_eq!(
        *log.borrow(),
        [
            "panel-capture",
            "target-capture",
            "target-target",
            "panel-bubble"
        ],
        "capture runs root→hit, target runs once, bubble runs from the hit parent"
    );
}

#[test]
fn enter_and_space_activate_a_focused_button_through_the_default() {
    let clicks: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
    let mut app = Runtime::new();
    app.mount({
        let clicks = clicks.clone();
        move || {
            let enter = clicks.clone();
            let space = clicks.clone();
            view! {
                <view>
                    <button aria:label="enter" on:click={move || record(&enter, "enter")}>
                        "Enter"
                    </button>
                    <button aria:label="space" on:click={move || record(&space, "space")}>
                        "Space"
                    </button>
                </view>
            }
        }
    })
    .expect("mount");
    let snapshot = app.presentation().expect("snapshot");
    let enter = snapshot.find_label("enter").expect("enter").id();
    let space = snapshot.find_label("space").expect("space").id();

    app.focus("enter").expect("focus enter");
    app.dispatch_intent(Intent::keydown(enter, Key::Enter, Modifiers::default()))
        .expect("enter turn");
    app.focus("space").expect("focus space");
    app.dispatch_intent(Intent::keydown(space, Key::Space, Modifiers::default()))
        .expect("space turn");

    assert_eq!(*clicks.borrow(), ["enter", "space"]);
}

#[test]
fn consuming_keydown_stops_the_default_action() {
    let clicks: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
    let mut app = Runtime::new();
    app.mount({
        let clicks = clicks.clone();
        move || {
            let click = clicks.clone();
            let key = clicks.clone();
            view! {
                <view>
                    <button
                        aria:label="button"
                        on:keydown={move |event: &mut KeyEvent| {
                            record(&key, "keydown");
                            event.consume();
                        }}
                        on:click={move || record(&click, "click")}
                    >"Button"</button>
                </view>
            }
        }
    })
    .expect("mount");
    let button = app
        .presentation()
        .expect("snapshot")
        .find_label("button")
        .expect("button")
        .id();
    app.focus("button").expect("focus the button");

    let report = app
        .dispatch_intent(Intent::keydown(button, Key::Enter, Modifiers::default()))
        .expect("keydown turn");
    assert!(report.outcome().consumed());
    assert_eq!(
        *clicks.borrow(),
        ["keydown"],
        "the click default is skipped"
    );
}

#[test]
fn editing_keys_update_the_committed_text_input() {
    #[component]
    fn Draft() -> MountResult<MountPlan> {
        let value = signal(String::from("abc"));
        view! {
            <text-input
                aria:label="draft"
                value={value}
                on:input={move |next: String| value.set(next)}
            />
        }
    }

    let mut app = Runtime::new();
    app.mount(|| view! { <Draft /> }).expect("mount");
    let input = app
        .presentation()
        .expect("snapshot")
        .find_label("draft")
        .expect("input")
        .id();
    app.focus("draft").expect("focus the input");
    app.set_caret_node(input, 3).expect("caret at the end");

    app.dispatch_intent(Intent::keydown(input, Key::Backspace, Modifiers::default()))
        .expect("backspace turn");
    assert_eq!(
        app.presentation()
            .expect("snapshot")
            .find_label("draft")
            .expect("input")
            .text_value(),
        Some("ab"),
        "the runtime applies the editing default and emits one input"
    );

    app.dispatch_intent(Intent::keydown(input, Key::Home, Modifiers::default()))
        .expect("home turn");
    app.dispatch_intent(Intent::keydown(input, Key::Delete, Modifiers::default()))
        .expect("delete turn");
    assert_eq!(
        app.presentation()
            .expect("snapshot")
            .find_label("draft")
            .expect("input")
            .text_value(),
        Some("b"),
        "Home moves the caret and Delete removes the first character"
    );
}

#[test]
fn tab_traverses_focus_inside_the_window() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| {
            view! {
                <view>
                    <button aria:label="first">"First"</button>
                    <button aria:label="second">"Second"</button>
                </view>
            }
        })
    })
    .expect("declare the window");
    let window = app.show("main").expect("show the window");
    app.focus("first").expect("focus first");
    let first = app
        .presentation_of(window)
        .expect("snapshot")
        .find_label("first")
        .expect("first")
        .id();

    app.dispatch_intent(Intent::keydown(first, Key::Tab, Modifiers::default()))
        .expect("tab turn");
    let snapshot = app.presentation_of(window).expect("snapshot");
    assert_eq!(
        snapshot
            .focus()
            .and_then(|focus| snapshot.node(focus))
            .and_then(|element| element.label()),
        Some("second"),
        "Tab moves focus to the next control"
    );

    let second = snapshot.find_label("second").expect("second").id();
    app.dispatch_intent(Intent::keydown(
        second,
        Key::Tab,
        Modifiers::new(true, false, false, false),
    ))
    .expect("shift-tab turn");
    let snapshot = app.presentation_of(window).expect("snapshot");
    assert_eq!(
        snapshot
            .focus()
            .and_then(|focus| snapshot.node(focus))
            .and_then(|element| element.label()),
        Some("first"),
        "Shift+Tab moves focus backwards"
    );
}

#[test]
fn window_keydown_is_the_fallback_without_focus() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = Runtime::new();
    app.declare_windows({
        let log = log.clone();
        move |catalog| {
            catalog.window(WindowSpec::new("main").primary(), move |window| {
                let log = log.clone();
                window
                    .on_keydown(move |event: &mut KeyEvent| {
                        record(&log, "window");
                        if event.key() == Key::Escape {
                            event.consume();
                        }
                        HandlerOutcome::Ok
                    })
                    .expect("register the window keydown fallback");
                view! { <view><text>"main"</text></view> }
            })
        }
    })
    .expect("declare the window");
    let window = app.show("main").expect("show the window");

    let report = app
        .dispatch_intent(Intent::keydown_window(
            window,
            Key::Enter,
            Modifiers::default(),
        ))
        .expect("window keydown turn");
    assert!(!report.outcome().consumed());
    assert_eq!(*log.borrow(), ["window"]);

    let report = app
        .dispatch_intent(Intent::keydown_window(
            window,
            Key::Escape,
            Modifiers::default(),
        ))
        .expect("window escape turn");
    assert!(report.outcome().consumed());
    assert_eq!(*log.borrow(), ["window", "window"]);
}

#[test]
fn a_window_keydown_panic_is_a_typed_turn_error_and_the_slot_stays_usable() {
    let calls = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.declare_windows({
        let calls = calls.clone();
        move |catalog| {
            catalog.window(WindowSpec::new("main").primary(), move |window| {
                let calls = calls.clone();
                window
                    .on_keydown(move |_event: &mut KeyEvent| {
                        calls.set(calls.get() + 1);
                        panic!("window keydown boom");
                    })
                    .expect("register the panicking keydown");
                view! { <view><text>"main"</text></view> }
            })
        }
    })
    .expect("declare the window");
    let window = app.show("main").expect("show the window");

    for expected in [1, 2] {
        let report = app
            .dispatch_intent(Intent::keydown_window(
                window,
                Key::Enter,
                Modifiers::default(),
            ))
            .expect("the panic is contained in the turn");
        assert!(matches!(
            report.errors(),
            [TurnError::HandlerPanicked(panic)] if panic.message() == "window keydown boom"
        ));
        assert_eq!(calls.get(), expected, "the handler slot is reusable");
    }
}

#[test]
fn reopening_a_window_requires_fresh_keydown_registration() {
    let calls = Rc::new(Cell::new(0));
    let first = Rc::new(Cell::new(true));
    let mut app = Runtime::new();
    app.declare_windows({
        let calls = calls.clone();
        let first = first.clone();
        move |catalog| {
            let calls = calls.clone();
            let first = first.clone();
            catalog.window(WindowSpec::new("main").primary(), move |window| {
                if first.replace(false) {
                    let calls = calls.clone();
                    window
                        .on_keydown(move |_event: &mut KeyEvent| {
                            calls.set(calls.get() + 1);
                            HandlerOutcome::Ok
                        })
                        .expect("register the first activation's handler");
                }
                view! { <view><text>"main"</text></view> }
            })
        }
    })
    .expect("declare the window");
    let window = app.show("main").expect("show the window");
    app.dispatch_intent(Intent::keydown_window(
        window,
        Key::Enter,
        Modifiers::default(),
    ))
    .expect("first activation keydown");
    assert_eq!(calls.get(), 1);

    app.close_window(window).expect("close the window");
    let reopened = app.show("main").expect("reopen the window");
    app.dispatch_intent(Intent::keydown_window(
        reopened,
        Key::Enter,
        Modifiers::default(),
    ))
    .expect("second activation keydown");
    assert_eq!(
        calls.get(),
        1,
        "closing released the old activation's keydown handler"
    );
}
