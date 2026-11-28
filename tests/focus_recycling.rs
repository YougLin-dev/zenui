//! Focus settlement must compare element identity, not a recyclable slot.
//!
//! A sibling `For` can mount a fresh control into the slot freed by the
//! unmounted focused row in the same refresh. Settlement must neither give
//! that fresh control the old focus nor blur it as if it were the old one.

#![allow(unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;

#[component]
fn Editor(
    label: String,
    title: String,
    focus: Rc<Cell<u32>>,
    blur: Rc<Cell<u32>>,
) -> MountResult<MountPlan> {
    let on_focus = focus.clone();
    let on_blur = blur.clone();
    view! {
        <text-input
            aria:label={once(label)}
            value={once(title)}
            on:focus={move |_event: FocusEvent| on_focus.set(on_focus.get() + 1)}
            on:blur={move |_event: BlurEvent| on_blur.set(on_blur.get() + 1)}
        />
    }
}

#[component]
fn SiblingFors(rows: usize, focus: Rc<Cell<u32>>, blur: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let store = Store::new();
    let active = memo(move || store.rows(Filter::Active));
    let completed = memo(move || store.rows(Filter::Completed));
    let active_focus = focus.clone();
    let active_blur = blur.clone();
    let completed_focus = focus;
    let completed_blur = blur;

    view! {
        <view>
            <view
                aria:label="seed"
                on:click={move || {
                    let mut transaction = Transaction::new();
                    for index in 0..rows {
                        transaction = transaction.write(store.insert(format!("row {index}")));
                    }
                    let _ = transaction.finish();
                }}
            >"Seed"</view>
            <view
                aria:label="move"
                on:click={move || {
                    let keys = store.keys(Filter::All);
                    let mut transaction = Transaction::new();
                    if let Some(first) = keys.first() {
                        transaction = transaction.write(store.set_completed(*first, true));
                    }
                    let _ = transaction.finish();
                }}
            >"Move"</view>
            <view>
                <For each={active} store={store} key={|row: &TodoRow| row.id}>
                    {move |row: RowView<TodoRow>| {
                        let title = row.get().title.clone();
                        let focus = active_focus.clone();
                        let blur = active_blur.clone();
                        view! {
                            <Editor
                                label={format!("active {title}")}
                                title={title}
                                focus={focus}
                                blur={blur}
                            />
                        }
                    }}
                </For>
            </view>
            <view>
                <For each={completed} store={store} key={|row: &TodoRow| row.id}>
                    {move |row: RowView<TodoRow>| {
                        let title = row.get().title.clone();
                        let focus = completed_focus.clone();
                        let blur = completed_blur.clone();
                        view! {
                            <Editor
                                label={format!("completed {title}")}
                                title={title}
                                focus={focus}
                                blur={blur}
                            />
                        }
                    }}
                </For>
            </view>
        </view>
    }
}

fn sibling_fixture(rows: usize) -> (Runtime, Rc<Cell<u32>>, Rc<Cell<u32>>) {
    let focus = Rc::new(Cell::new(0));
    let blur = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let focus = focus.clone();
        let blur = blur.clone();
        move || {
            view! { <SiblingFors rows={rows} focus={focus} blur={blur} /> }
        }
    })
    .expect("mount the sibling lists");
    app.click_label("seed").expect("seed the rows");
    (app, focus, blur)
}

#[test]
fn a_recycled_slot_does_not_take_focus_from_the_unmounted_row() {
    let (mut app, focus, blur) = sibling_fixture(2);
    app.focus("active row 0").expect("focus the first row");
    let old = app
        .presentation()
        .expect("snapshot")
        .find_label("active row 0")
        .expect("the first active editor")
        .id();
    assert_eq!(focus.get(), 1);
    assert_eq!(blur.get(), 0);

    app.click_label("move").expect("move the focused row");

    assert_eq!(app.node_status(old), NodeStatus::Stale);
    let snapshot = app.presentation().expect("snapshot");
    assert!(
        snapshot.find_label("completed row 0").is_some(),
        "the moved row is freshly mounted into a recycled slot"
    );
    let replacement = snapshot
        .find_label("active row 1")
        .expect("the other old row is still mounted")
        .id();
    assert_eq!(
        snapshot.focus(),
        Some(replacement),
        "settlement picks the next old control, never the fresh mount"
    );
    assert_eq!(focus.get(), 2, "one settlement focus event");
    assert_eq!(blur.get(), 0, "the unmounted row receives no blur");
}

#[test]
fn a_fresh_mount_in_the_only_freed_slot_does_not_receive_focus() {
    let (mut app, focus, blur) = sibling_fixture(1);
    app.focus("active row 0").expect("focus the only row");
    let old = app
        .presentation()
        .expect("snapshot")
        .find_label("active row 0")
        .expect("the active editor")
        .id();

    app.click_label("move").expect("move the focused row");

    assert_eq!(app.node_status(old), NodeStatus::Stale);
    let snapshot = app.presentation().expect("snapshot");
    let fresh = snapshot
        .find_label("completed row 0")
        .expect("the fresh completed editor");
    assert_eq!(snapshot.focus(), None, "no old control can inherit focus");
    assert_ne!(
        fresh.key(),
        old.presentation_key(),
        "the fresh editor is a different identity"
    );
    assert_eq!(focus.get(), 1, "no focus event for the fresh mount");
    assert_eq!(blur.get(), 0, "the unmounted row receives no blur");
}

#[component]
fn WindowedPair(focus: Rc<Cell<u32>>, blur: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let store = Store::new();
    let active = memo(move || store.rows(Filter::Active));
    let completed = memo(move || store.rows(Filter::Completed));
    let active_focus = focus.clone();
    let active_blur = blur.clone();
    let completed_focus = focus;
    let completed_blur = blur;

    view! {
        <view>
            <view
                aria:label="seed"
                on:click={move || {
                    let mut transaction = Transaction::new();
                    for index in 0..4 {
                        transaction =
                            transaction.write(store.insert(format!("row {index}")));
                    }
                    let _ = transaction.finish();
                }}
            >"Seed"</view>
            <view
                aria:label="complete"
                on:click={move || {
                    let keys = store.keys(Filter::All);
                    let mut transaction = Transaction::new();
                    for key in keys.iter().take(2) {
                        transaction = transaction.write(store.set_completed(*key, true));
                    }
                    let _ = transaction.finish();
                }}
            >"Complete"</view>
            <view class="gap-0">
                <For
                    each={active}
                    store={store}
                    key={|row: &TodoRow| row.id}
                    row-height={px(20.0)}
                >
                    {move |row: RowView<TodoRow>| {
                        let title = row.get().title.clone();
                        let focus = active_focus.clone();
                        let blur = active_blur.clone();
                        view! {
                            <view style:height={once(px(20.0))}>
                                <Editor
                                    label={format!("active {title}")}
                                    title={title}
                                    focus={focus}
                                    blur={blur}
                                />
                            </view>
                        }
                    }}
                </For>
            </view>
            <view class="gap-0">
                <For
                    each={completed}
                    store={store}
                    key={|row: &TodoRow| row.id}
                    row-height={px(20.0)}
                >
                    {move |row: RowView<TodoRow>| {
                        let title = row.get().title.clone();
                        let focus = completed_focus.clone();
                        let blur = completed_blur.clone();
                        view! {
                            <view style:height={once(px(20.0))}>
                                <Editor
                                    label={format!("completed {title}")}
                                    title={title}
                                    focus={focus}
                                    blur={blur}
                                />
                            </view>
                        }
                    }}
                </For>
            </view>
        </view>
    }
}

#[test]
fn a_windowed_drop_clears_focus_before_a_sibling_reuses_the_slot() {
    let focus = Rc::new(Cell::new(0));
    let blur = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.declare_windows({
        let focus = focus.clone();
        let blur = blur.clone();
        move |catalog| {
            catalog.window(WindowSpec::new("main").primary(), move |_| {
                let focus = focus.clone();
                let blur = blur.clone();
                view! { <WindowedPair focus={focus} blur={blur} /> }
            })?;
            Ok(())
        }
    })
    .expect("declare the window");
    let window = app.show("main").expect("show the window");
    app.click_label("seed").expect("seed four rows");
    app.click_label("complete")
        .expect("complete the first two rows");

    let bands = app
        .presentation_of(window)
        .expect("snapshot")
        .row_bands()
        .to_vec();
    assert_eq!(bands.len(), 2, "two windowed lists");
    let active_band = bands[0].key();
    let completed_band = bands[1].key();
    app.set_row_window(window, completed_band, 0, 0)
        .expect("unmount the completed list");
    app.focus("active row 2")
        .expect("focus the first mounted active row");
    let old = app
        .presentation_of(window)
        .expect("snapshot")
        .find_label("active row 2")
        .expect("the focused active editor")
        .id();
    assert_eq!(focus.get(), 1);

    app.set_row_windows(
        window,
        &[
            (active_band, RowWindow::new(2, 2)),
            (completed_band, RowWindow::new(0, 2)),
        ],
    )
    .expect("move the focus row out and mount the sibling list");

    assert_eq!(app.node_status(old), NodeStatus::Stale);
    let snapshot = app.presentation_of(window).expect("snapshot");
    assert!(
        snapshot.find_label("completed row 0").is_some(),
        "the sibling list mounted fresh rows into the freed slots"
    );
    assert_eq!(
        snapshot.focus(),
        None,
        "window drop clears focus instead of aliasing the recycled slot"
    );
    assert_eq!(focus.get(), 1, "the fresh mount receives no focus event");
    assert_eq!(blur.get(), 0, "the dropped row receives no blur");
}
