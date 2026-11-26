#![allow(dead_code, unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;
use zenui_demo::*;

fn runtime() -> Runtime {
    let mut app = Runtime::new();
    open_todo_window(&mut app).expect("open window");
    app
}

fn node_with_label(app: &Runtime, label: &str) -> NodeId {
    app.presentation()
        .expect("snapshot")
        .find_label(label)
        .unwrap_or_else(|| panic!("no element labelled {label}"))
        .id()
}

fn node_with_text(app: &Runtime, text: &str) -> NodeId {
    app.presentation()
        .expect("snapshot")
        .elements()
        .find(|element| element.text() == text)
        .unwrap_or_else(|| panic!("no element with text {text}"))
        .id()
}

fn add(app: &mut Runtime, title: &str) {
    app.input("What needs to be done?", title).expect("input");
    app.submit_node(node_with_label(app, "Add task"))
        .expect("submit by node");
}

#[test]
fn snapshot_node_ids_drive_the_todo_workflow() {
    let mut app = runtime();
    add(&mut app, "Alpha");
    add(&mut app, "Beta");
    assert_eq!(app.for_items().len(), 2);

    app.change_node(node_with_label(&app, "Complete Alpha"), true)
        .expect("toggle by node");
    assert_eq!(app.checked("Complete Alpha"), Some(true));

    app.click_node(node_with_text(&app, "Active"))
        .expect("filter by node");
    assert_eq!(app.for_items().len(), 1);
    app.click_node(node_with_text(&app, "All"))
        .expect("filter all by node");
    assert_eq!(app.for_items().len(), 2);

    let before: Vec<u64> = app.for_items().iter().map(|item| item.todo.raw()).collect();
    app.click_node(node_with_text(&app, "Reverse"))
        .expect("reorder by node");
    let after: Vec<u64> = app.for_items().iter().map(|item| item.todo.raw()).collect();
    assert_eq!(
        after,
        before.iter().rev().copied().collect::<Vec<_>>(),
        "the reorder command reached the same handler"
    );

    let edit = node_with_label(&app, "Edit Alpha");
    app.focus_node(edit).expect("focus by node");
    assert_eq!(
        app.focused().and_then(|focus| focus.label),
        Some(String::from("Edit Alpha"))
    );

    app.compose_start_node(edit).expect("start by node");
    app.compose_update_node(edit, "ni")
        .expect("preedit by node");
    assert_eq!(app.preedit("Edit Alpha"), Some(String::from("ni")));
    app.compose_commit_node(edit, "你").expect("commit by node");
    assert_eq!(
        app.presentation()
            .expect("snapshot")
            .find_label("Edit Alpha")
            .expect("the editor is still committed")
            .text_value(),
        Some("你")
    );

    app.click_node(node_with_label(&app, "Delete Beta"))
        .expect("delete by node");
    assert_eq!(app.for_items().len(), 1);
    assert!(!app.texts().iter().any(|text| text == "Beta"));
}

#[component]
fn Controls(clicks: Rc<Cell<u32>>, keys: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let plain = signal(String::from("plain"));
    let status = signal(String::from("idle"));
    let draft = signal(String::from("ab"));
    let expanded = signal(true);
    let branch_clicks = signal(0);
    let when = memo(move || expanded.get());
    let status_text = memo(move || status.get());
    let branch_text = memo(move || format!("branch={}", branch_clicks.get()));
    let click_count = clicks.clone();
    let key_count = keys.clone();

    view! {
        <view role="main">
            <text aria:label="plain">{plain}</text>
            <button aria:label="go" on:click={move || {
                click_count.set(click_count.get() + 1);
            }}>"Go"</button>
            <button aria:label="dead" disabled={true}>"Dead"</button>
            <checkbox aria:label="check" on:change={move |done: bool| {
                status.set(format!("checked={done}"));
            }} />
            <text-input
                aria:label="editor"
                value={draft}
                on:input={move |value: String| draft.set(value)}
                on:keydown={move |event: &mut KeyEvent| {
                    key_count.set(key_count.get() + 1);
                    if event.key() == Key::Escape {
                        draft.set(String::new());
                    }
                }}
            />
            <text-input aria:label="silent" />
            <checkbox aria:label="silent-check" />
            <form aria:label="form" on:submit={move |_event: &mut SubmitEvent| {
                status.set(String::from("submitted"));
            }}>
                <button aria:label="inner">"Inner"</button>
            </form>
            <button aria:label="hide" on:click={move || expanded.set(!expanded.get())}>"Hide"</button>
            <Show when={when}>
                <button aria:label="branch" on:click={move || {
                    branch_clicks.set(branch_clicks.get() + 1);
                }}>"Branch"</button>
            </Show>
            <text aria:label="status">{status_text}</text>
            <text aria:label="branch-count">{branch_text}</text>
        </view>
    }
}

fn controls() -> (Runtime, Rc<Cell<u32>>, Rc<Cell<u32>>) {
    let clicks = Rc::new(Cell::new(0));
    let keys = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let clicks = clicks.clone();
        let keys = keys.clone();
        move || view! { <Controls clicks={clicks} keys={keys} /> }
    })
    .expect("mount");
    (app, clicks, keys)
}

fn node(app: &Runtime, label: &str) -> NodeId {
    node_with_label(app, label)
}

#[test]
fn wrong_kind_ids_return_explicit_errors_without_retargeting() {
    let (mut app, clicks, keys) = controls();
    let plain = node(&app, "plain");
    let go = node(&app, "go");
    let dead = node(&app, "dead");
    let check = node(&app, "check");
    let form = node(&app, "form");
    let inner = node(&app, "inner");

    assert_eq!(
        app.click_node(plain),
        Err(DispatchError::WrongKind {
            node: plain,
            operation: NodeOperation::Click,
        })
    );
    assert_eq!(
        app.change_node(go, true),
        Err(DispatchError::WrongKind {
            node: go,
            operation: NodeOperation::Change,
        })
    );
    assert_eq!(
        app.submit_node(inner),
        Err(DispatchError::WrongKind {
            node: inner,
            operation: NodeOperation::Submit,
        })
    );
    assert_eq!(
        app.keydown_node(go, Key::Escape),
        Err(DispatchError::WrongKind {
            node: go,
            operation: NodeOperation::KeyDown,
        })
    );
    assert_eq!(
        app.focus_node(plain),
        Err(DispatchError::WrongKind {
            node: plain,
            operation: NodeOperation::Focus,
        })
    );
    assert_eq!(app.focused(), None);
    assert_eq!(
        app.focus_node(dead),
        Err(DispatchError::WrongKind {
            node: dead,
            operation: NodeOperation::Focus,
        })
    );
    assert_eq!(
        app.compose_start_node(check),
        Err(DispatchError::WrongKind {
            node: check,
            operation: NodeOperation::CompositionStart,
        })
    );
    assert_eq!(
        app.compose_update_node(go, "x"),
        Err(DispatchError::WrongKind {
            node: go,
            operation: NodeOperation::CompositionUpdate,
        })
    );
    assert_eq!(
        app.compose_commit_node(go, "x"),
        Err(DispatchError::WrongKind {
            node: go,
            operation: NodeOperation::CompositionCommit,
        })
    );
    assert_eq!(
        app.compose_cancel_node(go),
        Err(DispatchError::WrongKind {
            node: go,
            operation: NodeOperation::CompositionCancel,
        })
    );
    assert_eq!(clicks.get(), 0, "no click handler ran");
    assert_eq!(keys.get(), 0, "no key handler ran");
    assert_eq!(
        app.checked("check"),
        Some(false),
        "a failed change left local state untouched"
    );
    assert_eq!(
        app.presentation()
            .expect("snapshot")
            .find_label("plain")
            .expect("the plain text is committed")
            .text(),
        "plain",
        "a failed dispatch did not retarget to another element"
    );

    app.click_node(go).expect("a legal click still works");
    assert_eq!(clicks.get(), 1);
    app.submit_node(form).expect("a legal submit still works");
    app.change_node(check, true)
        .expect("a legal change still works");
    assert_eq!(app.checked("check"), Some(true));
}

#[test]
fn unknown_and_stale_ids_are_rejected_without_retargeting() {
    let (mut app, clicks, _keys) = controls();
    let branch = node(&app, "branch");
    let hide = node(&app, "hide");

    let (other, _, _) = controls();
    let foreign = node(&other, "go");
    assert_eq!(
        app.click_node(foreign),
        Err(DispatchError::UnknownNode(foreign)),
        "an ID from another Runtime was never minted here"
    );

    app.click_node(hide).expect("hide the branch");
    assert_eq!(
        app.click_node(branch),
        Err(DispatchError::StaleNode(branch)),
        "the unmounted branch is not silently retargeted"
    );
    assert_eq!(
        app.focus_node(branch),
        Err(DispatchError::StaleNode(branch))
    );
    assert_eq!(
        app.compose_start_node(branch),
        Err(DispatchError::StaleNode(branch))
    );
    assert_eq!(app.node_status(branch), NodeStatus::Stale);
    assert!(
        app.texts().iter().any(|text| text == "branch=0"),
        "the stale button's handler never ran, and no neighbour took the click"
    );

    app.click_node(node(&app, "hide"))
        .expect("the branch returns");
    let returned = node(&app, "branch");
    assert_ne!(returned, branch, "a new mount gets a new ID");
    app.click_node(returned).expect("the new identity is live");
    assert!(app.texts().iter().any(|text| text == "branch=1"));
    assert_eq!(
        clicks.get(),
        0,
        "the go button did not receive the branch click"
    );
}

#[test]
fn composition_rejects_wrong_state_on_a_text_input() {
    let (mut app, _clicks, keys) = controls();
    let editor = node(&app, "editor");

    assert_eq!(
        app.compose_update_node(editor, "x"),
        Err(DispatchError::Rejected {
            node: editor,
            operation: NodeOperation::CompositionUpdate,
            reason: "text input is not composing",
        })
    );
    assert_eq!(
        app.compose_commit_node(editor, "x"),
        Err(DispatchError::Rejected {
            node: editor,
            operation: NodeOperation::CompositionCommit,
            reason: "text input is not composing",
        })
    );
    assert_eq!(
        app.compose_cancel_node(editor),
        Err(DispatchError::Rejected {
            node: editor,
            operation: NodeOperation::CompositionCancel,
            reason: "text input is not composing",
        })
    );

    app.focus_node(editor).expect("focus the editor");
    app.compose_start_node(editor).expect("start composing");
    assert_eq!(
        app.compose_start_node(editor),
        Err(DispatchError::Rejected {
            node: editor,
            operation: NodeOperation::CompositionStart,
            reason: "text input is already composing",
        })
    );
    app.compose_update_node(editor, "ni").expect("preedit");
    assert_eq!(app.preedit("editor"), Some(String::from("ni")));
    assert_eq!(
        keys.get(),
        0,
        "composition events do not run the keydown handler"
    );

    app.keydown_node(editor, Key::Enter)
        .expect("candidate Enter is consumed");
    assert_eq!(keys.get(), 0, "Enter never reached the user handler");
    app.keydown_node(editor, Key::Escape)
        .expect("Escape cancels the composition");
    assert_eq!(app.preedit("editor"), None);
    assert_eq!(keys.get(), 0, "the cancel path dispatches no keydown");

    app.keydown_node(editor, Key::Other)
        .expect("ordinary keys reach the handler");
    assert_eq!(keys.get(), 1);
}

#[test]
fn blur_by_node_targets_only_the_focused_identity() {
    let (mut app, _clicks, _keys) = controls();
    let editor = node(&app, "editor");
    let go = node(&app, "go");

    app.focus_node(editor).expect("focus the editor");
    assert_eq!(
        app.blur_node(go),
        Err(DispatchError::Rejected {
            node: go,
            operation: NodeOperation::Blur,
            reason: "node is not focused",
        }),
        "a blur for another identity does not retarget"
    );
    assert_eq!(
        app.focused().and_then(|focus| focus.label),
        Some(String::from("editor"))
    );

    app.blur_node(editor).expect("blur the focused editor");
    assert_eq!(app.focused(), None);

    let branch = node(&app, "branch");
    app.click_node(node(&app, "hide")).expect("hide the branch");
    assert_eq!(
        app.blur_node(branch),
        Err(DispatchError::StaleNode(branch)),
        "a stale blur is explicitly rejected"
    );
}

#[test]
fn failed_label_commands_do_not_mutate_local_state() {
    let (mut app, _clicks, _keys) = controls();

    assert_eq!(
        app.input("silent", "abc"),
        Err(MountError::Component("input target not found"))
    );
    let snapshot = app.presentation().expect("snapshot");
    let edit = snapshot
        .find_label("silent")
        .expect("the silent input")
        .edit()
        .expect("edit state");
    assert_eq!(edit.caret(), 0, "a failed input leaves the caret alone");
    assert_eq!(edit.selection(), None);

    let report = app
        .change("silent-check", true)
        .expect("an uncontrolled checkbox toggles without a handler");
    assert!(report.changed());
    assert_eq!(
        app.checked("silent-check"),
        Some(true),
        "the uncontrolled local value changes"
    );

    let report = app
        .click_label("plain")
        .expect("a routed click without a handler is a no-op turn");
    assert!(
        report.is_ok(),
        "a routed click without a handler reports no error"
    );
}

#[derive(Debug, PartialEq)]
struct Observed {
    snapshot: String,
    texts: Vec<String>,
    rows: usize,
    checked: Option<bool>,
    focused: Option<FocusInfo>,
    preedit: Option<String>,
    caret: Option<usize>,
}

#[test]
fn label_helpers_and_node_dispatch_produce_identical_results() {
    fn run(by_node: bool) -> Observed {
        let mut app = runtime();
        for title in ["Alpha", "Beta"] {
            app.input("What needs to be done?", title).expect("input");
            if by_node {
                app.submit_node(node_with_label(&app, "Add task"))
                    .expect("submit by node");
            } else {
                app.submit("Add task").expect("submit by label");
            }
        }

        if by_node {
            app.change_node(node_with_label(&app, "Complete Alpha"), true)
                .expect("change by node");
        } else {
            app.change("Complete Alpha", true).expect("change by label");
        }

        for filter in ["Active", "All", "Reverse"] {
            if by_node {
                app.click_node(node_with_text(&app, filter))
                    .expect("click by node");
            } else {
                app.click_text(filter).expect("click by label");
            }
        }

        let editor = "Edit Alpha";
        if by_node {
            let edit = node_with_label(&app, editor);
            app.focus_node(edit).expect("focus by node");
            app.compose_start_node(edit).expect("start by node");
            app.compose_update_node(edit, "ni")
                .expect("preedit by node");
            app.compose_cancel_node(edit).expect("cancel by node");
            app.click_node(node_with_label(&app, "Delete Beta"))
                .expect("delete by node");
        } else {
            app.focus(editor).expect("focus by label");
            app.compose_start(editor).expect("start by label");
            app.compose_update(editor, "ni").expect("preedit by label");
            app.compose_cancel(editor).expect("cancel by label");
            app.click_label("Delete Beta").expect("delete by label");
        }

        let snapshot = app.presentation().expect("snapshot");
        Observed {
            snapshot: snapshot.to_string(),
            texts: app.texts(),
            rows: app.for_items().len(),
            checked: app.checked("Complete Alpha"),
            focused: app.focused(),
            preedit: app.preedit(editor),
            caret: app.caret(editor),
        }
    }

    assert_eq!(run(false), run(true));
}

#[test]
fn keydown_and_focus_by_node_match_the_label_helpers() {
    fn run(by_node: bool) -> (String, Option<FocusInfo>, u32, Vec<String>) {
        let (mut app, _clicks, keys) = controls();
        if by_node {
            let editor = node(&app, "editor");
            app.focus_node(editor).expect("focus by node");
            app.keydown_node(editor, Key::Escape)
                .expect("escape by node");
            app.keydown_node(editor, Key::Other).expect("other by node");
        } else {
            app.focus("editor").expect("focus by label");
            app.keydown("editor", Key::Escape).expect("escape by label");
            app.keydown("editor", Key::Other).expect("other by label");
        }
        (
            app.presentation().expect("snapshot").to_string(),
            app.focused(),
            keys.get(),
            app.texts(),
        )
    }

    assert_eq!(run(false), run(true));
}
