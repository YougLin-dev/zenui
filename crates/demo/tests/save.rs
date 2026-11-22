#![allow(unused_braces)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use zenui::prelude::*;
use zenui_demo::{DETAILS_WINDOW, open_todo_window, seed};

fn open_seeded() -> (Runtime, WindowCtx) {
    let mut app = Runtime::new();
    let main = open_todo_window(&mut app).expect("open the demo windows");
    seed(&mut app).expect("seed the todo rows");
    (app, main)
}

fn open_empty() -> (Runtime, WindowCtx) {
    let mut app = Runtime::new();
    let main = open_todo_window(&mut app).expect("open the demo windows");
    (app, main)
}

fn take_request(app: &mut Runtime, label: &str) -> FileDialogRequest {
    app.click_text(label).expect("click the dialog action");
    let mut requests = app.take_file_dialog_requests();
    assert_eq!(requests.len(), 1, "one dialog request is queued");
    requests.remove(0)
}

fn complete(app: &mut Runtime, request: FileDialogRequest, outcome: FileDialogOutcome) {
    app.complete_file_dialog_request(request, outcome)
        .expect("complete the dialog request");
    app.settle_tasks().expect("settle the save task");
}

fn details_window(app: &Runtime) -> WindowCtx {
    app.windows()
        .into_iter()
        .find(|info| info.spec.key() == DETAILS_WINDOW)
        .map(|info| info.window)
        .expect("the details window is live")
}

fn row_titles(app: &Runtime) -> Vec<String> {
    app.presentation()
        .expect("a window snapshot")
        .elements()
        .filter_map(SnapshotElement::label)
        .filter_map(|label| label.strip_prefix("Edit ").map(str::to_owned))
        .collect()
}

fn notice(app: &Runtime) -> String {
    app.presentation()
        .expect("a window snapshot")
        .find_label("Notice")
        .map(|element| element.text().to_owned())
        .unwrap_or_default()
}

fn error_text(app: &Runtime) -> String {
    app.presentation()
        .expect("a window snapshot")
        .find_label("Error")
        .map(|element| element.text().to_owned())
        .unwrap_or_default()
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "zenui-demo-save-{}-{}-{label}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("create the temp dir");
        Self { path }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read the saved file")
}

#[test]
fn save_uses_the_documented_options_and_writes_every_committed_title() {
    let temp = TempDir::new("options");
    let path = temp.file("tasks.txt");
    let (mut app, _main) = open_seeded();
    app.click_text("Done").expect("filter to completed rows");
    assert_eq!(
        row_titles(&app),
        ["Write tests"],
        "the completed filter is active"
    );

    let request = take_request(&mut app, "Save tasks");
    let options = request.options();
    assert_eq!(options.kind(), FileDialogKind::SaveFile);
    assert_eq!(options.title(), "Save tasks");
    assert_eq!(options.filters()[0].name(), "Text files");
    assert_eq!(
        options.filters()[0]
            .extensions()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["txt"]
    );
    assert_eq!(options.suggested_name(), "tasks.txt");
    assert_eq!(options.default_extension(), "txt");

    complete(
        &mut app,
        request,
        FileDialogOutcome::Picked(vec![path.clone()]),
    );
    assert_eq!(read(&path), "Write tests\nRead docs\n");
    assert_eq!(notice(&app), "Saved 2 tasks");
    assert!(error_text(&app).is_empty());
}

#[test]
fn save_keeps_committed_order_and_writes_one_newline_per_title() {
    let temp = TempDir::new("order");
    let path = temp.file("tasks.txt");
    let (mut app, _main) = open_seeded();
    app.click_text("Reverse")
        .expect("reverse the committed rows");
    app.input("Edit Write tests", "").expect("clear one title");

    let request = take_request(&mut app, "Save tasks");
    complete(
        &mut app,
        request,
        FileDialogOutcome::Picked(vec![path.clone()]),
    );
    assert_eq!(read(&path), "Read docs\n\n");
    assert_eq!(notice(&app), "Saved 2 tasks");
}

#[test]
fn save_rejects_line_breaks_before_touching_the_destination() {
    let temp = TempDir::new("linebreak");
    let path = temp.file("tasks.txt");
    std::fs::write(&path, "old contents").expect("seed the destination");
    let (mut app, _main) = open_seeded();
    let folder = take_request(&mut app, "Choose folder");
    complete(
        &mut app,
        folder,
        FileDialogOutcome::Picked(vec![temp.path.clone()]),
    );
    assert_eq!(notice(&app), format!("Folder: {}", temp.path.display()));
    app.input("Edit Write tests", "bad\nline")
        .expect("set a title with a line feed");

    let request = take_request(&mut app, "Save tasks");
    complete(
        &mut app,
        request,
        FileDialogOutcome::Picked(vec![path.clone()]),
    );

    assert_eq!(read(&path), "old contents", "the destination is untouched");
    assert!(error_text(&app).contains("line break"));
    assert!(notice(&app).is_empty());
}

#[test]
fn save_writes_an_empty_file_for_an_empty_list() {
    let temp = TempDir::new("empty");
    let path = temp.file("tasks.txt");
    let (mut app, _main) = open_empty();

    let request = take_request(&mut app, "Save tasks");
    complete(
        &mut app,
        request,
        FileDialogOutcome::Picked(vec![path.clone()]),
    );

    assert_eq!(read(&path), "");
    assert_eq!(notice(&app), "Saved 0 tasks");
}

#[test]
fn save_success_reports_one_task_and_clears_a_previous_error() {
    let temp = TempDir::new("singular");
    let path = temp.file("tasks.txt");
    let (mut app, _main) = open_empty();
    app.input("What needs to be done?", "Only task")
        .expect("add one task");
    app.submit("Add task").expect("submit the task");

    let failed = take_request(&mut app, "Choose folder");
    complete(
        &mut app,
        failed,
        FileDialogOutcome::Failed(FileDialogError::DialogFailed(String::from("folder failed"))),
    );
    assert!(error_text(&app).contains("folder failed"));

    let request = take_request(&mut app, "Save tasks");
    complete(
        &mut app,
        request,
        FileDialogOutcome::Picked(vec![path.clone()]),
    );

    assert_eq!(read(&path), "Only task\n");
    assert_eq!(notice(&app), "Saved 1 task");
    assert!(error_text(&app).is_empty(), "success clears the error");
}

#[test]
fn save_overwrites_an_existing_file_and_leaves_the_list_unchanged() {
    let temp = TempDir::new("overwrite");
    let path = temp.file("tasks.txt");
    std::fs::write(&path, "old contents").expect("seed the destination");
    let (mut app, _main) = open_seeded();
    let before = row_titles(&app);

    let request = take_request(&mut app, "Save tasks");
    complete(
        &mut app,
        request,
        FileDialogOutcome::Picked(vec![path.clone()]),
    );

    assert_eq!(read(&path), "Write tests\nRead docs\n");
    assert_eq!(row_titles(&app), before);
}

#[test]
fn save_reports_an_unwritable_destination_and_clears_the_notice() {
    let temp = TempDir::new("unwritable");
    let path = temp.file("missing").join("tasks.txt");
    let (mut app, _main) = open_seeded();
    let folder = take_request(&mut app, "Choose folder");
    complete(
        &mut app,
        folder,
        FileDialogOutcome::Picked(vec![temp.path.clone()]),
    );
    assert!(!notice(&app).is_empty());

    let request = take_request(&mut app, "Save tasks");
    complete(&mut app, request, FileDialogOutcome::Picked(vec![path]));

    assert!(error_text(&app).contains("Save failed:"));
    assert!(notice(&app).is_empty());
    assert_eq!(row_titles(&app), ["Write tests", "Read docs"]);
}

#[test]
fn save_dialog_failure_clears_the_notice_and_keeps_the_list() {
    let temp = TempDir::new("dialog-failure");
    let (mut app, _main) = open_seeded();
    let folder = take_request(&mut app, "Choose folder");
    complete(
        &mut app,
        folder,
        FileDialogOutcome::Picked(vec![temp.path.clone()]),
    );
    assert!(!notice(&app).is_empty());
    let before = row_titles(&app);

    let request = take_request(&mut app, "Save tasks");
    complete(
        &mut app,
        request,
        FileDialogOutcome::Failed(FileDialogError::DialogFailed(String::from("save failed"))),
    );

    assert!(error_text(&app).contains("save failed"));
    assert!(notice(&app).is_empty());
    assert_eq!(row_titles(&app), before);
}

#[test]
fn save_dialog_returning_no_path_reports_an_error_without_writing() {
    let temp = TempDir::new("save-empty-pick");
    let (mut app, _main) = open_seeded();
    let folder = take_request(&mut app, "Choose folder");
    complete(
        &mut app,
        folder,
        FileDialogOutcome::Picked(vec![temp.path.clone()]),
    );
    assert!(!notice(&app).is_empty());

    let request = take_request(&mut app, "Save tasks");
    complete(&mut app, request, FileDialogOutcome::Picked(Vec::new()));

    assert!(error_text(&app).contains("no path"));
    assert!(notice(&app).is_empty());
    assert_eq!(row_titles(&app), ["Write tests", "Read docs"]);
}

#[test]
fn folder_dialog_returning_no_path_reports_an_error_and_clears_the_notice() {
    let temp = TempDir::new("folder-empty-pick");
    let (mut app, _main) = open_seeded();
    let picked = take_request(&mut app, "Choose folder");
    complete(
        &mut app,
        picked,
        FileDialogOutcome::Picked(vec![temp.path.clone()]),
    );
    assert!(!notice(&app).is_empty());

    let empty = take_request(&mut app, "Choose folder");
    complete(&mut app, empty, FileDialogOutcome::Picked(Vec::new()));

    assert!(error_text(&app).contains("no path"));
    assert!(notice(&app).is_empty());
}

#[test]
fn save_cancel_leaves_the_previous_notice_and_error_unchanged() {
    let temp = TempDir::new("cancel");
    let path = temp.file("tasks.txt");
    let (mut app, _main) = open_seeded();
    let first = take_request(&mut app, "Save tasks");
    complete(&mut app, first, FileDialogOutcome::Picked(vec![path]));

    let notice_before = notice(&app);
    let error_before = error_text(&app);
    let cancelled = take_request(&mut app, "Save tasks");
    complete(&mut app, cancelled, FileDialogOutcome::Cancelled);

    assert!(!notice_before.is_empty());
    assert_eq!(notice(&app), notice_before);
    assert_eq!(error_text(&app), error_before);

    let failed = take_request(&mut app, "Choose folder");
    complete(
        &mut app,
        failed,
        FileDialogOutcome::Failed(FileDialogError::DialogFailed(String::from("folder failed"))),
    );
    let notice_before = notice(&app);
    let error_before = error_text(&app);
    let cancelled = take_request(&mut app, "Save tasks");
    complete(&mut app, cancelled, FileDialogOutcome::Cancelled);

    assert!(!error_before.is_empty());
    assert_eq!(notice(&app), notice_before);
    assert_eq!(error_text(&app), error_before);
}

#[test]
fn folder_uses_the_documented_options_and_reports_the_path() {
    let temp = TempDir::new("folder");
    let (mut app, _main) = open_seeded();

    let request = take_request(&mut app, "Choose folder");
    assert_eq!(request.options().kind(), FileDialogKind::PickFolder);
    assert_eq!(request.options().title(), "Choose a folder");
    assert!(request.options().filters().is_empty());

    complete(
        &mut app,
        request,
        FileDialogOutcome::Picked(vec![temp.path.clone()]),
    );
    assert_eq!(notice(&app), format!("Folder: {}", temp.path.display()));
    assert!(error_text(&app).is_empty());

    let request = take_request(&mut app, "Choose folder");
    complete(&mut app, request, FileDialogOutcome::Cancelled);
    assert_eq!(
        notice(&app),
        format!("Folder: {}", temp.path.display()),
        "cancel leaves the notice unchanged"
    );

    let request = take_request(&mut app, "Choose folder");
    complete(
        &mut app,
        request,
        FileDialogOutcome::Failed(FileDialogError::DialogFailed(String::from("folder failed"))),
    );
    assert!(notice(&app).is_empty());
    assert!(
        app.presentation()
            .expect("snapshot")
            .elements()
            .any(|element| element.text().contains("folder failed"))
    );
}

#[test]
fn the_shared_notice_renders_in_the_main_and_details_windows() {
    let temp = TempDir::new("details");
    let path = temp.file("tasks.txt");
    let (mut app, main) = open_seeded();
    app.click_label("Open details").expect("open details");
    app.apply_window_requests()
        .expect("settle the open request");

    let request = take_request(&mut app, "Save tasks");
    complete(&mut app, request, FileDialogOutcome::Picked(vec![path]));
    assert_eq!(notice(&app), "Saved 2 tasks");

    for window in [main, details_window(&app)] {
        assert!(
            app.presentation_of(window)
                .expect("window snapshot")
                .find_label("Notice")
                .is_some_and(|element| element.text() == "Saved 2 tasks"),
            "both windows render the shared notice"
        );
    }
}

#[test]
fn the_shared_error_renders_in_the_main_and_details_windows() {
    let (mut app, main) = open_seeded();
    app.click_label("Open details").expect("open details");
    app.apply_window_requests()
        .expect("settle the open request");

    let request = take_request(&mut app, "Choose folder");
    complete(
        &mut app,
        request,
        FileDialogOutcome::Failed(FileDialogError::DialogFailed(String::from("folder failed"))),
    );

    for window in [main, details_window(&app)] {
        assert!(
            app.presentation_of(window)
                .expect("window snapshot")
                .elements()
                .any(|element| element.text().contains("folder failed")),
            "both windows render the shared error"
        );
    }
}

#[test]
fn an_app_owned_save_can_complete_after_the_main_window_closes() {
    let temp = TempDir::new("late");
    let path = temp.file("tasks.txt");
    let (mut app, main) = open_seeded();
    let request = take_request(&mut app, "Save tasks");

    app.close_window(main).expect("close the main window");
    complete(
        &mut app,
        request,
        FileDialogOutcome::Picked(vec![path.clone()]),
    );
    assert_eq!(read(&path), "Write tests\nRead docs\n");

    let reopened = app.show("main").expect("reopen the main window");
    assert_eq!(notice(&app), "Saved 2 tasks");
    let details = app.show(DETAILS_WINDOW).expect("open the details window");
    assert!(
        app.presentation_of(details)
            .expect("details snapshot")
            .find_label("Notice")
            .is_some_and(|element| element.text() == "Saved 2 tasks")
    );
    assert_ne!(reopened.activation(), main.activation());
}

#[test]
fn an_app_owned_folder_pick_can_complete_after_the_main_window_closes() {
    let temp = TempDir::new("late-folder");
    let expected = format!("Folder: {}", temp.path.display());
    let (mut app, main) = open_seeded();
    let request = take_request(&mut app, "Choose folder");

    app.close_window(main).expect("close the main window");
    complete(
        &mut app,
        request,
        FileDialogOutcome::Picked(vec![temp.path.clone()]),
    );

    app.show("main").expect("reopen the main window");
    assert_eq!(notice(&app), expected);
    let details = app.show(DETAILS_WINDOW).expect("open the details window");
    assert!(
        app.presentation_of(details)
            .expect("details snapshot")
            .find_label("Notice")
            .is_some_and(|element| element.text() == expected.as_str()),
        "the details window shows the shared folder result"
    );
}
