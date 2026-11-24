#![allow(unused_braces)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use zenui::prelude::*;
use zenui_demo::{open_todo_window, seed};

fn open() -> (Runtime, WindowCtx) {
    let mut app = Runtime::new();
    let main = open_todo_window(&mut app).expect("open the demo windows");
    seed(&mut app).expect("seed the todo rows");
    (app, main)
}

fn import_request(app: &mut Runtime) -> FileDialogRequest {
    app.click_text("Import file").expect("click Import file");
    let mut requests = app.take_file_dialog_requests();
    assert_eq!(requests.len(), 1, "one picker request is queued");
    requests.remove(0)
}

fn complete(app: &mut Runtime, request: FileDialogRequest, outcome: FileDialogOutcome) {
    app.complete_file_dialog_request(request, outcome)
        .expect("complete the picker request");
    app.settle_tasks().expect("settle the import tasks");
}

fn row_titles(app: &Runtime) -> Vec<String> {
    app.presentation()
        .expect("a window snapshot")
        .elements()
        .filter_map(SnapshotElement::label)
        .filter_map(|label| label.strip_prefix("Edit ").map(str::to_owned))
        .collect()
}

fn has_text(app: &Runtime, needle: &str) -> bool {
    app.presentation()
        .expect("a window snapshot")
        .elements()
        .any(|element| element.text().contains(needle))
}

/// 每个用例一个独立目录；测试并行时不会互相删除文件。
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "zenui-demo-import-{}-{}-{label}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("create the temp dir");
        Self { path }
    }

    fn file(&self, name: &str, contents: &[u8]) -> PathBuf {
        let path = self.path.join(name);
        std::fs::write(&path, contents).expect("write the temp file");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn missing_file(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "zenui-demo-import-missing-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn picked_files_add_one_title_per_non_empty_line_in_order() {
    let temp = TempDir::new("picked");
    let first = temp.file("first.txt", b"\n  First task  \n\nSecond task\n");
    let second = temp.file("second.txt", b"Third task\r\n\r\n");
    let (mut app, _main) = open();

    let request = import_request(&mut app);
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
    assert!(request.allows_multiple(), "the demo imports several files");

    complete(
        &mut app,
        request,
        FileDialogOutcome::Picked(vec![first, second]),
    );

    assert_eq!(
        row_titles(&app),
        [
            "Write tests",
            "Read docs",
            "First task",
            "Second task",
            "Third task"
        ],
        "the titles keep the file and line order"
    );
    assert!(has_text(&app, "Imported 3 tasks"));
}

#[test]
fn dropped_files_add_one_title_per_line_in_source_order() {
    let temp = TempDir::new("dropped");
    let first = temp.file("first.txt", b"\n  First task  \n\nSecond task\n");
    let second = temp.file("second.txt", b"Third task\r\n\r\n");
    let (mut app, main) = open();

    app.drop_files(main, vec![first, second])
        .expect("deliver the dropped files");
    app.settle_tasks().expect("settle the import tasks");

    assert_eq!(
        row_titles(&app),
        [
            "Write tests",
            "Read docs",
            "First task",
            "Second task",
            "Third task"
        ],
        "drops use the same parser and keep the file and line order"
    );
    assert!(has_text(&app, "Imported 3 tasks"));
}

#[test]
fn a_dropped_import_can_finish_after_the_main_window_closes() {
    let temp = TempDir::new("dropped-late");
    let late = temp.file("late.txt", b"Late task\n");
    let (mut app, main) = open();

    app.drop_files(main, vec![late])
        .expect("deliver the dropped file");
    app.close_window(main).expect("close the main window");
    app.settle_tasks().expect("settle the app-owned import");

    let reopened = app.show("main").expect("reopen the main window");
    assert_ne!(reopened.activation(), main.activation());
    assert_eq!(
        row_titles(&app),
        ["Write tests", "Read docs", "Late task"],
        "the drop worker belongs to the application and commits after close"
    );
    assert!(has_text(&app, "Imported 1 task"));
}

#[test]
fn an_unreadable_dropped_file_reports_the_import_error() {
    let (mut app, main) = open();

    app.drop_files(main, vec![missing_file("unreadable-drop.txt")])
        .expect("deliver the dropped file");
    app.settle_tasks().expect("settle the failed import");

    assert_eq!(row_titles(&app), ["Write tests", "Read docs"]);
    assert!(has_text(&app, "Import failed: Could not read"));
}

#[test]
fn the_details_window_imports_drops_into_the_shared_model() {
    let temp = TempDir::new("dropped-details");
    let file = temp.file("details.txt", b"From details\n");
    let (mut app, main) = open();
    app.click_label("Open details").expect("open details");
    app.apply_window_requests()
        .expect("settle the open request");
    let details = app
        .windows()
        .into_iter()
        .find(|info| info.spec.key() == "details")
        .expect("the details window is live")
        .window;

    app.drop_files(details, vec![file])
        .expect("deliver a drop to details");
    app.settle_tasks().expect("settle the import");

    let titles: Vec<String> = app
        .presentation_of(main)
        .expect("main snapshot")
        .elements()
        .filter_map(SnapshotElement::label)
        .filter_map(|label| label.strip_prefix("Edit ").map(str::to_owned))
        .collect();
    assert_eq!(titles, ["Write tests", "Read docs", "From details"]);
    assert!(has_text(&app, "Imported 1 task"));
}

#[test]
fn cancelling_the_chooser_leaves_the_list_unchanged() {
    let (mut app, _main) = open();
    let request = import_request(&mut app);
    complete(&mut app, request, FileDialogOutcome::Cancelled);

    assert_eq!(row_titles(&app), ["Write tests", "Read docs"]);
    assert!(!has_text(&app, "Imported"));
}

#[test]
fn an_unreadable_file_reports_an_error_without_importing() {
    let (mut app, _main) = open();
    let request = import_request(&mut app);
    complete(
        &mut app,
        request,
        FileDialogOutcome::Picked(vec![missing_file("unreadable.txt")]),
    );

    assert_eq!(row_titles(&app), ["Write tests", "Read docs"]);
    assert!(has_text(&app, "Import failed: Could not read"));
}

#[test]
fn invalid_utf8_reports_an_error_without_importing() {
    let temp = TempDir::new("invalid");
    let invalid = temp.file("invalid.txt", &[0xff, 0xfe, 0x00]);
    let (mut app, _main) = open();
    let request = import_request(&mut app);
    complete(&mut app, request, FileDialogOutcome::Picked(vec![invalid]));

    assert_eq!(row_titles(&app), ["Write tests", "Read docs"]);
    assert!(has_text(&app, "Import failed: Could not read"));
}

#[test]
fn an_import_can_finish_after_the_main_window_closes() {
    let temp = TempDir::new("late");
    let late = temp.file("late.txt", b"Late task\n");
    let (mut app, main) = open();
    let request = import_request(&mut app);

    app.close_window(main).expect("close the main window");
    complete(&mut app, request, FileDialogOutcome::Picked(vec![late]));

    let reopened = app.show("main").expect("reopen the main window");
    assert_ne!(reopened.activation(), main.activation());
    assert_eq!(
        row_titles(&app),
        ["Write tests", "Read docs", "Late task"],
        "an app-owned import commits the shared model after the anchor closes"
    );
    assert!(has_text(&app, "Imported 1 task"));
    assert!(
        app.take_file_dialog_requests().is_empty(),
        "the old selection is not redirected to the reopened window"
    );
}
