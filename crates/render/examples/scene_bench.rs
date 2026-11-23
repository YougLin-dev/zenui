//! Fixed-workload probe for scene rebuilds after a large text import.
//!
//! Run with `cargo run --release -p zenui-render --example scene_bench --locked`.
//! It prints timings and element counts; it makes no wall-clock assertion.
//!
//! The demo list is a windowed `For` (ADR-0011): the runtime mounts a bounded
//! row window, so snapshot size, snapshot clone, layout and paint stay flat as
//! the import grows. Row-window updates are measured separately because they
//! only happen when the scroll crosses a row boundary.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use zenui::prelude::*;
use zenui_demo::{open_todo_window, seed};
use zenui_render::{Scene, SceneCache, SceneEnv, Viewport};

/// 与宿主一样，只挂载视口覆盖的行；固定 32 行让不同规模的基准可比。
const WINDOW_ROWS: usize = 32;

fn imported_runtime(rows: usize) -> (Runtime, WindowCtx, PathBuf) {
    let mut app = Runtime::new();
    let window = open_todo_window(&mut app).expect("open the demo windows");
    seed(&mut app).expect("seed");
    let path = std::env::temp_dir().join(format!("zenui-scene-bench-{rows}.txt"));
    let text = (0..rows)
        .map(|index| format!("Imported task {index:05}\n"))
        .collect::<String>();
    std::fs::write(&path, text).expect("write the import file");
    app.click_text("Import file").expect("request the picker");
    let request = app
        .take_file_dialog_requests()
        .pop()
        .expect("a picker request");
    app.complete_file_dialog_request(request, FileDialogOutcome::Picked(vec![path.clone()]))
        .expect("complete the picker");
    app.settle_tasks().expect("settle the import");
    (app, window, path)
}

fn measure(repetitions: u32, mut step: impl FnMut()) -> Duration {
    let start = Instant::now();
    for _ in 0..repetitions {
        step();
    }
    start.elapsed() / repetitions
}

fn main() {
    const REPETITIONS: u32 = 20;
    let viewport = Viewport::new(900.0, 700.0, 1.0).expect("viewport");

    for rows in [100usize, 1000, 10_000] {
        let (mut app, window, path) = imported_runtime(rows);
        let presentation = measure(REPETITIONS, || {
            let _ = app.presentation().expect("presentation");
        });
        let snapshot = app.presentation().expect("presentation");
        let snapshot_clone = measure(REPETITIONS, || {
            let _ = snapshot.clone();
        });
        let fresh = measure(REPETITIONS, || {
            let _ = Scene::build(&snapshot, viewport).expect("scene");
        });
        let mut cache = SceneCache::new();
        let cold = {
            let start = Instant::now();
            let _ = Scene::build_cached(&snapshot, viewport, &SceneEnv::default(), &mut cache)
                .expect("scene");
            start.elapsed()
        };
        let warm = measure(REPETITIONS, || {
            let _ = Scene::build_cached(&snapshot, viewport, &SceneEnv::default(), &mut cache)
                .expect("scene");
        });
        app.input("What needs to be done?", "x").expect("type");
        let input_snapshot = app.presentation().expect("presentation");
        let input_rebuild = {
            let start = Instant::now();
            let _ =
                Scene::build_cached(&input_snapshot, viewport, &SceneEnv::default(), &mut cache)
                    .expect("scene");
            start.elapsed()
        };
        let elements = input_snapshot.elements().count();
        let base = Scene::build_cached(&input_snapshot, viewport, &SceneEnv::default(), &mut cache)
            .expect("base scene");
        let max_offset = base.max_scroll_offset();
        let offset = max_offset / 2;
        let band = base.row_bands()[0];
        let first = (((offset as f32 - band.top()) / band.row_height())
            .floor()
            .max(0.0) as usize)
            .min(band.total_rows().saturating_sub(1));
        let window_key = input_snapshot.row_bands()[0].key();
        app.set_row_window(window, window_key, first, WINDOW_ROWS)
            .expect("apply the middle row window");
        let mid_snapshot = app.presentation().expect("mid presentation");
        let scroll_env = SceneEnv::default().with_scroll_offset(offset);
        let scroll_build = {
            let start = Instant::now();
            let _ = Scene::build_cached(&mid_snapshot, viewport, &scroll_env, &mut cache)
                .expect("scroll scene");
            start.elapsed()
        };
        let scroll_scene = Scene::build_cached(&mid_snapshot, viewport, &scroll_env, &mut cache)
            .expect("scroll scene");
        // 跨行边界的一次窗口更新：刷新挂载窗口并重建一次快照。
        let mut crossing = first;
        let window_update = measure(REPETITIONS, || {
            crossing = if crossing == first { first + 1 } else { first };
            app.set_row_window(window, window_key, crossing, WINDOW_ROWS)
                .expect("move the row window");
            let _ = app.presentation().expect("window presentation");
        });
        let visible = scroll_scene.visible_elements().count();
        let shaped = scroll_scene
            .elements()
            .iter()
            .filter(|element| !element.text().is_empty())
            .count();
        let total_rows = input_snapshot.row_bands()[0].total_rows();
        println!(
            "rows={rows} presentation={presentation:?} snapshot_clone={snapshot_clone:?} fresh={fresh:?} cold_cached={cold:?} warm_cached={warm:?} input_rebuild={input_rebuild:?} scroll_build={scroll_build:?} window_update={window_update:?} offset={offset} max_offset={max_offset} content_height={:.1} total_rows={total_rows} window_rows={WINDOW_ROWS} elements={elements} visible={visible} shaped={shaped}",
            scroll_scene.content_extent().height,
        );
        let _ = std::fs::remove_file(&path);
    }
}
