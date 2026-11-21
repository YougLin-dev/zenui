#![allow(unused_braces)]

use zenui::prelude::*;

#[component]
fn Panel(
    window: WindowCtx,
    value: Signal<String>,
    task_result: Signal<String>,
) -> MountResult<MountPlan> {
    let scope = window.task_scope();
    let run_task = move || {
        let result = (|| -> Result<(), TaskError> {
            let task = scope.spawn(|_| 7_u32)?;
            task.on_done(move |value| {
                task_result.set(value.to_string());
                Ok(())
            })
        })();
        if let Err(failure) = result {
            task_result.set(failure.to_string());
        }
    };

    view! {
        <view aria:label="root">
            <text aria:label="label">{value}</text>
            <text aria:label="task-result">{task_result}</text>
            <text-input aria:label="input" value="hello" />
            <button
                aria:label="button"
                on:click={move || value.set(String::from("clicked"))}
            >"Go"</button>
            <button aria:label="task" on:click={move || run_task()}>"Task"</button>
        </view>
    }
}

#[test]
fn a_single_window_keeps_its_identity_across_frames_focus_and_tasks() {
    let mut app = Runtime::new();
    let (value, task_result) = app
        .setup(|| {
            (
                signal(String::from("initial")),
                signal(String::from("idle")),
            )
        })
        .expect("application setup");
    let shared = value;
    let result = task_result;
    let window = app
        .open_window("main", move |window| {
            view! { <Panel window={window} value={shared} task_result={result} /> }
        })
        .expect("open window");

    let first = app.presentation().expect("snapshot");
    let root = first.find_label("root").expect("root").id();
    let label = first.find_label("label").expect("label").id();
    let input = first.find_label("input").expect("input").id();
    let button = first.find_label("button").expect("button").id();
    let task = first.find_label("task").expect("task").id();
    assert_eq!(first.find_label("label").expect("label").text(), "initial");

    app.focus_node(input).expect("focus the input");
    assert_eq!(
        app.focused().and_then(|focus| focus.label),
        Some(String::from("input"))
    );

    app.click_node(button).expect("update the shared value");
    let second = app.presentation().expect("snapshot");
    assert_eq!(second.find_label("root").expect("root").id(), root);
    assert_eq!(second.find_label("label").expect("label").id(), label);
    assert_eq!(second.find_label("input").expect("input").id(), input);
    assert_eq!(second.find_label("button").expect("button").id(), button);
    assert_eq!(second.find_label("task").expect("task").id(), task);
    assert_eq!(second.find_label("label").expect("label").text(), "clicked");
    assert_eq!(
        second.focus(),
        Some(input),
        "a state refresh keeps the focused Node ID"
    );
    assert_eq!(app.node_status(root), NodeStatus::Live);

    app.click_node(task).expect("start a window-owned task");
    let report = app.settle_tasks().expect("settle the window task");
    assert_eq!(report.delivered, 1);
    let delivered = app.presentation().expect("snapshot");
    assert_eq!(
        delivered
            .find_label("task-result")
            .expect("task result")
            .text(),
        "7"
    );

    app.close_window(window).expect("close the window");
    assert_eq!(app.node_status(root), NodeStatus::Stale);
    assert!(!app.window_alive(window));
    assert_eq!(app.close_window(window), Err(WindowError::Stale));
    assert!(
        app.presentation().is_err(),
        "a closed window has no committed snapshot"
    );

    let other = app
        .open_window("other", |window| view! { <text>{window.key()}</text> })
        .expect("open another key after the close");
    let reopened = app.presentation().expect("snapshot");
    assert!(
        reopened.elements().any(|element| element.text() == "other"),
        "the table accepts the next window key after a close"
    );
    app.close_window(other).expect("close the second window");
}
