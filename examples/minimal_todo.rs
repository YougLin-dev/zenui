//! 阶段一目标 API 的最小 headless Todo 驱动。
//!
//! 组件树与 Windows 宿主共用 `zenui-demo`，初始任务由 `seed` 提供。

use zenui::prelude::*;
use zenui_demo::{TodoApp, seed};

fn main() {
    let mut app = Runtime::new();
    app.open_window("main", |window| view! { <TodoApp window=window /> })
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
