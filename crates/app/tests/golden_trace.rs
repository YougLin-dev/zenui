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

/// Match a `key=value` field as a whole token, so `mounts=` never matches
/// inside `unmounts=`.
fn has_field(line: &str, field: &str) -> bool {
    let prefix = format!("{field}=");
    line.split_whitespace()
        .any(|token| token.starts_with(&prefix))
}

#[test]
fn every_step_reports_every_counter() {
    let trace = zenui_app::todolist_trace();
    let delta_lines: Vec<&str> = trace
        .lines()
        .filter(|line| line.trim_start().starts_with("delta "))
        .collect();
    let graph_lines: Vec<&str> = trace
        .lines()
        .filter(|line| line.trim_start().starts_with("graph "))
        .collect();

    // One `delta` line per step (the summary uses `total`), and one `graph`
    // line per step plus the summary's gauge line.
    assert_eq!(delta_lines.len(), 9, "expected one delta line per step");
    assert_eq!(
        graph_lines.len(),
        10,
        "expected one graph line per step plus the summary"
    );
    assert_eq!(
        trace.matches("[step ").count(),
        9,
        "the gate fixes nine scenario steps"
    );

    for line in delta_lines {
        for counter in REQUIRED_COUNTERS {
            assert!(
                has_field(line, counter),
                "counter {counter} missing from: {line}"
            );
        }
    }
    for line in graph_lines {
        for gauge in REQUIRED_GAUGES {
            assert!(has_field(line, gauge), "gauge {gauge} missing from: {line}");
        }
    }
}

#[test]
fn state_line_separates_store_from_visible_set() {
    let trace = zenui_app::todolist_trace();
    let states: Vec<&str> = trace
        .lines()
        .filter(|line| line.trim_start().starts_with("state "))
        .collect();
    assert_eq!(states.len(), 10, "nine steps plus the summary");

    for line in &states {
        for field in [
            "store_rows",
            "visible_rows",
            "store_titles",
            "visible_titles",
        ] {
            assert!(
                has_field(line, field),
                "state field {field} missing: {line}"
            );
        }
    }

    // Under `filter=active` the store still owns B while the view does not.
    let active = states
        .iter()
        .find(|line| line.contains("filter=active") && line.contains("store_titles=A,B,C"))
        .expect("a step where the active filter hides B");
    assert!(
        active.contains("visible_titles=A,C"),
        "visible set must exclude the filtered-out row: {active}"
    );
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
