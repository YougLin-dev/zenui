#![allow(unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;

#[component]
fn Editor(
    window: WindowCtx,
    shared: Signal<String>,
    task_result: Signal<String>,
) -> MountResult<MountPlan> {
    let draft = signal(String::from("start"));
    let scope = window.task_scope();
    let run_task = move || {
        if let Ok(task) = scope.spawn(|_| String::from("task-done")) {
            let _ = task.on_done(move |value| {
                task_result.set(value);
                Ok(())
            });
        }
    };

    view! {
        <view>
            <text aria:label="shared">{shared}</text>
            <text aria:label="task-result">{task_result}</text>
            <text-input
                aria:label="draft"
                value={draft}
                on:input={move |value: String| draft.set(value)}
            />
            <button
                aria:label="update"
                on:click={move || shared.set(String::from("updated"))}
            >"Update"</button>
            <button aria:label="task" on:click={move || run_task()}>"Task"</button>
        </view>
    }
}

#[test]
fn hide_preserves_state_and_rejects_input_until_show() {
    let mut app = Runtime::new();
    let (shared, task_result) = app
        .setup(|| {
            (
                signal(String::from("initial")),
                signal(String::from("idle")),
            )
        })
        .expect("application setup");
    let runs = Rc::new(Cell::new(0));
    let counter = runs.clone();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            counter.set(counter.get() + 1);
            view! { <Editor window={window} shared={shared} task_result={task_result} /> }
        })?;
        Ok(())
    })
    .expect("catalog");

    let main = app.show("main").expect("show main");
    assert_eq!(runs.get(), 1);
    let snapshot = app.presentation_of(main).expect("snapshot");
    let draft = snapshot.find_label("draft").expect("draft input").id();
    let task = snapshot.find_label("task").expect("task button").id();

    app.focus_node(draft).expect("focus the draft");
    app.input("draft", "typed").expect("edit the draft");
    assert_eq!(app.caret("draft"), Some(5));
    app.set_selection_node(draft, 1, 3)
        .expect("select part of the draft");
    app.click_node(task).expect("start a window task");

    app.hide(main).expect("hide main");
    assert!(!app.window_visible(main));
    assert!(app.window_alive(main), "hide keeps the activation");
    assert_eq!(app.presentation_of(main), Err(WindowError::Hidden));
    assert!(
        matches!(
            app.presentation(),
            Err(MountError::Component(
                "presentation snapshot requires a visible committed interface"
            ))
        ),
        "a hidden window is not presented"
    );
    assert_eq!(app.click_node(draft), Err(DispatchError::Hidden(draft)));
    assert_eq!(app.focus_node(draft), Err(DispatchError::Hidden(draft)));
    assert_eq!(
        app.keydown_node(draft, Key::Other),
        Err(DispatchError::Hidden(draft))
    );
    assert_eq!(
        app.compose_start_node(draft),
        Err(DispatchError::Hidden(draft))
    );
    assert_eq!(
        app.input("draft", "blocked"),
        Err(MountError::Component("target window is hidden"))
    );
    assert_eq!(
        app.click_label("update"),
        Err(MountError::Component("target window is hidden"))
    );
    assert!(!app.set_caret("draft", 0), "hide rejects caret changes");
    assert_eq!(
        app.set_selection_node(draft, 0, 1),
        Err(DispatchError::Hidden(draft))
    );
    assert_eq!(
        app.node_status(draft),
        NodeStatus::Live,
        "hide keeps the tree mounted"
    );
    assert_eq!(app.caret("draft"), Some(3), "hide keeps the caret");
    assert_eq!(
        app.presentation_of(main),
        Err(WindowError::Hidden),
        "hidden local state is not presented"
    );

    app.hide(main).expect("hide is idempotent");
    assert!(!app.window_visible(main));
    let report = app.settle_tasks().expect("deliver while hidden");
    assert_eq!(report.delivered, 1, "hide preserves window-owned tasks");

    let restored = app.show("main").expect("show restores the activation");
    assert_eq!(restored.activation(), main.activation());
    assert_eq!(runs.get(), 1, "show does not rerun the factory");
    assert!(app.window_visible(main));
    let snapshot = app.presentation_of(main).expect("restored snapshot");
    assert_eq!(
        snapshot.find_label("draft").expect("draft input").id(),
        draft,
        "Node IDs survive hide/show"
    );
    assert_eq!(snapshot.focus(), Some(draft), "focus is restored");
    assert_eq!(
        snapshot
            .find_label("draft")
            .expect("draft input")
            .text_value(),
        Some("typed"),
        "the draft is restored"
    );
    assert_eq!(
        snapshot
            .find_label("draft")
            .expect("draft input")
            .edit()
            .expect("edit state")
            .selection(),
        Some((1, 3)),
        "the selection is restored"
    );
    assert_eq!(app.caret("draft"), Some(3));
    assert_eq!(
        snapshot
            .find_label("task-result")
            .expect("task result")
            .text(),
        "task-done"
    );

    app.input("draft", "again").expect("input after show");
    app.click_label("update").expect("click after show");
    let snapshot = app.presentation_of(main).expect("snapshot after input");
    assert_eq!(
        snapshot
            .find_label("draft")
            .expect("draft input")
            .text_value(),
        Some("again"),
        "show restores input dispatch"
    );
    assert_eq!(
        snapshot.find_label("shared").expect("shared text").text(),
        "updated"
    );

    let again = app.show("main").expect("show is idempotent");
    assert_eq!(again.activation(), main.activation());
    assert_eq!(runs.get(), 1);

    app.hide(main).expect("hide before close");
    app.close_window(main).expect("close a hidden window");
    assert_eq!(app.hide(main), Err(WindowError::Stale));
    let reopened = app.show("main").expect("reopen after close");
    assert_eq!(reopened.activation(), main.activation() + 1);
    assert_eq!(runs.get(), 2, "reopen runs the factory again");
    let snapshot = app.presentation_of(reopened).expect("fresh snapshot");
    assert_ne!(
        snapshot.find_label("draft").expect("draft input").id(),
        draft,
        "a fresh factory run mints new Node IDs"
    );
    assert_eq!(
        snapshot
            .find_label("draft")
            .expect("draft input")
            .text_value(),
        Some("start"),
        "the reopened window starts from fresh local state"
    );
}

#[test]
fn hiding_one_window_keeps_the_other_visible_and_focus_independent() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("a").primary(), |_| {
            let draft = signal(String::from("a-draft"));
            view! {
                <text-input
                    aria:label="a-input"
                    value={draft}
                    on:input={move |value: String| draft.set(value)}
                />
            }
        })?;
        catalog.window(WindowSpec::new("b"), |_| {
            let draft = signal(String::from("b-draft"));
            view! {
                <text-input
                    aria:label="b-input"
                    value={draft}
                    on:input={move |value: String| draft.set(value)}
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
    app.focus_node(a_input).expect("focus a");
    app.focus_node(b_input).expect("focus b");
    app.input("a-input", "a-typed").expect("edit a");
    app.input("b-input", "b-typed").expect("edit b");

    app.hide(b).expect("hide b");
    assert!(!app.window_visible(b));
    assert!(app.window_visible(a));
    assert_eq!(
        app.presentation_of(a).expect("a snapshot").focus(),
        Some(a_input),
        "hiding b does not disturb a"
    );
    assert_eq!(app.presentation_of(b), Err(WindowError::Hidden));
    assert_eq!(
        app.focused_window().map(|window| window.key()),
        Some("a"),
        "focus falls back to the only visible window"
    );

    app.hide(a).expect("hide a");
    assert!(app.focused_window().is_none());
    assert!(app.presentation().is_err());

    app.show("b").expect("restore b");
    let snapshot = app.presentation_of(b).expect("b snapshot");
    assert_eq!(snapshot.focus(), Some(b_input));
    assert_eq!(
        snapshot
            .find_label("b-input")
            .expect("b input")
            .text_value(),
        Some("b-typed")
    );
    assert!(!app.window_visible(a));

    app.show("a").expect("restore a");
    let snapshot = app.presentation_of(a).expect("a snapshot");
    assert_eq!(snapshot.focus(), Some(a_input));
    assert_eq!(
        snapshot
            .find_label("a-input")
            .expect("a input")
            .text_value(),
        Some("a-typed")
    );
}
