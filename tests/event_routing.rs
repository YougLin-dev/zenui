//! Event routing: disabled subtrees, checkbox defaults, turn-change reporting,
//! and duplicate handler rejection.
//!
//! These are the runtime-side halves of #378. The native host click path is
//! covered by `crates/windows/src/host.rs` tests.

#![allow(unused_braces)]

use std::cell::{Cell, RefCell};
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

    let report = app
        .change_node(box_id, true)
        .expect("the handler panic is contained in the report");
    assert!(matches!(
        report.errors(),
        [TurnError::HandlerPanicked(panic)] if panic.message() == "change boom"
    ));
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
        vec![zenui::view::on_click0(|| {}), zenui::view::on_click0(|| {})],
        Vec::new(),
    );
    let mut app = Runtime::new();
    let failure = app
        .mount(move || Ok(MountPlan::new(vec![plan])))
        .expect_err("the duplicate handler is rejected");
    assert_eq!(failure, MountError::DuplicateHandler("click"));
}

type Log = Rc<RefCell<Vec<&'static str>>>;

fn record(log: &Log, entry: &'static str) {
    log.borrow_mut().push(entry);
}

#[test]
fn click_capture_and_bubble_use_the_fixed_anchor_order() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let mut app = Runtime::new();
    app.mount({
        let log = log.clone();
        move || {
            let root_capture = log.clone();
            let root_bubble = log.clone();
            let mid_capture = log.clone();
            let mid_bubble = log.clone();
            let target = log.clone();
            view! {
                <view
                    aria:label="root"
                    on:click_capture={move |_event: &mut ClickEvent| {
                        record(&root_capture, "root-capture")
                    }}
                    on:click_bubble={move |_event: &mut ClickEvent| {
                        record(&root_bubble, "root-bubble")
                    }}
                >
                    <view
                        aria:label="mid"
                        on:click_capture={move |_event: &mut ClickEvent| {
                            record(&mid_capture, "mid-capture")
                        }}
                        on:click_bubble={move |_event: &mut ClickEvent| {
                            record(&mid_bubble, "mid-bubble")
                        }}
                    >
                        <button
                            aria:label="target"
                            on:click={move |_event: &mut ClickEvent| {
                                record(&target, "target")
                            }}
                        >"Go"</button>
                    </view>
                </view>
            }
        }
    })
    .expect("mount");

    app.click_label("target").expect("the routed click");
    assert_eq!(
        *log.borrow(),
        [
            "root-capture",
            "mid-capture",
            "target",
            "mid-bubble",
            "root-bubble"
        ],
        "capture runs root→hit, then the closest handler, then bubble hit-parent→root"
    );
}

#[test]
fn a_double_click_runs_the_nearest_dblclick_handler_after_the_click_handler() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let counts = Rc::new(Cell::new(0_u8));
    let mut app = Runtime::new();
    app.mount({
        let log = log.clone();
        let counts = counts.clone();
        move || {
            let click = log.clone();
            let dbl = log.clone();
            let counts = counts.clone();
            view! {
                <view aria:label="row">
                    <button
                        aria:label="twice"
                        on:click={move |event: &mut ClickEvent| {
                            counts.set(event.click_count());
                            record(&click, "click");
                        }}
                        on:dblclick={move |event: &mut ClickEvent| {
                            assert_eq!(event.click_count(), 2);
                            record(&dbl, "dbl");
                        }}
                    >"Twice"</button>
                </view>
            }
        }
    })
    .expect("mount");
    let node = node(&app, "twice");
    app.dispatch_intent(Intent::click_detailed(
        node,
        None,
        None,
        2,
        Modifiers::default(),
    ))
    .expect("the double-click intent");
    assert_eq!(counts.get(), 2, "the click payload carries the count");
    assert_eq!(*log.borrow(), ["click", "dbl"]);
}

#[test]
fn a_click_handler_can_prevent_the_submit_default() {
    #[component]
    fn BlockedSubmit(submits: Rc<Cell<u32>>) -> MountResult<MountPlan> {
        view! {
            <form aria:label="form" on:submit={move |_event: &mut SubmitEvent| {
                submits.set(submits.get() + 1);
            }}>
                <button
                    aria:label="save"
                    type="submit"
                    on:click={move |event: &mut ClickEvent| {
                        event.prevent_default();
                    }}
                >"Save"</button>
            </form>
        }
    }

    let submits = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let submits = submits.clone();
        move || view! { <BlockedSubmit submits={submits} /> }
    })
    .expect("mount");
    let report = app.click_label("save").expect("the click turn");
    assert!(report.outcome().prevent_default());
    assert_eq!(submits.get(), 0, "prevent_default skips the submit default");
}

#[test]
fn label_clicks_route_to_the_closest_handler_while_node_clicks_run_only_the_target() {
    #[component]
    fn Row(clicks: Rc<Cell<u32>>) -> MountResult<MountPlan> {
        let clicks = clicks.clone();
        view! {
            <view
                aria:label="row"
                on:click={move |_event: &mut ClickEvent| clicks.set(clicks.get() + 1)}
            >
                <text aria:label="caption">"Row text"</text>
            </view>
        }
    }

    let clicks = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let clicks = clicks.clone();
        move || view! { <Row clicks={clicks} /> }
    })
    .expect("mount");

    app.click_text("Row text").expect("the routed click");
    assert_eq!(clicks.get(), 1, "click_text reaches the closest handler");

    let caption = node(&app, "caption");
    assert_eq!(
        app.click_node(caption),
        Err(DispatchError::WrongKind {
            node: caption,
            operation: NodeOperation::Click,
        }),
        "click_node is a direct primitive, not a routing helper"
    );
    assert_eq!(
        clicks.get(),
        1,
        "click_node does not fall back to ancestors"
    );
}

#[test]
fn click_and_change_sources_distinguish_pointer_keyboard_and_uia() {
    #[component]
    fn Sources(seen: Rc<RefCell<Vec<EventSource>>>) -> MountResult<MountPlan> {
        let clicked = seen.clone();
        let changed = seen;
        view! {
            <view>
                <button
                    aria:label="source"
                    on:click={move |event: &mut ClickEvent| {
                        clicked.borrow_mut().push(event.source());
                    }}
                >"Source"</button>
                <checkbox
                    aria:label="check"
                    on:change={move |event: &mut ChangeEvent| {
                        changed.borrow_mut().push(event.source());
                    }}
                />
            </view>
        }
    }

    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut app = Runtime::new();
    app.mount({
        let seen = seen.clone();
        move || view! { <Sources seen={seen} /> }
    })
    .expect("mount");
    let source = node(&app, "source");
    let check = node(&app, "check");

    app.click_label("source").expect("a pointer click");
    app.focus("source").expect("focus the button");
    app.dispatch_intent(Intent::keydown(source, Key::Enter, Modifiers::default()))
        .expect("a keyboard activation");
    app.click_node(source).expect("a direct/UIA click");
    app.focus("check").expect("focus the checkbox");
    app.dispatch_intent(Intent::keydown(check, Key::Enter, Modifiers::default()))
        .expect("a keyboard toggle");

    assert_eq!(
        *seen.borrow(),
        [
            EventSource::Pointer,
            EventSource::Keyboard,
            EventSource::Accessibility,
            EventSource::Keyboard
        ]
    );
}
