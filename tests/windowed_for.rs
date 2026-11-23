#![allow(unused_braces)]

use zenui::prelude::*;

/// 一个窗口化列表夹具：固定 20 点行高的 `For`，每行一个带标签按钮。
///
/// 行集通过 Seed / Reverse 按钮在事件阶段修改；测试只通过公开的
/// runtime/窗口接口观察挂载窗口、Node 身份与焦点。
#[component]
fn WindowedList(store: Store) -> MountResult<MountPlan> {
    let rows = memo(move || store.rows(Filter::All));
    let seed = move || {
        let mut transaction = Transaction::new();
        for index in 0..100 {
            transaction = transaction.write(store.insert(format!("r{index}")));
        }
        let _ = transaction.finish();
    };
    let reverse = move || {
        let ids: Vec<TodoId> = store
            .rows(Filter::All)
            .iter()
            .map(TodoId::from_key)
            .collect();
        let reversed: Vec<TodoId> = ids.into_iter().rev().collect();
        let _ = Transaction::new().write(store.reorder(&reversed)).finish();
    };

    view! {
        <view role="main">
            <text aria:label="Header">"windowed"</text>
            <button aria:label="Seed" on:click={move || seed()}>"Seed"</button>
            <button aria:label="Reverse" on:click={move || reverse()}>"Reverse"</button>
            <view class="gap-0">
                <For each={rows} key={|id: TodoId| id} row-height={px(20.0)}>
                    {move |id: TodoId| {
                        let title = store.title(id).read_once()?;
                        let label = format!("Row {title}");
                        view! {
                            <view style:height={px(20.0)}>
                                <button aria:label={once(label)}>"row"</button>
                            </view>
                        }
                    }}
                </For>
            </view>
        </view>
    }
}

fn fixture() -> (Runtime, WindowCtx) {
    let mut app = Runtime::new();
    let store = app.setup(Store::new).expect("application setup");
    app.declare_windows(move |catalog| {
        catalog.window(
            WindowSpec::new("main").primary().sized(900, 700),
            move |_| {
                view! { <WindowedList store={store} /> }
            },
        )
    })
    .expect("declare the window");
    let window = app.show("main").expect("show the window");
    app.click_label("Seed").expect("seed one hundred rows");
    (app, window)
}

fn band(app: &Runtime, window: WindowCtx) -> SnapshotRowBand {
    app.presentation_of(window)
        .expect("window snapshot")
        .row_bands()[0]
}

#[test]
fn a_windowed_for_mounts_only_the_requested_rows_and_remounts_fresh() {
    let (mut app, window) = fixture();
    let snapshot = app.presentation_of(window).expect("snapshot");
    let band = snapshot.row_bands()[0];
    assert_eq!(band.total_rows(), 100, "the whole list is declared");
    assert_eq!(band.first_row(), 0);
    assert!(
        snapshot.find_label("Header").is_some(),
        "chrome stays mounted"
    );
    assert!(snapshot.find_label("Row r0").is_some());
    assert!(
        snapshot.find_label("Row r40").is_none(),
        "rows outside the default window are not mounted"
    );
    let first = snapshot.find_label("Row r0").expect("row zero").id();
    let first_instance = app.for_items()[0].instance;

    assert!(
        app.set_row_window(window, band.key(), 40, 10)
            .expect("apply the window"),
        "the requested window differs"
    );
    let snapshot = app.presentation_of(window).expect("snapshot");
    assert_eq!(snapshot.row_bands()[0].first_row(), 40);
    assert_eq!(snapshot.row_bands()[0].total_rows(), 100);
    assert!(snapshot.find_label("Row r40").is_some());
    assert!(snapshot.find_label("Row r49").is_some());
    assert!(snapshot.find_label("Row r50").is_none());
    assert!(snapshot.find_label("Row r0").is_none());
    assert!(snapshot.find_label("Header").is_some());
    assert_eq!(
        app.node_status(first),
        NodeStatus::Stale,
        "a row outside the window is unmounted"
    );

    assert!(
        app.set_row_window(window, band.key(), 0, 10)
            .expect("return to the top")
    );
    let snapshot = app.presentation_of(window).expect("snapshot");
    assert_eq!(snapshot.row_bands()[0].first_row(), 0);
    let remounted = snapshot
        .find_label("Row r0")
        .expect("row zero is back")
        .id();
    assert_ne!(remounted, first, "a re-entering row gets a fresh NodeId");
    assert!(
        app.for_items()[0].instance > first_instance,
        "a re-entering row mounts a fresh instance"
    );
    assert_eq!(app.node_status(first), NodeStatus::Stale);
    assert_eq!(app.node_status(remounted), NodeStatus::Live);
}

#[test]
fn focus_keeps_a_row_one_outside_the_window_but_drops_it_further() {
    let (mut app, window) = fixture();
    app.focus("Row r0").expect("focus the first row");
    let band = band(&app, window);

    assert!(
        app.set_row_window(window, band.key(), 1, 10)
            .expect("scroll one row down")
    );
    let snapshot = app.presentation_of(window).expect("snapshot");
    assert_eq!(
        snapshot.row_bands()[0].first_row(),
        0,
        "the grace window expands upward to keep the focused row"
    );
    assert!(snapshot.find_label("Row r0").is_some());
    assert!(snapshot.focus().is_some());
    assert_eq!(
        app.focused().and_then(|info| info.label),
        Some(String::from("Row r0"))
    );

    assert!(
        app.set_row_window(window, band.key(), 5, 10)
            .expect("scroll far down")
    );
    let snapshot = app.presentation_of(window).expect("snapshot");
    assert_eq!(snapshot.row_bands()[0].first_row(), 5);
    assert!(
        snapshot.find_label("Row r0").is_none(),
        "the focused row is unmounted beyond one row of grace"
    );
    assert_eq!(
        snapshot.focus(),
        None,
        "the runtime clears focus instead of settling to a neighbour"
    );
    assert!(app.focused().is_none());
}

#[test]
fn tab_traverses_only_the_mounted_window_and_leaves_at_its_boundary() {
    let (mut app, window) = fixture();
    let band = band(&app, window);
    app.set_row_window(window, band.key(), 20, 5)
        .expect("mount rows 20..24");

    app.focus("Row r20").expect("focus the first mounted row");
    for expected in ["Row r21", "Row r22", "Row r23", "Row r24"] {
        app.focus_next().expect("traverse the mounted rows");
        assert_eq!(
            app.focused().and_then(|info| info.label),
            Some(String::from(expected)),
            "Tab stays inside the mounted window"
        );
    }
    // 最后一个已挂载行之后，Tab 离开列表去下一个可聚焦元素；
    // 窗口外的行在滚动挂载它们之前不可遍历。
    app.focus_next().expect("leave the list at its boundary");
    assert_eq!(
        app.focused().and_then(|info| info.label),
        Some(String::from("Seed")),
        "Tab leaves the list instead of mounting an off-window row"
    );
}

#[test]
fn reordering_rows_rewindows_without_duplicating_keys() {
    let (mut app, window) = fixture();
    let band = band(&app, window);
    app.set_row_window(window, band.key(), 0, 10)
        .expect("keep the first window");

    app.click_label("Reverse").expect("reverse the model");
    let snapshot = app.presentation_of(window).expect("snapshot");
    let band = snapshot.row_bands()[0];
    assert_eq!(band.total_rows(), 100);
    assert_eq!(band.first_row(), 0);
    assert!(
        snapshot.find_label("Row r99").is_some(),
        "the first window shows the new business order"
    );
    assert!(snapshot.find_label("Row r90").is_some());
    assert!(
        snapshot.find_label("Row r0").is_none(),
        "the old tail is outside the window"
    );
    assert_eq!(
        app.for_items().len(),
        10,
        "the window still mounts ten unique rows"
    );
}
