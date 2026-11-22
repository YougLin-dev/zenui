#![allow(dead_code, unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;

#[component]
fn HistoryEditor(inputs: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let draft = signal(String::from("abcd"));
    let count = inputs.clone();

    view! {
        <text-input
            aria:label="editor"
            value={draft}
            on:input={move |value: String| {
                count.set(count.get() + 1);
                draft.set(value);
            }}
        />
    }
}

fn editor() -> (Runtime, Rc<Cell<u32>>, NodeId) {
    let inputs = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let inputs = inputs.clone();
        move || view! { <HistoryEditor inputs={inputs} /> }
    })
    .expect("mount");
    let editor = node(&app, "editor");
    (app, inputs, editor)
}

fn node(app: &Runtime, label: &str) -> NodeId {
    app.presentation()
        .expect("snapshot")
        .find_label(label)
        .unwrap_or_else(|| panic!("no element labelled {label}"))
        .id()
}

fn value(app: &Runtime, label: &str) -> String {
    app.presentation()
        .expect("snapshot")
        .find_label(label)
        .unwrap_or_else(|| panic!("no element labelled {label}"))
        .text_value()
        .unwrap_or_default()
        .to_owned()
}

fn text_of(app: &Runtime, label: &str) -> String {
    app.presentation()
        .expect("snapshot")
        .find_label(label)
        .unwrap_or_else(|| panic!("no element labelled {label}"))
        .text()
}

fn edit_state(
    app: &Runtime,
    label: &str,
) -> (usize, usize, Option<(usize, usize)>, Option<String>) {
    let snapshot = app.presentation().expect("snapshot");
    let edit = snapshot
        .find_label(label)
        .unwrap_or_else(|| panic!("no element labelled {label}"))
        .edit()
        .expect("a text input carries edit state");
    (
        edit.caret(),
        edit.anchor(),
        edit.selection(),
        edit.preedit().map(str::to_owned),
    )
}

#[test]
fn one_edit_undoes_and_redoes_with_caret_anchor_and_selection() {
    let (mut app, inputs, editor) = editor();
    app.set_caret_node(editor, 2).expect("place the caret");
    app.insert_text_node(editor, "XY").expect("insert");
    assert_eq!(value(&app, "editor"), "abXYcd");
    assert_eq!(inputs.get(), 1);

    app.undo_node(editor).expect("undo the insert");
    assert_eq!(value(&app, "editor"), "abcd");
    assert_eq!(
        edit_state(&app, "editor"),
        (2, 2, Some((2, 2)), None),
        "undo restores the pre-edit caret and selection"
    );
    assert_eq!(inputs.get(), 2, "one undo dispatches exactly one on:input");

    app.redo_node(editor).expect("redo the insert");
    assert_eq!(value(&app, "editor"), "abXYcd");
    assert_eq!(
        edit_state(&app, "editor"),
        (4, 4, None, None),
        "redo restores the post-edit state"
    );
    assert_eq!(inputs.get(), 3);

    app.undo_node(editor).expect("undo again");
    assert_eq!(value(&app, "editor"), "abcd");
}

#[test]
fn selection_direction_survives_undo_and_redo() {
    let (mut app, inputs, editor) = editor();
    app.set_selection_node(editor, 3, 1)
        .expect("anchor 3, caret 1");
    app.insert_text_node(editor, "Z")
        .expect("replace the range");
    assert_eq!(value(&app, "editor"), "aZd");
    assert_eq!(edit_state(&app, "editor"), (2, 2, None, None));

    app.undo_node(editor).expect("undo");
    assert_eq!(value(&app, "editor"), "abcd");
    assert_eq!(
        edit_state(&app, "editor"),
        (1, 3, Some((1, 3)), None),
        "the anchor keeps the selection direction"
    );

    app.redo_node(editor).expect("redo");
    assert_eq!(value(&app, "editor"), "aZd");
    assert_eq!(edit_state(&app, "editor"), (2, 2, None, None));
    assert_eq!(inputs.get(), 3);
}

#[test]
fn backspace_and_delete_forward_each_undo_as_one_edit() {
    let (mut app, inputs, editor) = editor();
    app.set_caret_node(editor, 2).expect("caret");
    app.backspace_node(editor).expect("backspace");
    assert_eq!(value(&app, "editor"), "acd");

    app.undo_node(editor).expect("undo the backspace");
    assert_eq!(value(&app, "editor"), "abcd");
    assert_eq!(edit_state(&app, "editor"), (2, 2, Some((2, 2)), None));

    app.redo_node(editor).expect("redo the backspace");
    assert_eq!(value(&app, "editor"), "acd");
    assert_eq!(inputs.get(), 3);

    app.delete_forward_node(editor).expect("delete forward");
    assert_eq!(value(&app, "editor"), "ad");
    app.undo_node(editor).expect("undo delete forward");
    assert_eq!(value(&app, "editor"), "acd");
    assert_eq!(edit_state(&app, "editor"), (1, 1, None, None));
    assert_eq!(inputs.get(), 5);
}

#[test]
fn empty_stacks_are_silent_no_ops() {
    let (mut app, inputs, editor) = editor();
    app.undo_node(editor).expect("undo with no history");
    app.redo_node(editor).expect("redo with no history");
    assert_eq!(inputs.get(), 0, "empty stacks dispatch no on:input");
    assert_eq!(value(&app, "editor"), "abcd");
    assert_eq!(edit_state(&app, "editor"), (0, 0, None, None));

    app.insert_text_node(editor, "x").expect("insert");
    app.undo_node(editor).expect("undo");
    app.undo_node(editor).expect("the undo stack is empty");
    assert_eq!(inputs.get(), 2, "the second undo is a silent no-op");
}

#[test]
fn a_new_edit_clears_redo() {
    let (mut app, inputs, editor) = editor();
    app.set_caret_node(editor, 2).expect("caret");
    app.insert_text_node(editor, "X").expect("first edit");
    app.undo_node(editor).expect("undo");
    assert_eq!(value(&app, "editor"), "abcd");

    app.insert_text_node(editor, "Y")
        .expect("a new committed edit");
    assert_eq!(value(&app, "editor"), "abYcd");
    let before_redo = inputs.get();
    app.redo_node(editor)
        .expect("redo after a new edit is a no-op");
    assert_eq!(value(&app, "editor"), "abYcd");
    assert_eq!(inputs.get(), before_redo, "the stale redo runs nothing");
}

#[test]
fn movement_and_focus_record_nothing() {
    let (mut app, inputs, editor) = editor();
    app.set_caret_node(editor, 2).expect("caret");
    app.insert_text_node(editor, "X").expect("edit");
    let after_edit = inputs.get();

    app.set_caret_node(editor, 0).expect("move the caret");
    app.set_selection_node(editor, 0, 3)
        .expect("select a range");
    app.select_all_node(editor).expect("select all");
    app.focus_node(editor).expect("focus");
    app.blur_node(editor).expect("blur");
    assert_eq!(inputs.get(), after_edit, "movement dispatches no on:input");

    app.undo_node(editor).expect("undo the edit");
    assert_eq!(value(&app, "editor"), "abcd");
    assert_eq!(
        edit_state(&app, "editor"),
        (2, 2, Some((2, 2)), None),
        "undo restores the pre-edit state, not the later movement"
    );

    app.redo_node(editor).expect("redo the edit");
    assert_eq!(value(&app, "editor"), "abXcd");
    assert_eq!(
        edit_state(&app, "editor"),
        (3, 3, None, None),
        "redo restores the recorded post-edit state, not the later movement"
    );
}

#[test]
fn cjk_and_emoji_offsets_are_restored_at_character_boundaries() {
    let (mut app, inputs, editor) = editor();
    app.set_caret_node(editor, 2)
        .expect("caret between 好 and 世");
    app.insert_text_node(editor, "🙂").expect("insert an emoji");
    assert_eq!(value(&app, "editor"), "ab🙂cd");
    assert_eq!(edit_state(&app, "editor"), (3, 3, None, None));

    app.undo_node(editor).expect("undo the emoji");
    assert_eq!(value(&app, "editor"), "abcd");
    assert_eq!(edit_state(&app, "editor"), (2, 2, Some((2, 2)), None));
    assert_eq!(inputs.get(), 2);
}

#[test]
fn composition_commit_is_one_entry_and_other_phases_record_nothing() {
    let (mut app, inputs, editor) = editor();
    app.set_selection_node(editor, 1, 3)
        .expect("select a range");

    app.compose_start_node(editor).expect("start");
    app.compose_update_node(editor, "ni").expect("preedit");
    assert_eq!(inputs.get(), 0, "preedit writes no model");
    app.compose_cancel_node(editor).expect("cancel");
    assert_eq!(inputs.get(), 0);
    app.undo_node(editor)
        .expect("cancelled composition records nothing");
    assert_eq!(inputs.get(), 0);
    assert_eq!(value(&app, "editor"), "abcd");

    app.compose_start_node(editor).expect("start again");
    app.compose_update_node(editor, "ni").expect("preedit");
    app.compose_commit_node(editor, "你").expect("commit");
    assert_eq!(value(&app, "editor"), "a你d");
    assert_eq!(inputs.get(), 1, "the commit is exactly one edit");

    app.undo_node(editor).expect("undo the commit");
    assert_eq!(value(&app, "editor"), "abcd");
    assert_eq!(
        edit_state(&app, "editor"),
        (3, 1, Some((1, 3)), None),
        "undo restores the state before composition started"
    );
    assert_eq!(inputs.get(), 2);

    app.redo_node(editor).expect("redo the commit");
    assert_eq!(value(&app, "editor"), "a你d");
    assert_eq!(edit_state(&app, "editor"), (2, 2, None, None));
    assert_eq!(inputs.get(), 3);

    app.compose_start_node(editor).expect("start");
    app.compose_update_node(editor, "x").expect("preedit");
    app.compose_cancel_node(editor).expect("cancel");
    app.undo_node(editor)
        .expect("composition lifecycle adds no entries");
    assert_eq!(value(&app, "editor"), "abcd");
    assert_eq!(inputs.get(), 4);
}

#[test]
fn undo_is_rejected_during_composition() {
    let (mut app, inputs, editor) = editor();
    app.compose_start_node(editor).expect("start");
    assert_eq!(
        app.undo_node(editor),
        Err(DispatchError::Rejected {
            node: editor,
            operation: NodeOperation::Undo,
            reason: "text input is composing",
        })
    );
    assert_eq!(
        app.redo_node(editor),
        Err(DispatchError::Rejected {
            node: editor,
            operation: NodeOperation::Redo,
            reason: "text input is composing",
        })
    );
    app.compose_cancel_node(editor).expect("cancel");
    assert_eq!(inputs.get(), 0);
}

#[component]
fn ExternalEditor(inputs: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let draft = signal(String::from("abcd"));
    let count = inputs.clone();

    view! {
        <view>
            <text-input
                aria:label="editor"
                value={draft}
                on:input={move |value: String| {
                    count.set(count.get() + 1);
                    draft.set(value);
                }}
            />
            <button
                aria:label="external"
                on:click={move || draft.set(String::from("external"))}
            >"External"</button>
        </view>
    }
}

#[test]
fn external_value_changes_clear_both_stacks() {
    let inputs = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let inputs = inputs.clone();
        move || view! { <ExternalEditor inputs={inputs} /> }
    })
    .expect("mount");
    let editor = node(&app, "editor");

    app.set_caret_node(editor, 2).expect("caret");
    app.insert_text_node(editor, "X").expect("edit");
    app.click_label("external")
        .expect("replace the value externally");
    assert_eq!(value(&app, "editor"), "external");
    let after_external = inputs.get();
    app.undo_node(editor)
        .expect("the external change cleared the history");
    assert_eq!(
        inputs.get(),
        after_external,
        "no on:input for a silent undo"
    );
    assert_eq!(value(&app, "editor"), "external");

    app.set_caret_node(editor, 8).expect("caret at the end");
    app.insert_text_node(editor, "!").expect("edit again");
    app.input("editor", "replaced")
        .expect("the label helper replaces the whole value");
    assert_eq!(value(&app, "editor"), "replaced");
    let after_replace = inputs.get();
    app.undo_node(editor)
        .expect("an external whole-value replacement clears history");
    assert_eq!(inputs.get(), after_replace);
    assert_eq!(value(&app, "editor"), "replaced");
}

#[component]
fn TwoEditors(inputs: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let left = signal(String::from("left"));
    let right = signal(String::from("right"));
    let left_count = inputs.clone();
    let right_count = inputs.clone();

    view! {
        <view>
            <text-input
                aria:label="left"
                value={left}
                on:input={move |value: String| {
                    left_count.set(left_count.get() + 1);
                    left.set(value);
                }}
            />
            <text-input
                aria:label="right"
                value={right}
                on:input={move |value: String| {
                    right_count.set(right_count.get() + 1);
                    right.set(value);
                }}
            />
        </view>
    }
}

#[test]
fn histories_are_per_input() {
    let inputs = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let inputs = inputs.clone();
        move || view! { <TwoEditors inputs={inputs} /> }
    })
    .expect("mount");
    let left = node(&app, "left");
    let right = node(&app, "right");

    app.set_caret_node(left, 4).expect("left caret");
    app.insert_text_node(left, "L").expect("edit left");
    app.set_caret_node(right, 5).expect("right caret");
    app.insert_text_node(right, "R").expect("edit right");

    app.undo_node(left).expect("undo left only");
    assert_eq!(value(&app, "left"), "left");
    assert_eq!(value(&app, "right"), "rightR");
    app.undo_node(right).expect("undo right only");
    assert_eq!(value(&app, "right"), "right");
    assert_eq!(inputs.get(), 4);
}

#[test]
fn a_shared_signal_edit_invalidates_the_other_windows_history() {
    let counts = Rc::new([Cell::new(0), Cell::new(0)]);
    let a_count = counts.clone();
    let b_count = counts.clone();
    let mut app = Runtime::new();
    let shared = app
        .setup(|| signal(String::from("abcd")))
        .expect("application setup");
    app.declare_windows(move |catalog| {
        let a_count = a_count.clone();
        catalog.window(WindowSpec::new("a").primary(), move |_| {
            let count = a_count.clone();
            view! {
                <text-input
                    aria:label="a-input"
                    value={shared}
                    on:input={move |value: String| {
                        count[0].set(count[0].get() + 1);
                        shared.set(value);
                    }}
                />
            }
        })?;
        let b_count = b_count.clone();
        catalog.window(WindowSpec::new("b"), move |_| {
            let count = b_count.clone();
            view! {
                <text-input
                    aria:label="b-input"
                    value={shared}
                    on:input={move |value: String| {
                        count[1].set(count[1].get() + 1);
                        shared.set(value);
                    }}
                />
            }
        })?;
        Ok(())
    })
    .expect("catalog");

    let a = app.show("a").expect("show a");
    let b = app.show("b").expect("show b");
    let a_input = app
        .presentation_of(a)
        .expect("a snapshot")
        .find_label("a-input")
        .expect("a input")
        .id();
    let b_input = app
        .presentation_of(b)
        .expect("b snapshot")
        .find_label("b-input")
        .expect("b input")
        .id();

    app.set_caret_node(b_input, 4).expect("caret in b");
    app.insert_text_node(b_input, "B").expect("edit in b");
    assert_eq!(
        app.presentation_of(a)
            .expect("a snapshot")
            .find_label("a-input")
            .expect("a input")
            .text_value(),
        Some("abcdB"),
        "a shared Signal reaches the other window"
    );

    app.set_caret_node(a_input, 0).expect("caret in a");
    app.insert_text_node(a_input, "A").expect("edit in a");
    assert_eq!(counts[0].get(), 1);
    assert_eq!(counts[1].get(), 1);

    let b_before = counts[1].get();
    app.undo_node(b_input)
        .expect("a shared edit cleared b's history");
    assert_eq!(
        counts[1].get(),
        b_before,
        "a silent undo dispatches no on:input in b"
    );

    app.undo_node(a_input).expect("undo a");
    assert_eq!(
        app.presentation_of(a)
            .expect("a snapshot")
            .find_label("a-input")
            .expect("a input")
            .text_value(),
        Some("abcdB")
    );
    assert_eq!(counts[0].get(), 2);
    let a_before = counts[0].get();
    app.undo_node(a_input).expect("a's stack is empty");
    assert_eq!(counts[0].get(), a_before);
}

#[test]
fn hidden_windows_keep_history_but_reject_dispatch() {
    let inputs = Rc::new(Cell::new(0));
    let count = inputs.clone();
    let mut app = Runtime::new();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |_| {
            let draft = signal(String::from("abcd"));
            let count = count.clone();
            view! {
                <text-input
                    aria:label="hidden-input"
                    value={draft}
                    on:input={move |value: String| {
                        count.set(count.get() + 1);
                        draft.set(value);
                    }}
                />
            }
        })?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show main");
    let input = app
        .presentation_of(main)
        .expect("snapshot")
        .find_label("hidden-input")
        .expect("input")
        .id();
    app.set_caret_node(input, 4).expect("caret");
    app.insert_text_node(input, "X").expect("edit");
    assert_eq!(inputs.get(), 1);

    app.hide(main).expect("hide");
    assert_eq!(app.undo_node(input), Err(DispatchError::Hidden(input)));
    assert_eq!(inputs.get(), 1, "a rejected undo writes no model");

    app.show("main").expect("show the same activation");
    app.undo_node(input).expect("history survives hiding");
    assert_eq!(inputs.get(), 2);
}

#[component]
fn ErrorKinds() -> MountResult<MountPlan> {
    let disabled = signal(true);
    let shown = memo(move || disabled.get());

    view! {
        <view>
            <text-input
                aria:label="off"
                value={once(String::from("off"))}
                disabled={shown}
                on:input={move |_value: String| {}}
            />
            <button aria:label="plain">"Plain"</button>
        </view>
    }
}

#[test]
fn undo_rejects_disabled_non_text_foreign_and_stale_targets() {
    let mut disabled = {
        let mut app = Runtime::new();
        app.mount(|| view! { <ErrorKinds /> }).expect("mount");
        app
    };
    let off = node(&disabled, "off");
    assert_eq!(
        disabled.undo_node(off),
        Err(DispatchError::Rejected {
            node: off,
            operation: NodeOperation::Undo,
            reason: "text input is disabled",
        })
    );
    let plain = node(&disabled, "plain");
    assert_eq!(
        disabled.undo_node(plain),
        Err(DispatchError::WrongKind {
            node: plain,
            operation: NodeOperation::Undo,
        })
    );
    assert_eq!(
        disabled.redo_node(plain),
        Err(DispatchError::WrongKind {
            node: plain,
            operation: NodeOperation::Redo,
        })
    );

    let (mut app, _inputs, _editor) = editor();
    let mut other = Runtime::new();
    other
        .mount(|| view! { <HistoryEditor inputs={Rc::new(Cell::new(0))} /> })
        .expect("mount another runtime");
    let foreign = node(&other, "editor");
    assert_eq!(
        app.undo_node(foreign),
        Err(DispatchError::UnknownNode(foreign))
    );
}

#[component]
fn ToggleEditor() -> MountResult<MountPlan> {
    let visible = signal(true);
    let draft = signal(String::from("abcd"));
    let edits = signal(0u32);
    let edit_text = memo(move || edits.get().to_string());
    let shown = memo(move || visible.get());

    view! {
        <view>
            <Show when={shown}>
                <text-input
                    aria:label="toggle"
                    value={draft}
                    on:input={move |value: String| {
                        edits.set(edits.get() + 1);
                        draft.set(value);
                    }}
                />
            </Show>
            <text aria:label="edits">{edit_text}</text>
            <button aria:label="hide-toggle" on:click={move || visible.set(false)}>"Hide"</button>
            <button aria:label="show-toggle" on:click={move || visible.set(true)}>"Show"</button>
        </view>
    }
}

#[test]
fn unmounting_an_input_drops_its_history() {
    let mut app = Runtime::new();
    app.mount(|| view! { <ToggleEditor /> }).expect("mount");

    let first = node(&app, "toggle");
    app.set_caret_node(first, 4).expect("caret");
    app.insert_text_node(first, "X").expect("edit");
    assert_eq!(text_of(&app, "edits"), "1");
    assert_eq!(app.unmounts(), 0);

    app.click_label("hide-toggle").expect("unmount the input");
    assert_eq!(app.node_status(first), NodeStatus::Stale);
    assert_eq!(app.undo_node(first), Err(DispatchError::StaleNode(first)));

    app.click_label("show-toggle").expect("mount a fresh input");
    let second = node(&app, "toggle");
    assert_ne!(first, second, "a remount is a new node identity");
    app.undo_node(second)
        .expect("the new element starts with no history");
    assert_eq!(
        text_of(&app, "edits"),
        "1",
        "a silent undo dispatches no on:input"
    );
}

#[component]
fn HandlerKinds(inputs: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let short = signal(String::from("ab"));
    let three = signal(String::from("abc"));
    let upper = signal(String::from("ab"));
    let short_count = inputs.clone();
    let three_count = inputs.clone();
    let upper_count = inputs.clone();

    view! {
        <view>
            <text-input
                aria:label="short"
                value={short}
                on:input={move |value: String| {
                    short_count.set(short_count.get() + 1);
                    short.set(value.chars().take(2).collect());
                }}
            />
            <text-input
                aria:label="three"
                value={three}
                on:input={move |value: String| {
                    three_count.set(three_count.get() + 1);
                    three.set(value.chars().take(2).collect());
                }}
            />
            <text-input
                aria:label="upper"
                value={upper}
                on:input={move |value: String| {
                    upper_count.set(upper_count.get() + 1);
                    upper.set(value.to_uppercase());
                }}
            />
        </view>
    }
}

fn handler_kinds() -> (Runtime, Rc<Cell<u32>>) {
    let inputs = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let inputs = inputs.clone();
        move || view! { <HandlerKinds inputs={inputs} /> }
    })
    .expect("mount");
    (app, inputs)
}

#[test]
fn rewritten_values_are_recorded_as_committed() {
    let (mut app, inputs) = handler_kinds();
    let short = node(&app, "short");
    app.set_caret_node(short, 2).expect("caret");
    app.insert_text_node(short, "XYZ")
        .expect("the handler truncates to the current value");
    assert_eq!(value(&app, "short"), "ab");
    assert_eq!(inputs.get(), 1);
    app.undo_node(short)
        .expect("before and after values are equal");
    assert_eq!(inputs.get(), 1, "no entry was recorded");
    assert_eq!(value(&app, "short"), "ab");

    let three = node(&app, "three");
    app.set_caret_node(three, 3).expect("caret");
    app.insert_text_node(three, "Z")
        .expect("the handler commits a shorter value");
    assert_eq!(value(&app, "three"), "ab");
    assert_eq!(inputs.get(), 2);
    app.undo_node(three)
        .expect("the committed value differs from the pre-edit value");
    assert_eq!(inputs.get(), 3, "undo dispatches exactly once");
    assert_eq!(
        value(&app, "three"),
        "ab",
        "a truncating handler keeps model authority during undo"
    );
    app.redo_node(three).expect("redo");
    assert_eq!(inputs.get(), 4);
    assert_eq!(value(&app, "three"), "ab");

    let upper = node(&app, "upper");
    app.set_caret_node(upper, 2).expect("caret");
    app.insert_text_node(upper, "c").expect("uppercase edit");
    assert_eq!(value(&app, "upper"), "ABC");
    assert_eq!(inputs.get(), 5);
    app.undo_node(upper).expect("undo the committed value");
    assert_eq!(
        value(&app, "upper"),
        "AB",
        "exact restoration is only promised for identity handlers"
    );
    assert_eq!(inputs.get(), 6);
    app.redo_node(upper).expect("redo the committed value");
    assert_eq!(value(&app, "upper"), "ABC");
    assert_eq!(inputs.get(), 7);
}

#[component]
fn EntryCapEditor(inputs: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let draft = signal(String::new());
    let count = inputs.clone();

    view! {
        <text-input
            aria:label="cap"
            value={draft}
            on:input={move |value: String| {
                count.set(count.get() + 1);
                draft.set(value);
            }}
        />
    }
}

#[test]
fn the_entry_cap_evicts_the_oldest_history() {
    let inputs = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let inputs = inputs.clone();
        move || view! { <EntryCapEditor inputs={inputs} /> }
    })
    .expect("mount");
    let cap = node(&app, "cap");

    for _ in 0..150 {
        app.insert_text_node(cap, "x").expect("insert at the end");
    }
    assert_eq!(value(&app, "cap").chars().count(), 150);
    assert_eq!(inputs.get(), 150);

    let mut undos = 0;
    let mut last = 150;
    for _ in 0..150 {
        app.undo_node(cap).expect("undo");
        let length = value(&app, "cap").chars().count();
        if length == last {
            break;
        }
        assert_eq!(length + 1, last, "each undo removes exactly one insert");
        last = length;
        undos += 1;
    }
    assert_eq!(undos, 100, "only the newest 100 edits are retained");
    assert_eq!(last, 50);
    let after_undos = inputs.get();
    app.undo_node(cap).expect("the undo stack is exhausted");
    assert_eq!(inputs.get(), after_undos);

    let mut redos = 0;
    let mut last = 50;
    for _ in 0..150 {
        app.redo_node(cap).expect("redo");
        let length = value(&app, "cap").chars().count();
        if length == last {
            break;
        }
        assert_eq!(length, last + 1);
        last = length;
        redos += 1;
    }
    assert_eq!(redos, 100);
    assert_eq!(last, 150);
}

#[component]
fn ByteCapEditor(inputs: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let draft = signal(String::new());
    let count = inputs.clone();

    view! {
        <text-input
            aria:label="bytes"
            value={draft}
            on:input={move |value: String| {
                count.set(count.get() + 1);
                draft.set(value);
            }}
        />
    }
}

#[test]
fn the_byte_cap_evicts_oldest_snapshots() {
    let inputs = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let inputs = inputs.clone();
        move || view! { <ByteCapEditor inputs={inputs} /> }
    })
    .expect("mount");
    let bytes = node(&app, "bytes");

    let block = "a".repeat(40 * 1024);
    for _ in 0..3 {
        app.insert_text_node(bytes, &block).expect("append a block");
    }
    assert_eq!(value(&app, "bytes").len(), 120 * 1024);
    assert_eq!(inputs.get(), 3);

    app.undo_node(bytes)
        .expect("the newest snapshot is still within the byte cap");
    assert_eq!(
        value(&app, "bytes").len(),
        80 * 1024,
        "older snapshots were evicted"
    );
    let after_undo = inputs.get();
    app.undo_node(bytes).expect("the byte cap left one entry");
    assert_eq!(inputs.get(), after_undo);
}

#[test]
fn undo_rejects_a_window_awaiting_close_confirmation() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .close_policy(ClosePolicy::Confirm),
            move |window| {
                window
                    .on_close(move |_| {
                        window
                            .request_show("confirm")
                            .expect("show the confirmation");
                    })
                    .expect("register the close handler");
                view! {
                    <text-input
                        aria:label="draft"
                        value={once(String::from("abcd"))}
                        on:input={move |_value: String| {}}
                    />
                }
            },
        )?;
        catalog.window(
            WindowSpec::new("confirm").confirmation_for("main"),
            |window| {
                window
                    .close_request()
                    .expect("read the pending request")
                    .expect("a pending request exists");
                view! { <view /> }
            },
        )?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show main");
    let draft = app
        .presentation_of(main)
        .expect("snapshot")
        .find_label("draft")
        .expect("draft")
        .id();
    app.request_close(main).expect("begin the close epoch");
    assert_eq!(app.undo_node(draft), Err(DispatchError::Suspended(draft)));
}
