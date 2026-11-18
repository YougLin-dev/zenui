// crates/app/src/lib.rs
//! Headless Todo fixture for `SPEC-GATE-A0-01`.
//!
//! The fixture wires `zenui-core` into a single-threaded scenario and renders a
//! byte-stable trace. It is a *test application*, not a framework face: every
//! API it touches is reachable from the core crate.

#![forbid(unsafe_code)]

use zenui_core::{Counters, Fatal, Filter, RowId, RowKey, Runtime, Tx, ValueError, ViewId};

/// UI-only state: not part of the runtime's authority.
pub struct UiState {
    pub filter: Filter,
    pub sink: Vec<String>,
    pub remaining: u32,
    pub has_completed: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            filter: Filter::All,
            sink: Vec::new(),
            remaining: 0,
            has_completed: false,
        }
    }
}

/// Deterministic snapshot of everything the trace reports after one step.
#[derive(Debug)]
pub struct Snapshot {
    pub counters: Counters,
    /// Every row the store owns, whether or not a filter shows it.
    pub store_rows: usize,
    /// Rows currently mounted as views: the visible set.
    pub visible_rows: usize,
    pub remaining: u32,
    pub has_completed: bool,
    pub filter: Filter,
    /// Store titles in model order. This is the whole store, not the filter
    /// result; when a filter is active the visible set is `visible_titles`.
    pub store_titles: String,
    /// Mounted titles in view order.
    pub visible_titles: String,
    pub focus: String,
    pub caret: String,
    pub edges: usize,
    pub active_sources: usize,
    pub geometry_epoch: u64,
    pub live_owners: usize,
    pub cleanup_runs: u64,
    pub sink_len: usize,
}

/// The headless Todo application.
pub struct TodoApp {
    runtime: Runtime,
    ui: UiState,
}

impl TodoApp {
    pub fn new() -> Self {
        Self {
            runtime: Runtime::new(),
            ui: UiState::default(),
        }
    }

    /// Run one frame. The closure sees staged writes and may read its own
    /// uncommitted values; publishing happens once when it returns `Ok`.
    pub fn frame<R>(
        &mut self,
        f: impl FnOnce(&mut Tx<'_>, &mut UiState) -> Result<R, Fatal>,
    ) -> Result<R, Fatal> {
        let Self { runtime, ui } = self;
        runtime.frame(|tx| f(tx, ui))
    }

    pub fn render(&mut self) -> Result<(), Fatal> {
        self.frame(render)
    }

    pub const fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub const fn ui(&self) -> &UiState {
        &self.ui
    }

    pub fn row_by_title(&self, title: &str) -> Option<(RowKey, RowId)> {
        let store = self.runtime.committed();
        let row = store.rows().find(|row| row.title() == title)?;
        let key = row.key();
        Some((key, store.handle_for(key)?))
    }

    fn visible_titles(&self) -> Vec<String> {
        let store = self.runtime.committed();
        self.runtime
            .row_views()
            .iter()
            .filter_map(|(_, key)| {
                let handle = store.handle_for(*key)?;
                store.with_title(handle, str::to_owned).ok()
            })
            .collect()
    }

    pub fn snapshot(&self) -> Snapshot {
        let store = self.runtime.committed();
        let store_titles = if store.is_empty() {
            String::from("-")
        } else {
            store.titles().collect::<Vec<_>>().join(",")
        };
        let visible = self.visible_titles();
        let visible_titles = if visible.is_empty() {
            String::from("-")
        } else {
            visible.join(",")
        };

        let focused = self.runtime.focused();
        let focus = focused
            .map(|view| format!("view#{}", view.index()))
            .unwrap_or_else(|| String::from("-"));
        let caret = focused
            .and_then(|view| {
                self.runtime
                    .caret(view)
                    .map(|offset| format!("view#{}:{}", view.index(), offset))
            })
            .unwrap_or_else(|| String::from("-"));

        Snapshot {
            counters: self.runtime.counters(),
            store_rows: store.len(),
            visible_rows: self.runtime.row_views().len(),
            remaining: self.ui.remaining,
            has_completed: self.ui.has_completed,
            filter: self.ui.filter,
            store_titles,
            visible_titles,
            focus,
            caret,
            edges: self.runtime.edge_count(),
            active_sources: self.runtime.active_source_count(),
            geometry_epoch: self.runtime.geometry_epoch(),
            live_owners: self.runtime.live_owners(),
            cleanup_runs: self.runtime.cleanup_runs(),
            sink_len: self.ui.sink.len(),
        }
    }
}

impl Default for TodoApp {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TodoApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TodoApp")
            .field("runtime", &self.runtime)
            .finish()
    }
}

/// Bind the current query result to the retained view tree.
///
/// TODO(A0-09): duplicate-key rejection in the view layer and a dedicated
/// setup/identity fixture are still missing.
pub fn render(tx: &mut Tx<'_>, ui: &mut UiState) -> Result<(), Fatal> {
    let want = tx.rows(ui.filter);
    let want_keys: Vec<RowKey> = want.iter().collect();
    let existing = tx.row_views();

    for (view, key) in existing.iter().copied() {
        if !want_keys.contains(&key) {
            // The core records the focus intent before the structural change
            // and settles it once at the end of the commit (`SPEC-GATE-A0-10`).
            tx.unmount(view)?;
        }
    }

    let mut order = Vec::with_capacity(want_keys.len());
    for key in &want_keys {
        match existing
            .iter()
            .find(|(_, existing_key)| existing_key == key)
        {
            Some((view, _)) => order.push(*view),
            None => {
                if let Some(view) = mount_row(tx, ui, *key)? {
                    order.push(view);
                }
            }
        }
    }
    tx.reorder_views(&order)?;

    ui.remaining = tx.remaining();
    ui.has_completed = tx.has_completed();
    Ok(())
}

/// Mount one row view, or nothing at all.
///
/// `SPEC-RT-045`: one owned read per UI mount. `SPEC-RT-013`: a failed read
/// cancels the mount — no node, no owner, no zero-length ghost row.
fn mount_row(tx: &mut Tx<'_>, ui: &mut UiState, key: RowKey) -> Result<Option<ViewId>, Fatal> {
    let Some(row) = tx.handle_for(key) else {
        ui.sink
            .push(format!("mount skipped: row key {} is gone", key.raw()));
        return Ok(None);
    };
    let title = match tx.read_once(row) {
        Ok(title) => title,
        Err(error) => {
            ui.sink
                .push(format!("mount skipped: title read failed: {error}"));
            return Ok(None);
        }
    };
    let view = tx.mount_row(key, title.chars().count() as u32)?;
    Ok(Some(view))
}

fn format_counters(label: &str, counters: Counters) -> String {
    format!(
        "  {label} query_evals={} id_materializations={} mounts={} unmounts={} \
         memo_recomputes={} commits={} focus_moves={} caret_ops={} io_reads={} \
         io_writes={} string_clones={}",
        counters.query_evals,
        counters.id_materializations,
        counters.mounts,
        counters.unmounts,
        counters.memo_recomputes,
        counters.commits,
        counters.focus_moves,
        counters.caret_ops,
        counters.io_reads,
        counters.io_writes,
        counters.string_clones,
    )
}

fn format_state(snapshot: &Snapshot) -> String {
    format!(
        "  state store_rows={} visible_rows={} remaining={} has_completed={} filter={} \
         store_titles={} visible_titles={} focus={} caret={}",
        snapshot.store_rows,
        snapshot.visible_rows,
        snapshot.remaining,
        snapshot.has_completed,
        snapshot.filter.as_str(),
        snapshot.store_titles,
        snapshot.visible_titles,
        snapshot.focus,
        snapshot.caret,
    )
}

fn format_gauges(snapshot: &Snapshot) -> String {
    format!(
        "  graph edges={} active_sources={} geometry_epoch={} live_owners={} \
         cleanup_runs={} sink={}",
        snapshot.edges,
        snapshot.active_sources,
        snapshot.geometry_epoch,
        snapshot.live_owners,
        snapshot.cleanup_runs,
        snapshot.sink_len,
    )
}

struct Recorder {
    out: String,
    previous: Counters,
    step: u32,
}

impl Recorder {
    fn new() -> Self {
        let mut out = String::new();
        out.push_str("zenui headless trace v1\n");
        out.push_str("scenario=todolist-headless\n");
        out.push('\n');
        Self {
            out,
            previous: Counters::default(),
            step: 0,
        }
    }

    fn step(&mut self, snapshot: &Snapshot, name: &str, detail: &str) {
        self.step += 1;
        let delta = snapshot.counters.delta(self.previous);
        self.previous = snapshot.counters;

        self.out
            .push_str(&format!("[step {:02} {name}]\n", self.step));
        self.out.push_str(&format!("  detail={detail}\n"));
        self.out.push_str(&format_state(snapshot));
        self.out.push('\n');
        self.out.push_str(&format_counters("delta", delta));
        self.out.push('\n');
        self.out.push_str(&format_gauges(snapshot));
        self.out.push('\n');
        self.out.push('\n');
    }

    fn finish(mut self, snapshot: &Snapshot) -> String {
        self.out.push_str("[summary]\n");
        self.out.push_str(&format!("  steps={}\n", self.step));
        self.out.push_str(&format_state(snapshot));
        self.out.push('\n');
        self.out
            .push_str(&format_counters("total", snapshot.counters));
        self.out.push('\n');
        self.out.push_str(&format_gauges(snapshot));
        self.out.push('\n');
        self.out.push_str("[status] outcome=ok\n");
        self.out
    }
}

fn row_named(app: &TodoApp, title: &str) -> (RowKey, RowId) {
    app.row_by_title(title)
        .unwrap_or_else(|| panic!("scenario invariant: row {title} exists"))
}

/// Run the `SPEC-GATE-A0-01` scenario and return the trace as one string.
///
/// The output is byte-stable: no `HashMap` iteration, no addresses, no timing,
/// no thread ids, and no `RuntimeId` value. `crates/app/tests/golden_trace.rs`
/// pins it.
pub fn todolist_trace() -> String {
    let mut app = TodoApp::new();
    let mut recorder = Recorder::new();

    // 01 — empty store, no rows match any filter.
    app.render().expect("frame 01");
    recorder.step(&app.snapshot(), "empty", "no rows");

    // 02 — insert A, B, C inside a single transaction.
    app.frame(|tx, ui| {
        tx.insert("A")?;
        tx.insert("B")?;
        tx.insert("C")?;
        render(tx, ui)
    })
    .expect("frame 02");
    recorder.step(&app.snapshot(), "insert-abc", "insert A,B,C");

    // 03 — toggle B to completed.
    let b_row = row_named(&app, "B").1;
    app.frame(|tx, ui| {
        let _ = tx.toggle(b_row);
        render(tx, ui)
    })
    .expect("frame 03");
    recorder.step(&app.snapshot(), "toggle-b", "toggle B -> completed");

    // 04 — switch to the Active filter.
    app.frame(|tx, ui| {
        ui.filter = Filter::Active;
        render(tx, ui)
    })
    .expect("frame 04");
    recorder.step(&app.snapshot(), "filter-active", "filter=active");

    // 05 — reorder the model so the Active list becomes C, A; then focus A.
    //
    // Focus is UI state, not a store mutation, so it is applied inside the same
    // frame: the `delete-focused` delta then reflects only the delete and the
    // focus settlement it triggers.
    let order = [
        row_named(&app, "C").0,
        row_named(&app, "A").0,
        row_named(&app, "B").0,
    ];
    let (a_key, a_row) = row_named(&app, "A");
    app.frame(|tx, ui| {
        if tx.reorder(&order).is_err() {
            return Err(Fatal::OrderNotAPermutation);
        }
        render(tx, ui)?;
        let view = tx.view_of_row(a_key).expect("A is mounted");
        tx.focus(view)?;
        Ok(())
    })
    .expect("frame 05");
    recorder.step(&app.snapshot(), "reorder", "reorder C,A,B; focus A");

    // 06 — delete the focused row (A).
    app.frame(|tx, ui| {
        if tx.remove(a_row).is_err() {
            return Err(Fatal::StaleStagedRow { key: a_key });
        }
        render(tx, ui)
    })
    .expect("frame 06");
    recorder.step(&app.snapshot(), "delete-focused", "delete focused A");

    // 07 — bulk clear of completed rows (B), which is filtered out.
    app.frame(|tx, ui| {
        tx.clear_completed()?;
        render(tx, ui)
    })
    .expect("frame 07");
    recorder.step(&app.snapshot(), "bulk-clear", "clear completed");

    // 08 — reuse the deleted handle; the write must be rejected and the
    // committed state must not move.
    let rollback = app
        .frame(|tx, ui| {
            let result = tx.toggle(a_row);
            render(tx, ui)?;
            Ok(result)
        })
        .expect("frame 08");
    let detail = match rollback {
        Ok(_) => String::from("result=unexpected-ok"),
        Err(error) => format!("result=InvalidHandle({})", error_kind(&error)),
    };
    recorder.step(&app.snapshot(), "invalid-rollback", &detail);

    // 09 — the next clean insert still commits.
    app.frame(|tx, ui| {
        tx.insert("D")?;
        render(tx, ui)
    })
    .expect("frame 09");
    recorder.step(&app.snapshot(), "insert-d", "insert D");

    recorder.finish(&app.snapshot())
}

fn error_kind(error: &ValueError) -> &'static str {
    match error {
        ValueError::Handle(handle) => handle.reason_kind(),
        ValueError::Store(_) => "store",
        ValueError::Phase(_) => "phase",
        ValueError::Aborted => "aborted",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_row_skips_a_key_that_cannot_be_resolved() {
        let mut app = TodoApp::new();
        app.frame(|tx, ui| {
            tx.insert("A")?;
            tx.insert("B")?;
            render(tx, ui)
        })
        .expect("seed");

        let (b_key, b_row) = row_named(&app, "B");
        app.frame(|tx, _ui| {
            tx.remove(b_row)
                .map_err(|_| Fatal::Unbalanced { depth: 0 })?;
            Ok(())
        })
        .expect("remove B");

        let mut mounted = None;
        app.frame(|tx, ui| {
            mounted = mount_row(tx, ui, b_key)?;
            Ok(())
        })
        .expect("probe");

        assert!(mounted.is_none(), "a ghost key must not mount a view");
        assert_eq!(app.ui().sink.len(), 1);
        assert!(app.ui().sink[0].contains("mount skipped"));
    }

    #[test]
    fn a_stale_handle_is_rejected_by_the_value_channel() {
        let mut app = TodoApp::new();
        let row = app.frame(|tx, _ui| tx.insert("A")).expect("insert A");

        app.frame(|tx, _ui| {
            tx.remove(row).map_err(|_| Fatal::Unbalanced { depth: 0 })?;
            Ok(())
        })
        .expect("remove A");

        app.frame(|tx, _ui| {
            let result = tx.toggle(row);
            assert!(matches!(result, Err(ValueError::Handle(_))));
            // A stale-handle rejection happens before the journal, so this
            // particular frame still commits (`ADR-0019` decision 1). That is
            // not a general rule: `ValueError::Phase` and `Aborted` poison.
            Ok(())
        })
        .expect("frame");

        assert_eq!(app.runtime().committed().len(), 0);
    }
}
