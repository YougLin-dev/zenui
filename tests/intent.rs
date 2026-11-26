#![allow(unused_braces)]

use std::cell::RefCell;
use std::rc::Rc;

use zenui::prelude::*;

type Log = Rc<RefCell<Vec<&'static str>>>;

fn record(log: &Log, entry: &'static str) {
    log.borrow_mut().push(entry);
}

#[test]
fn closest_wins_runs_one_normal_handler_and_stops_at_silent_controls() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = Runtime::new();
    app.mount({
        let log = log.clone();
        move || {
            let row = log.clone();
            let inner = log.clone();
            view! {
                <view aria:label="row" on:click={move || record(&row, "row")}>
                    <button aria:label="inner" on:click={move || record(&inner, "inner")}>
                        "Inner"
                    </button>
                    <button aria:label="silent">"Silent"</button>
                    <text>"Row text"</text>
                </view>
            }
        }
    })
    .expect("mount");

    app.click_label("inner")
        .expect("the inner button click turn");
    assert_eq!(*log.borrow(), ["inner"], "the nested button wins alone");

    log.borrow_mut().clear();
    app.click_text("Row text").expect("the text click turn");
    assert_eq!(*log.borrow(), ["row"], "plain content routes to the row");

    log.borrow_mut().clear();
    let report = app
        .click_label("silent")
        .expect("a click on a silent interactive control is a no-op turn");
    assert!(report.is_ok());
    assert!(
        log.borrow().is_empty(),
        "an interactive control without a handler stops closest-wins"
    );
}

#[test]
fn a_handler_error_is_reported_and_stops_the_turn() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = Runtime::new();
    app.mount({
        let log = log.clone();
        move || {
            let row = log.clone();
            view! {
                <view aria:label="row" on:click={move || record(&row, "row")}>
                    <button
                        aria:label="fail"
                        on:click={move || -> Result<(), HandlerError> {
                            Err(HandlerError::new("nope"))
                        }}
                    >"Fail"</button>
                </view>
            }
        }
    })
    .expect("mount");

    let report = app
        .click_label("fail")
        .expect("the handler error is contained in the turn");
    assert!(matches!(
        report.errors(),
        [TurnError::HandlerFailed(error)] if error.message() == "nope"
    ));
    assert!(
        log.borrow().is_empty(),
        "the row handler does not run after the closest handler fails"
    );
}

#[test]
fn direct_click_keeps_its_own_target_semantics() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = Runtime::new();
    app.mount({
        let log = log.clone();
        move || {
            let row = log.clone();
            view! {
                <view aria:label="row" on:click={move || record(&row, "row")}>
                    <view aria:label="plain"><text>"Plain"</text></view>
                </view>
            }
        }
    })
    .expect("mount");

    let plain = app
        .presentation()
        .expect("snapshot")
        .find_label("plain")
        .expect("plain")
        .id();
    let error = app
        .click_node(plain)
        .expect_err("the direct primitive never routes to an ancestor");
    assert_eq!(
        error,
        DispatchError::WrongKind {
            node: plain,
            operation: NodeOperation::Click
        }
    );
    assert!(log.borrow().is_empty());
}

#[test]
fn window_and_app_intents_validate_the_activation() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| {
            view! { <text>"main"</text> }
        })
    })
    .expect("declare the window");
    let window = app.show("main").expect("show the window");

    let report = app
        .dispatch_intent(Intent::activate_window(window))
        .expect("the live activation is accepted");
    assert!(report.is_ok());
    assert!(
        app.dispatch_intent(Intent::activate_app())
            .expect("the app target is accepted")
            .is_ok()
    );

    app.close_window(window).expect("close the window");
    assert_eq!(
        app.dispatch_intent(Intent::activate_window(window)),
        Err(DispatchError::TargetClosed),
        "a closed activation is rejected"
    );

    let reopened = app.show("main").expect("reopen the window");
    assert_ne!(window, reopened, "reopening allocates a new activation");
    assert_eq!(
        app.dispatch_intent(Intent::activate_window(window)),
        Err(DispatchError::TargetClosed),
        "the superseded activation stays rejected"
    );
    app.dispatch_intent(Intent::activate_window(reopened))
        .expect("the replacement activation is accepted");
}
