// crates/app/tests/golden_trace.rs
//!
//! `SPEC-GATE-A0-01` requires a deterministic trace and a snapshot. These tests
//! pin the snapshot and refuse to let the scenario order or the report shape
//! drift silently.

const GOLDEN: &str = include_str!("golden/todolist_headless.trace");

/// Counter names the gate names explicitly: query / mount / materialization /
/// edge / geometry / focus / caret / I-O. `GRAPH` and `GAUGE` are the two lines
/// that carry the non-monotonic values.
const REQUIRED_COUNTERS: [&str; 11] = [
    "query_evals",
    "id_materializations",
    "mounts",
    "unmounts",
    "memo_recomputes",
    "commits",
    "focus_moves",
    "caret_ops",
    "io_reads",
    "io_writes",
    "string_clones",
];

const REQUIRED_GAUGES: [&str; 6] = [
    "edges",
    "active_sources",
    "geometry_epoch",
    "live_owners",
    "cleanup_runs",
    "sink",
];

#[test]
fn trace_matches_golden() {
    let actual = zenui_app::todolist_trace();
    assert_eq!(
        actual, GOLDEN,
        "headless trace drifted from crates/app/tests/golden/todolist_headless.trace"
    );
}

#[test]
fn two_runs_are_byte_identical() {
    let first = zenui_app::todolist_trace();
    let second = zenui_app::todolist_trace();
    assert_eq!(first, second, "trace must not depend on run state");
}

#[test]
fn scenario_keeps_the_gate_order() {
    let trace = zenui_app::todolist_trace();
    let expected = [
        "[step 01 empty]",
        "[step 02 insert-abc]",
        "[step 03 toggle-b]",
        "[step 04 filter-active]",
        "[step 05 reorder]",
        "[step 06 delete-focused]",
        "[step 07 bulk-clear]",
        "[step 08 invalid-rollback]",
        "[step 09 insert-d]",
    ];

    let mut cursor = 0;
    for step in expected {
        let offset = trace[cursor..].find(step).unwrap_or_else(|| {
            panic!("step header {step} missing or out of order");
        });
        cursor += offset + step.len();
    }
}

#[test]
fn every_step_reports_every_counter() {
    let trace = zenui_app::todolist_trace();
    for line in trace
        .lines()
        .filter(|line| line.trim_start().starts_with("delta "))
    {
        for counter in REQUIRED_COUNTERS {
            assert!(
                line.contains(&format!("{counter}=")),
                "counter {counter} missing from: {line}"
            );
        }
    }
    for line in trace
        .lines()
        .filter(|line| line.trim_start().starts_with("graph "))
    {
        for gauge in REQUIRED_GAUGES {
            assert!(
                line.contains(&format!("{gauge}=")),
                "gauge {gauge} missing from: {line}"
            );
        }
    }
}

#[test]
fn trace_carries_no_addresses_or_timings() {
    let trace = zenui_app::todolist_trace();
    for forbidden in ["0x", "Instant", "elapsed", "nanos", "micros", "thread_id"] {
        assert!(
            !trace.contains(forbidden),
            "trace must not contain {forbidden:?}"
        );
    }
}
