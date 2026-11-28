#![allow(unused_braces)]

//! #340 验收：flush step 0 quarantine、结构提交失败与 `PendingChanges` 累计。
//!
//! 观察点全部来自公开 UI/runtime：行 memo 的求值计数、存活资源数量、文本与
//! 焦点结算；不直接读取内部隔离集合。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use zenui::model::{FieldDef, FieldId, FilterId, FilterSpec, SchemaBuilder};
use zenui::prelude::*;

fn mount(root: impl FnOnce() -> MountResult<MountPlan> + 'static) -> Runtime {
    let mut app = Runtime::new();
    app.mount(root).expect("mount");
    app
}

fn text_present(app: &Runtime, wanted: &str) -> bool {
    app.texts().iter().any(|text| text == wanted)
}

fn report_runs(app: &mut Runtime) -> String {
    app.click_label("report").expect("report the memo runs");
    app.texts()
        .iter()
        .find(|text| text.starts_with("runs "))
        .cloned()
        .expect("the report text")
}

/// 行 memo 每求值一次就把共享计数器加一；quarantine 的效果因此可以从
/// 公开 runtime 直接观察。
#[component]
fn QuarantineHost() -> MountResult<MountPlan> {
    let store = Store::new();
    let rows = memo(move || store.rows(Filter::All));
    let a_key = signal(None::<TodoId>);
    let b_key = signal(None::<TodoId>);
    let runs = Rc::new(Cell::new(0u32));
    let report = signal(String::from("runs 0"));
    let report_runs = Rc::clone(&runs);

    view! {
        <view>
            <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                Transaction::new()
                    .write(store.insert("A"))
                    .write(store.insert("B"))
                    .finish()?;
                let keys = store.keys(Filter::All);
                a_key.set(Some(keys[0]));
                b_key.set(Some(keys[1]));
                Ok(())
            }}>"Seed"</button>
            <button aria:label="delete-a" on:click={move || -> Result<(), WriteError> {
                if let Some(key) = a_key.get() {
                    Transaction::new().write(store.remove(key)).finish()?;
                }
                Ok(())
            }}>"DeleteA"</button>
            <button aria:label="delete-both" on:click={move || -> Result<(), WriteError> {
                // 两个独立事务：PendingChanges 必须累计到 flush 结束。
                if let Some(key) = a_key.get() {
                    Transaction::new().write(store.remove(key)).finish()?;
                }
                if let Some(key) = b_key.get() {
                    Transaction::new().write(store.remove(key)).finish()?;
                }
                Ok(())
            }}>"DeleteBoth"</button>
            <button aria:label="break" on:click={move || -> Result<(), WriteError> {
                let mut transaction = Transaction::new();
                if let Some(key) = a_key.get() {
                    transaction = transaction.write(store.remove(key));
                }
                // "fresh" 会先成功挂载，"boom" 随后失败：拟议窗口必须把已经
                // 挂载成功的 fresh 行与资源一并回滚。
                transaction
                    .write(store.insert("fresh"))
                    .write(store.insert("boom"))
                    .finish()
            }}>"Break"</button>
            <button aria:label="rename-b" on:click={move || -> Result<(), WriteError> {
                if let Some(key) = b_key.get() {
                    Transaction::new().write(store.set_title(key, "B2")).finish()?;
                }
                Ok(())
            }}>"RenameB"</button>
            <button aria:label="heal" on:click={move || -> Result<(), WriteError> {
                let broken: Vec<TodoId> = store
                    .keys(Filter::All)
                    .into_iter()
                    .filter(|key| {
                        matches!(
                            store.title(*key).read_once().ok().as_deref(),
                            Some("boom" | "fresh")
                        )
                    })
                    .collect();
                let mut transaction = Transaction::new();
                for key in broken {
                    transaction = transaction.write(store.remove(key));
                }
                transaction.finish()
            }}>"Heal"</button>
            <button aria:label="report" on:click={move || {
                report.set(format!("runs {}", report_runs.get()));
            }}>"Report"</button>
            <text>{report}</text>
            <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                {move |row: RowView<TodoRow>| {
                    let id = row.key();
                    let title = row.get().title.clone();
                    let counter = Rc::clone(&runs);
                    let label = memo(move || {
                        counter.set(counter.get() + 1);
                        match store.title(id).get() {
                            Ok(title) => format!("memo {title}"),
                            Err(FieldError::Missing(_)) => String::from("memo gone"),
                            Err(other) => panic!("unexpected field error: {other}"),
                        }
                    });
                    // 行 owner 的资源：factory 失败时必须随拟议行一起释放。
                    let _marker = signal(title.clone());
                    if title == "boom" {
                        return Err(MountError::Component("row boom"));
                    }
                    view! { <text>{label}</text> }
                }}
            </For>
        </view>
    }
}

#[test]
fn a_deleted_rows_memo_is_not_recomputed_before_the_structure_commit() {
    let mut app = mount(|| view! { <QuarantineHost /> });
    app.click_label("seed").expect("seed A and B");
    assert!(text_present(&app, "memo A"));
    assert!(text_present(&app, "memo B"));
    assert_eq!(report_runs(&mut app), "runs 2", "each row memo ran once");

    app.click_label("delete-a").expect("delete A");
    assert!(
        !text_present(&app, "memo A"),
        "the mounted row is unmounted"
    );
    assert_eq!(
        report_runs(&mut app),
        "runs 2",
        "the deleted row's memo is quarantined, not recomputed before unmount"
    );
}

#[test]
fn a_failed_for_factory_releases_fresh_rows_and_keeps_retained_rows() {
    let mut app = mount(|| view! { <QuarantineHost /> });
    app.click_label("seed").expect("seed A and B");
    let signals = app.live_signals();
    let memos = app.live_memos();
    assert_eq!(report_runs(&mut app), "runs 2");

    let report = app
        .click_label("break")
        .expect("the boom row factory failure is contained");
    assert!(
        matches!(
            report.errors(),
            [TurnError::FactoryFailed(MountError::Component("row boom"))]
        ),
        "unexpected failure: {:?}",
        report.errors()
    );
    assert_eq!(
        app.live_signals(),
        signals,
        "the rejected fresh row releases its signal"
    );
    assert_eq!(
        app.live_memos(),
        memos,
        "the rejected fresh row releases its memo"
    );
    assert!(
        text_present(&app, "memo gone"),
        "the retained deleted row is still mounted and evaluates its binding"
    );
    assert!(text_present(&app, "memo B"));

    let report = app
        .click_label("rename-b")
        .expect("the failing row factory result is contained");
    assert!(
        matches!(
            report.errors(),
            [TurnError::FactoryFailed(MountError::Component("row boom"))]
        ),
        "unexpected failure: {:?}",
        report.errors()
    );
    assert!(
        text_present(&app, "memo B2"),
        "the retained row keeps its field subscription across failed commits"
    );

    app.click_label("heal").expect("remove the failing row");
    assert!(
        !text_present(&app, "memo gone"),
        "the deleted row finally unmounts"
    );
    assert!(text_present(&app, "memo B2"));
    assert_eq!(
        app.live_memos(),
        memos - 1,
        "commit-time unsubscribe releases the unmounted row's memo"
    );
}

#[test]
fn pending_changes_accumulate_every_commit_until_the_flush_completes() {
    let mut app = mount(|| view! { <QuarantineHost /> });
    app.click_label("seed").expect("seed A and B");
    assert_eq!(report_runs(&mut app), "runs 2");

    app.click_label("delete-both")
        .expect("delete A and B in two transactions");
    assert!(!text_present(&app, "memo A"));
    assert!(!text_present(&app, "memo B"));
    assert_eq!(
        report_runs(&mut app),
        "runs 2",
        "both removals stay quarantined: the accumulator keeps every commit"
    );
}

/// 焦点结算的 blur 处理器删除一行；该提交必须在回到 memo 求值前进入
/// quarantine。
#[component]
fn SettlementCommitHost() -> MountResult<MountPlan> {
    let store = Store::new();
    let rows = memo(move || store.rows(Filter::All));
    let disable = signal(false);
    let disabled = memo(move || disable.get());
    let x_key = signal(None::<TodoId>);
    let runs = Rc::new(Cell::new(0u32));
    let report = signal(String::from("runs 0"));
    let report_runs = Rc::clone(&runs);

    view! {
        <view>
            <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                Transaction::new()
                    .write(store.insert("A"))
                    .write(store.insert("X"))
                    .finish()?;
                x_key.set(store.keys(Filter::All).get(1).copied());
                Ok(())
            }}>"Seed"</button>
            <button aria:label="disable" on:click={move || disable.set(true)}>"Disable"</button>
            <button aria:label="report" on:click={move || {
                report.set(format!("runs {}", report_runs.get()));
            }}>"Report"</button>
            <text>{report}</text>
            <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                {move |row: RowView<TodoRow>| {
                    let id = row.key();
                    let title = row.get().title.clone();
                    let is_a = title == "A";
                    let counter = Rc::clone(&runs);
                    let label = memo(move || {
                        counter.set(counter.get() + 1);
                        match store.title(id).get() {
                            Ok(title) => format!("memo {title}"),
                            Err(FieldError::Missing(_)) => String::from("memo gone"),
                            Err(other) => panic!("unexpected field error: {other}"),
                        }
                    });
                    let row_disabled = memo(move || is_a && disabled.get());
                    view! {
                        <button
                            aria:label={once(format!("row {title}"))}
                            disabled={row_disabled}
                            on:blur={move |_event: BlurEvent| {
                                if let Some(key) = x_key.get()
                                    && store.row_id(key).is_some()
                                {
                                    Transaction::new()
                                        .write(store.remove(key))
                                        .finish()
                                        .expect("delete X from the settlement blur");
                                }
                            }}
                        >
                            <text>{label}</text>
                        </button>
                    }
                }}
            </For>
        </view>
    }
}

#[test]
fn a_commit_from_focus_settlement_is_quarantined_before_memos() {
    let mut app = mount(|| view! { <SettlementCommitHost /> });
    app.click_label("seed").expect("seed A and X");
    let x_node = app
        .presentation()
        .expect("snapshot")
        .find_label("row X")
        .expect("row X")
        .id();
    app.focus("row A").expect("focus A");
    assert_eq!(report_runs(&mut app), "runs 2");

    // 禁用聚焦行触发结算 blur；blur 处理器删除 X。结算轮次必须在 memo
    // 求值之前 quarantine 这次提交，否则 X 的 memo 会在卸载前重算一次。
    app.click_label("disable").expect("disable A");
    assert_eq!(app.node_status(x_node), NodeStatus::Stale);
    assert!(!text_present(&app, "memo X"));
    assert_eq!(
        report_runs(&mut app),
        "runs 2",
        "the settlement commit is quarantined before memo evaluation"
    );
}

/// Show/For 结构触发必须使用本 turn 写入的值。
#[component]
fn SameTurnShowHost() -> MountResult<MountPlan> {
    let open = signal(false);
    let label = signal(String::from("before"));
    let shown = memo(move || format!("shown {}", label.get()));

    view! {
        <view>
            <button aria:label="go" on:click={move || {
                label.set(String::from("after"));
                open.set(true);
            }}>"Go"</button>
            <Show when={open}>
                <text>{shown}</text>
            </Show>
        </view>
    }
}

#[test]
fn show_mounts_the_branch_with_the_value_written_in_the_same_turn() {
    let mut app = mount(|| view! { <SameTurnShowHost /> });
    assert!(!text_present(&app, "shown after"));

    app.click_label("go").expect("open the branch");
    assert!(
        text_present(&app, "shown after"),
        "the branch mounts with this turn's value: {:?}",
        app.texts()
    );
    assert!(!text_present(&app, "shown before"));
}

#[component]
fn SameTurnForHost() -> MountResult<MountPlan> {
    let store = Store::new();
    let rows = memo(move || store.rows(Filter::All));
    let label = signal(String::from("before"));
    let shown = memo(move || format!("shown {}", label.get()));

    view! {
        <view>
            <button aria:label="go" on:click={move || -> Result<(), WriteError> {
                label.set(String::from("after"));
                Transaction::new().write(store.insert("A")).finish()
            }}>"Go"</button>
            <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                {move |_row: RowView<TodoRow>| view! { <text>{shown}</text> }}
            </For>
        </view>
    }
}

#[test]
fn for_mounts_rows_with_the_value_written_in_the_same_turn() {
    let mut app = mount(|| view! { <SameTurnForHost /> });
    assert!(!text_present(&app, "shown after"));

    app.click_label("go").expect("insert the first row");
    assert!(
        text_present(&app, "shown after"),
        "the fresh row mounts with this turn's projection and value: {:?}",
        app.texts()
    );
}

/// 组件级 memo 只被隔离行上的绑定消费时也不能结算；行 memo 的上游依赖同理。
#[component]
fn DeepQuarantineHost() -> MountResult<MountPlan> {
    let store = Store::new();
    let rows = memo(move || store.rows(Filter::All));
    let a_key = signal(None::<TodoId>);
    let outer_runs = Rc::new(Cell::new(0u32));
    let inner_runs = Rc::new(Cell::new(0u32));
    let label_runs = Rc::new(Cell::new(0u32));
    let report = signal(String::from("runs 0 0 0"));

    let outer_count = Rc::clone(&outer_runs);
    let outer = memo(move || {
        outer_count.set(outer_count.get() + 1);
        match a_key.get() {
            Some(key) => match store.title(key).get() {
                Ok(title) => format!("outer {title}"),
                Err(FieldError::Missing(_)) => String::from("outer gone"),
                Err(other) => panic!("unexpected field error: {other}"),
            },
            None => String::from("outer idle"),
        }
    });
    let inner_count = Rc::clone(&inner_runs);
    let inner = memo(move || {
        inner_count.set(inner_count.get() + 1);
        match a_key.get() {
            Some(key) => match store.title(key).get() {
                Ok(title) => format!("inner {title}"),
                Err(FieldError::Missing(_)) => String::from("inner gone"),
                Err(other) => panic!("unexpected field error: {other}"),
            },
            None => String::from("inner idle"),
        }
    });
    let label_count = Rc::clone(&label_runs);
    let outer_report = Rc::clone(&outer_runs);
    let inner_report = Rc::clone(&inner_runs);
    let label_report = Rc::clone(&label_runs);

    view! {
        <view>
            <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                Transaction::new().write(store.insert("A")).finish()?;
                a_key.set(store.keys(Filter::All).first().copied());
                Ok(())
            }}>"Seed"</button>
            <button aria:label="delete-a" on:click={move || -> Result<(), WriteError> {
                if let Some(key) = a_key.get() {
                    Transaction::new().write(store.remove(key)).finish()?;
                }
                Ok(())
            }}>"DeleteA"</button>
            <button aria:label="report" on:click={move || {
                report.set(format!(
                    "runs {} {} {}",
                    outer_report.get(),
                    inner_report.get(),
                    label_report.get()
                ));
            }}>"Report"</button>
            <text>{report}</text>
            <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                {move |row: RowView<TodoRow>| {
                    let id = row.key();
                    debug_assert_eq!(id, row.key());
                    let counter = Rc::clone(&label_count);
                    let label = memo(move || {
                        counter.set(counter.get() + 1);
                        format!("label {} / {}", inner.get(), outer.get())
                    });
                    view! { <text>{label}</text> }
                }}
            </For>
        </view>
    }
}

#[test]
fn quarantine_prunes_row_bindings_and_their_upstream_memos() {
    let mut app = mount(|| view! { <DeepQuarantineHost /> });
    app.click_label("seed").expect("seed the row");
    assert_eq!(report_runs(&mut app), "runs 1 1 1");

    app.click_label("delete-a").expect("delete the row");
    assert_eq!(
        report_runs(&mut app),
        "runs 1 1 1",
        "component-level consumers and upstream deps of a quarantined row do not settle"
    );
}

/// 过滤移出：行仍存活但离开投影，也必须在本轮 memo 求值前进入隔离。
#[component]
fn FilterLeaveHost() -> MountResult<MountPlan> {
    let store = Store::new();
    let rows = memo(move || store.rows(Filter::Active));
    let a_key = signal(None::<TodoId>);
    let runs = Rc::new(Cell::new(0u32));
    let report = signal(String::from("runs 0"));
    let report_runs = Rc::clone(&runs);

    view! {
        <view>
            <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                Transaction::new()
                    .write(store.insert("A"))
                    .write(store.insert("B"))
                    .finish()?;
                a_key.set(store.keys(Filter::All).first().copied());
                Ok(())
            }}>"Seed"</button>
            <button aria:label="complete-a" on:click={move || -> Result<(), WriteError> {
                if let Some(key) = a_key.get() {
                    Transaction::new().write(store.set_completed(key, true)).finish()?;
                }
                Ok(())
            }}>"CompleteA"</button>
            <button aria:label="report" on:click={move || {
                report.set(format!("runs {}", report_runs.get()));
            }}>"Report"</button>
            <text>{report}</text>
            <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                {move |row: RowView<TodoRow>| {
                    let id = row.key();
                    let counter = Rc::clone(&runs);
                    let label = memo(move || {
                        counter.set(counter.get() + 1);
                        match store.completed(id).get() {
                            Ok(done) => format!("done {done}"),
                            Err(FieldError::Missing(_)) => String::from("gone"),
                            Err(other) => panic!("unexpected field error: {other}"),
                        }
                    });
                    view! { <text>{label}</text> }
                }}
            </For>
        </view>
    }
}

#[test]
fn a_row_leaving_the_projection_is_quarantined_before_memos() {
    let mut app = mount(|| view! { <FilterLeaveHost /> });
    app.click_label("seed").expect("seed two active rows");
    assert_eq!(report_runs(&mut app), "runs 2");

    app.click_label("complete-a").expect("move A out of Active");
    assert_eq!(
        report_runs(&mut app),
        "runs 2",
        "the leaving row's memo is quarantined before the structural commit"
    );
}

/// `ForSource` 可以在 runtime 外构造；提交观察者必须在挂载期安装。
#[derive(Clone, Debug, PartialEq)]
struct Note {
    key: u32,
    text: String,
}

impl Row for Note {
    type Key = u32;

    fn key(&self) -> u32 {
        self.key
    }
}

const NOTE_TEXT: FieldId = FieldId::new(0);

/// 没有注册任何 filter 的 Store：行删除只产生 `ChangeSet.removed`，
/// quarantine 因此只能来自累计的删除日志（成员迁移规则覆盖不到）。
fn note_store() -> RowStore<Note> {
    RowStore::new(
        SchemaBuilder::new()
            .field(FieldDef::comparing(NOTE_TEXT, |row: &Note| &row.text))
            .build()
            .expect("note schema"),
    )
}

#[component]
fn PrebuiltHost(
    prebuilt: For<Note, u32>,
    store: RowStore<Note>,
    target: RowId,
    runs: Rc<Cell<u32>>,
    tick: Rc<RefCell<Option<Signal<u32>>>>,
) -> MountResult<MountPlan> {
    let report = signal(String::from("runs 0"));
    let report_runs = Rc::clone(&runs);
    let delete_store = store;

    view! {
        <view>
            <button aria:label="delete" on:click={move || {
                if let Some(tick) = *tick.borrow() {
                    tick.set(tick.get() + 1);
                }
                delete_store.remove(target).expect("remove the prebuilt row");
            }}>"Delete"</button>
            <button aria:label="report" on:click={move || {
                report.set(format!("runs {}", report_runs.get()));
            }}>"Report"</button>
            <text>{report}</text>
            {prebuilt}
        </view>
    }
}

/// 构造一个挂载前就可用的 `For` 计划；工厂里的 signal/memo 只在真正挂载的
/// runtime 里创建。
fn prebuilt_plan(
    store: &RowStore<Note>,
    target: RowId,
    runs: &Rc<Cell<u32>>,
    tick: &Rc<RefCell<Option<Signal<u32>>>>,
) -> For<Note, u32> {
    let runs = Rc::clone(runs);
    let tick = Rc::clone(tick);
    For::new(
        ForSource::new(store.clone(), store.rowset([target])),
        |note: &Note| note.key,
        move |_row: RowView<Note>| {
            let row_tick = signal(0u32);
            *tick.borrow_mut() = Some(row_tick);
            let counter = Rc::clone(&runs);
            let label = memo(move || {
                counter.set(counter.get() + 1);
                format!("prebuilt {}", row_tick.get())
            });
            view! { <text>{label}</text> }
        },
    )
}

#[test]
fn a_prebuilt_for_source_installs_the_commit_observer_at_mount() {
    let store = note_store();
    let target = store
        .insert(Note {
            key: 1,
            text: String::from("A"),
        })
        .expect("insert the note")
        .0;
    let runs = Rc::new(Cell::new(0u32));
    let tick = Rc::new(RefCell::new(None::<Signal<u32>>));

    // 在 Runtime::new() 之前构造来源与 For：观察者必须在 For 真正挂载时
    // 安装，否则删除行不会进入 PendingChanges。
    let prebuilt = prebuilt_plan(&store, target, &runs, &tick);

    let mut app = Runtime::new();
    app.mount({
        let store = store.clone();
        let runs = Rc::clone(&runs);
        let tick = Rc::clone(&tick);
        move || {
            view! { <PrebuiltHost prebuilt={prebuilt} store={store} target={target} runs={runs} tick={tick} /> }
        }
    })
    .expect("mount the prebuilt For");
    assert!(text_present(&app, "prebuilt 0"));
    assert_eq!(runs.get(), 1);

    app.click_label("delete").expect("delete the row");
    app.click_label("report").expect("report the runs");
    assert!(
        text_present(&app, "runs 1"),
        "the mount-time observer quarantines the removed row: {:?}",
        app.texts()
    );
}

#[test]
fn two_runtimes_observe_one_store_without_stealing_commits() {
    let store = note_store();
    let target = store
        .insert(Note {
            key: 1,
            text: String::from("A"),
        })
        .expect("insert the note")
        .0;
    let runs_a = Rc::new(Cell::new(0u32));
    let runs_b = Rc::new(Cell::new(0u32));
    let tick_a = Rc::new(RefCell::new(None::<Signal<u32>>));
    let tick_b = Rc::new(RefCell::new(None::<Signal<u32>>));

    let mut first = Runtime::new();
    first
        .mount({
            let store = store.clone();
            let runs = Rc::clone(&runs_a);
            let tick = Rc::clone(&tick_a);
            let prebuilt = prebuilt_plan(&store, target, &runs, &tick);
            move || {
                view! { <PrebuiltHost prebuilt={prebuilt} store={store} target={target} runs={runs} tick={tick} /> }
            }
        })
        .expect("mount the first runtime");

    let mut second = Runtime::new();
    second
        .mount({
            let store = store.clone();
            let runs = Rc::clone(&runs_b);
            let tick = Rc::clone(&tick_b);
            let prebuilt = prebuilt_plan(&store, target, &runs, &tick);
            move || {
                view! { <PrebuiltHost prebuilt={prebuilt} store={store} target={target} runs={runs} tick={tick} /> }
            }
        })
        .expect("mount the second runtime");

    first
        .click_label("delete")
        .expect("delete in the first runtime");
    first
        .click_label("report")
        .expect("report the first runtime");
    assert!(
        text_present(&first, "runs 1"),
        "the first runtime keeps observing after the second mounts: {:?}",
        first.texts()
    );
}

/// 没有 filter 的静态行集：两个独立删除事务必须都进入隔离日志。
#[component]
fn StaticPendingHost(store: RowStore<Note>, first: RowId, second: RowId) -> MountResult<MountPlan> {
    let tick = signal(0u32);
    let runs = Rc::new(Cell::new(0u32));
    let report = signal(String::from("runs 0"));
    let report_runs = Rc::clone(&runs);
    let row_runs = Rc::clone(&runs);
    let prebuilt = For::new(
        ForSource::new(store.clone(), store.rowset([first, second])),
        |note: &Note| note.key,
        move |_row: RowView<Note>| {
            let counter = Rc::clone(&row_runs);
            let label = memo(move || {
                counter.set(counter.get() + 1);
                format!("memo {}", tick.get())
            });
            view! { <text>{label}</text> }
        },
    );

    view! {
        <view>
            <button aria:label="remove-both" on:click={
                let delete_store = store.clone();
                move || {
                    delete_store.remove(first).expect("remove the first row");
                    delete_store.remove(second).expect("remove the second row");
                    tick.set(tick.get() + 1);
                }
            }>"RemoveBoth"</button>
            <button aria:label="report" on:click={move || {
                report.set(format!("runs {}", report_runs.get()));
            }}>"Report"</button>
            <text>{report}</text>
            {prebuilt}
        </view>
    }
}

#[test]
fn pending_changes_keep_every_commit_for_static_rows() {
    let store = note_store();
    let first = store
        .insert(Note {
            key: 1,
            text: String::from("A"),
        })
        .expect("insert the first note")
        .0;
    let second = store
        .insert(Note {
            key: 2,
            text: String::from("B"),
        })
        .expect("insert the second note")
        .0;

    let mut app = Runtime::new();
    app.mount({
        let store = store.clone();
        move || view! { <StaticPendingHost store={store} first={first} second={second} /> }
    })
    .expect("mount the static rows");
    assert_eq!(app.for_items().len(), 2);

    app.click_label("remove-both").expect("remove both rows");
    app.click_label("report").expect("report the runs");
    assert!(
        text_present(&app, "runs 2"),
        "both removals stay quarantined: the log keeps every commit: {:?}",
        app.texts()
    );
}

/// 结构提交失败的帧保留删除行；下一次 flush 的 step 0 必须仍然能看到它。
///
/// 使用无 filter 的 Store + `Signal<RowSet>` 行源：heal 帧自身没有成员迁移，
/// 因此隔离只能来自错误帧保留的删除日志。
#[component]
fn ErrorRetentionHost(store: RowStore<Note>, target: RowId) -> MountResult<MountPlan> {
    let rows = signal(store.rowset([target]));
    let tick = signal(0u32);
    let runs = Rc::new(Cell::new(0u32));
    let report = signal(String::from("runs 0"));
    let report_runs = Rc::clone(&runs);
    let label_runs = Rc::clone(&runs);

    view! {
        <view>
            <button aria:label="break" on:click={
                let break_store = store.clone();
                move || {
                    break_store.remove(target).expect("remove the first row");
                    break_store
                        .insert(Note {
                            key: 9,
                            text: String::from("boom"),
                        })
                        .expect("insert the failing row");
                    rows.set(break_store.rowset(break_store.ids()));
                }
            }>"Break"</button>
            <button aria:label="heal" on:click={
                let heal_store = store.clone();
                move || {
                    let boom: Vec<RowId> = heal_store
                        .ids()
                        .into_iter()
                        .filter(|id| {
                            heal_store.with_row(*id, |row| row.text == "boom") == Some(true)
                        })
                        .collect();
                    for id in boom {
                        heal_store.remove(id).expect("remove the failing row");
                    }
                    tick.set(tick.get() + 1);
                    rows.set(heal_store.rowset(heal_store.ids()));
                }
            }>"Heal"</button>
            <button aria:label="report" on:click={move || {
                report.set(format!("runs {}", report_runs.get()));
            }}>"Report"</button>
            <text>{report}</text>
            <For each={rows} store={store} key={|note: &Note| note.key}>
                {move |row: RowView<Note>| {
                    let text = row.get().text.clone();
                    if text == "boom" {
                        return Err(MountError::Component("row boom"));
                    }
                    let counter = Rc::clone(&label_runs);
                    let label = memo(move || {
                        counter.set(counter.get() + 1);
                        // tick 让 memo 在 heal 帧变脏：若错误帧清空了累计器，
                        // 这一帧的 step 1 会在卸载前重算它。
                        let _ = tick.get();
                        format!("memo {text}")
                    });
                    view! { <text>{label}</text> }
                }}
            </For>
        </view>
    }
}

#[test]
fn an_error_flush_keeps_pending_changes_for_the_next_flush() {
    let store = note_store();
    let target = store
        .insert(Note {
            key: 1,
            text: String::from("A"),
        })
        .expect("insert the note")
        .0;

    let mut app = Runtime::new();
    app.mount({
        let store = store.clone();
        move || view! { <ErrorRetentionHost store={store} target={target} /> }
    })
    .expect("mount");
    assert_eq!(report_runs(&mut app), "runs 1");

    let report = app
        .click_label("break")
        .expect("the boom row failure is contained");
    assert!(
        matches!(
            report.errors(),
            [TurnError::FactoryFailed(MountError::Component("row boom"))]
        ),
        "unexpected failure: {:?}",
        report.errors()
    );

    app.click_label("heal").expect("heal the row set");
    assert_eq!(
        report_runs(&mut app),
        "runs 1",
        "the failed frame keeps the removal pending until the next flush quarantines it"
    );
}

const ALL_NOTES: FilterId = FilterId::new(0);

struct AllNotes;

impl FilterSpec<Note> for AllNotes {
    fn id(&self) -> FilterId {
        ALL_NOTES
    }

    fn watched_fields(&self) -> &'static [FieldId] {
        &[]
    }

    fn matches(&self, _row: &Note) -> bool {
        true
    }
}

fn filtered_note_store() -> RowStore<Note> {
    RowStore::new(
        SchemaBuilder::new()
            .field(FieldDef::comparing(NOTE_TEXT, |row: &Note| &row.text))
            .filter(AllNotes)
            .build()
            .expect("note schema"),
    )
}

/// flush 执行中（结构阶段挂载新行）产生的提交必须保留到下一次 flush。
#[component]
fn MidFlushForHost(store: RowStore<Note>, target: RowId) -> MountResult<MountPlan> {
    let rows = store.rows(AllNotes)?;
    let tick = signal(0u32);
    let runs = Rc::new(Cell::new(0u32));
    let report = signal(String::from("runs 0"));
    let row_runs = Rc::clone(&runs);
    let report_runs = Rc::clone(&runs);
    let factory_store = store.clone();

    view! {
        <view>
            <button aria:label="grow" on:click={move || {
                store
                    .insert(Note {
                        key: 3,
                        text: String::from("C"),
                    })
                    .expect("insert the third row");
            }}>"Grow"</button>
            <button aria:label="tick" on:click={move || tick.set(tick.get() + 1)}>"Tick"</button>
            <button aria:label="report" on:click={move || {
                report.set(format!("runs {}", report_runs.get()));
            }}>"Report"</button>
            <text>{report}</text>
            <For each={rows} key={|note: &Note| note.key}>
                {move |row: RowView<Note>| {
                    let text = row.get().text.clone();
                    if text == "C" {
                        // 新行的 factory 在结构阶段执行；泛型 Store 允许在
                        // 任何阶段提交，这个删除发生在 step 0 之后。
                        factory_store
                            .remove(target)
                            .expect("remove the first row mid-flush");
                    }
                    let counter = Rc::clone(&row_runs);
                    let label = memo(move || {
                        counter.set(counter.get() + 1);
                        let _ = tick.get();
                        format!("memo {text}")
                    });
                    view! { <text>{label}</text> }
                }}
            </For>
        </view>
    }
}

#[test]
fn a_commit_from_inside_the_structural_pass_waits_for_the_next_flush() {
    let store = filtered_note_store();
    let first = store
        .insert(Note {
            key: 1,
            text: String::from("A"),
        })
        .expect("insert the first note")
        .0;
    store
        .insert(Note {
            key: 2,
            text: String::from("B"),
        })
        .expect("insert the second note");

    let mut app = Runtime::new();
    app.mount({
        let store = store.clone();
        move || view! { <MidFlushForHost store={store} target={first} /> }
    })
    .expect("mount");
    assert!(text_present(&app, "memo A"));
    assert!(text_present(&app, "memo B"));

    // grow 帧：新行 C 的 factory 删除 A。提交发生在 step 0 之后，行的卸载
    // 要等到下一次结构刷新；删除本身必须保留到下一次 flush 的 step 0。
    app.click_label("grow").expect("mount the third row");
    assert!(
        store.get(first).is_none(),
        "the factory committed the removal"
    );
    assert!(
        text_present(&app, "memo A"),
        "the stale row stays mounted until the next structural pass"
    );
    assert!(text_present(&app, "memo C"));

    app.click_label("tick").expect("dirty the row memos");
    app.click_label("report").expect("report the runs");
    assert!(
        text_present(&app, "runs 5"),
        "A's memo is quarantined from the retained mid-flush commit: {:?}",
        app.texts()
    );
    assert!(!text_present(&app, "memo A"));
}
