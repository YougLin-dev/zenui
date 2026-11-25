#![allow(dead_code, unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;

#[component]
fn ResetEditor(inputs: Rc<Cell<u32>>) -> MountResult<MountPlan> {
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
            >
                "External"
            </button>
            <button aria:label="short" on:click={move || draft.set(String::from("xy"))}>
                "Short"
            </button>
            <button aria:label="clear" on:click={move || draft.set(String::new())}>
                "Clear"
            </button>
            <button
                aria:label="cjk-long"
                on:click={move || draft.set(String::from("你好世界🙂"))}
            >
                "CJK long"
            </button>
            <button aria:label="cjk-short" on:click={move || draft.set(String::from("你好"))}>
                "CJK short"
            </button>
        </view>
    }
}

fn editor() -> (Runtime, Rc<Cell<u32>>, NodeId) {
    let inputs = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let inputs = inputs.clone();
        move || view! { <ResetEditor inputs={inputs} /> }
    })
    .expect("mount");
    let node = app
        .presentation()
        .expect("snapshot")
        .find_label("editor")
        .expect("editor")
        .id();
    (app, inputs, node)
}

/// caret、anchor、selection 与 preedit；全部以字符计。
fn edit_state(
    app: &Runtime,
    label: &str,
) -> (usize, usize, Option<(usize, usize)>, Option<String>) {
    let snapshot = app.presentation().expect("snapshot");
    let element = snapshot.find_label(label).expect("the input");
    let edit = element.edit().expect("edit");
    (
        edit.caret(),
        edit.anchor(),
        edit.selection(),
        edit.preedit().map(str::to_owned),
    )
}

fn text_of(app: &Runtime, label: &str) -> String {
    app.presentation()
        .expect("snapshot")
        .find_label(label)
        .expect("the input")
        .text_value()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn an_in_range_selection_survives_an_external_reset() {
    let (mut app, inputs, editor) = editor();
    // 先制造一个撤销步并回退，让 undo 与 redo 两个栈都非空。
    app.set_caret_node(editor, 4).expect("caret at the end");
    app.insert_text_node(editor, "X").expect("edit");
    app.undo_node(editor).expect("populate the redo stack");
    assert_eq!(inputs.get(), 2);
    app.set_selection_node(editor, 1, 3).expect("selection");
    app.click_label("external").expect("external reset");

    assert_eq!(text_of(&app, "editor"), "external");
    assert_eq!(
        edit_state(&app, "editor"),
        (3, 1, Some((1, 3)), None),
        "a selection wholly inside the new value is preserved"
    );
    assert_eq!(inputs.get(), 2, "an external reset dispatches no on:input");

    app.undo_node(editor)
        .expect("the reset cleared the undo stack");
    app.redo_node(editor)
        .expect("the reset cleared the redo stack");
    assert_eq!(inputs.get(), 2, "neither stack replays an edit");
}

#[test]
fn an_overflowing_selection_and_caret_collapse_to_the_end() {
    let (mut app, _inputs, editor) = editor();
    app.click_label("external").expect("seed a longer value");
    app.set_caret_node(editor, 6).expect("caret near the end");
    app.click_label("short").expect("shrink the value");
    assert_eq!(text_of(&app, "editor"), "xy");
    assert_eq!(
        edit_state(&app, "editor"),
        (2, 2, None, None),
        "an off-boundary caret clamps to the character length"
    );

    app.click_label("external").expect("grow again");
    app.set_selection_node(editor, 2, 4).expect("selection");
    app.click_label("short")
        .expect("shrink under the selection");
    assert_eq!(
        edit_state(&app, "editor"),
        (2, 2, None, None),
        "a selection extending past the end collapses to the end"
    );
}

#[test]
fn an_empty_clear_zeroes_the_offsets() {
    let (mut app, _inputs, editor) = editor();
    app.set_selection_node(editor, 1, 3).expect("selection");
    app.click_label("clear").expect("clear the value");

    assert_eq!(text_of(&app, "editor"), "");
    assert_eq!(edit_state(&app, "editor"), (0, 0, None, None));
}

#[test]
fn an_external_reset_during_composition_cancels_the_preedit() {
    let (mut app, inputs, editor) = editor();
    // 组合前先有一个本地编辑，让重置必须清空它的历史。
    app.set_caret_node(editor, 4).expect("caret at the end");
    app.insert_text_node(editor, "X").expect("edit");
    assert_eq!(inputs.get(), 1);
    app.set_selection_node(editor, 1, 3).expect("selection");
    app.compose_start_node(editor).expect("start composing");
    app.compose_update_node(editor, "ni").expect("preedit");
    assert_eq!(edit_state(&app, "editor").3.as_deref(), Some("ni"));

    app.click_label("external")
        .expect("reset the value while composing");
    assert_eq!(text_of(&app, "editor"), "external");
    assert_eq!(
        edit_state(&app, "editor"),
        (8, 8, None, None),
        "the preedit is discarded and the caret moves to the end"
    );
    assert_eq!(
        inputs.get(),
        1,
        "cancelling the composition dispatches no on:input"
    );

    app.undo_node(editor).expect("no history was kept");
    assert_eq!(inputs.get(), 1);
}

#[test]
fn reset_offsets_stay_on_character_boundaries_for_cjk_and_emoji() {
    let (mut app, _inputs, editor) = editor();
    app.click_label("cjk-long").expect("five-character value");
    assert_eq!(text_of(&app, "editor"), "你好世界🙂");
    app.set_selection_node(editor, 3, 5)
        .expect("selection over the emoji");
    app.click_label("external").expect("ascii reset");
    assert_eq!(
        edit_state(&app, "editor"),
        (5, 3, Some((3, 5)), None),
        "character offsets survive a longer replacement"
    );

    app.click_label("cjk-short")
        .expect("shrink to two characters");
    assert_eq!(text_of(&app, "editor"), "你好");
    assert_eq!(edit_state(&app, "editor"), (2, 2, None, None));
}

#[test]
fn the_explicit_input_path_places_the_caret_at_the_end() {
    let (mut app, inputs, editor) = editor();
    app.set_selection_node(editor, 1, 3).expect("selection");
    app.input("editor", "你好🙂").expect("explicit replacement");

    assert_eq!(text_of(&app, "editor"), "你好🙂");
    assert_eq!(edit_state(&app, "editor"), (3, 3, None, None));
    assert_eq!(inputs.get(), 1, "the explicit path still dispatches once");
}

#[test]
fn an_external_change_while_hidden_applies_on_show() {
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

    app.set_caret_node(a_input, 0).expect("caret in a");
    app.insert_text_node(a_input, "A").expect("edit in a");
    app.set_selection_node(a_input, 1, 3)
        .expect("selection in a");
    app.hide(a).expect("hide a");

    app.set_caret_node(b_input, 5).expect("caret in b");
    app.insert_text_node(b_input, "B").expect("edit in b");
    let a = app.show("a").expect("show a again");

    let snapshot = app.presentation_of(a).expect("a snapshot");
    let field = snapshot.find_label("a-input").expect("a input");
    assert_eq!(field.text_value(), Some("AabcdB"));
    let edit = field.edit().expect("edit");
    assert_eq!((edit.caret(), edit.anchor()), (3, 1));
    assert_eq!(edit.selection(), Some((1, 3)));
    assert_eq!(counts[0].get(), 1, "the hidden editor recorded no input");

    app.undo_node(a_input)
        .expect("the external change cleared a's history");
    assert_eq!(counts[0].get(), 1);
}
