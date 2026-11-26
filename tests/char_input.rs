#![allow(unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;

fn input_value(app: &Runtime, label: &str) -> String {
    app.presentation()
        .expect("snapshot")
        .find_label(label)
        .expect("the input")
        .text_value()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn printable_text_runs_one_input_and_reports_the_change() {
    #[component]
    fn Probe(calls: Rc<Cell<u32>>) -> MountResult<MountPlan> {
        let value = signal(String::from("ab"));
        let count = calls.clone();
        view! {
            <text-input
                aria:label="probe"
                value={value}
                on:input={move |next: String| {
                    count.set(count.get() + 1);
                    value.set(next);
                }}
            />
        }
    }

    let calls = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let calls = calls.clone();
        move || view! { <Probe calls={calls} /> }
    })
    .expect("mount");
    let input = app
        .presentation()
        .expect("snapshot")
        .find_label("probe")
        .expect("input")
        .id();
    app.focus("probe").expect("focus");
    app.set_caret_node(input, 2).expect("caret");

    let report = app.type_text_node(input, "X").expect("type text");
    assert!(report.changed());
    assert!(report.is_ok());
    assert_eq!(calls.get(), 1, "one insertion runs on:input exactly once");
    assert_eq!(input_value(&app, "probe"), "abX");
}

#[test]
fn a_handler_error_is_reported_and_keeps_the_committed_edit() {
    #[component]
    fn Probe() -> MountResult<MountPlan> {
        let value = signal(String::from("ab"));
        view! {
            <text-input
                aria:label="error"
                value={value}
                on:input={move |_next: String| -> Result<(), HandlerError> {
                    Err(HandlerError::new("input refused"))
                }}
            />
        }
    }

    let mut app = Runtime::new();
    app.mount(|| view! { <Probe /> }).expect("mount");
    let input = app
        .presentation()
        .expect("snapshot")
        .find_label("error")
        .expect("input")
        .id();
    app.focus("error").expect("focus");
    app.set_caret_node(input, 2).expect("caret");

    let report = app.type_text_node(input, "X").expect("type text");
    assert!(matches!(
        report.errors(),
        [TurnError::HandlerFailed(error)] if error.message() == "input refused"
    ));
    assert_eq!(
        input_value(&app, "error"),
        "abX",
        "the editor keeps the committed edit after the handler error"
    );
}

#[test]
fn a_handler_panic_is_reported_and_the_editor_stays_usable() {
    #[component]
    fn Probe(first: Rc<Cell<bool>>) -> MountResult<MountPlan> {
        let value = signal(String::from("ab"));
        view! {
            <text-input
                aria:label="panic"
                value={value}
                on:input={move |next: String| {
                    if first.replace(false) {
                        panic!("input boom");
                    }
                    value.set(next);
                }}
            />
        }
    }

    let first = Rc::new(Cell::new(true));
    let mut app = Runtime::new();
    app.mount({
        let first = first.clone();
        move || view! { <Probe first={first} /> }
    })
    .expect("mount");
    let input = app
        .presentation()
        .expect("snapshot")
        .find_label("panic")
        .expect("input")
        .id();
    app.focus("panic").expect("focus");
    app.set_caret_node(input, 2).expect("caret");

    let report = app
        .type_text_node(input, "X")
        .expect("the panic is contained");
    assert!(matches!(
        report.errors(),
        [TurnError::HandlerPanicked(panic)] if panic.message() == "input boom"
    ));
    assert!(!report.changed());
    assert_eq!(
        input_value(&app, "panic"),
        "ab",
        "the aborted local edit leaves the editor at the committed value"
    );

    let report = app
        .type_text_node(input, "Y")
        .expect("the slot is reusable");
    assert!(report.is_ok());
    assert!(report.changed());
    assert_eq!(input_value(&app, "panic"), "abY");
}
