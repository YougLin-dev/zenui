//! Fixed-workload probe for scene rebuilds after a large text import.
//!
//! Run with `cargo run --release -p zenui-render --example scene_bench --locked`.
//! It prints timings and element counts; it makes no wall-clock assertion.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use zenui::prelude::*;
use zenui_demo::{open_todo_window, seed};
use zenui_render::{Scene, SceneCache, SceneEnv, Viewport};

fn imported_runtime(rows: usize) -> (Runtime, PathBuf) {
    let mut app = Runtime::new();
    open_todo_window(&mut app).expect("open the demo windows");
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
    (app, path)
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

    for rows in [100usize, 500, 1000, 5000] {
        let (mut app, path) = imported_runtime(rows);
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
        let scroll_env = SceneEnv::default().with_scroll_offset(offset);
        let scroll_build = {
            let start = Instant::now();
            let _ = Scene::build_cached(&input_snapshot, viewport, &scroll_env, &mut cache)
                .expect("scroll scene");
            start.elapsed()
        };
        let scroll_scene = Scene::build_cached(&input_snapshot, viewport, &scroll_env, &mut cache)
            .expect("scroll scene");
        let visible = scroll_scene.visible_elements().count();
        let shaped = scroll_scene
            .elements()
            .iter()
            .filter(|element| !element.text().is_empty())
            .count();
        println!(
            "rows={rows} presentation={presentation:?} snapshot_clone={snapshot_clone:?} fresh={fresh:?} cold_cached={cold:?} warm_cached={warm:?} input_rebuild={input_rebuild:?} scroll_build={scroll_build:?} offset={offset} max_offset={max_offset} content_height={:.1} visible={visible} shaped={shaped} elements={elements}",
            scroll_scene.content_extent().height
        );
        let _ = std::fs::remove_file(&path);
    }
}
