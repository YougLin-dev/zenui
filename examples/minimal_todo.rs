//! 阶段一目标 API 的最小 headless Todo 驱动。
//!
//! 组件树与 Windows 宿主共用 `zenui-demo`，初始任务由 `seed` 提供。

use zenui::prelude::*;
use zenui_demo::{TodoApp, TodoState, seed};

fn main() {
    let mut app = Runtime::new();
    let state = app.setup(TodoState::new).expect("setup");
    app.open_window(
        "main",
        move |window| view! { <TodoApp window=window state=state /> },
    )
    .expect("open window");
    seed(&mut app).expect("seed");

    app.click_text("Active").expect("filter");
    println!("active: {}", app.texts().join(" | "));

    app.click_text("All").expect("filter");
    app.click_text("Reverse").expect("reorder");
    app.click_label("Delete Read docs").expect("delete");
    println!("all: {}", app.texts().join(" | "));

    app.click_text("Preview import").expect("preview task");
    app.click_text("Load sample").expect("app task");
    let report = app.settle_tasks().expect("settle tasks");
    println!(
        "tasks: delivered={} | {}",
        report.delivered,
        app.texts().join(" | ")
    );
}
