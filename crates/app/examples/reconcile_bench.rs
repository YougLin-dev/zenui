//! Fixed-workload probe for unchanged Todo reconciliation and empty commits.
//!
//! Run with `cargo run --release -p zenui-app --example reconcile_bench --locked`.
//! The probe prints timings and structural counters; it makes no wall-clock
//! assertion, so it is evidence alongside the deterministic tests.

use std::time::{Duration, Instant};

use zenui_app::{TodoApp, render};

fn build(rows: usize) -> TodoApp {
    let mut app = TodoApp::new();
    app.frame(|tx, ui| {
        for index in 0..rows {
            tx.insert(format!("row-{index:05}"))?;
        }
        render(tx, ui)
    })
    .expect("seed");
    app
}

fn measure(repetitions: u32, mut step: impl FnMut()) -> Duration {
    let start = Instant::now();
    for _ in 0..repetitions {
        step();
    }
    start.elapsed() / repetitions
}

fn main() {
    const REPETITIONS: u32 = 100;

    for rows in [500usize, 1000, 2000, 4000] {
        let mut app = build(rows);
        let reconcile = measure(REPETITIONS, || app.render().expect("reconcile"));
        let commit = measure(REPETITIONS, || {
            app.frame(|_, _| Ok(())).expect("empty commit");
        });
        let snapshot = app.snapshot();
        println!(
            "rows={rows} reconcile={reconcile:?} empty_commit={commit:?} \
             mounts={} unmounts={} publications={} live_owners={} owner_storage={}",
            snapshot.counters.mounts,
            snapshot.counters.unmounts,
            app.runtime().txn().publications().len(),
            snapshot.live_owners,
            app.runtime().owner_storage_len(),
        );
    }
}
