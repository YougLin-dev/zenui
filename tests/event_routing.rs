//! Event routing: disabled subtrees, checkbox defaults, turn-change reporting,
//! and duplicate handler rejection.
//!
//! These are the runtime-side halves of #378. The native host click path is
//! covered by `crates/windows/src/host.rs` tests.

#![allow(unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;

#[component]
fn Guarded(clicks: Rc<Cell<u32>>, changes: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let locked = signal(false);
    let group_disabled = memo(move || locked.get());
    let delete_clicks = clicks.clone();
    let check_changes = changes.clone();
    view! {
        <view>
            <view aria:label="lock" on:click={move || locked.set(!locked.get())}>"Lock"</view>
            <button aria:label="before" on:click={move || {}}>"Before"</button>
            <view aria:label="group" disabled={group_disabled}>
                <button
                    aria:label="delete"
                    on:click={move || delete_clicks.set(delete_clicks.get() + 1)}
                >"Delete"</button>
                <checkbox
                    aria:label="check"
                    on:change={move |_next: bool| check_changes.set(check_changes.get() + 1)}
                />
            </view>
            <button aria:label="after" on:click={move || {}}>"After"</button>
        </view>
    }
}

fn guarded() -> (Runtime, Rc<Cell<u32>>, Rc<Cell<u32>>) {
    let clicks = Rc::new(Cell::new(0));
    let changes = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let clicks = clicks.clone();
        let changes = changes.clone();
        move || view! { <Guarded clicks={clicks} changes={changes} /> }
    })
    .expect("mount the guarded tree");
    (app, clicks, changes)
}

fn node(app: &Runtime, label: &str) -> NodeId {
    app.presentation()
        .expect("snapshot")
        .find_label(label)
        .expect("element by label")
        .id()
}

#[test]
fn a_disabled_ancestor_blocks_click_change_focus_and_tab() {
    let (mut app, clicks, changes) = guarded();
    let delete = node(&app, "delete");
    let check = node(&app, "check");

    app.click_label("lock").expect("lock the group");

    let report = app.activate_node(delete).expect("routed activate");
    assert!(!report.changed(), "a disabled target is a no-op");
    assert_eq!(clicks.get(), 0, "the child button never runs");
    let report = app.click_node(delete).expect("direct click");
    assert!(!report.changed());
    assert_eq!(clicks.get(), 0, "direct click is blocked too");
    let report = app.click_label("delete").expect("label click");
    assert!(!report.changed());
    assert_eq!(clicks.get(), 0, "label dispatch shares the same rule");

    let report = app.change_node(check, true).expect("change on a child");
    assert!(!report.changed());
    assert_eq!(changes.get(), 0, "the child checkbox never changes");
    assert_eq!(app.checked("check"), Some(false));

    assert!(
        app.focus("delete").is_err(),
        "focus cannot enter a disabled subtree"
    );
    assert!(
        app.focus("check").is_err(),
        "Tab/focus candidates skip disabled descendants"
    );

    app.focus("before").expect("focus before the group");
    app.focus_next().expect("tab forward");
    assert_eq!(
        app.focused().and_then(|info| info.label),
        Some(String::from("after")),
        "Tab skips both disabled descendants"
    );
    app.focus_previous().expect("tab backward");
    assert_eq!(
        app.focused().and_then(|info| info.label),
        Some(String::from("before")),
        "Shift+Tab skips them in the other direction"
    );
}

#[component]
fn Checkboxes(changes: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    view! {
        <checkbox
            aria:label="box"
            on:change={move |_next: bool| changes.set(changes.get() + 1)}
        />
    }
}

#[test]
fn enter_and_space_both_toggle_a_focused_checkbox() {
    let changes = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let changes = changes.clone();
        move || view! { <Checkboxes changes={changes} /> }
    })
    .expect("mount");
    let box_id = node(&app, "box");
    app.focus("box").expect("focus the checkbox");

    app.dispatch_intent(Intent::keydown(box_id, Key::Enter, Modifiers::default()))
        .expect("Enter on the checkbox");
    assert_eq!(app.checked("box"), Some(true));
    assert_eq!(changes.get(), 1, "Enter runs on:change once");

    app.dispatch_intent(Intent::keydown(box_id, Key::Space, Modifiers::default()))
        .expect("Space on the checkbox");
    assert_eq!(app.checked("box"), Some(false));
    assert_eq!(changes.get(), 2, "Space runs on:change once");
}

#[component]
fn PanickingChange() -> MountResult<MountPlan> {
    view! {
        <checkbox
            aria:label="boom"
            on:change={move |_next: bool| -> () { panic!("change boom") }}
        />
    }
}

#[test]
fn a_panicking_change_handler_does_not_apply_the_proposed_value() {
    let mut app = Runtime::new();
    app.mount(|| view! { <PanickingChange /> }).expect("mount");
    let box_id = node(&app, "boom");

    let failure = app
        .change_node(box_id, true)
        .expect_err("the handler panic is returned");
    assert!(matches!(failure, DispatchError::Panic(panic) if panic.message() == "change boom"));
    assert_eq!(
        app.checked("boom"),
        Some(false),
        "the proposed toggle is not applied when the handler fails"
    );
}

#[component]
fn RejectingChange() -> MountResult<MountPlan> {
    view! {
        <checkbox
            aria:label="reject"
            on:change={move |_next: bool| {
                HandlerOutcome::Err(HandlerError::new("rejected"))
            }}
        />
    }
}

#[test]
fn a_rejected_change_handler_leaves_the_checkbox_unchecked() {
    let mut app = Runtime::new();
    app.mount(|| view! { <RejectingChange /> }).expect("mount");
    let box_id = node(&app, "reject");

    let report = app.change_node(box_id, true).expect("contained error");
    assert!(matches!(
        report.errors(),
        [TurnError::HandlerFailed(error)] if error.message() == "rejected"
    ));
    assert_eq!(app.checked("reject"), Some(false));
    assert!(!report.changed());
}

#[component]
fn Writer() -> MountResult<MountPlan> {
    let value = signal(String::from("a"));
    let shown = memo(move || value.get());
    view! {
        <view>
            <text>{shown}</text>
            <button aria:label="write" on:click={move || value.set(String::from("b"))}>
                "Write"
            </button>
            <form aria:label="silent" on:submit={move |_event: &mut SubmitEvent| {}}>
                <text-input aria:label="draft" />
            </form>
        </view>
    }
}

#[test]
fn click_and_submit_report_changed_only_when_a_revision_advanced() {
    let mut app = Runtime::new();
    app.mount(|| view! { <Writer /> }).expect("mount");

    let report = app.click_label("write").expect("write the value");
    assert!(report.changed(), "a handler write advances the revision");
    let report = app.click_label("write").expect("write the same value");
    assert!(!report.changed(), "an equal write does not advance it");

    let silent = node(&app, "silent");
    let report = app.submit_node(silent).expect("silent submit");
    assert!(
        !report.changed(),
        "a submit handler with no writes is not a presentation change"
    );
}

#[test]
fn tab_with_no_other_focusable_control_does_not_report_a_change() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| {
            view! { <button aria:label="only">"Only"</button> }
        })?;
        Ok(())
    })
    .expect("declare the window");
    let window = app.show("main").expect("show the window");
    let only = node(&app, "only");
    app.focus("only").expect("focus the only control");

    let report = app
        .dispatch_intent(Intent::keydown_window(
            window,
            Key::Tab,
            Modifiers::default(),
        ))
        .expect("tab turn");
    assert!(
        !report.changed(),
        "focus staying on the same NodeId is not a presentation change"
    );
    assert_eq!(
        app.presentation_of(window).expect("snapshot").focus(),
        Some(only)
    );
}

#[test]
fn programmatic_duplicate_event_handlers_are_rejected_at_mount() {
    let plan = zenui::view::element(
        "button",
        vec![zenui::view::on_click(|| {}), zenui::view::on_click(|| {})],
        Vec::new(),
    );
    let mut app = Runtime::new();
    let failure = app
        .mount(move || Ok(MountPlan::new(vec![plan])))
        .expect_err("the duplicate handler is rejected");
    assert_eq!(failure, MountError::DuplicateHandler("click"));
}
