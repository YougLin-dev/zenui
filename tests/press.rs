#![allow(unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;

fn node_id(app: &Runtime, label: &str) -> NodeId {
    app.presentation()
        .expect("snapshot")
        .find_label(label)
        .expect("the labelled element")
        .id()
}

#[test]
fn press_focuses_the_nearest_focusable_ancestor() {
    #[component]
    fn Tree(focused: Rc<Cell<u32>>) -> MountResult<MountPlan> {
        view! {
            <view>
                <button aria:label="before">"Before"</button>
                <view aria:label="row" on:focus={move |_event: FocusEvent| {
                    focused.set(focused.get() + 1);
                }}>
                    <view aria:label="child"><text>"Child"</text></view>
                </view>
            </view>
        }
    }

    let focused = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let focused = focused.clone();
        move || view! { <Tree focused={focused} /> }
    })
    .expect("mount");
    let before = node_id(&app, "before");
    app.focus("before").expect("focus before");
    let child = node_id(&app, "child");
    let row = node_id(&app, "row");

    let report = app
        .dispatch_intent(Intent::press(child, None))
        .expect("press the child");
    assert!(report.is_ok());
    assert!(report.changed());
    let snapshot = app.presentation().expect("snapshot");
    assert_eq!(
        snapshot.focus(),
        Some(row),
        "the nearest focusable ancestor receives focus"
    );
    assert_eq!(focused.get(), 1, "the focus handler runs once");

    app.dispatch_intent(Intent::press(before, None))
        .expect("press the button");
    assert_eq!(
        app.presentation().expect("snapshot").focus(),
        Some(before),
        "an inherently focusable target receives focus directly"
    );
}

#[test]
fn press_skips_disabled_targets_and_rejects_hidden_ones() {
    #[component]
    fn Tree() -> MountResult<MountPlan> {
        view! {
            <view>
                <button aria:label="live">"Live"</button>
                <button aria:label="blocked" disabled={true}>"Blocked"</button>
            </view>
        }
    }

    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| {
            view! { <Tree /> }
        })
    })
    .expect("declare the window");
    let window = app.show("main").expect("show the window");
    let live = app
        .presentation_of(window)
        .expect("snapshot")
        .find_label("live")
        .expect("live")
        .id();
    let blocked = app
        .presentation_of(window)
        .expect("snapshot")
        .find_label("blocked")
        .expect("blocked")
        .id();
    app.focus_node(live).expect("focus live");

    let report = app
        .dispatch_intent(Intent::press(blocked, None))
        .expect("press the disabled button");
    assert!(!report.changed());
    assert_eq!(
        app.presentation_of(window).expect("snapshot").focus(),
        Some(live),
        "a disabled target leaves focus alone"
    );

    app.hide(window).expect("hide the window");
    assert_eq!(
        app.dispatch_intent(Intent::press(live, None)),
        Err(DispatchError::Hidden(live)),
        "a hidden target cannot take focus"
    );
}

#[test]
fn press_places_the_caret_at_the_host_offset_including_non_bmp_text() {
    #[component]
    fn Editor() -> MountResult<MountPlan> {
        let value = signal(String::from("a🙂bc"));
        view! {
            <text-input
                aria:label="emoji"
                value={value}
                on:input={move |next: String| value.set(next)}
            />
        }
    }

    let mut app = Runtime::new();
    app.mount(|| view! { <Editor /> }).expect("mount");
    let input = node_id(&app, "emoji");

    app.dispatch_intent(Intent::press(input, Some(2)))
        .expect("press after the emoji");
    let snapshot = app.presentation().expect("snapshot");
    let edit = snapshot.node(input).expect("input").edit().expect("edit");
    assert_eq!(snapshot.focus(), Some(input));
    assert_eq!(edit.caret(), 2, "the caret counts scalar values");
    assert_eq!(edit.anchor(), 2);
    assert_eq!(edit.selection(), Some((2, 2)));

    app.dispatch_intent(Intent::press(input, Some(99)))
        .expect("press past the end");
    let snapshot = app.presentation().expect("snapshot");
    let edit = snapshot.node(input).expect("input").edit().expect("edit");
    assert_eq!(edit.caret(), 4, "the host offset is clamped to the value");
}

#[test]
fn controlled_checkboxes_align_to_the_binding_after_one_change() {
    #[component]
    fn Controlled(calls: Rc<Cell<u32>>) -> MountResult<MountPlan> {
        let checked = signal(false);
        let writes = calls.clone();
        view! {
            <view>
                <button
                    aria:label="external"
                    on:click={move || checked.set(true)}
                >"External"</button>
                <checkbox
                    aria:label="controlled"
                    checked={checked}
                    on:change={move |next: bool| {
                        writes.set(writes.get() + 1);
                        checked.set(next);
                    }}
                />
            </view>
        }
    }

    let calls = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let calls = calls.clone();
        move || view! { <Controlled calls={calls} /> }
    })
    .expect("mount");
    let checkbox = node_id(&app, "controlled");

    let report = app.change_node(checkbox, true).expect("user change");
    assert!(report.changed());
    assert_eq!(calls.get(), 1, "one user change runs on:change once");
    assert_eq!(app.checked("controlled"), Some(true));

    app.click_label("external").expect("programmatic write");
    assert_eq!(
        calls.get(),
        1,
        "programmatic binding writes do not dispatch on:change"
    );
    assert_eq!(app.checked("controlled"), Some(true));
}

#[test]
fn a_controlled_checkbox_realigns_when_the_handler_does_not_accept() {
    #[component]
    fn Controlled() -> MountResult<MountPlan> {
        let checked = signal(false);
        view! {
            <checkbox
                aria:label="controlled"
                checked={checked}
                on:change={move |_next: bool| {}}
            />
        }
    }

    let mut app = Runtime::new();
    app.mount(|| view! { <Controlled /> }).expect("mount");
    let checkbox = node_id(&app, "controlled");

    let report = app.change_node(checkbox, true).expect("user change");
    assert!(!report.changed());
    assert_eq!(
        app.checked("controlled"),
        Some(false),
        "the checked binding remains authoritative"
    );
}

#[test]
fn an_uncontrolled_checkbox_toggles_without_a_handler() {
    let mut app = Runtime::new();
    app.mount(|| view! { <checkbox aria:label="bare" /> })
        .expect("mount");
    let checkbox = node_id(&app, "bare");
    assert_eq!(app.checked("bare"), Some(false));

    let report = app.change_node(checkbox, true).expect("local toggle");
    assert!(report.changed());
    assert_eq!(app.checked("bare"), Some(true));
}

#[test]
fn a_controlled_checkbox_without_a_handler_does_not_toggle() {
    #[component]
    fn Controlled() -> MountResult<MountPlan> {
        let checked = signal(false);
        view! { <checkbox aria:label="controlled" checked={checked} /> }
    }

    let mut app = Runtime::new();
    app.mount(|| view! { <Controlled /> }).expect("mount");
    let checkbox = node_id(&app, "controlled");

    let report = app.change_node(checkbox, true).expect("user change");
    assert!(!report.changed());
    assert_eq!(
        app.checked("controlled"),
        Some(false),
        "the binding remains authoritative without a handler"
    );
}
