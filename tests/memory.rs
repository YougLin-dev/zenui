//! Split memory reporting for the runtime v2 close-out gates.
//!
//! The UI/For working set must follow the mounted row window, while the shared
//! `RowSet` identity projection follows the total row count and is reported
//! separately from the handles that share it.

#![allow(unused_braces)]

use zenui::prelude::*;

#[component]
fn List(store: Store, rows: usize) -> MountResult<MountPlan> {
    let filter = signal(Filter::All);
    let visible = memo(move || store.rows(filter.get()));

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
                aria:label="filter"
                on:click={move || {
                    filter.set(if filter.get() == Filter::All {
                        Filter::Active
                    } else {
                        Filter::All
                    });
                }}
            >"Filter"</button>
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
            <view>
                <For
                    each={visible}
                    store={store}
                    key={|row: &TodoRow| row.id}
                    row-height={px(20.0)}
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
                                />
                            </view>
                        }
                    }}
                </For>
            </view>
        </view>
    }
}

fn fixture(rows: usize) -> (Runtime, WindowCtx) {
    let mut app = Runtime::new();
    let store = app.setup(Store::new).expect("application setup");
    let window = app
        .open_window(
            "main",
            move |_| view! { <List store={store} rows={rows} /> },
        )
        .expect("open the list window");
    app.click_label("seed").expect("seed the rows");
    (app, window)
}

fn band(app: &Runtime, window: WindowCtx) -> u64 {
    app.presentation_of(window)
        .expect("window snapshot")
        .row_bands()
        .first()
        .expect("a windowed list")
        .key()
}

#[test]
fn mounted_nodes_follow_the_window_while_the_rowset_follows_the_total() {
    let (small, _) = fixture(1_000);
    let (large, _) = fixture(10_000);
    let small = small.memory_report();
    let large = large.memory_report();

    // 共享投影是 8 B/行，单独计费；两套 1k/10k 行必须在字节上明显不同。
    assert!(
        small.rowset_bytes >= 1_000 * 8,
        "the shared projection counts every row: {small:?}"
    );
    assert!(
        large.rowset_bytes >= 10_000 * 8,
        "the shared projection counts every row: {large:?}"
    );
    assert!(
        large.rowset_bytes > small.rowset_bytes * 5,
        "RowSet bytes scale with the total row count: small={small:?} large={large:?}"
    );

    // UI/For 持久内存只看默认/声明的行窗口（32 行），与总行数无关。
    assert!(
        large.mounted_node_bytes < small.mounted_node_bytes + 16 * 1024,
        "mounted nodes must follow the window, not the store: small={small:?} large={large:?}"
    );
    assert!(
        large.for_bookkeeping_bytes < small.for_bookkeeping_bytes + 16 * 1024,
        "For bookkeeping must follow the window: small={small:?} large={large:?}"
    );

    // 缓存、For 的已提交投影、memo 值与绑定槽共享同一个 Arc。
    assert!(
        large.rowset_shared_count >= 2,
        "the shared projection has multiple live handles: {large:?}"
    );
    assert_eq!(
        large.rowset_count, 1,
        "only the All projection is materialized: {large:?}"
    );
}

#[component]
fn Payload() -> MountResult<MountPlan> {
    // 4000+ 字节的 owned 值：释放后必须从 `signal_memo_bytes` 消失。
    let draft = signal("payload-".repeat(500));
    let len = memo(move || draft.get().len().to_string());
    view! { <text>{len}</text> }
}

#[component]
fn ReactiveChurn() -> MountResult<MountPlan> {
    let open = signal(false);
    view! {
        <view>
            <button aria:label="toggle" on:click={move || open.set(!open.get())}>"Toggle"</button>
            <Show when={open}>
                <Payload />
            </Show>
        </view>
    }
}

#[test]
fn reactive_slots_recycle_and_release_owned_value_bytes() {
    let mut app = Runtime::new();
    app.mount(|| view! { <ReactiveChurn /> }).expect("mount");
    let baseline = app.memory_report();
    let base_storage = app.storage_report();
    assert_eq!(baseline.live_signals, 1, "only the host toggle signal");
    assert_eq!(baseline.live_memos, 0);

    app.click_label("toggle").expect("open the payload");
    let opened = app.memory_report();
    let opened_storage = app.storage_report();
    assert_eq!(opened.live_signals, 2);
    assert_eq!(opened.live_memos, 1);
    assert!(
        opened.signal_memo_bytes >= 4_000,
        "the owned payload is counted: {opened:?}"
    );

    app.click_label("toggle").expect("close the payload");
    let closed = app.memory_report();
    assert_eq!(closed.live_signals, 1);
    assert_eq!(closed.live_memos, 0);
    assert!(
        closed.signal_memo_bytes <= baseline.signal_memo_bytes + 512,
        "the released payload bytes leave the report: baseline={baseline:?} closed={closed:?}"
    );
    let closed_storage = app.storage_report();
    assert_eq!(
        closed_storage.free_signal_slots,
        base_storage.free_signal_slots + 1,
        "the payload signal returns to the free list"
    );
    assert_eq!(
        closed_storage.free_memo_slots,
        base_storage.free_memo_slots + 1,
        "the payload memo returns to the free list"
    );

    for _ in 0..20 {
        app.click_label("toggle").expect("remount the payload");
        app.click_label("toggle").expect("release the payload");
    }
    let after = app.storage_report();
    assert_eq!(
        after.signal_slots, opened_storage.signal_slots,
        "signal slots are reused instead of appended: opened={opened_storage:?} after={after:?}"
    );
    assert_eq!(
        after.memo_slots, opened_storage.memo_slots,
        "memo slots are reused instead of appended: opened={opened_storage:?} after={after:?}"
    );
    assert_eq!(
        after.binding_consumer_edges, closed_storage.binding_consumer_edges,
        "consumer edges follow the mounted payload: after={after:?}"
    );
    assert_eq!(
        after.binding_consumer_nodes, closed_storage.binding_consumer_nodes,
        "consumer nodes follow the mounted payload: after={after:?}"
    );
    let final_mem = app.memory_report();
    assert!(
        final_mem.signal_memo_bytes <= baseline.signal_memo_bytes + 512,
        "churn does not retain payload bytes: baseline={baseline:?} final={final_mem:?}"
    );
}

#[test]
fn two_windows_share_one_rowset_projection() {
    let mut app = Runtime::new();
    let store = app.setup(Store::new).expect("application setup");
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |_| {
            view! { <List store={store} rows={200usize} /> }
        })?;
        catalog.window(WindowSpec::new("side"), move |_| {
            view! { <List store={store} rows={200usize} /> }
        })?;
        Ok(())
    })
    .expect("declare the two windows");
    let main = app.show("main").expect("show the main window");
    let side = app.show("side").expect("show the side window");
    app.click_label("seed")
        .expect("seed through the focused window");

    let report = app.memory_report();
    assert_eq!(report.rowset_count, 1, "both windows read the All filter");
    assert!(
        report.rowset_shared_count >= 4,
        "cache, two For projections, memos and bindings share one Arc: {report:?}"
    );
    for window in [main, side] {
        assert!(
            !app.presentation_of(window)
                .expect("snapshot")
                .row_bands()
                .is_empty(),
            "both windows mount a windowed list"
        );
    }
}

#[test]
fn storage_stays_bounded_across_window_and_row_churn() {
    let (mut app, window) = fixture(1_000);
    let cycle = |app: &mut Runtime| {
        for first in [0usize, 900, 0] {
            let band = band(app, window);
            app.set_row_window(window, band, first, 32)
                .expect("move the row window");
            app.click_label("mutate").expect("remove and add a row");
        }
        app.click_label("filter").expect("hide completed");
        app.click_label("filter").expect("show all");
    };

    // 第一轮建立与 churn 历史无关的高水位。
    cycle(&mut app);
    let high = app.storage_report();
    for _ in 0..10 {
        cycle(&mut app);
    }
    let after = app.storage_report();
    assert!(
        after.node_slots <= high.node_slots + 32,
        "node slots reuse their pages: high={high:?} after={after:?}"
    );
    assert!(
        after.element_payloads <= high.element_payloads + 32,
        "element payloads reuse slots: high={high:?} after={after:?}"
    );
    assert!(
        after.binding_slots <= high.binding_slots + 128,
        "binding slots reuse cells: high={high:?} after={after:?}"
    );
    assert!(
        after.aux_slots <= high.aux_slots + 256,
        "aux lists reuse slots: high={high:?} after={after:?}"
    );
    assert!(
        after.dep_entries <= high.dep_entries + 8
            && after.consumer_edges <= high.consumer_edges + 64,
        "dependency storage does not grow with churn history: high={high:?} after={after:?}"
    );
    assert!(
        after.signal_slots == high.signal_slots && after.memo_slots == high.memo_slots,
        "no signals or memos are attached per churn: high={high:?} after={after:?}"
    );
    assert!(
        after.memo_deps_storage <= high.memo_deps_storage + 8
            && after.memo_storage_bytes == high.memo_storage_bytes
            && after.memo_deps_storage_bytes <= high.memo_deps_storage_bytes + 256,
        "memo and dependency storage are reused across churn: high={high:?} after={after:?}"
    );
    assert!(
        after.binding_consumer_edges <= high.binding_consumer_edges + 64
            && after.binding_consumer_nodes <= high.binding_consumer_nodes + 64,
        "binding consumer edges follow mounted nodes: high={high:?} after={after:?}"
    );
    assert!(
        after.field_write_entries <= high.field_write_entries + 8
            && after.write_epoch_entries <= high.write_epoch_entries + 8,
        "deleted rows release their interaction history: high={high:?} after={after:?}"
    );
    assert!(
        after.owner_records <= high.owner_records + 4,
        "owner records follow mounted structure: high={high:?} after={after:?}"
    );
    assert!(
        app.memory_report().owner_bytes <= 64 * 1024,
        "owner bookkeeping stays bounded: {:?}",
        app.memory_report()
    );
}
