#![allow(dead_code, unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;

#[component]
fn Editor(inputs: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let draft = signal(String::from("abcd"));
    let short = signal(String::from("ab"));
    let count = inputs.clone();
    let shown = memo(move || draft.get());

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
            <text-input
                aria:label="short"
                value={short}
                on:input={move |value: String| {
                    short.set(value.chars().take(2).collect());
                }}
            />
            <text-input
                aria:label="off"
                value={once(String::from("off"))}
                disabled={true}
                on:input={move |_value: String| {}}
            />
            <button aria:label="plain">"Plain"</button>
            <text>{shown}</text>
        </view>
    }
}

fn editor() -> (Runtime, Rc<Cell<u32>>, NodeId) {
    let inputs = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let inputs = inputs.clone();
        move || view! { <Editor inputs={inputs} /> }
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

fn edit(app: &Runtime, label: &str) -> (usize, Option<(usize, usize)>, Option<String>) {
    let snapshot = app.presentation().expect("snapshot");
    let edit = snapshot
        .find_label(label)
        .unwrap_or_else(|| panic!("no element labelled {label}"))
        .edit()
        .expect("a text input carries edit state");
    (
        edit.caret(),
        edit.selection(),
        edit.preedit().map(str::to_owned),
    )
}

#[test]
fn insert_replaces_the_selection_and_moves_the_caret() {
    let (mut app, inputs, editor) = editor();
    app.set_caret_node(editor, 2).expect("place the caret");
    assert_eq!(edit(&app, "editor"), (2, Some((2, 2)), None));

    app.insert_text_node(editor, "XY").expect("insert");
    assert_eq!(value(&app, "editor"), "abXYcd");
    assert_eq!(edit(&app, "editor"), (4, None, None));
    assert_eq!(inputs.get(), 1, "one edit dispatches one on:input");

    app.set_selection_node(editor, 1, 3)
        .expect("select a range");
    app.insert_text_node(editor, "Z")
        .expect("replace the range");
    assert_eq!(value(&app, "editor"), "aZYcd");
    assert_eq!(edit(&app, "editor").0, 2);
    assert_eq!(inputs.get(), 2);

    app.select_all_node(editor).expect("select all");
    app.insert_text_node(editor, "all")
        .expect("replace everything");
    assert_eq!(value(&app, "editor"), "all");
    assert_eq!(edit(&app, "editor"), (3, None, None));
    assert_eq!(inputs.get(), 3);
}

#[test]
fn the_caret_is_reconciled_with_the_value_the_handler_committed() {
    let (mut app, _inputs, _editor) = editor();
    let short = node(&app, "short");

    app.set_caret_node(short, 2).expect("caret at the end");
    app.insert_text_node(short, "XYZ").expect("insert");
    assert_eq!(
        value(&app, "short"),
        "ab",
        "the handler truncated the value"
    );
    assert_eq!(
        edit(&app, "short").0,
        2,
        "the caret is clamped to the committed value"
    );
}

#[test]
fn backspace_and_delete_forward_respect_caret_and_selection() {
    let (mut app, inputs, editor) = editor();
    app.set_caret_node(editor, 2).expect("caret");
    app.backspace_node(editor).expect("backspace");
    assert_eq!(value(&app, "editor"), "acd");
    assert_eq!(edit(&app, "editor"), (1, None, None));
    assert_eq!(inputs.get(), 1);

    app.backspace_node(editor).expect("backspace again");
    assert_eq!(value(&app, "editor"), "cd");
    assert_eq!(edit(&app, "editor").0, 0);
    assert_eq!(inputs.get(), 2);

    app.backspace_node(editor).expect("start is a no-op");
    assert_eq!(value(&app, "editor"), "cd");
    assert_eq!(inputs.get(), 2, "a no-op edit dispatches no on:input");

    app.set_selection_node(editor, 0, 2).expect("select all");
    app.delete_forward_node(editor)
        .expect("delete the selection");
    assert_eq!(value(&app, "editor"), "");
    assert_eq!(inputs.get(), 3);

    app.delete_forward_node(editor).expect("end is a no-op");
    assert_eq!(inputs.get(), 3);

    app.set_caret_node(editor, 0).expect("caret");
    app.insert_text_node(editor, "xy").expect("insert");
    app.set_caret_node(editor, 0).expect("caret");
    app.delete_forward_node(editor).expect("delete forward");
    assert_eq!(value(&app, "editor"), "y");
    assert_eq!(edit(&app, "editor").0, 0);
    assert_eq!(inputs.get(), 5);
}

#[component]
fn CjkEditor(inputs: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let text = signal(String::from("你好世界"));
    let count = inputs.clone();

    view! {
        <text-input
            aria:label="cjk"
            value={text}
            on:input={move |value: String| {
                count.set(count.get() + 1);
                text.set(value);
            }}
        />
    }
}

#[test]
fn cjk_values_are_edited_at_character_boundaries() {
    let inputs = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let inputs = inputs.clone();
        move || view! { <CjkEditor inputs={inputs} /> }
    })
    .expect("mount");
    let cjk = node(&app, "cjk");

    app.set_caret_node(cjk, 2).expect("caret between 好 and 世");
    app.backspace_node(cjk).expect("backspace");
    assert_eq!(value(&app, "cjk"), "你世界");
    assert_eq!(edit(&app, "cjk").0, 1);

    app.insert_text_node(cjk, "🙂").expect("insert an emoji");
    assert_eq!(value(&app, "cjk"), "你🙂世界");
    assert_eq!(edit(&app, "cjk").0, 2);
    assert_eq!(inputs.get(), 2);
}

#[test]
fn movement_clamps_offsets_and_dispatches_no_input() {
    let (mut app, inputs, editor) = editor();

    app.set_caret_node(editor, 99).expect("clamped caret");
    assert_eq!(edit(&app, "editor"), (4, Some((4, 4)), None));

    app.set_selection_node(editor, 3, 1)
        .expect("normalized selection");
    assert_eq!(edit(&app, "editor"), (1, Some((1, 3)), None));
    let snapshot = app.presentation().expect("snapshot");
    let state = snapshot
        .find_label("editor")
        .expect("editor")
        .edit()
        .expect("edit state");
    assert_eq!(
        state.anchor(),
        3,
        "the anchor keeps the selection direction"
    );
    assert_eq!(state.caret(), 1, "the caret stays at the focus end");

    app.select_all_node(editor).expect("select all");
    assert_eq!(edit(&app, "editor"), (4, Some((0, 4)), None));

    assert_eq!(inputs.get(), 0, "movement dispatches no on:input");
    assert_eq!(value(&app, "editor"), "abcd");
}

#[test]
fn composition_commit_is_one_edit_and_cancel_restores_state() {
    let (mut app, inputs, editor) = editor();
    app.set_selection_node(editor, 1, 3)
        .expect("select a range");

    app.compose_start_node(editor).expect("start");
    app.compose_update_node(editor, "ni").expect("preedit");
    assert_eq!(
        edit(&app, "editor"),
        (3, None, Some(String::from("ni"))),
        "the snapshot carries the preedit"
    );
    assert_eq!(
        app.insert_text_node(editor, "x"),
        Err(DispatchError::Rejected {
            node: editor,
            operation: NodeOperation::InsertText,
            reason: "text input is composing",
        })
    );
    assert_eq!(
        app.set_caret_node(editor, 0),
        Err(DispatchError::Rejected {
            node: editor,
            operation: NodeOperation::SetCaret,
            reason: "text input is composing",
        })
    );
    app.compose_cancel_node(editor).expect("cancel");
    assert_eq!(value(&app, "editor"), "abcd", "cancel writes no model");
    assert_eq!(
        edit(&app, "editor"),
        (3, Some((1, 3)), None),
        "cancel restores the pre-edit caret and selection"
    );
    assert_eq!(inputs.get(), 0);

    app.compose_start_node(editor).expect("start again");
    app.compose_update_node(editor, "ni").expect("preedit");
    app.compose_commit_node(editor, "你").expect("commit");
    assert_eq!(value(&app, "editor"), "a你d");
    assert_eq!(edit(&app, "editor"), (2, None, None));
    assert_eq!(inputs.get(), 1, "the commit is exactly one edit");
}

#[component]
fn DisabledDuringComposition() -> MountResult<MountPlan> {
    let disabled = signal(false);
    let text = signal(String::from("ab"));
    let when = memo(move || disabled.get());

    view! {
        <text-input
            aria:label="victim"
            value={text}
            disabled={when}
            on:input={move |value: String| text.set(value)}
            on:compositionupdate={move |_event: CompositionEvent| disabled.set(true)}
        />
    }
}

#[test]
fn a_composition_can_always_be_cancelled_even_if_the_input_becomes_disabled() {
    let mut app = Runtime::new();
    app.mount(|| view! { <DisabledDuringComposition /> })
        .expect("mount");
    let victim = node(&app, "victim");

    app.set_caret_node(victim, 1).expect("caret");
    app.compose_start_node(victim).expect("start");
    app.compose_update_node(victim, "ni")
        .expect("disables the input");

    let snapshot = app.presentation().expect("snapshot");
    assert!(
        snapshot.find_label("victim").expect("the input").disabled(),
        "the input is disabled mid-composition"
    );
    assert_eq!(
        app.compose_commit_node(victim, "你"),
        Err(DispatchError::Rejected {
            node: victim,
            operation: NodeOperation::CompositionCommit,
            reason: "text input is disabled",
        }),
        "a disabled input cannot commit"
    );
    app.compose_cancel_node(victim)
        .expect("cancel still recovers");
    assert_eq!(edit(&app, "victim").2, None, "the preedit is gone");
    assert_eq!(edit(&app, "victim").0, 1, "the caret is restored");
    assert_eq!(value(&app, "victim"), "ab");
}

#[test]
fn disabled_inputs_and_wrong_kinds_reject_edits_and_composition() {
    let (mut app, inputs, _editor) = editor();
    let off = node(&app, "off");
    let plain = node(&app, "plain");

    let rejected = [
        app.insert_text_node(off, "x").map(|_| ()),
        app.backspace_node(off).map(|_| ()),
        app.delete_forward_node(off).map(|_| ()),
        app.set_caret_node(off, 1),
        app.set_selection_node(off, 0, 1),
        app.select_all_node(off),
        app.compose_start_node(off),
        app.compose_update_node(off, "x"),
        app.compose_commit_node(off, "x").map(|_| ()),
    ];
    for result in rejected {
        match result {
            Err(DispatchError::Rejected { reason, .. }) => {
                assert_eq!(reason, "text input is disabled");
            }
            other => panic!("expected a disabled rejection, got {other:?}"),
        }
    }
    assert_eq!(
        app.compose_cancel_node(off),
        Err(DispatchError::Rejected {
            node: off,
            operation: NodeOperation::CompositionCancel,
            reason: "text input is not composing",
        }),
        "cancel does not reject a disabled input; it only needs a composition"
    );
    assert_eq!(value(&app, "off"), "off", "the value is untouched");
    assert_eq!(inputs.get(), 0, "no handler ran");

    assert_eq!(
        app.insert_text_node(plain, "x"),
        Err(DispatchError::WrongKind {
            node: plain,
            operation: NodeOperation::InsertText,
        })
    );
    assert_eq!(
        app.set_caret_node(plain, 0),
        Err(DispatchError::WrongKind {
            node: plain,
            operation: NodeOperation::SetCaret,
        })
    );
    assert_eq!(
        app.compose_start_node(plain),
        Err(DispatchError::WrongKind {
            node: plain,
            operation: NodeOperation::CompositionStart,
        })
    );
}

#[test]
fn label_helpers_keep_their_whole_value_replacement() {
    let (mut app, inputs, editor) = editor();
    app.set_caret_node(editor, 2).expect("caret");

    app.input("editor", "replaced").expect("label input");
    assert_eq!(value(&app, "editor"), "replaced");
    assert_eq!(
        edit(&app, "editor").0,
        8,
        "the label helper places the caret at the end"
    );
    assert_eq!(inputs.get(), 1);

    assert!(app.set_caret("editor", 2));
    assert_eq!(edit(&app, "editor"), (2, Some((2, 2)), None));
    assert_eq!(inputs.get(), 1, "set_caret dispatches no input");
    assert!(!app.set_caret("missing", 2));
}
