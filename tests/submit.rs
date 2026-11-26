#![allow(unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;

#[component]
fn Form(calls: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let draft = signal(String::new());
    view! {
        <form aria:label="form" on:submit={move |_event: &mut SubmitEvent| {
            calls.set(calls.get() + 1);
        }}>
            <text-input
                aria:label="draft"
                value={draft}
                on:input={move |next: String| draft.set(next)}
            />
            <button aria:label="submit" type="submit">"Save"</button>
            <button aria:label="plain" on:click={move || {}}>"Plain"</button>
        </form>
    }
}

fn form(calls: &Rc<Cell<u32>>) -> Runtime {
    let mut app = Runtime::new();
    app.mount({
        let calls = calls.clone();
        move || view! { <Form calls={calls} /> }
    })
    .expect("mount");
    app
}

fn draft(app: &Runtime) -> NodeId {
    app.presentation()
        .expect("snapshot")
        .find_label("draft")
        .expect("draft")
        .id()
}

#[test]
fn a_submit_button_runs_submit_once_and_a_plain_button_does_not() {
    let calls = Rc::new(Cell::new(0));
    let mut app = form(&calls);

    let report = app.click_label("submit").expect("click the submit button");
    assert!(report.is_ok());
    assert_eq!(calls.get(), 1, "the form submits exactly once");

    app.click_label("plain").expect("click the plain button");
    assert_eq!(
        calls.get(),
        1,
        "a button without type=submit does not submit"
    );
}

#[test]
fn enter_in_a_text_input_submits_once_unless_the_key_is_handled() {
    let calls = Rc::new(Cell::new(0));
    let mut app = form(&calls);
    let input = draft(&app);
    app.focus("draft").expect("focus the draft");
    app.dispatch_intent(Intent::keydown(input, Key::Enter, Modifiers::default()))
        .expect("enter turn");
    assert_eq!(calls.get(), 1, "Enter submits the nearest form once");

    #[component]
    fn Guarded(consume: bool, prevent: bool, calls: Rc<Cell<u32>>) -> MountResult<MountPlan> {
        let draft = signal(String::new());
        view! {
            <form aria:label="form" on:submit={move |_event: &mut SubmitEvent| {
                calls.set(calls.get() + 1);
            }}>
                <text-input
                    aria:label="draft"
                    value={draft}
                    on:input={move |next: String| draft.set(next)}
                    on:keydown={move |event: &mut KeyEvent| {
                        if consume {
                            event.consume();
                        } else if prevent {
                            event.prevent_default();
                        }
                    }}
                />
                <button aria:label="submit" type="submit">"Save"</button>
            </form>
        }
    }

    for (consume, prevent) in [(true, false), (false, true)] {
        let calls = Rc::new(Cell::new(0));
        let mut app = Runtime::new();
        app.mount({
            let calls = calls.clone();
            move || view! { <Guarded consume={consume} prevent={prevent} calls={calls} /> }
        })
        .expect("mount");
        let input = draft(&app);
        app.focus("draft").expect("focus");
        app.dispatch_intent(Intent::keydown(input, Key::Enter, Modifiers::default()))
            .expect("handled enter");
        assert_eq!(
            calls.get(),
            0,
            "consume or prevent_default skips the submit default"
        );
    }
}

#[test]
fn submit_can_consume_and_prevent_default_has_no_extra_effect() {
    #[component]
    fn Consuming(mode: u32, calls: Rc<Cell<u32>>) -> MountResult<MountPlan> {
        view! {
            <form aria:label="form" on:submit={move |event: &mut SubmitEvent| {
                calls.set(calls.get() + 1);
                if mode == 0 {
                    event.consume();
                } else {
                    event.prevent_default();
                }
            }}>
                <button aria:label="submit" type="submit">"Save"</button>
            </form>
        }
    }

    let calls = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let calls = calls.clone();
        move || view! { <Consuming mode={0_u32} calls={calls} /> }
    })
    .expect("mount");
    let report = app.click_label("submit").expect("submit");
    assert!(report.outcome().consumed(), "submit can consume the turn");
    assert_eq!(calls.get(), 1, "the handler still runs exactly once");

    let calls = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let calls = calls.clone();
        move || view! { <Consuming mode={1_u32} calls={calls} /> }
    })
    .expect("mount");
    let report = app.click_label("submit").expect("submit");
    assert!(report.outcome().prevent_default());
    assert_eq!(
        calls.get(),
        1,
        "prevent_default is a documented no-op for submit"
    );
}

#[test]
fn the_headless_add_flow_submits_the_demo_form() {
    let mut app = Runtime::new();
    zenui_demo::open_todo_window(&mut app).expect("open the todo window");
    app.input("What needs to be done?", "Via submit")
        .expect("fill the draft");
    app.click_text("Add").expect("activate the submit button");

    assert_eq!(app.for_items().len(), 1, "the Add button submits the form");
    assert!(app.texts().iter().any(|text| text == "Via submit"));
}
