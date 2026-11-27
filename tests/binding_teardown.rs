//! 绑定表 teardown 回归：窗口关闭/退出、行窗口收缩与扩展、行删除、Show 切换、
//! 刷新期 panic 与挂载失败的路径都必须把每个绑定槽归还它所属的树表。
//!
//! 这些场景不直接读取内部计数：泄漏会让树的 `drop_binding_table` 在 debug
//! 构建里触发 live-slot 断言，测试因此失败；单元测试另有显式的槽位计数断言。

#![allow(unused_braces)]

use zenui::prelude::*;

#[component]
fn Broken() -> MountResult<MountPlan> {
    Err(MountError::Component("broken sibling"))
}

#[component]
fn PanicText() -> MountResult<MountPlan> {
    let failing: Memo<String> = memo(|| panic!("virgin memo boom"));
    view! {
        <view>
            <text>{failing}</text>
            <button aria:label="bound" disabled={once(true)}>"x"</button>
        </view>
    }
}

#[component]
fn TeardownWindow(store: Store) -> MountResult<MountPlan> {
    let rows = memo(move || store.rows(Filter::All));
    let visible = signal(true);
    view! {
        <view role="main">
            <button aria:label="Seed" on:click={move || -> Result<(), WriteError> {
                let mut transaction = Transaction::new();
                for index in 0..60 {
                    transaction = transaction.write(store.insert(format!("r{index}")));
                }
                transaction.finish()?;
                Ok(())
            }}>"Seed"</button>
            <button aria:label="Remove" on:click={move || -> Result<(), WriteError> {
                let ids: Vec<TodoId> =
                    store.keys(Filter::All).into_iter().take(5).collect();
                let mut transaction = Transaction::new();
                for id in ids {
                    transaction = transaction.write(store.remove(id));
                }
                transaction.finish()
            }}>"Remove"</button>
            <button aria:label="Toggle" on:click={move || visible.set(!visible.get())}>"Toggle"</button>
            <view class="gap-0">
                <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                    {move |row: RowView<TodoRow>| {
                        let id = row.key();
                        view! {
                            <view style:height={once(px(20.0))}>
                                <Show when={visible}>
                                    <view aria:label={once(format!("Inner {}", id.raw()))}>
                                        <text>{store.title(id)}</text>
                                    </view>
                                </Show>
                                <button aria:label={once(format!("Row {}", id.raw()))}>"row"</button>
                            </view>
                        }
                    }}
                </For>
            </view>
        </view>
    }
}

fn teardown_fixture() -> (Runtime, WindowCtx) {
    let mut app = Runtime::new();
    let store = app.setup(Store::new).expect("application setup");
    app.declare_windows(move |catalog| {
        catalog.window(
            WindowSpec::new("main").primary().sized(900, 700),
            move |_| view! { <TeardownWindow store={store} /> },
        )
    })
    .expect("declare the window");
    let window = app.show("main").expect("show the window");
    (app, window)
}

#[test]
fn closing_a_window_after_row_windows_removals_and_show_toggles_releases_every_slot() {
    let (mut app, window) = teardown_fixture();
    app.click_label("Seed").expect("seed");
    let band = app.presentation_of(window).expect("snapshot").row_bands()[0].key();
    for _ in 0..8 {
        app.set_row_window(window, band, 30, 5)
            .expect("shrink the window");
        app.set_row_window(window, band, 0, 12)
            .expect("grow the window");
        app.click_label("Toggle").expect("hide inner branches");
        app.click_label("Toggle").expect("show inner branches");
    }
    app.click_label("Remove").expect("remove rows");
    app.set_row_window(window, band, 0, 96)
        .expect("mount all rows");
    app.set_row_window(window, band, 10, 3)
        .expect("shrink again");
    app.click_label("Toggle")
        .expect("hide inner branches again");
    app.close_window(window).expect("close the window");
}

#[test]
fn quitting_after_row_windows_and_removals_releases_every_slot() {
    let (mut app, window) = teardown_fixture();
    app.click_label("Seed").expect("seed");
    let band = app.presentation_of(window).expect("snapshot").row_bands()[0].key();
    app.set_row_window(window, band, 20, 4)
        .expect("shrink the window");
    app.click_label("Remove").expect("remove rows");
    app.click_label("Toggle").expect("hide inner branches");
    app.quit().expect("quit the session");
}

#[test]
fn a_window_whose_first_refresh_panics_releases_its_bindings() {
    let mut app = Runtime::new();
    let armed = std::rc::Rc::new(std::cell::Cell::new(true));
    {
        let armed = armed.clone();
        app.declare_windows(move |catalog| {
            catalog.window(WindowSpec::new("main").primary(), move |_| {
                if armed.replace(false) {
                    view! { <PanicText /> }
                } else {
                    view! { <text>"calm"</text> }
                }
            })
        })
        .expect("declare the window");
    }
    let error = app
        .show("main")
        .expect_err("the refresh panic fails the open");
    match error {
        WindowError::Mount(MountError::Panic(panic)) => {
            assert!(panic.message().contains("virgin memo boom"));
        }
        other => panic!("expected a contained panic, got {other:?}"),
    }
    let window = app
        .show("main")
        .expect("the aborted activation can be retried");
    assert!(app.texts().iter().any(|text| text == "calm"));
    app.close_window(window).expect("close the calm window");
}

#[component]
fn RowsThenBroken(store: Store) -> MountResult<MountPlan> {
    let rows = memo(move || store.rows(Filter::All));
    view! {
        <view>
            <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                {move |row: RowView<TodoRow>| view! {
                    <text>{once(format!("row {}", row.key().raw()))}</text>
                }}
            </For>
            <Broken />
        </view>
    }
}

#[test]
fn a_window_mount_failure_after_spacers_releases_the_table() {
    let mut app = Runtime::new();
    let store = app.setup(Store::new).expect("setup");
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |_| {
            view! { <RowsThenBroken store={store} /> }
        })
    })
    .expect("declare the window");
    let error = app.show("main").expect_err("the sibling fails the mount");
    assert!(matches!(
        error,
        WindowError::Mount(MountError::Component("broken sibling"))
    ));
}

#[component]
fn ShowThenBroken() -> MountResult<MountPlan> {
    let shown = signal(true);
    view! {
        <view>
            <Show when={shown}>
                <view aria:label="branch">
                    <text>{once("branch text")}</text>
                    <button disabled={once(true)}>"inside"</button>
                </view>
            </Show>
            <Broken />
        </view>
    }
}

#[test]
fn a_window_mount_failure_after_a_show_branch_releases_the_table() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("main").primary(), |_| {
            view! { <ShowThenBroken /> }
        })
    })
    .expect("declare the window");
    let error = app.show("main").expect_err("the sibling fails the mount");
    assert!(matches!(
        error,
        WindowError::Mount(MountError::Component("broken sibling"))
    ));
}

#[test]
fn a_root_context_fault_with_structure_releases_the_table() {
    let mut app = Runtime::new();
    let error = app
        .mount(|| {
            let store = Store::new();
            let rows = memo(move || store.rows(Filter::All));
            let shown = signal(true);
            let trigger = signal(0u32);
            trigger.set(1); // idle-phase write: a context fault after the plan mounts
            view! {
                <view role="root" style:width={once(px(10.0))}>
                    <text>"root text"</text>
                    <Show when={shown}>
                        <text>{once("branch text")}</text>
                    </Show>
                    <For each={rows} store={store} key={|row: &TodoRow| row.id} row-height={px(20.0)}>
                        {move |row: RowView<TodoRow>| view! {
                            <text>{once(format!("row {}", row.key().raw()))}</text>
                        }}
                    </For>
                </view>
            }
        })
        .expect_err("the idle-phase write is a context fault");
    match error {
        MountError::Context { .. } => {}
        other => panic!("expected a context fault, got {other:?}"),
    }
}
