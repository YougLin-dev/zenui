//! 绑定求值计数：空的 turn 不拉入已结算的绑定，单行变化只求值它的槽。
//!
//! 性能门禁依赖这两个计数（`binding_evaluations`、`memo_recomputes`）；
//! 这里用固定窗口的 headless 运行时锁定语义，普通 CI 也会运行。

#![allow(unused_braces)]

use zenui::prelude::*;

#[component]
fn List(store: Store, rows: usize, shared: Signal<String>) -> MountResult<MountPlan> {
    let filter = signal(Filter::All);
    let visible = memo(move || store.rows(filter.get()));

    view! {
        <view>
            <button
                aria:label="seed"
                on:click={move || {
                    let mut transaction = Transaction::new();
                    for index in 0..rows {
                        transaction = transaction.write(store.insert(format!("row {index:04}")));
                    }
                    let _ = transaction.finish();
                }}
            >"Seed"</button>
            <button aria:label="noop" on:click={move || {
                let _ = Transaction::new().finish();
            }}>"Noop"</button>
            <button aria:label="equal" on:click={move || shared.set(shared.get())}>"Equal"</button>
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
                        let label = shared;
                        view! {
                            <view>
                                <text>{label}</text>
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

fn fixture(rows: usize) -> (Runtime, WindowCtx) {
    let mut app = Runtime::new();
    let (store, shared) = app
        .setup(|| (Store::new(), signal(String::from("shared"))))
        .expect("application setup");
    let window = app
        .open_window("main", move |_| {
            view! { <List store={store} rows={rows} shared={shared} /> }
        })
        .expect("open the list window");
    app.click_label("seed").expect("seed the rows");
    (app, window)
}

#[test]
fn settled_turns_evaluate_no_dynamic_bindings() {
    let (mut app, window) = fixture(100);
    // 第一次点击只建立焦点；之后的空 turn 必须完全跳过已结算的槽。
    app.click_label("noop").expect("focus the button");
    let bindings = app.binding_evaluations();
    let memos = app.memo_recomputes();
    let builds = app.presentation_builds();
    let revision = app.presentation_revision_of(window).expect("revision");

    let report = app.click_label("noop").expect("empty turn");
    assert!(!report.changed(), "an empty turn changes nothing");
    assert_eq!(
        app.binding_evaluations(),
        bindings,
        "no dynamic binding is re-evaluated when its source epoch is unchanged"
    );
    assert_eq!(app.memo_recomputes(), memos, "no memo recomputes");
    assert_eq!(
        app.presentation_builds(),
        builds,
        "no snapshot root is built"
    );
    assert_eq!(
        app.presentation_revision_of(window).expect("revision"),
        revision,
        "the tree revision stays put"
    );
}

#[test]
fn an_equal_signal_write_skips_direct_row_bindings() {
    let (mut app, _) = fixture(100);
    app.click_label("equal").expect("focus the button");
    let bindings = app.binding_evaluations();
    let memos = app.memo_recomputes();

    let report = app.click_label("equal").expect("equal write");
    assert!(!report.changed(), "an equal write changes nothing visible");
    assert_eq!(
        app.binding_evaluations(),
        bindings,
        "equal writes do not re-evaluate the direct row bindings"
    );
    assert_eq!(app.memo_recomputes(), memos, "no memo recomputes");
}

#[test]
fn a_single_row_change_stays_inside_its_window() {
    let (mut app, window) = fixture(1_000);
    let checkbox = app
        .presentation_of(window)
        .expect("snapshot")
        .find_label("done row 0000")
        .expect("the first row checkbox")
        .id();
    // 先发布一次建立 delta 基线；真实宿主也先有首帧。
    app.presentation_update_of(window)
        .expect("publish the seed frame");
    let bindings = app.binding_evaluations();
    let nodes = app.memory_report().live_nodes;

    app.change_node(checkbox, true)
        .expect("toggle the first row");

    assert_eq!(
        app.memory_report().live_nodes,
        nodes,
        "toggling does not mount or unmount rows"
    );
    let evaluated = app.binding_evaluations() - bindings;
    assert!(
        evaluated <= 8,
        "only the changed row's slots evaluate: {evaluated}"
    );
    let update = app.presentation_update_of(window).expect("publish");
    let delta = update.delta().expect("a contiguous delta");
    assert!(
        delta.changed().len() + delta.inserted().len() <= 32,
        "the delta stays within one row path: {} changed, {} inserted",
        delta.changed().len(),
        delta.inserted().len()
    );
}
