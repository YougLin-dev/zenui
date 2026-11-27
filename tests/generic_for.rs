#![allow(dead_code, unused_braces)]

//! #338 验收：泛型 `For` 在第二个模型上经 `ForSource`/`RowView` 挂载。
//!
//! 覆盖：非 Todo 模型的行工厂、RowId 身份、投影顺序与重排可见性、按已解析
//! 行快照计算键与重复检测、工厂错误/panic 保留旧映射、跨 Store 行集拒绝、
//! 静态 RowSet 快照语义，以及 Todo 行仍经同一 `ForSource` 通道挂载。

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use zenui::model::{FieldDef, FieldId, FilterId, FilterSpec, SchemaBuilder};
use zenui::prelude::*;

#[derive(Clone, Debug, PartialEq)]
struct Note {
    key: u32,
    text: String,
    pinned: bool,
}

impl Row for Note {
    type Key = u32;

    fn key(&self) -> u32 {
        self.key
    }
}

const TEXT: FieldId = FieldId::new(0);
const PINNED: FieldId = FieldId::new(1);
const ALL: FilterId = FilterId::new(0);
const PINNED_ONLY: FilterId = FilterId::new(1);

struct AllNotes;

impl FilterSpec<Note> for AllNotes {
    fn id(&self) -> FilterId {
        ALL
    }

    fn watched_fields(&self) -> &'static [FieldId] {
        &[]
    }

    fn matches(&self, _row: &Note) -> bool {
        true
    }
}

struct PinnedNotes;

impl FilterSpec<Note> for PinnedNotes {
    fn id(&self) -> FilterId {
        PINNED_ONLY
    }

    fn watched_fields(&self) -> &'static [FieldId] {
        &[PINNED]
    }

    fn matches(&self, row: &Note) -> bool {
        row.pinned
    }
}

fn note_store() -> RowStore<Note> {
    let schema = SchemaBuilder::new()
        .field(FieldDef::comparing(TEXT, |row: &Note| &row.text))
        .field(FieldDef::comparing(PINNED, |row: &Note| &row.pinned))
        .filter(AllNotes)
        .filter(PinnedNotes)
        .build()
        .expect("note schema");
    RowStore::new(schema)
}

fn note(key: u32, text: &str) -> Note {
    Note {
        key,
        text: text.to_owned(),
        pinned: false,
    }
}

fn pinned_note(key: u32, text: &str) -> Note {
    Note {
        pinned: true,
        ..note(key, text)
    }
}

fn insert(store: &RowStore<Note>, row: Note) -> RowId {
    store.insert(row).expect("insert note").0
}

fn rows_of(app: &Runtime) -> Vec<RowId> {
    app.for_items().iter().map(|item| item.row).collect()
}

fn instances_by_row(app: &Runtime) -> BTreeMap<RowId, u64> {
    app.for_items()
        .iter()
        .map(|item| (item.row, item.instance))
        .collect()
}

/// 跟随 `Rows` 投影的列表：重排、成员迁移与字段写入都在下一次 refresh 可见。
#[component]
fn NoteList(store: RowStore<Note>) -> MountResult<MountPlan> {
    let rows = store.rows(AllNotes)?;
    view! {
        <view>
            <For each={rows} key={|note: &Note| note.key}>
                {move |row: RowView<Note>| {
                    let label = format!("note {}:{}", row.key(), row.get().text);
                    view! { <text>{once(label)}</text> }
                }}
            </For>
        </view>
    }
}

/// 记录每个 factory 收到的 RowId；用于证明身份来自 RowSet，而不是业务键。
#[component]
fn RecordedList(
    store: RowStore<Note>,
    seen: Rc<RefCell<Vec<RowId>>>,
    mounts: Rc<Cell<u32>>,
) -> MountResult<MountPlan> {
    let rows = store.rows(AllNotes)?;
    view! {
        <view>
            <For each={rows} key={|note: &Note| note.key}>
                {move |row: RowView<Note>| {
                    mounts.set(mounts.get() + 1);
                    seen.borrow_mut().push(row.id());
                    let label = format!("note {}:{}", row.key(), row.get().text);
                    view! { <text>{once(label)}</text> }
                }}
            </For>
        </view>
    }
}

/// 键选择器只观察已解析的行字段：两个不同业务键可以因为文本长度相同而冲突。
#[component]
fn LengthKeyList(
    store: RowStore<Note>,
    seen: Rc<RefCell<Vec<(u32, usize)>>>,
    mounts: Rc<Cell<u32>>,
) -> MountResult<MountPlan> {
    let rows = store.rows(AllNotes)?;
    view! {
        <view>
            <For each={rows} key={move |note: &Note| {
                seen.borrow_mut().push((note.key, note.text.len()));
                note.text.len()
            }}>
                {move |row: RowView<Note>| {
                    mounts.set(mounts.get() + 1);
                    let label = format!("len {}", row.get().text.len());
                    view! { <text>{once(label)}</text> }
                }}
            </For>
        </view>
    }
}

#[component]
fn PinnedList(store: RowStore<Note>) -> MountResult<MountPlan> {
    let rows = store.rows(PinnedNotes)?;
    view! {
        <view>
            <For each={rows} key={|note: &Note| note.key}>
                {move |row: RowView<Note>| {
                    let label = format!("note {}:{}", row.key(), row.get().text);
                    view! { <text>{once(label)}</text> }
                }}
            </For>
        </view>
    }
}

#[component]
fn FailingList(store: RowStore<Note>, armed: Rc<Cell<bool>>) -> MountResult<MountPlan> {
    let rows = store.rows(AllNotes)?;
    view! {
        <view>
            <For each={rows} key={|note: &Note| note.key}>
                {move |row: RowView<Note>| {
                    if armed.get() {
                        return Err(MountError::Component("note boom"));
                    }
                    let label = format!("note {}:{}", row.key(), row.get().text);
                    view! { <text>{once(label)}</text> }
                }}
            </For>
        </view>
    }
}

#[component]
fn PanicList(store: RowStore<Note>, armed: Rc<Cell<bool>>) -> MountResult<MountPlan> {
    let rows = store.rows(AllNotes)?;
    view! {
        <view>
            <For each={rows} key={|note: &Note| note.key}>
                {move |row: RowView<Note>| {
                    if armed.get() {
                        panic!("row boom");
                    }
                    let label = format!("note {}:{}", row.key(), row.get().text);
                    view! { <text>{once(label)}</text> }
                }}
            </For>
        </view>
    }
}

#[component]
fn BoomKeyList(store: RowStore<Note>) -> MountResult<MountPlan> {
    let rows = store.rows(AllNotes)?;
    view! {
        <view>
            <For each={rows} key={|note: &Note| {
                if note.text == "boom" {
                    panic!("key boom");
                }
                note.key
            }}>
                {move |row: RowView<Note>| {
                    let label = format!("note {}:{}", row.key(), row.get().text);
                    view! { <text>{once(label)}</text> }
                }}
            </For>
        </view>
    }
}

#[component]
fn StaticList(store: RowStore<Note>, set: RowSet) -> MountResult<MountPlan> {
    view! {
        <view>
            <For each={set} store={store} key={|note: &Note| note.key}>
                {move |row: RowView<Note>| {
                    let label = format!("note {}:{}", row.key(), row.get().text);
                    view! { <text>{once(label)}</text> }
                }}
            </For>
            <text>{once(format!("count:{}", store.len()))}</text>
        </view>
    }
}

#[component]
fn WindowedStaticList(store: RowStore<Note>, set: RowSet) -> MountResult<MountPlan> {
    view! {
        <view class="gap-0">
            <For each={set} store={store} key={|note: &Note| note.key} row-height={px(20.0)}>
                {move |row: RowView<Note>| view! {
                    <view style:height={once(px(20.0))}>
                        <text>{once(format!("row {}", row.key()))}</text>
                    </view>
                }}
            </For>
        </view>
    }
}

/// 两个窗口化静态列表，各自按行键拒绝新挂载；用于批量回滚失败后的窗口对账。
#[component]
fn FaultedStaticBands(
    store: RowStore<Note>,
    set: RowSet,
    first_fails: Rc<RefCell<Vec<u32>>>,
    second_fails: Rc<RefCell<Vec<u32>>>,
) -> MountResult<MountPlan> {
    let other = set.clone();
    let second_fails = Rc::clone(&second_fails);
    view! {
        <view class="gap-0">
            <For each={set} store={store} key={|note: &Note| note.key} row-height={px(20.0)}>
                {move |row: RowView<Note>| {
                    if first_fails.borrow().contains(&row.key()) {
                        return Err(MountError::Component("first boom"));
                    }
                    view! {
                        <view style:height={once(px(20.0))}>
                            <text>{once(format!("row {}", row.key()))}</text>
                        </view>
                    }
                }}
            </For>
            <For each={other} store={store} key={|note: &Note| note.key} row-height={px(20.0)}>
                {move |row: RowView<Note>| {
                    if second_fails.borrow().contains(&row.key()) {
                        return Err(MountError::Component("second boom"));
                    }
                    view! {
                        <view style:height={once(px(20.0))}>
                            <text>{once(format!("other {}", row.key()))}</text>
                        </view>
                    }
                }}
            </For>
        </view>
    }
}

/// Todo 列表保持既有形态：`Store` 属性 + `Memo<RowSet>` 读源，行工厂收 RowView。
#[component]
fn TodoList(keys: Rc<RefCell<Vec<TodoId>>>) -> MountResult<MountPlan> {
    let store = Store::new();
    let rows = memo(move || store.rows(Filter::All));
    view! {
        <view>
            <button on:click={move || {
                let _ = Transaction::new()
                    .write(store.insert("first"))
                    .write(store.insert("second"))
                    .finish();
            }}>"Seed"</button>
            <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                {move |row: RowView<TodoRow>| {
                    keys.borrow_mut().push(row.key());
                    let label = format!("todo {}:{}", row.key().raw(), row.get().title);
                    view! { <text>{once(label)}</text> }
                }}
            </For>
        </view>
    }
}

#[test]
fn a_second_model_mounts_rows_through_row_view() {
    let store = note_store();
    let alpha = insert(&store, note(1, "alpha"));
    let beta = insert(&store, note(2, "beta"));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mounts = Rc::new(Cell::new(0));

    let mut app = Runtime::new();
    app.mount({
        let store = store.clone();
        let seen = Rc::clone(&seen);
        let mounts = Rc::clone(&mounts);
        move || view! { <RecordedList store={store} seen={seen} mounts={mounts} /> }
    })
    .expect("a second model mounts");

    assert_eq!(*seen.borrow(), vec![alpha, beta], "factories see RowIds");
    assert_eq!(mounts.get(), 2);
    assert_eq!(app.texts(), vec!["note 1:alpha", "note 2:beta"]);
    let items = app.for_items();
    assert_eq!(rows_of(&app), vec![alpha, beta]);
    assert!(
        items.iter().all(|item| item.store == store.id()),
        "every row reports the note store"
    );
    assert!(
        items.iter().all(|item| item.todo.is_none()),
        "a second model has no Todo business key"
    );

    // 直接写入普通模型 Store 后，投影在下一次 refresh 取到新行。
    let gamma = insert(&store, note(3, "gamma"));
    app.refresh().expect("refresh the live projection");
    assert_eq!(
        app.texts(),
        vec!["note 1:alpha", "note 2:beta", "note 3:gamma"]
    );
    assert_eq!(rows_of(&app), store.ids());
    assert_eq!(
        seen.borrow().last(),
        Some(&gamma),
        "only the fresh row mounts"
    );
    assert_eq!(mounts.get(), 3);
}

#[test]
fn reordering_the_model_reorders_rows_without_remounting() {
    let store = note_store();
    let one = insert(&store, note(1, "one"));
    let two = insert(&store, note(2, "two"));
    let three = insert(&store, note(3, "three"));

    let mut app = Runtime::new();
    app.mount({
        let store = store.clone();
        move || view! { <NoteList store={store} /> }
    })
    .expect("mount the note list");
    let before = instances_by_row(&app);
    let unmounts = app.unmounts();
    assert_eq!(
        app.texts(),
        vec!["note 1:one", "note 2:two", "note 3:three"]
    );

    store
        .reorder(&[three, two, one])
        .expect("reorder the note store");
    assert_eq!(store.ids(), vec![three, two, one]);
    app.refresh().expect("the model order is visible");

    assert_eq!(
        app.texts(),
        vec!["note 3:three", "note 2:two", "note 1:one"],
        "RowSet order equals Store order"
    );
    assert_eq!(rows_of(&app), vec![three, two, one]);
    assert_eq!(app.unmounts(), unmounts, "reordering mounts nothing");
    for (row, instance) in instances_by_row(&app) {
        assert_eq!(
            before.get(&row),
            Some(&instance),
            "row {row:?} keeps its mounted instance across reorder"
        );
    }
}

#[test]
fn a_live_projection_tracks_field_updates() {
    let store = note_store();
    let anchor = insert(&store, pinned_note(1, "anchor"));
    let later = insert(&store, note(2, "later"));

    let mut app = Runtime::new();
    app.mount({
        let store = store.clone();
        move || view! { <PinnedList store={store} /> }
    })
    .expect("mount the pinned list");
    assert_eq!(app.texts(), vec!["note 1:anchor"]);
    assert_eq!(rows_of(&app), vec![anchor]);

    store
        .update(later, |row| row.pinned = true)
        .expect("pin the second note");
    app.refresh().expect("membership follows the field write");
    assert_eq!(app.texts(), vec!["note 1:anchor", "note 2:later"]);
    assert_eq!(rows_of(&app), vec![anchor, later]);

    store
        .update(anchor, |row| row.pinned = false)
        .expect("unpin the first note");
    app.refresh().expect("the row leaves the projection");
    assert_eq!(app.texts(), vec!["note 2:later"]);
    assert_eq!(rows_of(&app), vec![later]);
}

#[test]
fn duplicate_keys_are_resolved_from_rows_before_any_factory_runs() {
    let store = note_store();
    insert(&store, note(1, "aa"));
    insert(&store, note(2, "bb"));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mounts = Rc::new(Cell::new(0));

    let mut app = Runtime::new();
    let failure = app
        .mount({
            let store = store.clone();
            let seen = Rc::clone(&seen);
            let mounts = Rc::clone(&mounts);
            move || view! { <LengthKeyList store={store} seen={seen} mounts={mounts} /> }
        })
        .expect_err("equal text lengths collide even though the business keys differ");

    assert_eq!(failure, MountError::DuplicateKey);
    // 每次键相等比较也会对两个已解析行重跑选择器，因此这里只断言观察到的输入。
    assert!(
        seen.borrow()
            .iter()
            .all(|&(_, length)| length == "aa".len()),
        "the selector only ever saw the resolved text: {:?}",
        seen.borrow()
    );
    assert!(seen.borrow().contains(&(1, 2)));
    assert!(seen.borrow().contains(&(2, 2)));
    assert_eq!(mounts.get(), 0, "no row factory runs before publication");
}

#[test]
fn a_duplicate_key_in_a_new_projection_keeps_the_old_mapping() {
    let store = note_store();
    let one = insert(&store, note(1, "a"));
    let two = insert(&store, note(2, "bb"));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mounts = Rc::new(Cell::new(0));

    let mut app = Runtime::new();
    app.mount({
        let store = store.clone();
        let seen = Rc::clone(&seen);
        let mounts = Rc::clone(&mounts);
        move || view! { <LengthKeyList store={store} seen={seen} mounts={mounts} /> }
    })
    .expect("mount the length-keyed list");
    let before = app.texts();
    let before_rows = rows_of(&app);
    seen.borrow_mut().clear();

    let colliding = insert(&store, note(3, "c"));
    let failure = app
        .refresh()
        .expect_err("the third note collides with the first by text length");
    assert_eq!(failure, MountError::DuplicateKey);
    assert!(
        seen.borrow().contains(&(3, 1)),
        "the colliding row was resolved before key mapping: {:?}",
        seen.borrow()
    );
    assert_eq!(app.texts(), before, "the committed mapping is untouched");
    assert_eq!(rows_of(&app), before_rows);
    assert_eq!(
        mounts.get(),
        2,
        "no fresh factory runs in the rejected frame"
    );

    store.remove(colliding).expect("remove the collision");
    app.refresh()
        .expect("the previous mapping refreshes cleanly");
    assert_eq!(app.texts(), before);
    assert_eq!(rows_of(&app), vec![one, two]);
}

#[test]
fn a_factory_error_keeps_the_old_mapping_until_the_row_heals() {
    let store = note_store();
    let alpha = insert(&store, note(1, "alpha"));
    let beta = insert(&store, note(2, "beta"));
    let armed = Rc::new(Cell::new(false));

    let mut app = Runtime::new();
    app.mount({
        let store = store.clone();
        let armed = Rc::clone(&armed);
        move || view! { <FailingList store={store} armed={armed} /> }
    })
    .expect("mount the failing list");
    let before = app.texts();
    let before_instances = instances_by_row(&app);

    armed.set(true);
    insert(&store, note(3, "gamma"));
    let failure = app.refresh().expect_err("the fresh row factory fails");
    assert!(
        matches!(&failure, MountError::FactoryFailed(error) if **error == MountError::Component("note boom")),
        "unexpected failure: {failure:?}"
    );
    assert_eq!(app.texts(), before, "the committed interface is untouched");
    assert_eq!(rows_of(&app), vec![alpha, beta]);
    assert_eq!(instances_by_row(&app), before_instances);

    armed.set(false);
    app.refresh().expect("the healed row mounts");
    assert_eq!(
        app.texts(),
        vec!["note 1:alpha", "note 2:beta", "note 3:gamma"]
    );
    assert_eq!(rows_of(&app), store.ids());
}

#[test]
fn a_factory_panic_keeps_the_old_mapping() {
    let store = note_store();
    let alpha = insert(&store, note(1, "alpha"));
    let armed = Rc::new(Cell::new(false));

    let mut app = Runtime::new();
    app.mount({
        let store = store.clone();
        let armed = Rc::clone(&armed);
        move || view! { <PanicList store={store} armed={armed} /> }
    })
    .expect("mount the panic list");
    let before = app.texts();

    armed.set(true);
    insert(&store, note(2, "beta"));
    let failure = app.refresh().expect_err("the fresh row factory panics");
    assert!(
        matches!(&failure, MountError::FactoryPanicked(_)),
        "unexpected failure: {failure:?}"
    );
    assert_eq!(app.texts(), before);
    assert_eq!(rows_of(&app), vec![alpha]);

    armed.set(false);
    app.refresh().expect("the next frame mounts the row");
    assert_eq!(app.texts(), vec!["note 1:alpha", "note 2:beta"]);
}

#[test]
fn a_key_selector_panic_keeps_the_old_mapping() {
    let store = note_store();
    let safe = insert(&store, note(1, "safe"));

    let mut app = Runtime::new();
    app.mount({
        let store = store.clone();
        move || view! { <BoomKeyList store={store} /> }
    })
    .expect("mount the boom-keyed list");
    let before = app.texts();

    let boom = insert(&store, note(2, "boom"));
    let failure = app.refresh().expect_err("the key selector panics");
    assert!(
        matches!(&failure, MountError::Panic(_)),
        "unexpected failure: {failure:?}"
    );
    assert_eq!(
        app.texts(),
        before,
        "the committed key mapping is untouched"
    );
    assert_eq!(rows_of(&app), vec![safe]);

    store
        .update(boom, |row| row.text = String::from("fixed"))
        .expect("heal the row");
    app.refresh().expect("the healed row mounts");
    assert_eq!(app.texts(), vec!["note 1:safe", "note 2:fixed"]);
}

#[test]
fn a_rowset_from_another_store_is_rejected() {
    let foreign = note_store();
    insert(&foreign, note(1, "alpha"));
    let set = foreign.rowset(foreign.ids());
    let target = note_store();

    let mut app = Runtime::new();
    let failure = app
        .mount({
            let target = target.clone();
            move || view! { <StaticList store={target} set={set.clone()} /> }
        })
        .expect_err("a row set from another store cannot resolve rows");

    assert_eq!(
        failure,
        MountError::WrongStore {
            expected: target.id(),
            found: foreign.id(),
        }
    );
}

#[test]
fn a_static_rowset_is_a_snapshot_of_the_store_order() {
    let store = note_store();
    let one = insert(&store, note(1, "one"));
    let two = insert(&store, note(2, "two"));
    let set = store.rowset([one, two]);

    let mut app = Runtime::new();
    app.mount({
        let store = store.clone();
        move || view! { <StaticList store={store} set={set.clone()} /> }
    })
    .expect("mount the static set");
    assert_eq!(app.texts(), vec!["note 1:one", "note 2:two", "count:2"]);
    assert_eq!(rows_of(&app), vec![one, two]);

    store.reorder(&[two, one]).expect("reorder the store");
    app.refresh()
        .expect("a static set keeps refreshing its rows");
    assert_eq!(
        app.texts(),
        vec!["note 1:one", "note 2:two", "count:2"],
        "a store-built RowSet is an immutable snapshot, not a live projection"
    );
    assert_eq!(rows_of(&app), vec![one, two]);
    assert_eq!(store.ids(), vec![two, one], "the store itself did reorder");
}

#[test]
fn a_stale_static_rowset_mounts_and_drops_deleted_rows() {
    let store = note_store();
    let one = insert(&store, note(1, "one"));
    let dead = insert(&store, note(2, "dead"));
    let three = insert(&store, note(3, "three"));
    let set = store.rowset([one, dead, three]);
    store
        .remove(dead)
        .expect("delete the middle row after the snapshot");

    let mut app = Runtime::new();
    app.mount({
        let store = store.clone();
        move || view! { <StaticList store={store} set={set.clone()} /> }
    })
    .expect("a stale snapshot still mounts its live rows");
    assert_eq!(app.texts(), vec!["note 1:one", "note 3:three", "count:2"]);
    assert_eq!(rows_of(&app), vec![one, three]);

    store.remove(three).expect("delete a mounted row");
    app.refresh().expect("the live rows keep refreshing");
    assert_eq!(app.texts(), vec!["note 1:one", "count:2"]);
    assert_eq!(rows_of(&app), vec![one]);
}

#[test]
fn a_windowed_static_rowset_crosses_a_deleted_row_without_failing() {
    let store = note_store();
    let ids: Vec<RowId> = (0..60)
        .map(|index| insert(&store, note(index, &format!("n{index}"))))
        .collect();
    let set = store.rowset(ids.iter().copied());

    let mut app = Runtime::new();
    app.declare_windows({
        let store = store.clone();
        let set = set.clone();
        move |catalog| {
            catalog.window(WindowSpec::new("main").primary(), move |_| {
                let store = store.clone();
                let set = set.clone();
                view! { <WindowedStaticList store={store} set={set} /> }
            })
        }
    })
    .expect("declare the window");
    let window = app.show("main").expect("show the window");

    let band = app.presentation_of(window).expect("snapshot").row_bands()[0];
    let band_key = band.key();
    assert_eq!(band.total_rows(), 60);
    assert_eq!(band.first_row(), 0);
    assert_eq!(
        app.for_items().len(),
        32,
        "the default window mounts 32 rows"
    );

    store.remove(ids[5]).expect("delete a mounted row");
    app.refresh().expect("the stale snapshot keeps refreshing");
    assert_eq!(app.for_items().len(), 31);
    assert!(
        !rows_of(&app).contains(&ids[5]),
        "the deleted row is unmounted instead of staying a ghost"
    );

    assert!(
        app.set_row_window(window, band_key, 40, 10)
            .expect("scroll past the deleted row")
            .changed()
    );
    assert!(
        app.set_row_window(window, band_key, 0, 10)
            .expect("a range crossing the deleted row still applies")
            .changed(),
        "the requested window differs"
    );
    assert_eq!(
        app.presentation_of(window).expect("snapshot").row_bands()[0].first_row(),
        0,
        "the rejected frame cannot strand the host below the deleted row"
    );
    assert_eq!(
        rows_of(&app),
        vec![
            ids[0], ids[1], ids[2], ids[3], ids[4], ids[6], ids[7], ids[8], ids[9]
        ],
        "the window mounts the live rows and skips the deleted slot"
    );
}

#[test]
fn a_failed_repair_reconciles_the_window_span_not_the_mounted_count() {
    let store = note_store();
    let ids: Vec<RowId> = (0..8)
        .map(|index| insert(&store, note(index, &format!("n{index}"))))
        .collect();
    let set = store.rowset(ids.iter().copied());
    store
        .remove(ids[2])
        .expect("delete a row inside the window span");

    let first_fails = Rc::new(RefCell::new(Vec::new()));
    let second_fails = Rc::new(RefCell::new(Vec::new()));

    let mut app = Runtime::new();
    app.declare_windows({
        let store = store.clone();
        let set = set.clone();
        let first_fails = Rc::clone(&first_fails);
        let second_fails = Rc::clone(&second_fails);
        move |catalog| {
            catalog.window(WindowSpec::new("main").primary(), move |_| {
                let store = store.clone();
                let set = set.clone();
                let first_fails = Rc::clone(&first_fails);
                let second_fails = Rc::clone(&second_fails);
                view! {
                    <FaultedStaticBands
                        store={store}
                        set={set}
                        first_fails={first_fails}
                        second_fails={second_fails}
                    />
                }
            })
        }
    })
    .expect("declare the window");
    let window = app.show("main").expect("show the window");
    let snapshot = app.presentation_of(window).expect("snapshot");
    assert_eq!(snapshot.row_bands().len(), 2);
    let (head, tail) = (snapshot.row_bands()[0].key(), snapshot.row_bands()[1].key());

    let moved = app
        .set_row_windows(
            window,
            &[(head, RowWindow::new(3, 5)), (tail, RowWindow::new(3, 5))],
        )
        .expect("both empty batches apply");
    assert!(moved.changed());

    // 首个行带的提交窗口 (0,7) 不需要 ids[7]，失败的是修复回旧窗口；
    // 第二个行带在同一批里挂 ids[0] 时失败。
    first_fails.borrow_mut().push(7);
    second_fails.borrow_mut().push(0);
    app.set_row_windows(
        window,
        &[(head, RowWindow::new(0, 7)), (tail, RowWindow::new(0, 7))],
    )
    .expect_err("the second band cannot mount the new window");

    let texts = app.texts();
    assert!(
        !texts.iter().any(|text| text == "row 7"),
        "committed window excludes ids[7]"
    );
    assert!(
        texts.iter().any(|text| text == "row 6"),
        "the committed span still holds the live row after the deleted slot"
    );
    assert!(
        texts.iter().any(|text| text == "other 7"),
        "the failed band keeps its committed rows"
    );

    // 修复失败后请求映射描述实际提交的窗口跨度 (0,7)。若按已挂行数
    // (6) 改写，接下来的普通刷新会把窗口收窄到 (0,6)，丢掉 span 末尾
    // 仍存活的 ids[6]。
    first_fails.borrow_mut().clear();
    second_fails.borrow_mut().clear();
    app.refresh()
        .expect("the reconciled windows keep refreshing");
    let texts = app.texts();
    assert!(
        texts.iter().any(|text| text == "row 6"),
        "the reconciled span keeps the live trailing row"
    );
    assert!(!texts.iter().any(|text| text == "row 7"));
    assert_eq!(
        app.presentation_of(window).expect("snapshot").row_bands()[0].first_row(),
        0
    );
}

#[test]
fn todo_rows_still_mount_through_for_source_and_row_view() {
    let keys = Rc::new(RefCell::new(Vec::new()));

    let mut app = Runtime::new();
    app.mount({
        let keys = Rc::clone(&keys);
        move || view! { <TodoList keys={keys} /> }
    })
    .expect("mount the todo list");
    app.click_text("Seed").expect("seed two todos");

    let items = app.for_items();
    assert_eq!(items.len(), 2);
    let todo_keys: Vec<TodoId> = items
        .iter()
        .map(|item| item.todo.expect("todo rows keep their business key"))
        .collect();
    assert_eq!(
        todo_keys,
        *keys.borrow(),
        "RowView::key and ItemInfo::todo resolve the same business key"
    );
    assert_eq!(
        app.texts()
            .iter()
            .filter(|text| text.starts_with("todo "))
            .count(),
        2
    );
}

fn editor(app: &Runtime, label: &str) -> NodeId {
    app.presentation()
        .expect("snapshot")
        .find_label(label)
        .expect("the note editor")
        .id()
}

/// 行数据来自应用自己的模型；业务 key 由应用定义，因此可以同 key 重建。
///
/// `reinsert` 用不同正文重建，证明新行读取自己的快照而不是复用旧缓存；
/// `swap` 在同一帧里先删后建并保持相同正文，用来验证仅按业务 key 合并的路径。
#[component]
fn EditableNotes(store: RowStore<Note>) -> MountResult<MountPlan> {
    let rows = store.rows(AllNotes)?;
    let delete_store = store.clone();
    let reinsert_store = store.clone();
    let swap_store = store.clone();
    view! {
        <view>
            <button aria:label="delete" on:click={move || {
                if let Some(id) = delete_store.id_of_key(1) {
                    delete_store.remove(id).expect("delete note 1");
                }
            }}>"Delete"</button>
            <button aria:label="reinsert" on:click={move || {
                reinsert_store.insert(note(1, "omega")).expect("reinsert note 1");
            }}>"Reinsert"</button>
            <button aria:label="swap" on:click={move || {
                if let Some(id) = swap_store.id_of_key(1) {
                    swap_store.remove(id).expect("remove note 1");
                    swap_store.insert(note(1, "alpha")).expect("reinsert note 1");
                }
            }}>"Swap"</button>
            <For each={rows} key={|note: &Note| note.key}>
                {move |row: RowView<Note>| {
                    let key = row.key();
                    // 可编辑的 value 需要动态来源；静态值会在刷新时把本地编辑
                    // 当成外部重置。
                    let value = signal(row.get().text.clone());
                    view! {
                        <text-input
                            aria:label={once(format!("note {key}"))}
                            value={value}
                            on:input={move |_text: String| {}}
                        />
                    }
                }}
            </For>
        </view>
    }
}

#[test]
fn delete_and_reinsert_of_the_same_model_key_starts_a_fresh_row() {
    let store = note_store();
    let old_row = insert(&store, note(1, "alpha"));
    let other = insert(&store, note(2, "beta"));

    let mut app = Runtime::new();
    app.mount({
        let store = store.clone();
        move || view! { <EditableNotes store={store} /> }
    })
    .expect("mount the note list");
    assert_eq!(rows_of(&app), vec![old_row, other]);
    let old_node = editor(&app, "note 1");
    let other_node = editor(&app, "note 2");
    let baseline_signals = app.live_signals();
    let old_item = app.for_items()[0];

    app.focus("note 1").expect("focus note 1");
    app.set_caret_node(old_node, 1).expect("place the caret");
    app.insert_text_node(old_node, "X")
        .expect("edit the old note");
    assert_eq!(
        app.presentation()
            .expect("snapshot")
            .find_label("note 1")
            .expect("the old editor")
            .text_value(),
        Some("aXlpha")
    );

    app.click_label("delete").expect("delete note 1");
    assert_eq!(app.node_status(old_node), NodeStatus::Stale);
    assert_eq!(rows_of(&app), vec![other]);
    assert!(store.id_of_key(1).is_none());
    assert_eq!(
        app.live_signals(),
        baseline_signals - 1,
        "the deleted row's owner resources are released"
    );
    assert_eq!(
        app.presentation().expect("snapshot").focus(),
        Some(other_node),
        "deleting the focused row settles focus on the surviving editor"
    );

    app.click_label("reinsert")
        .expect("reinsert the same business key");
    let new_row = store.id_of_key(1).expect("note 1 is back");
    assert_ne!(new_row, old_row, "the same key gets a new RowId");
    assert_eq!(rows_of(&app), vec![other, new_row]);

    let new_node = editor(&app, "note 1");
    assert_ne!(new_node, old_node, "the reinserted row mounts a fresh node");
    assert_eq!(app.node_status(old_node), NodeStatus::Stale);
    assert_eq!(
        app.live_signals(),
        baseline_signals,
        "the reinserted row owns exactly one fresh signal"
    );
    let new_item = app.for_items()[1];
    assert_ne!(new_item.instance, old_item.instance, "fresh instance");
    assert_ne!(new_item.owner, old_item.owner, "fresh owner");

    let snapshot = app.presentation().expect("snapshot");
    assert_eq!(
        snapshot.focus(),
        Some(other_node),
        "the reinserted row does not inherit focus"
    );
    let element = snapshot.find_label("note 1").expect("the new editor");
    assert_eq!(
        element.text_value(),
        Some("omega"),
        "the fresh row reads its own snapshot, not the deleted row's"
    );
    let edit = element.edit().expect("edit state");
    assert_eq!(edit.caret(), 0);
    assert_eq!(edit.selection(), None);
    assert_eq!(
        app.insert_text_node(old_node, "Z"),
        Err(DispatchError::StaleNode(old_node)),
        "the old mount rejects delivery"
    );
}

#[test]
fn deleting_and_reinserting_the_same_key_in_one_turn_mounts_a_fresh_row() {
    let store = note_store();
    let old_row = insert(&store, note(1, "alpha"));
    let other = insert(&store, note(2, "beta"));

    let mut app = Runtime::new();
    app.mount({
        let store = store.clone();
        move || view! { <EditableNotes store={store} /> }
    })
    .expect("mount the note list");
    let old_node = editor(&app, "note 1");
    let other_node = editor(&app, "note 2");
    let baseline_signals = app.live_signals();
    let old_item = app.for_items()[0];
    app.focus("note 1").expect("focus note 1");
    app.set_caret_node(old_node, 1).expect("place the caret");
    app.insert_text_node(old_node, "X")
        .expect("edit the old note");

    app.click_label("swap")
        .expect("delete and reinsert the same key in one turn");
    let new_row = store.id_of_key(1).expect("note 1 is back");
    assert_ne!(new_row, old_row, "the same key gets a new RowId");
    assert_eq!(rows_of(&app), vec![other, new_row]);

    let new_node = editor(&app, "note 1");
    assert_ne!(new_node, old_node, "the frame mounts a fresh row element");
    assert_eq!(app.node_status(old_node), NodeStatus::Stale);
    assert_eq!(
        app.live_signals(),
        baseline_signals,
        "the swap releases the old row owner and allocates one for the new row"
    );
    let new_item = app.for_items()[1];
    assert_ne!(new_item.instance, old_item.instance, "fresh instance");
    assert_ne!(new_item.owner, old_item.owner, "fresh owner");

    let snapshot = app.presentation().expect("snapshot");
    assert_eq!(
        snapshot.focus(),
        Some(other_node),
        "stale focus settles on the surviving editor, not the reinserted row"
    );
    let element = snapshot.find_label("note 1").expect("the new editor");
    assert_eq!(element.text_value(), Some("alpha"));
    let edit = element.edit().expect("edit state");
    assert_eq!(edit.caret(), 0);
    assert_eq!(edit.selection(), None);
    assert_eq!(
        app.insert_text_node(old_node, "Z"),
        Err(DispatchError::StaleNode(old_node))
    );
}
