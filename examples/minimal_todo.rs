//! 阶段一目标 API 的最小 headless Todo 驱动。
//!
//! 组件树与 Windows 宿主共用 `zenui-demo`，初始任务由 `seed` 提供。

use zenui::prelude::*;
use zenui_demo::{open_todo_window, seed};

fn main() {
    let mut app = Runtime::new();
    open_todo_window(&mut app).expect("open window");
    seed(&mut app).expect("seed");

    app.click_text("Active").expect("filter");
    println!("active: {}", app.texts().join(" | "));

    app.click_text("All").expect("filter");
    app.click_text("Reverse").expect("reorder");
    app.click_label("Delete Read docs").expect("delete");
    println!("all: {}", app.texts().join(" | "));

    let file =
        std::env::temp_dir().join(format!("zenui-minimal-import-{}.txt", std::process::id()));
    std::fs::write(&file, "From file\n").expect("write the import file");
    app.click_text("Import file").expect("pick a file");
    let request = app
        .take_file_dialog_requests()
        .pop()
        .expect("a file dialog request");
    app.complete_file_dialog_request(request, FileDialogOutcome::Picked(vec![file.clone()]))
        .expect("complete the headless picker");
    let mut delivered = app.settle_tasks().expect("settle the import").delivered;
    app.click_text("Load sample").expect("app task");
    delivered += app.settle_tasks().expect("settle tasks").delivered;
    let _ = std::fs::remove_file(&file);
    println!("tasks: delivered={delivered} | {}", app.texts().join(" | "));
}
