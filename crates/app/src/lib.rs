#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use zenui_core::{Counters, Fatal, Filter, RowId, RowKey, Runtime, Tx, ValueError, ViewId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiState {
    pub filter: Filter,
    pub draft: String,
    pub sink: Vec<String>,
    pub remaining: u32,
    pub has_completed: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            filter: Filter::All,
            draft: String::new(),
            sink: Vec::new(),
            remaining: 0,
            has_completed: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiResetFault;

#[derive(Debug, PartialEq, Eq)]
pub enum AddError {
    Model(Fatal),
    UiReset(UiResetFault),
}

impl std::fmt::Display for AddError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Model(error) => write!(f, "model add failed: {error}"),
            Self::UiReset(_) => f.write_str("UI draft reset failed after the model committed"),
        }
    }
}

impl std::error::Error for AddError {}

#[derive(Debug)]
pub struct Snapshot {
    pub counters: Counters,

    pub store_rows: usize,

    pub visible_rows: usize,
    pub remaining: u32,
    pub has_completed: bool,
    pub filter: Filter,

    pub store_titles: String,

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

pub struct TodoApp {
    runtime: Runtime,
    ui: UiState,
    pending_add: Option<PendingAdd>,
}

struct PendingAdd {
    row: RowId,
    title: String,
}

impl TodoApp {
    pub fn new() -> Self {
        Self {
            runtime: Runtime::new(),
            ui: UiState::default(),
            pending_add: None,
        }
    }

    pub fn frame<R>(
        &mut self,
        f: impl FnOnce(&mut Tx<'_>, &mut UiState) -> Result<R, Fatal>,
    ) -> Result<R, Fatal> {
        let Self { runtime, ui, .. } = self;
        // 闭包内的 UI 修改是暂存值；Err 时与 Runtime 的暂存状态一起丢弃。
        let previous = ui.clone();
        let result = runtime.frame(|tx| f(tx, ui));
        if result.is_err() {
            *ui = previous;
        }
        result
    }

    pub fn render(&mut self) -> Result<(), Fatal> {
        self.frame(render)
    }

    pub fn set_draft(&mut self, draft: impl Into<String>) {
        self.ui.draft = draft.into();
    }

    pub fn draft(&self) -> &str {
        &self.ui.draft
    }

    /// 模型先独立提交；草稿 reset 是随后独立的 UI 状态更新，不从模型事务回滚。
    pub fn add_todo(&mut self) -> Result<(), AddError> {
        self.add_todo_with_ui_reset(|ui| {
            ui.draft.clear();
            Ok(())
        })
    }

    fn add_todo_with_ui_reset(
        &mut self,
        reset: impl FnOnce(&mut UiState) -> Result<(), UiResetFault>,
    ) -> Result<(), AddError> {
        if let Some(pending) = self.pending_add.take() {
            let committed = self
                .runtime
                .committed()
                .with_title(pending.row, |_| ())
                .is_ok();
            if committed && self.ui.draft.trim() == pending.title {
                if let Err(fault) = Self::apply_ui_reset(&mut self.ui, reset) {
                    self.pending_add = Some(pending);
                    return Err(AddError::UiReset(fault));
                }
                return Ok(());
            }
            // A changed draft is a new Add; the committed pending row is not repeated.
        }

        let title = self.ui.draft.trim().to_owned();
        if title.is_empty() {
            return Ok(());
        }
        let row = self
            .frame(|tx, ui| {
                let row = tx.insert(title.clone())?;
                render(tx, ui)?;
                Ok(row)
            })
            .map_err(AddError::Model)?;
        if let Err(fault) = Self::apply_ui_reset(&mut self.ui, reset) {
            self.pending_add = Some(PendingAdd {
                row,
                title: title.clone(),
            });
            return Err(AddError::UiReset(fault));
        }
        Ok(())
    }

    fn apply_ui_reset(
        ui: &mut UiState,
        reset: impl FnOnce(&mut UiState) -> Result<(), UiResetFault>,
    ) -> Result<(), UiResetFault> {
        let before = ui.clone();
        match reset(ui) {
            Ok(()) => Ok(()),
            Err(fault) => {
                *ui = before;
                Err(fault)
            }
        }
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

pub fn render(tx: &mut Tx<'_>, ui: &mut UiState) -> Result<(), Fatal> {
    let want = tx.rows(ui.filter);
    let want_keys: Vec<RowKey> = want.iter().collect();
    let want_set: BTreeSet<RowKey> = want_keys.iter().copied().collect();
    let existing = tx.row_views();
    let existing_by_key: BTreeMap<RowKey, ViewId> =
        existing.iter().map(|(view, key)| (*key, *view)).collect();

    for (view, key) in existing.iter().copied() {
        if !want_set.contains(&key) {
            tx.unmount(view)?;
        }
    }

    let mut order = Vec::with_capacity(want_keys.len());
    for key in &want_keys {
        match existing_by_key.get(key) {
            Some(view) => order.push(*view),
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

pub fn todolist_trace() -> String {
    let mut app = TodoApp::new();
    let mut recorder = Recorder::new();

    app.render().expect("frame 01");
    recorder.step(&app.snapshot(), "empty", "no rows");

    app.frame(|tx, ui| {
        tx.insert("A")?;
        tx.insert("B")?;
        tx.insert("C")?;
        render(tx, ui)
    })
    .expect("frame 02");
    recorder.step(&app.snapshot(), "insert-abc", "insert A,B,C");

    let b_row = row_named(&app, "B").1;
    app.frame(|tx, ui| {
        let _ = tx.toggle(b_row);
        render(tx, ui)
    })
    .expect("frame 03");
    recorder.step(&app.snapshot(), "toggle-b", "toggle B -> completed");

    app.frame(|tx, ui| {
        ui.filter = Filter::Active;
        render(tx, ui)
    })
    .expect("frame 04");
    recorder.step(&app.snapshot(), "filter-active", "filter=active");

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

    app.frame(|tx, ui| {
        if tx.remove(a_row).is_err() {
            return Err(Fatal::StaleStagedRow { key: a_key });
        }
        render(tx, ui)
    })
    .expect("frame 06");
    recorder.step(&app.snapshot(), "delete-focused", "delete focused A");

    app.frame(|tx, ui| {
        tx.clear_completed()?;
        render(tx, ui)
    })
    .expect("frame 07");
    recorder.step(&app.snapshot(), "bulk-clear", "clear completed");

    let rejected = app
        .frame(|tx, ui| {
            let result = tx.toggle(a_row);
            render(tx, ui)?;
            Ok(result)
        })
        .expect("frame 08");
    let detail = match rejected {
        Ok(_) => String::from("result=unexpected-ok"),
        Err(error) => format!("result=InvalidHandle({})", error_kind(&error)),
    };
    recorder.step(&app.snapshot(), "rejected-handle", &detail);

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
    fn a_ui_reset_fault_preserves_the_committed_add_and_previous_draft() {
        let mut app = TodoApp::new();
        app.set_draft("Write tests");

        let commits_before = app.runtime().txn().commits();
        let result = app.add_todo_with_ui_reset(|ui| {
            ui.filter = Filter::Completed;
            ui.draft.clear();
            ui.sink.push(String::from("abandoned reset output"));
            ui.remaining = 99;
            ui.has_completed = true;
            Err(UiResetFault)
        });

        assert_eq!(
            result,
            Err(AddError::UiReset(UiResetFault)),
            "the fault must be reported through the command boundary"
        );
        assert_eq!(
            app.runtime().txn().commits(),
            commits_before + 1,
            "the model Add committed before the independent UI reset"
        );
        assert_eq!(app.runtime().committed().len(), 1);
        assert_eq!(
            app.runtime().committed().titles().collect::<Vec<_>>(),
            vec!["Write tests"]
        );
        assert_eq!(app.snapshot().visible_titles, "Write tests");
        assert_eq!(
            app.ui(),
            &UiState {
                filter: Filter::All,
                draft: String::from("Write tests"),
                sink: Vec::new(),
                remaining: 1,
                has_completed: false,
            },
            "a partial UI reset is discarded; committed model-derived UI stays"
        );

        app.add_todo()
            .expect("retrying the command finishes the pending UI reset");

        assert_eq!(
            app.runtime().committed().len(),
            1,
            "the retry must not insert the preserved draft again"
        );
        assert_eq!(app.snapshot().visible_titles, "Write tests");
        assert_eq!(app.ui().draft, "", "the retry commits the draft reset");
    }

    #[test]
    fn a_changed_draft_after_a_fault_is_added_without_reusing_the_pending_row() {
        let mut app = TodoApp::new();
        app.set_draft("Write tests");
        let _ = app.add_todo_with_ui_reset(|_| Err(UiResetFault));

        app.set_draft("Second task");
        app.add_todo().expect("the changed draft is added normally");
        app.set_draft("Write tests");
        app.add_todo().expect("the old title is no longer pending");

        assert_eq!(
            app.runtime().committed().titles().collect::<Vec<_>>(),
            vec!["Write tests", "Second task", "Write tests"],
            "the pending Add is neither repeated nor allowed to swallow later input"
        );
        assert_eq!(app.ui().draft, "");
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

            Ok(())
        })
        .expect("frame");

        assert_eq!(app.runtime().committed().len(), 0);
    }
}
