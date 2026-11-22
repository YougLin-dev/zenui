#![allow(unused_braces)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use zenui::prelude::*;

type Pickers = Rc<RefCell<Vec<Task<FileDialogOutcome>>>>;
type Outcomes = Rc<RefCell<Vec<FileDialogOutcome>>>;
type Errors = Rc<RefCell<Vec<WindowError>>>;
type CtxSlot = Rc<RefCell<Option<WindowCtx>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickerScope {
    Window,
    App,
}

#[component]
fn PickerProbe(
    window: WindowCtx,
    pickers: Pickers,
    errors: Errors,
    ctx: CtxSlot,
    scope: PickerScope,
) -> MountResult<MountPlan> {
    let pick = move || {
        ctx.borrow_mut().replace(window);
        let scope = match scope {
            PickerScope::Window => window.task_scope(),
            PickerScope::App => window.app_task_scope(),
        };
        match window.request_file_dialog(options(), scope) {
            Ok(task) => pickers.borrow_mut().push(task),
            Err(error) => errors.borrow_mut().push(error),
        }
    };
    view! {
        <view role="main">
            <button aria:label="Pick" on:click={move || pick()}>"Pick"</button>
        </view>
    }
}

#[component]
fn CloseAndPick(window: WindowCtx, pickers: Pickers, errors: Errors) -> MountResult<MountPlan> {
    let act = move || {
        let _ = window.request_close();
        match window.request_file_dialog(options(), window.task_scope()) {
            Ok(task) => pickers.borrow_mut().push(task),
            Err(error) => errors.borrow_mut().push(error),
        }
    };
    view! {
        <view role="main">
            <button aria:label="Close and pick" on:click={move || act()}>
                "Close and pick"
            </button>
        </view>
    }
}

fn options() -> FileDialogOptions {
    FileDialogOptions::open_files()
        .titled("Import tasks")
        .filter("Text files", ["txt"])
}

fn open_picker(app: &mut Runtime, scope: PickerScope) -> (WindowCtx, Pickers, Errors, CtxSlot) {
    let pickers: Pickers = Rc::new(RefCell::new(Vec::new()));
    let errors: Errors = Rc::new(RefCell::new(Vec::new()));
    let ctx: CtxSlot = Rc::new(RefCell::new(None));
    let factory_pickers = pickers.clone();
    let factory_errors = errors.clone();
    let factory_ctx = ctx.clone();
    let main = app
        .open_window("main", move |window| {
            let pickers = factory_pickers.clone();
            let errors = factory_errors.clone();
            let ctx = factory_ctx.clone();
            view! {
                <PickerProbe
                    window={window}
                    pickers={pickers}
                    errors={errors}
                    ctx={ctx}
                    scope={scope}
                />
            }
        })
        .expect("open the picker window");
    (main, pickers, errors, ctx)
}

fn open_catalog_picker(app: &mut Runtime, scope: PickerScope) -> (Pickers, Errors, CtxSlot) {
    let pickers: Pickers = Rc::new(RefCell::new(Vec::new()));
    let errors: Errors = Rc::new(RefCell::new(Vec::new()));
    let ctx: CtxSlot = Rc::new(RefCell::new(None));
    let factory_pickers = pickers.clone();
    let factory_errors = errors.clone();
    let factory_ctx = ctx.clone();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            let pickers = factory_pickers.clone();
            let errors = factory_errors.clone();
            let ctx = factory_ctx.clone();
            view! {
                <PickerProbe
                    window={window}
                    pickers={pickers}
                    errors={errors}
                    ctx={ctx}
                    scope={scope}
                />
            }
        })?;
        Ok(())
    })
    .expect("declare the picker catalog");
    (pickers, errors, ctx)
}

fn click_pick(app: &mut Runtime, pickers: &Pickers) {
    app.click_label("Pick").expect("click Pick");
    assert_eq!(
        pickers.borrow().len(),
        1,
        "exactly one picker was requested"
    );
}

fn last_task(pickers: &Pickers) -> Task<FileDialogOutcome> {
    *pickers.borrow().last().expect("a picker task")
}

fn take_one(app: &mut Runtime) -> FileDialogRequest {
    let mut requests = app.take_file_dialog_requests();
    assert_eq!(requests.len(), 1, "one dialog request is queued");
    requests.remove(0)
}

fn collect(app: &mut Runtime, task: Task<FileDialogOutcome>, outcomes: &Outcomes) {
    let slot = outcomes.clone();
    app.task_done(task, move |outcome| {
        slot.borrow_mut().push(outcome);
        Ok(())
    })
    .expect("register the file dialog continuation");
}

fn new_outcomes() -> Outcomes {
    Rc::new(RefCell::new(Vec::new()))
}

#[component]
fn KindProbe(window: WindowCtx, pickers: Pickers, errors: Errors) -> MountResult<MountPlan> {
    let single_pickers = pickers.clone();
    let single_errors = errors.clone();
    let open_one = move || {
        request_dialog(
            window,
            FileDialogOptions::open_file()
                .titled("Open one")
                .filter("Text files", ["txt"]),
            &single_pickers,
            &single_errors,
        );
    };

    let open_pickers = pickers.clone();
    let open_errors = errors.clone();
    let open = move || {
        request_dialog(
            window,
            FileDialogOptions::open_files()
                .titled("Open tasks")
                .filter("Text files", ["txt"]),
            &open_pickers,
            &open_errors,
        );
    };

    let save_pickers = pickers.clone();
    let save_errors = errors.clone();
    let save = move || {
        request_dialog(
            window,
            FileDialogOptions::save_file()
                .titled("Save tasks")
                .filter("Text files", [".txt"])
                .with_suggested_name("tasks")
                .with_default_extension(".txt"),
            &save_pickers,
            &save_errors,
        );
    };

    let folder_pickers = pickers.clone();
    let folder_errors = errors.clone();
    let folder = move || {
        request_dialog(
            window,
            FileDialogOptions::pick_folder()
                .titled("Pick folder")
                .filter("Ignored", ["txt"])
                .with_suggested_name("ignored")
                .with_default_extension(".ignored"),
            &folder_pickers,
            &folder_errors,
        );
    };

    view! {
        <view role="main">
            <button aria:label="Open file" on:click={move || open_one()}>"Open file"</button>
            <button aria:label="Open files" on:click={move || open()}>"Open files"</button>
            <button aria:label="Save" on:click={move || save()}>"Save"</button>
            <button aria:label="Folder" on:click={move || folder()}>"Folder"</button>
        </view>
    }
}

fn request_dialog(
    window: WindowCtx,
    options: FileDialogOptions,
    pickers: &Pickers,
    errors: &Errors,
) {
    match window.request_file_dialog(options, window.app_task_scope()) {
        Ok(task) => pickers.borrow_mut().push(task),
        Err(error) => errors.borrow_mut().push(error),
    }
}

fn open_kind_probe(app: &mut Runtime) -> (Pickers, Errors) {
    let pickers: Pickers = Rc::new(RefCell::new(Vec::new()));
    let errors: Errors = Rc::new(RefCell::new(Vec::new()));
    let factory_pickers = pickers.clone();
    let factory_errors = errors.clone();
    app.open_window("main", move |window| {
        let pickers = factory_pickers.clone();
        let errors = factory_errors.clone();
        view! { <KindProbe window={window} pickers={pickers} errors={errors} /> }
    })
    .expect("open the kind probe");
    (pickers, errors)
}

#[test]
fn a_pick_queues_one_running_request() {
    let mut app = Runtime::new();
    let (main, pickers, errors, _) = open_picker(&mut app, PickerScope::App);
    click_pick(&mut app, &pickers);
    assert!(errors.borrow().is_empty());

    let task = last_task(&pickers);
    assert_eq!(app.task_status(task), Some(TaskStatus::Running));

    let request = take_one(&mut app);
    assert_eq!(request.anchor(), main);
    assert!(request.allows_multiple());
    assert_eq!(request.options().title(), "Import tasks");
    assert_eq!(request.options().filters()[0].name(), "Text files");
    assert_eq!(
        request.options().filters()[0]
            .extensions()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["txt"]
    );
    assert!(app.take_file_dialog_requests().is_empty());
}

#[test]
fn a_pending_pick_survives_drive_and_settle() {
    let mut app = Runtime::new();
    let (_, pickers, _, _) = open_picker(&mut app, PickerScope::App);
    click_pick(&mut app, &pickers);
    let task = last_task(&pickers);
    let outcomes = new_outcomes();
    collect(&mut app, task, &outcomes);

    app.drive_tasks().expect("drive without results");
    app.settle_tasks().expect("settle workers only");
    assert_eq!(
        app.task_status(task),
        Some(TaskStatus::Running),
        "a host task has no receiver and must not be treated as a dead worker"
    );

    let request = take_one(&mut app);
    app.complete_file_dialog_request(
        request,
        FileDialogOutcome::Picked(vec![PathBuf::from("tasks.txt")]),
    )
    .expect("complete the picker");
    assert_eq!(app.task_status(task), Some(TaskStatus::Completed));

    app.drive_tasks().expect("deliver the picker");
    assert_eq!(app.task_status(task), Some(TaskStatus::Delivered));
    assert_eq!(
        *outcomes.borrow(),
        vec![FileDialogOutcome::Picked(vec![PathBuf::from("tasks.txt")])]
    );
}

#[test]
fn completion_before_on_done_is_buffered() {
    let mut app = Runtime::new();
    let (_, pickers, _, _) = open_picker(&mut app, PickerScope::App);
    click_pick(&mut app, &pickers);
    let task = last_task(&pickers);
    let outcomes = new_outcomes();

    let request = take_one(&mut app);
    app.complete_file_dialog_request(request, FileDialogOutcome::Cancelled)
        .expect("complete before registering");
    assert_eq!(app.task_status(task), Some(TaskStatus::Completed));
    app.drive_tasks().expect("nothing is registered yet");
    assert!(outcomes.borrow().is_empty());

    collect(&mut app, task, &outcomes);
    app.drive_tasks().expect("deliver the buffered value");
    assert_eq!(app.task_status(task), Some(TaskStatus::Delivered));
    assert_eq!(*outcomes.borrow(), vec![FileDialogOutcome::Cancelled]);
}

#[test]
fn a_window_owned_pick_is_cancelled_when_its_window_closes() {
    let mut app = Runtime::new();
    let (main, pickers, _, _) = open_picker(&mut app, PickerScope::Window);
    click_pick(&mut app, &pickers);
    let task = last_task(&pickers);
    let outcomes = new_outcomes();
    collect(&mut app, task, &outcomes);

    app.close_window(main).expect("close the picker window");
    assert_eq!(app.task_status(task), Some(TaskStatus::Cancelled));
    assert!(
        app.take_file_dialog_requests().is_empty(),
        "a request for a closed activation is not handed to the host"
    );

    app.drive_tasks().expect("deliver nothing");
    assert!(outcomes.borrow().is_empty());
}

#[test]
fn completing_a_window_owned_request_after_close_is_stale() {
    let mut app = Runtime::new();
    let (main, pickers, _, _) = open_picker(&mut app, PickerScope::Window);
    click_pick(&mut app, &pickers);
    let task = last_task(&pickers);
    let outcomes = new_outcomes();
    collect(&mut app, task, &outcomes);
    let request = take_one(&mut app);

    app.close_window(main).expect("close the picker window");
    assert_eq!(
        app.complete_file_dialog_request(
            request,
            FileDialogOutcome::Picked(vec![PathBuf::from("late.txt")])
        ),
        Err(FileDialogRequestError::Stale)
    );
    assert_eq!(app.task_status(task), Some(TaskStatus::Cancelled));
    app.drive_tasks().expect("deliver nothing");
    assert!(outcomes.borrow().is_empty());
}

#[test]
fn an_app_owned_pick_delivers_after_the_anchor_closes() {
    let mut app = Runtime::new();
    let (pickers, errors, _) = open_catalog_picker(&mut app, PickerScope::App);
    let main = app.show("main").expect("show the picker");
    click_pick(&mut app, &pickers);
    let task = last_task(&pickers);
    let outcomes = new_outcomes();
    collect(&mut app, task, &outcomes);
    let request = take_one(&mut app);

    app.close_window(main).expect("close the picker window");
    assert_eq!(
        app.task_status(task),
        Some(TaskStatus::Running),
        "an app-owned picker survives its anchor"
    );
    app.complete_file_dialog_request(
        request,
        FileDialogOutcome::Picked(vec![PathBuf::from("shared.txt")]),
    )
    .expect("complete the app-owned picker");
    app.drive_tasks().expect("deliver the picker");

    assert_eq!(
        *outcomes.borrow(),
        vec![FileDialogOutcome::Picked(vec![PathBuf::from("shared.txt")])]
    );
    let reopened = app.show("main").expect("reopen the picker");
    assert_ne!(reopened.activation(), main.activation());
    assert!(
        app.take_file_dialog_requests().is_empty(),
        "an old selection is not redirected to the reopened window"
    );
    assert!(errors.borrow().is_empty());
}

#[test]
fn an_app_owned_pick_queued_before_close_completes_with_anchor_closed() {
    let mut app = Runtime::new();
    let (main, pickers, _, _) = open_picker(&mut app, PickerScope::App);
    click_pick(&mut app, &pickers);
    let task = last_task(&pickers);
    let outcomes = new_outcomes();
    collect(&mut app, task, &outcomes);

    app.close_window(main).expect("close the picker window");
    assert!(app.take_file_dialog_requests().is_empty());
    assert_eq!(app.task_status(task), Some(TaskStatus::Completed));

    app.drive_tasks().expect("deliver the anchor-closed value");
    assert_eq!(app.task_status(task), Some(TaskStatus::Delivered));
    assert_eq!(
        *outcomes.borrow(),
        vec![FileDialogOutcome::Failed(FileDialogError::AnchorClosed)]
    );
}

#[test]
fn a_hidden_anchor_is_rejected_from_a_delivery_handler() {
    let mut app = Runtime::new();
    let (main, _, errors, _) = open_picker(&mut app, PickerScope::App);
    app.hide(main).expect("hide the picker window");

    let slot = errors.clone();
    let task = app
        .spawn_task(app.task_scope(), |_| ())
        .expect("spawn a delivery driver");
    app.task_done(task, move |_| {
        slot.borrow_mut().push(
            main.request_file_dialog(options(), main.task_scope())
                .expect_err("a hidden window cannot own a picker"),
        );
        Ok(())
    })
    .expect("register the driver");
    app.settle_tasks().expect("run the driver");

    assert_eq!(errors.borrow().as_slice(), [WindowError::Hidden]);
}

#[test]
fn a_suspended_parent_rejects_a_picker() {
    let mut app = Runtime::new();
    let main = zenui_demo::open_todo_window(&mut app).expect("open the demo");
    zenui_demo::seed(&mut app).expect("seed the demo");
    app.input("What needs to be done?", "draft")
        .expect("type a draft");
    app.request_close(main).expect("request the close");
    assert!(
        !app.window_accepts_input(main),
        "the parent is suspended behind the confirmation"
    );

    let errors: Errors = Rc::new(RefCell::new(Vec::new()));
    let slot = errors.clone();
    let task = app
        .spawn_task(app.task_scope(), |_| ())
        .expect("spawn a delivery driver");
    app.task_done(task, move |_| {
        slot.borrow_mut().push(
            main.request_file_dialog(options(), main.task_scope())
                .expect_err("a suspended parent cannot own a picker"),
        );
        Ok(())
    })
    .expect("register the driver");
    app.settle_tasks().expect("run the driver");

    assert_eq!(
        errors.borrow().as_slice(),
        [WindowError::State("window input is suspended")]
    );
}

#[component]
fn PhaseProbe(window: WindowCtx, errors: Errors) -> MountResult<MountPlan> {
    if let Err(error) = window.request_file_dialog(options(), window.task_scope()) {
        errors.borrow_mut().push(error);
    }
    let memo_error = memo(move || {
        window
            .request_file_dialog(options(), window.task_scope())
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default()
    });
    let text = memo(move || memo_error.get());
    view! { <text>{text}</text> }
}

#[test]
fn factory_and_memo_phases_are_rejected() {
    let mut app = Runtime::new();
    let errors: Errors = Rc::new(RefCell::new(Vec::new()));
    let factory_errors = errors.clone();
    app.open_window("main", move |window| {
        let errors = factory_errors.clone();
        view! { <PhaseProbe window={window} errors={errors} /> }
    })
    .expect("open the phase probe");
    app.refresh().expect("compute the memo");

    let errors = errors.borrow();
    assert_eq!(errors.len(), 1, "the factory attempt is rejected");
    assert!(matches!(errors[0], WindowError::State(_)));
    assert!(errors[0].to_string().contains("event or delivery"));
    assert!(
        app.texts()
            .iter()
            .any(|text| text.contains("event or delivery")),
        "the memo attempt is rejected"
    );
}

#[test]
fn a_foreign_runtime_picker_is_rejected_without_panicking() {
    let mut first = Runtime::new();
    let (first_main, _, _, _) = open_picker(&mut first, PickerScope::App);

    let mut second = Runtime::new();
    let errors: Errors = Rc::new(RefCell::new(Vec::new()));
    let factory_errors = errors.clone();
    second
        .open_window("main", move |_| {
            let errors = factory_errors.clone();
            let foreign =
                move || match first_main.request_file_dialog(options(), first_main.task_scope()) {
                    Ok(_) => {}
                    Err(error) => errors.borrow_mut().push(error),
                };
            view! { <button aria:label="Foreign" on:click={move || foreign()}>"Foreign"</button> }
        })
        .expect("open the second runtime");
    second.click_label("Foreign").expect("click Foreign");

    assert_eq!(errors.borrow().as_slice(), [WindowError::WrongRuntime]);
}

#[test]
fn a_scope_from_another_window_is_rejected() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| {
            view! { <text>"main"</text> }
        })?;
        catalog.window(WindowSpec::new("secondary"), |_| {
            view! { <text>"secondary"</text> }
        })?;
        Ok(())
    })
    .expect("declare both windows");
    let main = app.show("main").expect("show main");
    let secondary = app.show("secondary").expect("show secondary");

    let errors: Errors = Rc::new(RefCell::new(Vec::new()));
    let slot = errors.clone();
    let task = app
        .spawn_task(app.task_scope(), |_| ())
        .expect("spawn a delivery driver");
    app.task_done(task, move |_| {
        slot.borrow_mut().push(
            main.request_file_dialog(options(), secondary.task_scope())
                .expect_err("a window scope must name the same anchor"),
        );
        Ok(())
    })
    .expect("register the driver");
    app.settle_tasks().expect("run the driver");

    assert!(
        matches!(errors.borrow().as_slice(), [WindowError::State(_)]),
        "the mismatched scope is rejected"
    );
}

#[test]
fn a_stale_picker_handle_is_rejected() {
    let mut app = Runtime::new();
    let (main, _, errors, _) = open_picker(&mut app, PickerScope::App);
    app.close_window(main).expect("close the picker window");

    let slot = errors.clone();
    let task = app
        .spawn_task(app.task_scope(), |_| ())
        .expect("spawn a delivery driver");
    app.task_done(task, move |_| {
        slot.borrow_mut().push(
            main.request_file_dialog(options(), main.task_scope())
                .expect_err("a closed activation is stale"),
        );
        Ok(())
    })
    .expect("register the driver");
    app.settle_tasks().expect("run the driver");

    assert_eq!(errors.borrow().as_slice(), [WindowError::Stale]);
}

#[test]
fn two_requests_are_independent_and_fifo() {
    let mut app = Runtime::new();
    let (_, pickers, _, _) = open_picker(&mut app, PickerScope::App);
    app.click_label("Pick").expect("first pick");
    app.click_label("Pick").expect("second pick");
    assert_eq!(pickers.borrow().len(), 2);
    let first = *pickers.borrow().first().expect("first task");
    let second = *pickers.borrow().last().expect("second task");
    let outcomes = new_outcomes();
    collect(&mut app, first, &outcomes);
    collect(&mut app, second, &outcomes);

    let mut requests = app.take_file_dialog_requests();
    assert_eq!(requests.len(), 2);
    let second_request = requests.remove(1);
    let first_request = requests.remove(0);
    app.complete_file_dialog_request(
        first_request,
        FileDialogOutcome::Picked(vec![PathBuf::from("first.txt")]),
    )
    .expect("complete the first");
    app.complete_file_dialog_request(
        second_request,
        FileDialogOutcome::Picked(vec![PathBuf::from("second.txt")]),
    )
    .expect("complete the second");
    app.drive_tasks().expect("deliver both");

    assert_eq!(
        *outcomes.borrow(),
        vec![
            FileDialogOutcome::Picked(vec![PathBuf::from("first.txt")]),
            FileDialogOutcome::Picked(vec![PathBuf::from("second.txt")]),
        ]
    );
    assert_eq!(app.task_status(first), Some(TaskStatus::Delivered));
    assert_eq!(app.task_status(second), Some(TaskStatus::Delivered));
}

#[test]
fn a_continuation_can_queue_another_pick() {
    let mut app = Runtime::new();
    let (_, pickers, errors, ctx) = open_picker(&mut app, PickerScope::App);
    click_pick(&mut app, &pickers);
    let first = last_task(&pickers);
    let outcomes = new_outcomes();
    let outcomes_slot = outcomes.clone();
    let second_outcomes = new_outcomes();
    let second_outcomes_slot = second_outcomes.clone();
    let ctx_slot = ctx.clone();
    let pickers_slot = pickers.clone();
    let errors_slot = errors.clone();
    app.task_done(first, move |first_outcome| {
        outcomes_slot.borrow_mut().push(first_outcome);
        let window = ctx_slot.borrow().expect("the picker window");
        match window.request_file_dialog(options(), window.app_task_scope()) {
            Ok(task) => {
                pickers_slot.borrow_mut().push(task);
                let nested_outcomes = second_outcomes_slot.clone();
                task.on_done(move |outcome| {
                    nested_outcomes.borrow_mut().push(outcome);
                    Ok(())
                })
                .map_err(|_| MountError::Component("register the nested picker"))?;
            }
            Err(error) => errors_slot.borrow_mut().push(error),
        }
        Ok(())
    })
    .expect("register the first continuation");

    let first_request = take_one(&mut app);
    app.complete_file_dialog_request(first_request, FileDialogOutcome::Cancelled)
        .expect("complete the first");
    app.drive_tasks().expect("run the first continuation");

    assert_eq!(*outcomes.borrow(), vec![FileDialogOutcome::Cancelled]);
    assert_eq!(
        pickers.borrow().len(),
        2,
        "the first continuation queued a pick"
    );
    let second_request = take_one(&mut app);
    app.complete_file_dialog_request(second_request, FileDialogOutcome::Cancelled)
        .expect("complete the second");
    app.drive_tasks().expect("deliver the second");
    assert_eq!(
        *second_outcomes.borrow(),
        vec![FileDialogOutcome::Cancelled]
    );
}

#[test]
fn take_and_complete_are_safe_at_top_level() {
    let mut app = Runtime::new();
    let (_, pickers, _, _) = open_picker(&mut app, PickerScope::App);
    click_pick(&mut app, &pickers);
    let task = last_task(&pickers);
    let request = take_one(&mut app);
    app.complete_file_dialog_request(
        request,
        FileDialogOutcome::Picked(vec![PathBuf::from("top.txt")]),
    )
    .expect("complete outside any installed turn");
    let outcomes = new_outcomes();
    collect(&mut app, task, &outcomes);
    app.drive_tasks().expect("deliver");

    assert_eq!(
        *outcomes.borrow(),
        vec![FileDialogOutcome::Picked(vec![PathBuf::from("top.txt")])]
    );
    assert!(app.take_file_dialog_requests().is_empty());
}

#[test]
fn a_close_and_pick_in_one_turn_settles_the_picker() {
    let mut app = Runtime::new();
    let pickers: Pickers = Rc::new(RefCell::new(Vec::new()));
    let errors: Errors = Rc::new(RefCell::new(Vec::new()));
    let factory_pickers = pickers.clone();
    let factory_errors = errors.clone();
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |window| {
            let pickers = factory_pickers.clone();
            let errors = factory_errors.clone();
            view! { <CloseAndPick window={window} pickers={pickers} errors={errors} /> }
        })?;
        Ok(())
    })
    .expect("declare the close-and-pick window");
    app.show("main").expect("show the close-and-pick window");
    app.click_label("Close and pick").expect("close and pick");
    app.apply_window_requests().expect("apply the close");

    assert_eq!(pickers.borrow().len(), 1);
    assert!(errors.borrow().is_empty());
    let task = last_task(&pickers);
    assert_eq!(
        app.task_status(task),
        Some(TaskStatus::Cancelled),
        "the close wins over the same-turn pick"
    );
    assert!(
        app.take_file_dialog_requests().is_empty(),
        "no dialog is requested for the closed activation"
    );
}

#[test]
fn completing_a_request_on_another_runtime_is_rejected() {
    let mut first = Runtime::new();
    let (_, pickers, _, _) = open_picker(&mut first, PickerScope::App);
    click_pick(&mut first, &pickers);
    let request = take_one(&mut first);

    let mut second = Runtime::new();
    assert_eq!(
        second.complete_file_dialog_request(request, FileDialogOutcome::Cancelled),
        Err(FileDialogRequestError::WrongRuntime)
    );
}

#[test]
fn dropping_a_runtime_with_a_pending_request_does_not_panic() {
    let mut app = Runtime::new();
    let (_, pickers, _, _) = open_picker(&mut app, PickerScope::App);
    click_pick(&mut app, &pickers);
    let request = take_one(&mut app);
    drop(app);
    drop(request);
}

#[test]
fn mixed_kind_requests_keep_queued_options_and_fifo_order() {
    let mut app = Runtime::new();
    let (pickers, errors) = open_kind_probe(&mut app);
    app.click_label("Open file").expect("single open request");
    app.click_label("Open files")
        .expect("multiple open request");
    app.click_label("Save").expect("save request");
    app.click_label("Folder").expect("folder request");
    assert!(errors.borrow().is_empty());

    let requests = app.take_file_dialog_requests();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.options().kind())
            .collect::<Vec<_>>(),
        [
            FileDialogKind::OpenFile,
            FileDialogKind::OpenFiles,
            FileDialogKind::SaveFile,
            FileDialogKind::PickFolder,
        ]
    );

    let open_one = requests[0].options();
    assert_eq!(open_one.title(), "Open one");
    assert_eq!(open_one.filters()[0].name(), "Text files");
    assert_eq!(open_one.suggested_name(), "");
    assert_eq!(open_one.default_extension(), "");

    let open = requests[1].options();
    assert_eq!(open.title(), "Open tasks");
    assert_eq!(open.filters()[0].name(), "Text files");
    assert_eq!(open.suggested_name(), "");
    assert_eq!(open.default_extension(), "");

    let save = requests[2].options();
    assert_eq!(save.title(), "Save tasks");
    assert_eq!(save.filters()[0].name(), "Text files");
    assert_eq!(save.suggested_name(), "tasks");
    assert_eq!(save.default_extension(), "txt");

    let folder = requests[3].options();
    assert_eq!(folder.title(), "Pick folder");
    assert!(folder.filters().is_empty());
    assert_eq!(folder.suggested_name(), "");
    assert_eq!(folder.default_extension(), "");

    assert_eq!(
        requests
            .iter()
            .map(FileDialogRequest::allows_multiple)
            .collect::<Vec<_>>(),
        [false, true, false, false],
        "only open-multiple expects more than one path"
    );
    assert_eq!(pickers.borrow().len(), 4);
}

#[test]
fn save_and_folder_requests_survive_drive_and_settle() {
    let mut app = Runtime::new();
    let (pickers, errors) = open_kind_probe(&mut app);
    app.click_label("Save").expect("save request");
    app.click_label("Folder").expect("folder request");
    assert!(errors.borrow().is_empty());

    let save = pickers.borrow()[0];
    let folder = pickers.borrow()[1];
    let outcomes = new_outcomes();
    collect(&mut app, save, &outcomes);
    collect(&mut app, folder, &outcomes);

    app.drive_tasks().expect("drive without results");
    app.settle_tasks().expect("settle workers only");
    assert_eq!(
        app.task_status(save),
        Some(TaskStatus::Running),
        "a save request has no receiver and is not a dead worker"
    );
    assert_eq!(app.task_status(folder), Some(TaskStatus::Running));

    let mut requests = app.take_file_dialog_requests();
    assert_eq!(requests.len(), 2);
    let folder_request = requests.remove(1);
    let save_request = requests.remove(0);
    assert_eq!(save_request.options().kind(), FileDialogKind::SaveFile);
    assert_eq!(folder_request.options().kind(), FileDialogKind::PickFolder);
    app.complete_file_dialog_request(
        save_request,
        FileDialogOutcome::Picked(vec![PathBuf::from("tasks.txt")]),
    )
    .expect("complete the save");
    app.complete_file_dialog_request(folder_request, FileDialogOutcome::Cancelled)
        .expect("complete the folder");
    app.drive_tasks().expect("deliver both");

    assert_eq!(
        *outcomes.borrow(),
        vec![
            FileDialogOutcome::Picked(vec![PathBuf::from("tasks.txt")]),
            FileDialogOutcome::Cancelled,
        ]
    );
    assert_eq!(app.task_status(save), Some(TaskStatus::Delivered));
    assert_eq!(app.task_status(folder), Some(TaskStatus::Delivered));
}
