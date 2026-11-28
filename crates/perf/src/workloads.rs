//! 固定负载：每条负载给出计数、时间与硬上限（§9.2 的最小硬门禁表）。
//!
//! 计数是确定性的；上限写进基线文件后按 turn 检查。时间只在与基线相同
//! 的 runner/指纹上强制，见 [`crate::baseline`]。

#![allow(unused_braces)]

use std::collections::BTreeMap;
use std::time::Instant;

use zenui::prelude::*;

use crate::baseline::Timing;
use crate::harness::{
    Counters, MultiWindowHarness, PerfResult, SampleSet, WindowHarness, duration_ns, fail, put,
    repeat,
};

const ROWS: usize = 10_000;
const WINDOW_ROWS: usize = 32;
const ROW_HEIGHT_PX: f32 = 20.0;

/// 一条负载的完整测量结果。
pub struct Measurement {
    /// 每个 turn 的最坏计数；状态类 counter 是最终值。
    pub counters: Counters,
    /// 基线记录并检查的硬上限；只能由 `record` 写入基线。
    pub limits: BTreeMap<String, u64>,
    /// 正式样本的耗时，纳秒；空表示只记录计数与状态（soak）。
    pub times_ns: Vec<u64>,
    /// 记录到基线文件附近的人类可读说明。
    pub notes: Vec<String>,
    /// 每个时间样本包含的连续 turn 数。
    pub repeats: usize,
    /// 本次测量后的时钟校准值。
    pub clock_ns: u64,
}

impl Measurement {
    /// 样本耗时统计；没有样本时全为 0。
    pub fn timing(&self) -> Timing {
        Timing::from_samples(&self.times_ns, self.repeats)
    }

    /// 基线缺失时用于打印的空测量；不参与比较。
    pub fn empty() -> Self {
        Self {
            counters: BTreeMap::new(),
            limits: BTreeMap::new(),
            times_ns: Vec::new(),
            notes: Vec::new(),
            repeats: 0,
            clock_ns: 0,
        }
    }
}

/// 一条可执行负载。
pub struct Workload {
    pub name: &'static str,
    pub run: fn() -> PerfResult<Measurement>,
}

/// 门禁注册表；顺序即输出顺序。
pub const WORKLOADS: &[Workload] = &[
    Workload {
        name: "intent_noop",
        run: intent_noop,
    },
    Workload {
        name: "signal_equal_write",
        run: signal_equal_write,
    },
    Workload {
        name: "toggle_row_10k",
        run: toggle_row_10k,
    },
    Workload {
        name: "typing_10k",
        run: typing_10k,
    },
    Workload {
        name: "scroll_10k_within_row",
        run: scroll_10k_within_row,
    },
    Workload {
        name: "scroll_10k_cross_row",
        run: scroll_10k_cross_row,
    },
    Workload {
        name: "theme_switch",
        run: theme_switch,
    },
    Workload {
        name: "multi_window_3",
        run: multi_window_3,
    },
    Workload {
        name: "memory_soak",
        run: memory_soak,
    },
];

#[component]
fn BenchWindow(store: Store, rows: usize) -> MountResult<MountPlan> {
    let draft = signal(String::new());
    let filter = signal(Filter::All);
    let visible = memo(move || store.rows(filter.get()));

    view! {
        <view>
            <text-input
                aria:label="draft"
                value={draft}
                on:input={move |value: String| draft.set(value)}
            />
            <button
                aria:label="seed"
                on:click={move || {
                    let mut transaction = Transaction::new();
                    for index in 0..rows {
                        transaction = transaction.write(store.insert(format!("row {index:05}")));
                    }
                    let _ = transaction.finish();
                }}
            >"Seed"</button>
            <button
                aria:label="noop"
                on:click={move || {
                    let _ = Transaction::new().finish();
                }}
            >"Noop"</button>
            <button
                aria:label="mutate"
                on:click={move || {
                    let keys = store.keys(Filter::All);
                    let mut transaction = Transaction::new();
                    if let Some(first) = keys.first() {
                        transaction = transaction.write(store.remove(*first));
                    }
                    transaction = transaction.write(store.insert("churn"));
                    let _ = transaction.finish();
                }}
            >"Mutate"</button>
            <button
                aria:label="filter"
                on:click={move || {
                    filter.set(if filter.get() == Filter::All {
                        Filter::Active
                    } else {
                        Filter::All
                    });
                }}
            >"Filter"</button>
            <view>
                <For
                    each={visible}
                    store={store}
                    key={|row: &TodoRow| row.id}
                    row-height={px(ROW_HEIGHT_PX)}
                >
                    {move |row: RowView<TodoRow>| {
                        let id = row.key();
                        let completed = store.completed(id);
                        view! {
                            <view>
                                <text>{once(row.get().title.clone())}</text>
                                <checkbox
                                    aria:label={once(format!("done {}", row.get().title))}
                                    checked={completed}
                                    on:change={move |done: bool| {
                                        let _ = Transaction::new()
                                            .write(store.set_completed(id, done))
                                            .finish();
                                    }}
                                />
                            </view>
                        }
                    }}
                </For>
            </view>
        </view>
    }
}

#[component]
fn DirectRows(store: Store, rows: usize, shared: Signal<String>) -> MountResult<MountPlan> {
    let visible = memo(move || store.rows(Filter::All));

    view! {
        <view>
            <button
                aria:label="seed"
                on:click={move || {
                    let mut transaction = Transaction::new();
                    for index in 0..rows {
                        transaction = transaction.write(store.insert(format!("row {index:05}")));
                    }
                    let _ = transaction.finish();
                }}
            >"Seed"</button>
            <button
                aria:label="equal"
                on:click={move || shared.set(shared.get())}
            >"Equal"</button>
            <For each={visible} store={store} key={|row: &TodoRow| row.id} row-height={px(ROW_HEIGHT_PX)}>
                {move |_row: RowView<TodoRow>| {
                    let value = shared;
                    view! { <view><text>{value}</text></view> }
                }}
            </For>
        </view>
    }
}

#[component]
fn MultiPane(detail: Signal<u64>, show_detail: bool) -> MountResult<MountPlan> {
    let shown = memo(move || format!("detail {}", detail.get()));

    view! {
        <view>
            <button
                aria:label="bump"
                on:click={move || detail.set(detail.get().wrapping_add(1))}
            >"Bump"</button>
            <Show when={once(show_detail)}><text>{shown}</text></Show>
            <text class="dark:text-muted">{once("summary")}</text>
        </view>
    }
}

fn single(rows: usize) -> PerfResult<(Runtime, WindowCtx)> {
    let mut app = Runtime::new();
    let store = app
        .setup(Store::new)
        .map_err(|error| fail(error.to_string()))?;
    let window = app
        .open_window("main", move |_| {
            view! { <BenchWindow store={store} rows={rows} /> }
        })
        .map_err(|error| fail(error.to_string()))?;
    app.click_label("seed")
        .map_err(|error| fail(error.to_string()))?;
    Ok((app, window))
}

fn direct(rows: usize) -> PerfResult<(Runtime, WindowCtx)> {
    let mut app = Runtime::new();
    let (store, shared) = app
        .setup(|| (Store::new(), signal(String::from("shared"))))
        .map_err(|error| fail(error.to_string()))?;
    let window = app
        .open_window("main", move |_| {
            view! { <DirectRows store={store} rows={rows} shared={shared} /> }
        })
        .map_err(|error| fail(error.to_string()))?;
    app.click_label("seed")
        .map_err(|error| fail(error.to_string()))?;
    let band = row_band(&app, window)?;
    app.set_row_window(window, band, 0, rows)
        .map_err(|error| fail(error.to_string()))?;
    Ok((app, window))
}

fn three_windows_with(
    mut app: Runtime,
    detail: Signal<u64>,
) -> PerfResult<(Runtime, Vec<WindowCtx>)> {
    app.declare_windows(move |catalog| {
        for (key, primary, show_detail) in
            [("a", true, true), ("b", false, false), ("c", false, false)]
        {
            let mut spec = WindowSpec::new(key);
            if primary {
                spec = spec.primary();
            }
            catalog.window(spec, move |_| {
                view! { <MultiPane detail={detail} show_detail={show_detail} /> }
            })?;
        }
        Ok(())
    })
    .map_err(|error| fail(error.to_string()))?;
    let mut windows = Vec::with_capacity(3);
    for key in ["a", "b", "c"] {
        windows.push(app.show(key).map_err(|error| fail(error.to_string()))?);
    }
    Ok((app, windows))
}

fn row_band(app: &Runtime, window: WindowCtx) -> PerfResult<u64> {
    app.presentation_of(window)
        .map_err(|error| fail(error.to_string()))?
        .row_bands()
        .first()
        .map(|band| band.key())
        .ok_or_else(|| fail("the fixture must mount a windowed For"))
}

fn limits(entries: &[(&str, u64)]) -> BTreeMap<String, u64> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect()
}

fn intent_noop() -> PerfResult<Measurement> {
    let (app, window) = single(ROWS)?;
    let mut harness = WindowHarness::new(app, window, true)?;
    let samples = SampleSet::run(|| {
        repeat(128, || {
            harness.measure(|app| {
                app.click_label("noop")
                    .map_err(|error| fail(error.to_string()))?;
                Ok(())
            })
        })
    })?;
    Ok(Measurement {
        counters: samples.max,
        limits: limits(&[
            ("bindings_evaluated", 0),
            ("memo_recomputed", 0),
            ("snapshot_roots_built", 0),
            ("snapshot_nodes_rebuilt", 0),
            ("delta_full_roots", 0),
            ("layout_roots", 0),
            ("scene_builds", 0),
            ("changed_turns", 0),
            ("revision_advanced", 0),
            ("mounted_nodes_growth", 0),
            ("allocations", 192),
        ]),
        times_ns: samples.times_ns,
        repeats: 128,
        clock_ns: samples.clock_ns,
        notes: vec![String::from(
            "empty handler plus an empty Store transaction; fixed per-turn scratch may allocate",
        )],
    })
}

fn signal_equal_write() -> PerfResult<Measurement> {
    let (app, window) = direct(ROWS)?;
    let mut harness = WindowHarness::new(app, window, false)?;
    let samples = SampleSet::run(|| {
        repeat(4, || {
            harness.measure(|app| {
                app.click_label("equal")
                    .map_err(|error| fail(error.to_string()))?;
                Ok(())
            })
        })
    })?;
    Ok(Measurement {
        counters: samples.max,
        limits: limits(&[
            ("bindings_evaluated", 1),
            ("memo_recomputed", 0),
            ("snapshot_roots_built", 0),
            ("snapshot_nodes_rebuilt", 0),
            ("delta_full_roots", 0),
            ("scene_builds", 0),
            ("changed_turns", 0),
            ("revision_advanced", 0),
            ("mounted_nodes_growth", 0),
            ("allocations", 1000),
        ]),
        times_ns: samples.times_ns,
        repeats: 4,
        clock_ns: samples.clock_ns,
        notes: vec![format!(
            "{ROWS} mounted rows read one shared Signal directly; an equal write must not evaluate them"
        )],
    })
}

fn toggle_row_10k() -> PerfResult<Measurement> {
    let (app, window) = single(ROWS)?;
    let mut harness = WindowHarness::new(app, window, true)?;
    let checkbox = harness.labeled_node("done row 00000")?;
    let mut done = false;
    let samples = SampleSet::run(|| {
        repeat(16, || {
            done = !done;
            harness.measure(|app| {
                // 宿主 checkbox 的用户改变；click 只派发 click，不改变受控状态。
                app.change_node(checkbox, done)
                    .map_err(|error| fail(error.to_string()))?;
                Ok(())
            })
        })
    })?;
    Ok(Measurement {
        counters: samples.max,
        limits: limits(&[
            ("bindings_evaluated", 8),
            ("memo_recomputed", 0),
            ("snapshot_roots_built", 1),
            ("snapshot_nodes_rebuilt", 32),
            ("delta_full_roots", 0),
            ("layout_roots", 1),
            ("scene_builds", 1),
            ("changed_turns", 1),
            ("revision_advanced", 1),
            ("mounted_nodes_growth", 0),
            ("scene_elements", 128),
            ("allocations", 3600),
        ]),
        times_ns: samples.times_ns,
        repeats: 16,
        clock_ns: samples.clock_ns,
        notes: vec![String::from(
            "alternating completion on the first visible row of a 32-row window over 10k rows",
        )],
    })
}

fn typing_10k() -> PerfResult<Measurement> {
    let (app, window) = single(ROWS)?;
    let mut harness = WindowHarness::new(app, window, true)?;
    let editor = harness.labeled_node("draft")?;
    let samples = SampleSet::run(|| {
        repeat(32, || {
            let sample = harness.measure(|app| {
                app.type_text_node(editor, "x")
                    .map_err(|error| fail(error.to_string()))?;
                Ok(())
            })?;
            // 计时区外撤销这个字符；输入长度保持常量，布局成本不随样本增长。
            harness
                .app
                .backspace_node(editor)
                .map_err(|error| fail(error.to_string()))?;
            harness.resync()?;
            Ok(sample)
        })
    })?;
    Ok(Measurement {
        counters: samples.max,
        limits: limits(&[
            ("bindings_evaluated", 4),
            ("memo_recomputed", 0),
            ("snapshot_roots_built", 1),
            ("snapshot_nodes_rebuilt", 4),
            ("delta_full_roots", 0),
            ("layout_roots", 1),
            ("scene_builds", 1),
            ("changed_turns", 1),
            ("revision_advanced", 1),
            ("mounted_nodes_growth", 0),
            ("scene_elements", 128),
            ("allocations", 2000),
        ]),
        times_ns: samples.times_ns,
        repeats: 32,
        clock_ns: samples.clock_ns,
        notes: vec![String::from(
            "one inserted character per turn in an editor beside a 10k-row windowed list",
        )],
    })
}

fn scroll_10k_within_row() -> PerfResult<Measurement> {
    let (app, window) = single(ROWS)?;
    let mut harness = WindowHarness::new(app, window, true)?;
    let mut offset = 0_i32;
    let samples = SampleSet::run(|| {
        repeat(64, || {
            offset = if offset == 0 { 5 } else { 0 };
            harness.set_scroll_offset(offset);
            harness.measure_scene()
        })
    })?;
    Ok(Measurement {
        counters: samples.max,
        limits: limits(&[
            ("changed_turns", 0),
            ("snapshot_roots_built", 0),
            ("snapshot_nodes_rebuilt", 0),
            ("bindings_evaluated", 0),
            ("memo_recomputed", 0),
            ("scene_builds", 1),
            ("scene_elements", 128),
            ("allocations", 1200),
        ]),
        times_ns: samples.times_ns,
        repeats: 64,
        clock_ns: samples.clock_ns,
        notes: vec![String::from(
            "5 px offset changes inside a 20 px row: scene-only, no runtime turn",
        )],
    })
}

fn scroll_10k_cross_row() -> PerfResult<Measurement> {
    let (app, window) = single(ROWS)?;
    let mut harness = WindowHarness::new(app, window, true)?;
    let band = row_band(&harness.app, harness.window)?;
    let window = harness.window;
    let mut first = 0_usize;
    let samples = SampleSet::run(|| {
        repeat(32, || {
            first = if first == 0 { 1 } else { 0 };
            harness.measure(|app| {
                app.set_row_window(window, band, first, WINDOW_ROWS)
                    .map_err(|error| fail(error.to_string()))?;
                Ok(())
            })
        })
    })?;
    Ok(Measurement {
        counters: samples.max,
        limits: limits(&[
            ("changed_turns", 1),
            ("bindings_evaluated", 8),
            ("memo_recomputed", 0),
            ("snapshot_roots_built", 1),
            ("snapshot_nodes_rebuilt", 32),
            ("delta_full_roots", 0),
            ("layout_roots", 1),
            ("scene_builds", 2),
            ("revision_advanced", 1),
            ("scene_elements", 128),
            ("allocations", 1900),
        ]),
        times_ns: samples.times_ns,
        repeats: 32,
        clock_ns: samples.clock_ns,
        notes: vec![String::from(
            "crossing one row mounts/unmounts one row; at most one structure turn and one scene build",
        )],
    })
}

fn theme_switch() -> PerfResult<Measurement> {
    let mut app = Runtime::new();
    let detail = app
        .setup(|| signal(0_u64))
        .map_err(|error| fail(error.to_string()))?;
    let (app, windows) = three_windows_with(app, detail)?;
    let mut harness = MultiWindowHarness::new(app, windows, true)?;
    let mut dark = false;
    let samples = SampleSet::run(|| {
        repeat(128, || {
            dark = !dark;
            harness.measure(|app| {
                app.set_appearance(if dark {
                    Appearance::Dark
                } else {
                    Appearance::Light
                })
                .map_err(|error| fail(error.to_string()))?;
                Ok(())
            })
        })
    })?;
    Ok(Measurement {
        counters: samples.max,
        limits: limits(&[
            ("windows_revised", 3),
            ("windows_unchanged", 0),
            ("unchanged_window_snapshot_builds", 0),
            ("snapshot_roots_built", 3),
            ("snapshot_nodes_rebuilt", 0),
            ("delta_full_roots", 0),
            ("appearance_changed_turns", 3),
            ("revision_advanced", 1),
            ("scene_builds", 3),
            ("snapshot_nodes_changed", 0),
            ("bindings_evaluated", 0),
            ("memo_recomputed", 0),
            ("scene_elements", 32),
            ("allocations", 450),
        ]),
        times_ns: samples.times_ns,
        repeats: 128,
        clock_ns: samples.clock_ns,
        notes: vec![String::from(
            "all three windows take a new revision; nodes are unchanged, scenes rebuild",
        )],
    })
}

fn multi_window_3() -> PerfResult<Measurement> {
    let mut app = Runtime::new();
    let detail = app
        .setup(|| signal(0_u64))
        .map_err(|error| fail(error.to_string()))?;
    let (app, windows) = three_windows_with(app, detail)?;
    let mut harness = MultiWindowHarness::new(app, windows, true)?;
    // 焦点路由是测量前的宿主准备；click 本身不会切换窗口。
    harness
        .app
        .show("a")
        .map_err(|error| fail(error.to_string()))?;
    let samples = SampleSet::run(|| {
        repeat(128, || {
            harness.measure(|app| {
                app.click_label("bump")
                    .map_err(|error| fail(error.to_string()))?;
                Ok(())
            })
        })
    })?;
    Ok(Measurement {
        counters: samples.max,
        limits: limits(&[
            ("windows_revised", 1),
            ("windows_unchanged", 2),
            ("unchanged_window_snapshot_builds", 0),
            ("snapshot_roots_built", 1),
            ("snapshot_nodes_rebuilt", 8),
            ("delta_full_roots", 0),
            ("scene_builds", 1),
            ("revision_advanced", 1),
            ("mounted_nodes_growth", 0),
            ("bindings_evaluated", 4),
            ("memo_recomputed", 4),
            ("layout_roots", 1),
            ("scene_elements", 16),
            ("allocations", 260),
        ]),
        times_ns: samples.times_ns,
        repeats: 128,
        clock_ns: samples.clock_ns,
        notes: vec![String::from(
            "only window a reads the changed signal; b and c must not build snapshots or scenes",
        )],
    })
}

fn memory_soak() -> PerfResult<Measurement> {
    let (app, window) = single(ROWS)?;
    let mut harness = WindowHarness::new(app, window, false)?;
    let band = row_band(&harness.app, harness.window)?;
    let window_ctx = harness.window;
    let cycle = |harness: &mut WindowHarness| -> PerfResult<()> {
        for first in [0_usize, 7_000, 0] {
            harness
                .app
                .set_row_window(window_ctx, band, first, WINDOW_ROWS)
                .map_err(|error| fail(error.to_string()))?;
            harness
                .app
                .click_label("mutate")
                .map_err(|error| fail(error.to_string()))?;
        }
        harness
            .app
            .click_label("filter")
            .map_err(|error| fail(error.to_string()))?;
        harness
            .app
            .click_label("filter")
            .map_err(|error| fail(error.to_string()))?;
        let _ = harness
            .app
            .refresh()
            .map_err(|error| fail(error.to_string()))?;
        Ok(())
    };

    cycle(&mut harness)?;
    let high = harness.app.storage_report();
    let started = Instant::now();
    for _ in 0..10 {
        cycle(&mut harness)?;
    }
    let elapsed = duration_ns(started.elapsed());
    let after = harness.app.storage_report();
    let memory = harness.app.memory_report();

    if memory.rowset_bytes < ROWS * std::mem::size_of::<RowId>() {
        return Err(fail(format!(
            "rowset bytes must count every row: {memory:?}"
        )));
    }
    if memory.rowset_count == 0 || memory.rowset_shared_count < memory.rowset_count * 2 {
        return Err(fail(format!(
            "each materialized projection must be shared by cache, memos and bindings: {memory:?}"
        )));
    }

    let mut counters = Counters::new();
    put(
        &mut counters,
        "mounted_node_bytes",
        memory.mounted_node_bytes as u64,
    );
    put(
        &mut counters,
        "for_bookkeeping_bytes",
        memory.for_bookkeeping_bytes as u64,
    );
    put(&mut counters, "rowset_bytes", memory.rowset_bytes as u64);
    put(
        &mut counters,
        "rowset_shared_count",
        memory.rowset_shared_count as u64,
    );
    put(&mut counters, "rowset_count", memory.rowset_count as u64);
    put(&mut counters, "mounted_nodes", memory.live_nodes as u64);
    let mut growth = |name: &str, high: usize, after: usize| {
        put(
            &mut counters,
            &format!("storage_growth_{name}"),
            after.saturating_sub(high) as u64,
        );
        put(&mut counters, &format!("storage_{name}"), after as u64);
    };
    growth("node_slots", high.node_slots, after.node_slots);
    growth(
        "element_payloads",
        high.element_payloads,
        after.element_payloads,
    );
    growth("binding_slots", high.binding_slots, after.binding_slots);
    growth("aux_slots", high.aux_slots, after.aux_slots);
    growth("dep_entries", high.dep_entries, after.dep_entries);
    growth("consumer_edges", high.consumer_edges, after.consumer_edges);
    growth("signal_slots", high.signal_slots, after.signal_slots);
    growth("memo_slots", high.memo_slots, after.memo_slots);
    growth("memo_deps_live", high.memo_deps_live, after.memo_deps_live);
    growth(
        "memo_deps_storage",
        high.memo_deps_storage,
        after.memo_deps_storage,
    );
    growth(
        "memo_storage_bytes",
        high.memo_storage_bytes,
        after.memo_storage_bytes,
    );
    growth(
        "memo_deps_storage_bytes",
        high.memo_deps_storage_bytes,
        after.memo_deps_storage_bytes,
    );
    growth(
        "free_signal_slots",
        high.free_signal_slots,
        after.free_signal_slots,
    );
    growth(
        "free_memo_slots",
        high.free_memo_slots,
        after.free_memo_slots,
    );
    growth(
        "binding_consumer_edges",
        high.binding_consumer_edges,
        after.binding_consumer_edges,
    );
    growth(
        "binding_consumer_nodes",
        high.binding_consumer_nodes,
        after.binding_consumer_nodes,
    );
    growth(
        "field_write_entries",
        high.field_write_entries,
        after.field_write_entries,
    );
    growth(
        "write_epoch_entries",
        high.write_epoch_entries,
        after.write_epoch_entries,
    );
    growth("owner_records", high.owner_records, after.owner_records);
    put(
        &mut counters,
        "signal_memo_bytes",
        memory.signal_memo_bytes as u64,
    );
    put(&mut counters, "owner_bytes", memory.owner_bytes as u64);

    Ok(Measurement {
        counters,
        limits: limits(&[
            ("storage_growth_node_slots", 32),
            ("storage_growth_element_payloads", 32),
            ("storage_growth_binding_slots", 128),
            ("storage_growth_aux_slots", 256),
            ("storage_growth_dep_entries", 8),
            ("storage_growth_consumer_edges", 64),
            ("storage_growth_signal_slots", 0),
            ("storage_growth_memo_slots", 0),
            ("storage_growth_memo_deps_live", 16),
            ("storage_growth_memo_deps_storage", 16),
            ("storage_growth_memo_storage_bytes", 256),
            ("storage_growth_memo_deps_storage_bytes", 256),
            ("storage_growth_free_signal_slots", 4),
            ("storage_growth_free_memo_slots", 4),
            ("storage_growth_binding_consumer_edges", 64),
            ("storage_growth_binding_consumer_nodes", 16),
            ("storage_growth_field_write_entries", 8),
            ("storage_growth_write_epoch_entries", 8),
            ("storage_growth_owner_records", 4),
            ("signal_memo_bytes", 512 * 1024),
            ("owner_bytes", 256 * 1024),
            (
                "rowset_bytes",
                (ROWS * std::mem::size_of::<RowId>() * 2 + 512) as u64,
            ),
            ("mounted_node_bytes", 512 * 1024),
            ("for_bookkeeping_bytes", 512 * 1024),
            ("rowset_count", 2),
            ("rowset_shared_count", 32),
        ]),
        times_ns: Vec::new(),
        repeats: 0,
        clock_ns: crate::harness::clock_probe(),
        notes: vec![
            String::from("ten churn cycles after a warm-up high-water mark"),
            format!("soak wall time: {elapsed} ns"),
        ],
    })
}
