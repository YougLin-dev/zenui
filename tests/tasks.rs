#![allow(unused_braces)]

use zenui::prelude::*;

fn report(result: Result<(), TaskError>, error: Signal<String>) {
    if let Err(failure) = result {
        error.set(failure.to_string());
    }
}

#[component]
fn TaskProbe(window: WindowCtx) -> MountResult<MountPlan> {
    let todos = Store::new();
    let preview = signal(String::new());
    let error = signal(String::new());
    let count = memo(move || todos.rows(Filter::All).len());
    let count_txt = memo(move || format!("n={}", count.get()));

    let window_scope = window.task_scope();
    let app_scope = window.app_task_scope();
    let memo_rejected = memo(move || {
        matches!(
            window.task_scope().spawn(|_| 1),
            Err(TaskError::Context { .. })
        )
    });
    let memo_rejected_txt = memo(move || {
        if memo_rejected.get() {
            String::from("memo spawn rejected")
        } else {
            String::from("memo spawn allowed")
        }
    });

    let window_task = move || {
        let result = (|| -> Result<(), TaskError> {
            let task = window_scope.spawn(|_| String::from("window result"))?;
            task.on_done(move |value| {
                preview.set(value);
                Ok(())
            })
        })();
        report(result, error);
    };
    let slow_window_task = move || {
        let result = (|| -> Result<(), TaskError> {
            let task = window_scope.spawn(|token| {
                while !token.is_cancelled() {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                String::from("late window result")
            })?;
            task.on_done(move |value| {
                preview.set(value);
                Ok(())
            })
        })();
        report(result, error);
    };
    let app_task = move || {
        let result = (|| -> Result<(), TaskError> {
            let task = app_scope.spawn(|_| vec![String::from("Shared")])?;
            task.on_done(move |titles| {
                let mut transaction = Transaction::new();
                for title in titles {
                    transaction = transaction.write(todos.insert(title));
                }
                transaction.finish()?;
                Ok(())
            })
        })();
        report(result, error);
    };
    let bad_app_task = move || {
        let result = (|| -> Result<(), TaskError> {
            let task = app_scope.spawn(|_| String::from("bad"))?;
            task.on_done(move |_| {
                preview.with(|_| {
                    Transaction::new()
                        .write(todos.insert("must not commit"))
                        .finish()
                        .map_err(MountError::from)
                })
            })
        })();
        report(result, error);
    };
    let disposed_app_task = move || {
        let result = (|| -> Result<(), TaskError> {
            let task = app_scope.spawn(|_| String::from("too late"))?;
            task.on_done(move |value| {
                preview.set(value);
                Ok(())
            })
        })();
        report(result, error);
    };
    let with_disposed_app_task = move || {
        let result = (|| -> Result<(), TaskError> {
            let task = app_scope.spawn(|_| String::from("too late"))?;
            task.on_done(move |_| preview.with(|_| Ok(())))
        })();
        report(result, error);
    };

    view! {
        <view role="main">
            <text>{count_txt}</text>
            <text>{memo_rejected_txt}</text>
            <text>{preview}</text>
            <text>{error}</text>
            <button on:click={move || window_task()}>"Window task"</button>
            <button on:click={move || slow_window_task()}>"Slow window task"</button>
            <button on:click={move || app_task()}>"App task"</button>
            <button on:click={move || bad_app_task()}>"Bad app task"</button>
            <button on:click={move || disposed_app_task()}>"Disposed app task"</button>
            <button on:click={move || with_disposed_app_task()}>"With disposed app task"</button>
        </view>
    }
}

#[component]
fn FailingInner() -> MountResult<MountPlan> {
    let _ = signal(1);
    Err(MountError::Component("inner"))
}

#[component]
fn FaultingBinding() -> MountResult<MountPlan> {
    let state = signal(0);
    let bad = memo(move || {
        state.set(1);
        String::from("bad binding")
    });

    view! { <text>{bad}</text> }
}

fn open(app: &mut Runtime) -> WindowCtx {
    app.open_window("main", |window| view! { <TaskProbe window={window} /> })
        .expect("open window")
}

#[test]
fn a_window_owned_task_delivers_inside_its_activation() {
    let mut app = Runtime::new();
    let window = open(&mut app);

    app.click_text("Window task").expect("spawn");
    let report = app.settle_tasks().expect("settle");

    assert_eq!(report.delivered, 1);
    assert_eq!(report.discarded, 0);
    assert!(app.texts().iter().any(|text| text == "window result"));
    assert!(
        app.texts().iter().any(|text| text == "memo spawn rejected"),
        "spawning from a Memo is rejected before a worker starts"
    );
    assert!(app.window_alive(window));
}

#[test]
fn drive_tasks_polls_ready_results_without_blocking() {
    let mut app = Runtime::new();
    let window = open(&mut app);

    let task = app.spawn_task(window.task_scope(), |_| 9).expect("spawn");
    app.task_done(task, |_| Ok(())).expect("continuation");

    let delivered = (0..1000).find_map(|_| {
        let report = app.drive_tasks().expect("poll");
        if report.delivered > 0 {
            Some(report.delivered)
        } else {
            std::thread::sleep(std::time::Duration::from_millis(1));
            None
        }
    });
    assert_eq!(delivered, Some(1));
    assert_eq!(app.task_status(task), Some(TaskStatus::Delivered));
}

#[test]
fn an_app_continuation_writing_a_disposed_window_signal_is_rejected() {
    let mut app = Runtime::new();
    let window = open(&mut app);

    app.click_text("Disposed app task").expect("spawn");
    app.close_window(window).expect("close");

    let error = app
        .settle_tasks()
        .expect_err("writing a disposed signal is rejected");
    assert!(matches!(
        error,
        MountError::Context {
            operation: "Signal::set",
            ..
        }
    ));
    assert!(!app.texts().iter().any(|text| text == "too late"));
}

#[test]
fn a_panicking_read_scope_does_not_poison_later_delivery() {
    let mut app = Runtime::new();
    let window = open(&mut app);

    app.click_text("With disposed app task").expect("spawn");
    app.close_window(window).expect("close");
    let error = app
        .settle_tasks()
        .expect_err("reading a disposed signal panics");
    assert!(matches!(error, MountError::Delivery(_)));

    let _second = open(&mut app);
    app.click_text("Window task").expect("window task");
    let report = app
        .settle_tasks()
        .expect("the read-only scope was released with the panic");
    assert_eq!(report.delivered, 1);
    assert!(app.texts().iter().any(|text| text == "window result"));
}

#[test]
fn a_panicking_worker_is_visible_and_never_delivers() {
    let mut app = Runtime::new();
    let window = open(&mut app);

    let task = app
        .spawn_task(window.task_scope(), |_| panic!("worker boom"))
        .expect("spawn");
    app.wait_for_tasks();
    assert_eq!(app.task_status(task), Some(TaskStatus::Panicked));

    let report = app.drive_tasks().expect("nothing to deliver");
    assert_eq!(report.delivered, 0);
    assert_eq!(app.task_status(task), Some(TaskStatus::Panicked));
}

#[test]
fn a_panicking_continuation_is_reported_and_restores_the_context() {
    let mut app = Runtime::new();
    let window = open(&mut app);

    let task = app.spawn_task(window.task_scope(), |_| 3).expect("spawn");
    app.task_done(task, |_| panic!("continuation boom"))
        .expect("continuation");

    let error = app
        .settle_tasks()
        .expect_err("a continuation panic is a delivery error");
    assert!(matches!(error, MountError::Delivery(_)));
    assert_eq!(app.task_status(task), Some(TaskStatus::Panicked));

    app.click_text("Window task")
        .expect("a later event still runs");
    let report = app.settle_tasks().expect("later delivery");
    assert_eq!(report.delivered, 1);
}

#[test]
fn completion_is_distinct_from_delivery_and_a_late_continuation_runs() {
    let mut app = Runtime::new();
    let window = open(&mut app);

    let task = app.spawn_task(window.task_scope(), |_| 7).expect("spawn");
    app.wait_for_tasks();
    assert_eq!(
        app.task_status(task),
        Some(TaskStatus::Completed),
        "worker completion is not UI delivery"
    );
    let report = app.drive_tasks().expect("no continuation yet");
    assert_eq!(report.delivered, 0);

    app.task_done(task, |value| {
        assert_eq!(value, 7);
        Ok(())
    })
    .expect("late continuation");
    let report = app.drive_tasks().expect("deliver the buffered result");
    assert_eq!(report.delivered, 1);
    assert_eq!(app.task_status(task), Some(TaskStatus::Delivered));
}

#[test]
fn closing_a_window_cancels_window_owned_delivery() {
    let mut app = Runtime::new();
    let window = open(&mut app);

    app.click_text("Slow window task").expect("spawn");
    app.close_window(window).expect("close");
    let report = app.settle_tasks().expect("settle");

    assert_eq!(report.delivered, 0);
    assert_eq!(report.discarded, 1);
    assert!(
        !app.texts().iter().any(|text| text == "late window result"),
        "a cancelled delivery never reaches the interface"
    );
    assert!(!app.window_alive(window));
    assert!(matches!(app.close_window(window), Err(WindowError::Stale)));
    assert!(matches!(
        app.spawn_task(window.task_scope(), |_| 1),
        Err(TaskError::TargetClosed)
    ));
}

#[test]
fn reopening_uses_a_new_activation_and_discards_the_old_delivery() {
    let mut app = Runtime::new();
    let first = open(&mut app);
    let stale_scope = first.task_scope();

    app.click_text("Slow window task").expect("spawn");
    app.close_window(first).expect("close");
    let second = open(&mut app);
    assert_eq!(second.activation(), first.activation() + 1);
    assert_eq!(app.setup_count("TaskProbe"), 2, "reopen remounts the UI");

    let report = app.settle_tasks().expect("settle");
    assert_eq!(report.delivered, 0);
    assert_eq!(report.discarded, 1);
    assert!(
        !app.texts().iter().any(|text| text == "late window result"),
        "the old delivery must not hit the new interface generation"
    );
    assert!(!app.window_alive(first));
    assert!(app.window_alive(second));
    assert!(matches!(
        app.spawn_task(stale_scope, |_| 2),
        Err(TaskError::TargetClosed)
    ));
    assert!(matches!(app.close_window(first), Err(WindowError::Stale)));
}

#[test]
fn an_app_owned_task_commits_the_shared_model_after_the_window_closes() {
    let mut app = Runtime::new();
    let first = open(&mut app);

    app.click_text("App task").expect("spawn");
    app.close_window(first).expect("close");
    let report = app.settle_tasks().expect("settle");
    assert_eq!(report.delivered, 1, "app delivery survives window close");

    let second = open(&mut app);
    assert!(app.window_alive(second));
    assert!(
        app.texts().iter().any(|text| text == "n=1"),
        "the reopened interface reads the committed model"
    );
    assert_eq!(app.setup_count("TaskProbe"), 2);
}

#[test]
fn delivery_obeys_context_checks_and_later_delivery_recovers() {
    let mut app = Runtime::new();
    let _window = open(&mut app);

    app.click_text("Bad app task").expect("spawn");
    let error = app
        .settle_tasks()
        .expect_err("a read-only delivery must be rejected");
    assert!(matches!(
        error,
        MountError::Write(WriteError::Context { .. })
    ));
    assert!(
        app.texts().iter().any(|text| text == "n=0"),
        "the rejected delivery committed nothing"
    );

    app.click_text("App task").expect("spawn");
    let report = app.settle_tasks().expect("a later legal delivery runs");
    assert_eq!(report.delivered, 1);
    assert!(app.texts().iter().any(|text| text == "n=1"));
}

#[test]
fn unsupported_abilities_and_stale_windows_are_clear_errors() {
    let mut app = Runtime::new();
    let first = open(&mut app);

    assert!(matches!(
        app.open_window("second", |_| Ok(MountPlan::new(Vec::new()))),
        Err(WindowError::Unsupported(_))
    ));

    app.close_window(first).expect("close");
    let second = app
        .open_window("second", |_| Ok(MountPlan::new(Vec::new())))
        .expect("a new key opens after the previous window closed");
    assert!(app.window_alive(second));
    assert!(!app.window_alive(first));
}

#[test]
fn task_status_tracks_completion_and_delivery() {
    let mut app = Runtime::new();
    let window = open(&mut app);

    let task = app.spawn_task(window.task_scope(), |_| 5).expect("spawn");
    assert_eq!(app.task_status(task), Some(TaskStatus::Running));
    app.task_done(task, |value| {
        assert_eq!(value, 5);
        Ok(())
    })
    .expect("continuation");

    let report = app.settle_tasks().expect("settle");
    assert_eq!(report.delivered, 1);
    assert_eq!(app.task_status(task), Some(TaskStatus::Delivered));
}

#[test]
fn a_closed_window_cancels_status_and_rejects_late_registration() {
    let mut app = Runtime::new();
    let window = open(&mut app);

    let task = app.spawn_task(window.task_scope(), |_| 5).expect("spawn");
    app.close_window(window).expect("close");
    assert_eq!(app.task_status(task), Some(TaskStatus::Cancelled));
    assert!(matches!(
        app.task_done(task, |_| Ok(())),
        Err(TaskError::TargetClosed)
    ));

    let report = app.settle_tasks().expect("settle");
    assert_eq!(report.discarded, 1);
    assert_eq!(app.task_status(task), Some(TaskStatus::Stale));
}

#[test]
fn handles_from_another_runtime_are_clear_errors() {
    let mut first = Runtime::new();
    let mut second = Runtime::new();
    let first_window = open(&mut first);
    let second_window = open(&mut second);

    assert!(matches!(
        first.close_window(second_window),
        Err(WindowError::WrongRuntime)
    ));
    assert!(!first.window_alive(second_window));
    assert!(matches!(
        first.spawn_task(second_window.task_scope(), |_| 1),
        Err(TaskError::WrongRuntime)
    ));

    let foreign = second
        .spawn_task(second.task_scope(), |_| 2)
        .expect("foreign task");
    assert!(matches!(
        first.task_done(foreign, |_| Ok(())),
        Err(TaskError::WrongRuntime)
    ));
    assert_eq!(first.task_status(foreign), None);
    assert!(first.window_alive(first_window));

    first.close_window(first_window).expect("close");
    let foreign_scope = second.task_scope();
    let foreign_task = second
        .spawn_task(second.task_scope(), |_| 3)
        .expect("foreign app task");
    let scope_rejected = std::rc::Rc::new(std::cell::Cell::new(false));
    let done_rejected = std::rc::Rc::new(std::cell::Cell::new(false));
    let scope_flag = std::rc::Rc::clone(&scope_rejected);
    let done_flag = std::rc::Rc::clone(&done_rejected);
    first
        .open_window("cross", move |_| {
            scope_flag.set(matches!(
                foreign_scope.spawn(|_| 1),
                Err(TaskError::WrongRuntime)
            ));
            done_flag.set(matches!(
                foreign_task.on_done(|_| Ok(())),
                Err(TaskError::WrongRuntime)
            ));
            Ok(MountPlan::new(Vec::new()))
        })
        .expect("cross-runtime factory does not panic");
    assert!(scope_rejected.get());
    assert!(done_rejected.get());
}

#[test]
fn a_failing_window_factory_releases_resources_and_can_retry() {
    let mut app = Runtime::new();

    let failure = app.open_window("main", |_| {
        let created = signal(1);
        created.set(2);
        Err(MountError::Component("boom"))
    });
    assert!(matches!(
        failure,
        Err(WindowError::Mount(MountError::Component("boom")))
    ));
    assert_eq!(app.live_signals(), 0, "the proposed window is released");
    assert!(app.texts().is_empty());

    let window = app
        .open_window("main", |_| Ok(MountPlan::new(Vec::new())))
        .expect("retry");
    assert_eq!(
        window.activation(),
        2,
        "a failed open still consumes a generation"
    );
    assert!(app.window_alive(window));
}

#[test]
fn a_failing_child_mount_aborts_the_window_plan_and_can_retry() {
    let mut app = Runtime::new();

    let failure = app.open_window("main", |_| view! { <FailingInner /> });
    assert!(matches!(
        failure,
        Err(WindowError::Mount(MountError::Component("inner")))
    ));
    assert_eq!(app.live_signals(), 0);

    let window = app
        .open_window("main", |_| Ok(MountPlan::new(Vec::new())))
        .expect("retry");
    assert_eq!(window.activation(), 2);
}

#[test]
fn a_binding_fault_aborts_the_window_plan_and_can_retry() {
    let mut app = Runtime::new();

    let failure = app.open_window("main", |_| view! { <FaultingBinding /> });
    assert!(matches!(
        failure,
        Err(WindowError::Mount(MountError::Context {
            operation: "Signal::set",
            ..
        }))
    ));
    assert_eq!(app.live_signals(), 0);

    let window = app
        .open_window("main", |_| Ok(MountPlan::new(Vec::new())))
        .expect("retry after the fault is cleared");
    assert_eq!(window.activation(), 2);
}

#[test]
fn a_failed_window_factory_cancels_tasks_started_in_its_setup() {
    use std::cell::Cell;
    use std::rc::Rc;

    let mut app = Runtime::new();
    let started: Rc<Cell<Option<Task<()>>>> = Rc::new(Cell::new(None));
    let slot = Rc::clone(&started);

    let failure = app.open_window("main", move |window| {
        let task = window
            .app_task_scope()
            .spawn(|token| {
                while !token.is_cancelled() {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            })
            .expect("spawn in factory");
        slot.set(Some(task));
        Err(MountError::Component("boom"))
    });
    assert!(matches!(
        failure,
        Err(WindowError::Mount(MountError::Component("boom")))
    ));

    let task = started.get().expect("factory task");
    assert_eq!(app.task_status(task), Some(TaskStatus::Cancelled));
    app.settle_tasks()
        .expect("the cancelled setup task is discarded");
    assert_eq!(app.task_status(task), Some(TaskStatus::Stale));
}
