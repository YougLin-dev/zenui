#![allow(unused_braces)]

use zenui::prelude::*;

fn window_fixture() -> (Runtime, WindowCtx) {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary().sized(400, 300), |_| {
            let draft = signal(String::from("ab"));
            view! {
                <view>
                    <button aria:label="first">"First"</button>
                    <button aria:label="second">"Second"</button>
                    <text-input aria:label="draft" value={draft} />
                </view>
            }
        })
    })
    .expect("declare the window");
    let window = app.show("main").expect("show the window");
    (app, window)
}

fn input_fixture() -> (Runtime, NodeId) {
    let mut app = Runtime::new();
    app.mount(|| {
        let value = signal(String::from("ab"));
        view! {
            <text-input
                aria:label="probe"
                value={value}
                on:input={move |next: String| value.set(next)}
            />
        }
    })
    .expect("mount");
    let input = app
        .presentation()
        .expect("snapshot")
        .find_label("probe")
        .expect("input")
        .id();
    app.focus("probe").expect("focus the input");

    (app, input)
}

#[test]
fn tab_reports_a_focus_reveal() {
    let (mut app, window) = window_fixture();
    assert_eq!(
        app.presentation_of(window).expect("snapshot").focus(),
        None,
        "the window starts without focus"
    );
    let report = app
        .dispatch_intent(Intent::keydown_window(
            window,
            Key::Tab,
            Modifiers::default(),
        ))
        .expect("tab turn");
    assert!(
        report.reveal().focus_changed(),
        "Tab moves focus and reports a reveal"
    );
    assert!(report.reveal().needs_reveal());
    let after = app.presentation_of(window).expect("snapshot");
    assert!(after.focus().is_some(), "the snapshot has a focus target");
}

#[test]
fn a_key_that_moves_nothing_reports_no_reveal() {
    let (mut app, window) = window_fixture();
    app.focus("first").expect("focus the first button");
    let report = app
        .dispatch_intent(Intent::keydown_window(
            window,
            Key::Other,
            Modifiers::default(),
        ))
        .expect("key turn");
    assert!(
        !report.reveal().needs_reveal(),
        "a key with no focus, caret or composition change does not reveal"
    );
}

#[test]
fn a_press_reports_focus_and_caret_changes() {
    let (mut app, window) = window_fixture();
    let snapshot = app.presentation_of(window).expect("snapshot");
    let draft = snapshot.find_label("draft").expect("draft").id();
    let first = snapshot.find_label("first").expect("first").id();

    let focused = app
        .dispatch_intent(Intent::press(first, None))
        .expect("press the button");
    assert!(
        focused.reveal().focus_changed(),
        "a press that moves focus reports it"
    );
    assert!(!focused.reveal().caret_changed());

    let repeated = app
        .dispatch_intent(Intent::press(first, None))
        .expect("press the focused button again");
    assert!(
        !repeated.reveal().needs_reveal(),
        "pressing the already focused target changes no reveal target"
    );

    let caret = app
        .dispatch_intent(Intent::press(draft, Some(1)))
        .expect("press the input");
    assert!(caret.reveal().focus_changed(), "the draft gains focus");
    assert!(
        !caret.reveal().caret_changed(),
        "a focus change is reported as focus, not as a caret delta across inputs"
    );

    let moved = app
        .dispatch_intent(Intent::press(draft, Some(2)))
        .expect("press the focused input again");
    assert!(!moved.reveal().focus_changed(), "the draft keeps focus");
    assert!(
        moved.reveal().caret_changed(),
        "placing the caret at a new click offset reports it"
    );
}

#[test]
fn edits_and_composition_report_caret_and_composition_changes() {
    let (mut app, input) = input_fixture();
    let report = app.type_text_node(input, "X").expect("type");
    assert!(
        report.reveal().caret_changed(),
        "an insertion moves the caret"
    );
    assert!(!report.reveal().focus_changed());
    assert!(!report.reveal().composition_changed());

    app.compose_start_node(input).expect("start composing");
    app.compose_update_node(input, "ni")
        .expect("update preedit");
    let commit = app.compose_commit_node(input, "你").expect("commit");
    assert!(
        commit.reveal().composition_changed(),
        "committing ends the composition"
    );
    assert!(
        commit.reveal().caret_changed(),
        "the caret moves after commit"
    );
}

#[test]
fn a_no_op_turn_reports_no_reveal_flags() {
    let report = TurnReport::default();
    assert!(!report.reveal().needs_reveal());
    assert!(!report.reveal().focus_changed());
    assert!(!report.reveal().caret_changed());
    assert!(!report.reveal().composition_changed());
}

#[test]
fn a_collapsed_selection_at_the_caret_reports_no_reveal() {
    let (mut app, input) = input_fixture();
    app.set_caret_node(input, 1).expect("place the caret");
    let collapsed = app
        .set_selection_node(input, 1, 1)
        .expect("collapse the selection onto the caret");
    assert!(
        !collapsed.reveal().needs_reveal(),
        "a collapsed selection at the caret moves nothing, whatever its stored encoding"
    );

    let selected = app
        .set_selection_node(input, 2, 0)
        .expect("select backwards");
    assert!(
        selected.reveal().caret_changed(),
        "a selection that moves the caret or its anchor requests reveal"
    );
    assert!(!selected.reveal().focus_changed());
}

#[test]
fn empty_history_reports_no_reveal_and_an_undo_reports_its_caret_move() {
    let (mut app, input) = input_fixture();
    let undo = app.undo_node(input).expect("undo without history");
    assert!(
        !undo.reveal().needs_reveal(),
        "an undo without entries changes no reveal target"
    );
    let redo = app.redo_node(input).expect("redo without history");
    assert!(!redo.reveal().needs_reveal());

    app.set_caret_node(input, 2).expect("caret at the end");
    app.type_text_node(input, "X").expect("type");
    app.set_caret_node(input, 0).expect("caret at the start");
    let undone = app.undo_node(input).expect("undo the insertion");
    assert!(
        undone.reveal().caret_changed(),
        "undo restores the pre-edit caret and requests reveal"
    );
}

/// 焦点草稿加一个删除按钮：click handler 卸载当前焦点。
fn unmount_fixture() -> Runtime {
    let mut app = Runtime::new();
    app.mount(|| {
        let shown = signal(true);
        let draft = signal(String::from("ab"));
        view! {
            <view>
                <Show
                    when={memo(move || shown.get())}
                    fallback={|| view! { <text>"gone"</text> }}
                >
                    <text-input aria:label="draft" value={draft} />
                </Show>
                <button aria:label="remove" on:click={move || shown.set(false)}>"Remove"</button>
            </view>
        }
    })
    .expect("mount");
    app
}

/// 提交或按键后卸载聚焦输入的表单：handler 让焦点结算到后面的按钮。
fn submitting_fixture() -> (Runtime, NodeId, NodeId) {
    let mut app = Runtime::new();
    app.mount(|| {
        let shown = signal(true);
        let draft = signal(String::from("ab"));
        view! {
            <view>
                <form on:submit={move |_event: &mut SubmitEvent| shown.set(false)}>
                    <Show
                        when={memo(move || shown.get())}
                        fallback={|| view! { <text>"gone"</text> }}
                    >
                        <text-input
                            aria:label="draft"
                            value={draft}
                            on:keydown={move |event: &mut KeyEvent| {
                                if event.key() == Key::PageDown {
                                    shown.set(false);
                                }
                            }}
                        />
                    </Show>
                </form>
                <button aria:label="tail">"Tail"</button>
            </view>
        }
    })
    .expect("mount");
    let snapshot = app.presentation().expect("snapshot");
    let form = snapshot
        .elements()
        .find(|element| element.tag() == "form")
        .expect("form")
        .id();
    let tail = snapshot.find_label("tail").expect("tail").id();
    (app, form, tail)
}

#[test]
fn a_focus_command_reports_the_focus_it_moves() {
    let (mut app, window) = window_fixture();
    let first = app
        .presentation_of(window)
        .expect("snapshot")
        .find_label("first")
        .expect("first")
        .id();
    let report = app.focus_node(first).expect("focus the first button");
    assert!(
        report.reveal().focus_changed(),
        "a focus command reports its target"
    );
    assert!(report.changed(), "the focus command commits a change");
    let repeated = app.focus_node(first).expect("refocus the first button");
    assert!(
        !repeated.reveal().needs_reveal(),
        "refocusing the current target requests no reveal"
    );
    assert!(
        !repeated.changed(),
        "refocusing the current target changes nothing"
    );
}

#[test]
fn a_submit_that_settles_focus_reports_reveal() {
    let (mut app, form, tail) = submitting_fixture();
    app.focus("draft").expect("focus the draft");
    let report = app.submit_node(form).expect("submit the form");
    assert_eq!(
        app.presentation().expect("snapshot").focus(),
        Some(tail),
        "the submit handler unmounted the focused input"
    );
    assert!(
        report.reveal().focus_changed(),
        "a submit turn that settles focus reports it"
    );
}

#[test]
fn a_direct_keydown_reports_the_focus_it_settles() {
    let (mut app, _form, tail) = submitting_fixture();
    app.focus("draft").expect("focus the draft");
    let draft = app
        .presentation()
        .expect("snapshot")
        .find_label("draft")
        .expect("draft")
        .id();
    let report = app
        .keydown_node(draft, Key::PageDown)
        .expect("keydown the draft");
    assert_eq!(
        app.presentation().expect("snapshot").focus(),
        Some(tail),
        "the key handler unmounted the focused input"
    );
    assert!(
        report.reveal().focus_changed(),
        "a direct keydown turn that settles focus reports it"
    );
}

/// 切换后禁用自身的复选框：change handler 让焦点结算到后面的按钮。
fn self_disabling_fixture() -> (Runtime, NodeId, NodeId) {
    let mut app = Runtime::new();
    app.mount(|| {
        let checked = signal(false);
        let disabled = signal(false);
        view! {
            <view>
                <checkbox
                    aria:label="gate"
                    checked={checked}
                    disabled={disabled}
                    on:change={move |next: bool| {
                        checked.set(next);
                        disabled.set(true);
                    }}
                />
                <button aria:label="later">"Later"</button>
            </view>
        }
    })
    .expect("mount");
    let snapshot = app.presentation().expect("snapshot");
    let gate = snapshot.find_label("gate").expect("gate").id();
    let later = snapshot.find_label("later").expect("later").id();
    (app, gate, later)
}

#[test]
fn a_change_that_settles_focus_reports_reveal() {
    let (mut app, gate, later) = self_disabling_fixture();
    app.focus("gate").expect("focus the gate");
    let report = app.change_node(gate, true).expect("flip the gate");
    assert_eq!(
        app.presentation().expect("snapshot").focus(),
        Some(later),
        "disabling the focused checkbox settles focus on the surviving button"
    );
    assert!(
        report.reveal().focus_changed(),
        "a change turn that settles focus reports it"
    );
}

#[test]
fn a_direct_click_reports_the_focus_it_settles() {
    let mut app = unmount_fixture();
    app.focus("draft").expect("focus the draft");
    let remove = app
        .presentation()
        .expect("snapshot")
        .find_label("remove")
        .expect("remove")
        .id();
    let report = app.click_node(remove).expect("click the remover directly");
    assert!(
        report.reveal().focus_changed(),
        "click_node runs the same focus settlement report as routed activation"
    );
}

#[test]
fn an_activation_reports_the_focus_its_click_settles() {
    let mut app = unmount_fixture();
    app.focus("draft").expect("focus the draft");
    let snapshot = app.presentation().expect("snapshot");
    let draft = snapshot.find_label("draft").expect("draft").id();
    let remove = snapshot.find_label("remove").expect("remove").id();
    assert_eq!(snapshot.focus(), Some(draft));
    let report = app.click_text("Remove").expect("remove the focused draft");
    assert!(
        report.reveal().focus_changed(),
        "clicking a control that unmounts the focused editor settles focus"
    );
    assert!(
        report.reveal().needs_reveal(),
        "the settled focus requests reveal"
    );
    assert_eq!(
        app.presentation().expect("snapshot").focus(),
        Some(remove),
        "focus settles on the surviving focusable control"
    );
}
