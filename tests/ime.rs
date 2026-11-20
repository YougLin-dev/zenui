#![allow(unused_braces)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use zenui::prelude::*;

#[component]
fn Editor(events: Rc<RefCell<Vec<CompositionEvent>>>) -> MountResult<MountPlan> {
    let draft = signal(String::new());
    let shown = memo(move || draft.get());
    let start_log = events.clone();
    let update_log = events.clone();
    let end_log = events.clone();
    let cancel_log = events;

    view! {
        <view>
            <text>{shown}</text>
            <text-input
                value={draft}
                aria:label="editor"
                on:input={move |value: String| draft.set(value)}
                on:compositionstart={move |event: CompositionEvent| {
                    start_log.borrow_mut().push(event);
                }}
                on:compositionupdate={move |event: CompositionEvent| {
                    update_log.borrow_mut().push(event);
                }}
                on:compositionend={move |event: CompositionEvent| {
                    end_log.borrow_mut().push(event);
                }}
                on:compositioncancel={move |event: CompositionEvent| {
                    cancel_log.borrow_mut().push(event);
                }}
            />
        </view>
    }
}

fn editor() -> (Runtime, Rc<RefCell<Vec<CompositionEvent>>>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut app = Runtime::new();
    app.mount({
        let events = events.clone();
        move || view! { <Editor events={events} /> }
    })
    .expect("mount");
    (app, events)
}

#[test]
fn commit_inserts_at_the_local_caret_and_cancel_restores_it() {
    let (mut app, events) = editor();
    app.input("editor", "ab").expect("type the committed text");
    assert_eq!(app.texts(), vec![String::from("ab")]);
    assert!(
        app.set_caret("editor", 1),
        "place the caret between a and b"
    );

    app.compose_start("editor").expect("start");
    app.compose_update("editor", "ni").expect("preedit");
    assert_eq!(app.preedit("editor"), Some(String::from("ni")));
    assert_eq!(
        app.caret("editor"),
        Some(3),
        "the caret follows the preedit end"
    );
    assert_eq!(
        app.texts(),
        vec![String::from("ab")],
        "the model is untouched while composing"
    );

    app.compose_cancel("editor").expect("cancel");
    assert_eq!(app.preedit("editor"), None);
    assert_eq!(app.caret("editor"), Some(1), "cancel restores the caret");
    assert_eq!(app.texts(), vec![String::from("ab")]);

    {
        let events = events.borrow();
        let cancel = events.last().expect("a cancel event");
        assert_eq!(cancel.phase(), CompositionPhase::Cancel);
        assert_eq!(cancel.text(), "ni", "the discarded preedit is reported");
        assert_eq!(cancel.caret(), 1);
        assert_eq!(cancel.selection(), Some((1, 1)));
    }

    events.borrow_mut().clear();
    app.compose_start("editor").expect("start again");
    app.compose_update("editor", "ni").expect("preedit");
    app.compose_commit("editor", "你")
        .expect("confirm the candidate");

    assert_eq!(
        app.texts(),
        vec![String::from("a你b")],
        "the candidate replaces the collapsed caret range"
    );
    assert_eq!(app.caret("editor"), Some(2));
    assert_eq!(app.preedit("editor"), None);

    let events = events.borrow();
    let end = events.last().expect("an end event");
    assert_eq!(end.phase(), CompositionPhase::End);
    assert_eq!(end.text(), "你");
    assert_eq!(end.caret(), 2);
    assert_eq!(end.selection(), None);
}

#[test]
fn composition_requires_a_text_input_and_ordered_phases() {
    let (mut app, _events) = editor();

    assert_eq!(
        app.compose_start("missing").expect_err("unknown target"),
        MountError::Component("composition target not found")
    );
    assert_eq!(
        app.compose_update("editor", "x")
            .expect_err("updates need a started composition"),
        MountError::Component("text input is not composing")
    );
    assert_eq!(
        app.compose_cancel("editor")
            .expect_err("cancels need a started composition"),
        MountError::Component("text input is not composing")
    );

    app.compose_start("editor").expect("start");
    assert_eq!(
        app.compose_start("editor")
            .expect_err("a second start is rejected"),
        MountError::Component("text input is already composing")
    );
    assert!(
        !app.set_caret("editor", 1),
        "the caret is owned by the composition"
    );
    assert_eq!(
        app.input("editor", "plain").expect_err("input is rejected"),
        MountError::Component("text input is composing")
    );
    assert_eq!(app.preedit("editor"), Some(String::new()));

    app.compose_cancel("editor").expect("cancel");
    app.input("editor", "plain")
        .expect("input works after the composition ends");

    #[component]
    fn Plain() -> MountResult<MountPlan> {
        view! {
            <view aria:label="plain">
                <text>"plain"</text>
            </view>
        }
    }

    let mut plain = Runtime::new();
    plain.mount(|| view! { <Plain /> }).expect("mount");
    assert_eq!(
        plain
            .compose_start("plain")
            .expect_err("a plain view is not an editor"),
        MountError::Component("composition target is not a text input")
    );
}

#[component]
fn KeyEditor(keydowns: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let draft = signal(String::new());
    let shown = memo(move || draft.get());
    let handler_count = keydowns.clone();

    view! {
        <view>
            <text>{shown}</text>
            <text-input
                value={draft}
                aria:label="editor"
                on:input={move |value: String| draft.set(value)}
                on:keydown={move |_event: KeyEvent| {
                    handler_count.set(handler_count.get() + 1);
                }}
            />
        </view>
    }
}

#[test]
fn candidate_enter_and_escape_are_consumed_before_the_key_handler() {
    let keydowns = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let keydowns = keydowns.clone();
        move || view! { <KeyEditor keydowns={keydowns} /> }
    })
    .expect("mount");

    app.compose_start("editor").expect("start");
    app.compose_update("editor", "nihao").expect("preedit");
    app.keydown("editor", Key::Enter)
        .expect("candidate confirmation");

    assert_eq!(
        app.preedit("editor"),
        Some(String::from("nihao")),
        "the candidate still belongs to the IME"
    );
    assert_eq!(
        app.texts(),
        vec![String::new()],
        "candidate Enter does not write the pinyin preedit"
    );
    assert_eq!(
        keydowns.get(),
        0,
        "candidate Enter never reaches the key handler"
    );

    app.compose_commit("editor", "你好")
        .expect("the IME confirms the Chinese candidate");
    assert_eq!(app.texts(), vec![String::from("你好")]);
    assert_eq!(app.preedit("editor"), None);

    app.keydown("editor", Key::Enter)
        .expect("a later plain Enter");
    assert_eq!(
        keydowns.get(),
        1,
        "after the composition ends Enter is a normal key event"
    );

    app.compose_start("editor").expect("start again");
    app.compose_update("editor", "取消").expect("preedit");
    app.keydown("editor", Key::Escape)
        .expect("escape cancels the composition");

    assert_eq!(app.preedit("editor"), None);
    assert_eq!(
        app.texts(),
        vec![String::from("你好")],
        "cancel keeps the committed draft"
    );
    assert_eq!(keydowns.get(), 1, "Escape during composition is consumed");

    app.keydown("editor", Key::Escape)
        .expect("a later plain Escape");
    assert_eq!(keydowns.get(), 2);
}

#[component]
fn ExternalReset() -> MountResult<MountPlan> {
    let draft = signal(String::new());
    let shown = memo(move || draft.get());

    view! {
        <view>
            <text>{shown}</text>
            <text-input
                value={draft}
                aria:label="editor"
                on:input={move |value: String| draft.set(value)}
            />
            <view aria:label="reset" on:click={move || draft.set(String::from("x"))} />
        </view>
    }
}

#[test]
fn an_external_value_change_is_not_clobbered_by_commit() {
    let mut app = Runtime::new();
    app.mount(|| view! { <ExternalReset /> }).expect("mount");
    app.input("editor", "ab").expect("type the committed text");
    assert!(app.set_caret("editor", 2), "place the caret at the end");

    app.compose_start("editor").expect("start");
    app.compose_update("editor", "ni").expect("preedit");
    app.click_label("reset")
        .expect("an external write replaces the value mid-composition");
    assert_eq!(app.texts(), vec![String::from("x")]);

    app.compose_commit("editor", "你")
        .expect("confirm the candidate");
    assert_eq!(
        app.texts(),
        vec![String::from("x你")],
        "the recorded caret is clamped into the current value, not the start snapshot"
    );
    assert_eq!(app.caret("editor"), Some(2));
    assert_eq!(app.preedit("editor"), None);
}

#[test]
fn commit_and_cancel_are_mutually_exclusive_terminal_operations() {
    let (mut app, _events) = editor();

    app.compose_start("editor").expect("start");
    app.compose_update("editor", "ni").expect("preedit");
    app.compose_commit("editor", "你").expect("commit");
    assert_eq!(app.preedit("editor"), None);
    assert_eq!(app.texts(), vec![String::from("你")]);
    assert_eq!(
        app.compose_cancel("editor")
            .expect_err("commit already ended the composition"),
        MountError::Component("text input is not composing")
    );

    app.compose_start("editor").expect("start again");
    app.compose_update("editor", "ni").expect("preedit");
    app.compose_cancel("editor").expect("cancel");
    assert_eq!(app.preedit("editor"), None);
    assert_eq!(
        app.texts(),
        vec![String::from("你")],
        "cancel keeps the previously committed text"
    );
    assert_eq!(
        app.compose_commit("editor", "你")
            .expect_err("cancel already ended the composition"),
        MountError::Component("text input is not composing")
    );
}

#[test]
fn preedit_stays_local_until_commit() {
    let (mut app, events) = editor();
    assert_eq!(app.texts(), vec![String::new()]);

    app.compose_start("editor").expect("start composition");
    assert_eq!(app.preedit("editor"), Some(String::new()));
    assert_eq!(
        app.texts(),
        vec![String::new()],
        "starting a composition does not write the model"
    );

    app.compose_update("editor", "ni").expect("update");
    assert_eq!(app.preedit("editor"), Some(String::from("ni")));
    assert_eq!(app.caret("editor"), Some(2));
    assert_eq!(
        app.texts(),
        vec![String::new()],
        "preedit stays local until the candidate is confirmed"
    );

    app.compose_update("editor", "你好").expect("update again");
    assert_eq!(app.preedit("editor"), Some(String::from("你好")));
    assert_eq!(app.caret("editor"), Some(2));

    app.compose_commit("editor", "你好").expect("commit");
    assert_eq!(app.preedit("editor"), None);
    assert_eq!(
        app.texts(),
        vec![String::from("你好")],
        "commit publishes one input to the model"
    );
    assert_eq!(app.caret("editor"), Some(2));

    let events = events.borrow();
    let phases: Vec<CompositionPhase> = events.iter().map(|event| event.phase()).collect();
    assert_eq!(
        phases,
        vec![
            CompositionPhase::Start,
            CompositionPhase::Update,
            CompositionPhase::Update,
            CompositionPhase::End,
        ]
    );
    assert_eq!(events[0].text(), "");
    assert_eq!(events[1].text(), "ni");
    assert_eq!(events[2].text(), "你好");
    assert_eq!(events[3].text(), "你好");
    assert_eq!(events[3].caret(), 2);
    assert_eq!(events[3].selection(), None);
}
