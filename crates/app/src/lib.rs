// crates/app/src/lib.rs
//! Headless Todo fixture for `SPEC-GATE-A0-01`.
//!
//! The fixture wires `zenui-core` into a single-threaded scenario and renders a
//! byte-stable trace. It is a *test application*, not a framework face: every
//! API it touches is reachable from the core crate.

#![forbid(unsafe_code)]

use zenui_core::{
    Counters, Filter, FocusIntent, InvalidHandle, MemoGraph, NodeId, OwnerId, OwnerTree,
    QueryCache, RowId, RowKey, RowSet, Runtime, StoreError, Value, ViewId, ViewTree,
};

/// Memo graph nodes owned by the fixture.
#[derive(Clone, Copy, Debug)]
struct Nodes {
    store: NodeId,
    all: NodeId,
    active: NodeId,
    completed: NodeId,
    remaining: NodeId,
    has_completed: NodeId,
}

impl Nodes {
    const fn for_filter(self, filter: Filter) -> NodeId {
        match filter {
            Filter::All => self.all,
            Filter::Active => self.active,
            Filter::Completed => self.completed,
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
    query: QueryCache,
    graph: MemoGraph,
    views: ViewTree,
    owners: OwnerTree,
    root_owner: OwnerId,
    nodes: Nodes,
    filter: Filter,
    focus_intent: Option<FocusIntent>,
    sink: Vec<String>,
}

impl TodoApp {
    pub fn new() -> Self {
        let owners = OwnerTree::new();
        let root_owner = owners.root();
        let views = ViewTree::new(root_owner);

        let mut graph = MemoGraph::new();
        let nodes = Nodes {
            store: graph.source("store"),
            all: graph.memo("rows/all"),
            active: graph.memo("rows/active"),
            completed: graph.memo("rows/completed"),
            remaining: graph.memo("remaining"),
            has_completed: graph.memo("has-completed"),
        };

        Self {
            runtime: Runtime::new(),
            query: QueryCache::new(),
            graph,
            views,
            owners,
            root_owner,
            nodes,
            filter: Filter::All,
            focus_intent: None,
            sink: Vec::new(),
        }
    }

    /// Run one frame transaction.
    ///
    /// TODO(A0-04): the batch is a depth counter. Nested batches do not yet
    /// stage values, so a reader inside an open batch can observe an
    /// intermediate value. The staging journal is P2 work.
    pub fn frame<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.graph.begin_batch();
        let out = f(self);
        self.graph.end_batch();
        out
    }

    pub fn set_filter(&mut self, filter: Filter) {
        self.filter = filter;
    }

    pub const fn filter(&self) -> Filter {
        self.filter
    }

    pub fn insert(&mut self, title: &str) -> RowId {
        let row = self.runtime.store_mut().insert(title);
        self.graph.bump_source(self.nodes.store);
        row
    }

    pub fn toggle(&mut self, row: RowId) -> Result<bool, InvalidHandle> {
        let value = self.runtime.store_mut().toggle(row)?;
        self.graph.bump_source(self.nodes.store);
        Ok(value)
    }

    pub fn delete(&mut self, row: RowId) -> Result<(), InvalidHandle> {
        self.runtime.store_mut().remove(row)?;
        self.graph.bump_source(self.nodes.store);
        Ok(())
    }

    pub fn reorder(&mut self, keys: &[RowKey]) -> Result<(), StoreError> {
        self.runtime.store_mut().reorder(keys)?;
        self.graph.bump_source(self.nodes.store);
        Ok(())
    }

    pub fn clear_completed(&mut self) -> Vec<RowKey> {
        let removed = self.runtime.store_mut().clear_completed();
        if !removed.is_empty() {
            self.graph.bump_source(self.nodes.store);
        }
        removed
    }

    pub fn row_by_title(&self, title: &str) -> Option<(RowKey, RowId)> {
        let row = self.runtime.store().rows().find(|row| row.title() == title)?;
        let key = row.key();
        Some((key, self.runtime.store().handle_for(key)?))
    }

    /// Move focus to the mounted view of a row. Returns false if it is not
    /// mounted (for example, filtered out).
    pub fn focus_row(&mut self, key: RowKey) -> bool {
        match self.views.view_of_row(key) {
            Some(view) => {
                self.views.focus(view);
                true
            }
            None => false,
        }
    }

    /// Bind the current query result to the retained view tree.
    ///
    /// TODO(A0-06): this is the TreeCommit path, and it patches the live tree
    /// directly. There is no preflight for key/topology/focus/geometry, no
    /// staging tree, and therefore no way to restore the previous commit if a
    /// patch fails part-way. P2 owns the journal and the single swap.
    pub fn render(&mut self) {
        let filter = self.filter;
        let want = self.rows(filter);
        let want_keys: Vec<RowKey> = want.iter().collect();
        let existing = self.views.row_views();

        for (view, key) in existing.iter().copied() {
            if !want_keys.contains(&key) {
                self.unmount_row(view);
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
                    if let Some(view) = self.mount_row(*key) {
                        order.push(view);
                    }
                }
            }
        }
        self.views.reorder(&order);
        self.settle_focus();
    }

    /// Current query result. TODO(A0-07): this is a whole-epoch cache, not a
    /// membership-indexed cache with reverse field edges.
    fn rows(&mut self, filter: Filter) -> RowSet {
        let store_node = self.nodes.store;
        let node = self.nodes.for_filter(filter);
        let Self {
            graph,
            query,
            runtime,
            sink,
            ..
        } = self;
        let store = runtime.store();
        match graph.compute(node, |reader| {
            reader.read(store_node);
            Value::Rows(query.rows(store, filter))
        }) {
            Ok(Value::Rows(rows)) => rows,
            Ok(other) => {
                sink.push(format!("rows/{} produced {other:?}", filter.as_str()));
                RowSet::empty()
            }
            Err(error) => {
                sink.push(format!("rows/{}: {error}", filter.as_str()));
                RowSet::empty()
            }
        }
    }

    fn remaining(&mut self) -> u32 {
        let store_node = self.nodes.store;
        let node = self.nodes.remaining;
        let Self {
            graph, runtime, sink, ..
        } = self;
        let store = runtime.store();
        match graph.compute(node, |reader| {
            reader.read(store_node);
            Value::Count(store.remaining() as u32)
        }) {
            Ok(Value::Count(value)) => value,
            Ok(other) => {
                sink.push(format!("remaining produced {other:?}"));
                0
            }
            Err(error) => {
                sink.push(format!("remaining: {error}"));
                0
            }
        }
    }

    fn has_completed(&mut self) -> bool {
        let store_node = self.nodes.store;
        let node = self.nodes.has_completed;
        let Self {
            graph, runtime, sink, ..
        } = self;
        let store = runtime.store();
        match graph.compute(node, |reader| {
            reader.read(store_node);
            Value::Flag(store.has_completed())
        }) {
            Ok(Value::Flag(value)) => value,
            Ok(other) => {
                sink.push(format!("has-completed produced {other:?}"));
                false
            }
            Err(error) => {
                sink.push(format!("has-completed: {error}"));
                false
            }
        }
    }

    /// Mount one row view, or nothing at all.
    ///
    /// `SPEC-RT-045`: one owned read per UI mount. `SPEC-RT-013`: a failed read
    /// cancels the mount — no node, no owner, no zero-length ghost row. The
    /// error goes to the sink and the caller simply omits the row for this
    /// frame.
    fn mount_row(&mut self, key: RowKey) -> Option<ViewId> {
        let Some(row) = self.runtime.store().handle_for(key) else {
            self.sink
                .push(format!("mount skipped: row key {} is gone", key.raw()));
            return None;
        };
        let title = match self.runtime.store_mut().read_owned_title(row) {
            Ok(title) => title,
            Err(error) => {
                self.sink
                    .push(format!("mount skipped: title read failed: {error}"));
                return None;
            }
        };
        let title_len = title.chars().count() as u32;

        let owner = match self.owners.child_of(self.root_owner) {
            Ok(owner) => owner,
            Err(error) => {
                self.sink.push(format!("owner mount failed: {error}"));
                self.root_owner
            }
        };
        let view = self.views.mount_row(key, owner, title_len);
        self.owners.on_cleanup(
            owner,
            Box::new(move || {
                // TODO(A0-11): release the row subscription and assert the live
                // edge count returns to baseline. P1 only proves cleanup runs.
                let _ = (key, view);
            }),
        );
        Some(view)
    }

    fn unmount_row(&mut self, view: ViewId) {
        if self.views.focused() == Some(view) && self.focus_intent.is_none() {
            self.focus_intent = self.views.focus_intent_for(view);
        }
        if let Some(node) = self.views.unmount(view) {
            self.owners.dispose(node.owner);
        }
    }

    /// TODO(A0-10): settle order is next → previous only. Input/toolbar fallback
    /// and cancellation by a later user focus move are not implemented.
    fn settle_focus(&mut self) {
        let Some(intent) = self.focus_intent.take() else {
            return;
        };
        let target = intent
            .next
            .filter(|view| self.views.is_live(*view))
            .or_else(|| intent.previous.filter(|view| self.views.is_live(*view)));
        match target {
            Some(view) => self.views.focus(view),
            None => self.views.clear_focus(),
        }
    }

    /// Titles of the rows that are currently mounted, in view order. This is a
    /// subset of `Store::titles`, and a *proper* subset only while the filter
    /// hides a live row.
    fn visible_titles(&self) -> Vec<String> {
        self.views
            .row_views()
            .iter()
            .filter_map(|(_, key)| {
                let handle = self.runtime.store().handle_for(*key)?;
                self.runtime.store().with_title(handle, str::to_owned).ok()
            })
            .collect()
    }

    fn counters(&self) -> Counters {
        let mut total = self.runtime.store().counters();
        total += self.query.counters();
        total += self.graph.counters();
        total += self.views.counters();
        total
    }

    /// Read the observable state. This evaluates the derived memos, so the
    /// counters it returns include that evaluation.
    pub fn snapshot(&mut self) -> Snapshot {
        let remaining = self.remaining();
        let has_completed = self.has_completed();

        let store_titles = if self.runtime.store().is_empty() {
            String::from("-")
        } else {
            self.runtime.store().titles().collect::<Vec<_>>().join(",")
        };
        let visible = self.visible_titles();
        let visible_titles = if visible.is_empty() {
            String::from("-")
        } else {
            visible.join(",")
        };

        let focused = self.views.focused();
        let focus = focused
            .map(|view| format!("view#{}", view.index()))
            .unwrap_or_else(|| String::from("-"));
        let caret = focused
            .and_then(|view| {
                self.views
                    .caret(view)
                    .map(|offset| format!("view#{}:{}", view.index(), offset))
            })
            .unwrap_or_else(|| String::from("-"));

        Snapshot {
            counters: self.counters(),
            store_rows: self.runtime.store().len(),
            visible_rows: self.views.row_views().len(),
            remaining,
            has_completed,
            filter: self.filter,
            store_titles,
            visible_titles,
            focus,
            caret,
            edges: self.graph.edge_count(),
            active_sources: self.graph.active_source_count(),
            geometry_epoch: self.views.geometry_epoch(),
            live_owners: self.owners.live(),
            cleanup_runs: self.owners.cleanup_runs(),
            sink_len: self.sink.len(),
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
            .field("views", &self.views)
            .finish()
    }
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

/// Run the `SPEC-GATE-A0-01` scenario and return the trace as one string.
///
/// The output is byte-stable: no `HashMap` iteration, no addresses, no timing,
/// no thread ids. `crates/app/tests/golden_trace.rs` pins it.
pub fn todolist_trace() -> String {
    let mut app = TodoApp::new();
    let mut recorder = Recorder::new();

    // 01 — empty store, no rows match any filter.
    app.frame(|app| app.render());
    recorder.step(&app.snapshot(), "empty", "no rows");

    // 02 — insert A, B, C inside a single transaction.
    app.frame(|app| {
        app.insert("A");
        app.insert("B");
        app.insert("C");
        app.render();
    });
    recorder.step(&app.snapshot(), "insert-abc", "insert A,B,C");

    // 03 — toggle B to completed.
    let b_row = row_named(&app, "B").1;
    app.frame(|app| {
        app.toggle(b_row).expect("row B is live");
        app.render();
    });
    recorder.step(&app.snapshot(), "toggle-b", "toggle B -> completed");

    // 04 — switch to the Active filter.
    app.frame(|app| {
        app.set_filter(Filter::Active);
        app.render();
    });
    recorder.step(&app.snapshot(), "filter-active", "filter=active");

    // 05 — reorder the model so the Active list becomes C, A.
    //
    // Focus is UI state, not a store mutation, so it is placed before the step
    // snapshot: the `delete-focused` delta then reflects only the delete and
    // the focus settlement it triggers.
    let order = [
        row_named(&app, "C").0,
        row_named(&app, "A").0,
        row_named(&app, "B").0,
    ];
    let (a_key, a_row) = row_named(&app, "A");
    app.frame(|app| {
        app.reorder(&order).expect("reorder is a permutation");
        app.render();
    });
    assert!(app.focus_row(a_key), "row A must be mounted");
    recorder.step(&app.snapshot(), "reorder", "reorder C,A,B; focus A");

    // 06 — delete the focused row (A).
    app.frame(|app| {
        app.delete(a_row).expect("row A is live");
        app.render();
    });
    recorder.step(&app.snapshot(), "delete-focused", "delete focused A");

    // 07 — bulk clear of completed rows (B), which is filtered out.
    app.frame(|app| {
        app.clear_completed();
        app.render();
    });
    recorder.step(&app.snapshot(), "bulk-clear", "clear completed");

    // 08 — reuse the deleted handle; the write must be rejected and the
    // committed state must not move.
    let rollback = app.frame(|app| {
        let result = app.toggle(a_row);
        app.render();
        result
    });
    let detail = match rollback {
        Ok(_) => String::from("result=unexpected-ok"),
        Err(error) => format!("result=InvalidHandle({})", error.reason_kind()),
    };
    recorder.step(&app.snapshot(), "invalid-rollback", &detail);

    // 09 — the next clean insert still commits.
    app.frame(|app| {
        app.insert("D");
        app.render();
    });
    recorder.step(&app.snapshot(), "insert-d", "insert D");

    recorder.finish(&app.snapshot())
}

fn row_named(app: &TodoApp, title: &str) -> (RowKey, RowId) {
    app.row_by_title(title)
        .unwrap_or_else(|| panic!("scenario invariant: row {title} exists"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_title_read_does_not_mount_a_row() {
        let mut app = TodoApp::new();
        let row = app.insert("A");
        app.frame(|app| app.render());

        let (view, _) = app.views.row_views()[0];
        // Remove the row and the view behind the query cache's back, so the
        // cached RowSet still lists the key while nothing can resolve it.
        app.runtime.store_mut().remove(row).expect("live row");
        app.views.unmount(view);

        app.render();

        assert_eq!(
            app.views.row_views().len(),
            0,
            "a failed read must not mount a zero-length ghost row"
        );
        assert_eq!(
            app.views.counters().mounts,
            1,
            "only the first mount counts"
        );
        assert_eq!(app.sink.len(), 1, "the failure must reach the sink");
        assert!(
            app.sink[0].contains("mount skipped"),
            "unexpected sink message: {}",
            app.sink[0]
        );
    }
}
