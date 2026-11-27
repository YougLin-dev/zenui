//! `RowSet` 的门禁：Store 拥有的顺序快照、顺序敏感相等与唯一构造入口。

use std::panic::AssertUnwindSafe;

use zenui_core::model::{
    FieldDef, FieldId, FilterId, FilterSpec, Row, RowId, RowSet, SchemaBuilder, Store, StoreError,
};

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

const TEXT: FieldId = FieldId::new(0);
const ALL: FilterId = FilterId::new(0);
const NEVER: FilterId = FilterId::new(1);

struct All;

impl FilterSpec<Note> for All {
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

struct Never;

impl FilterSpec<Note> for Never {
    fn id(&self) -> FilterId {
        NEVER
    }

    fn watched_fields(&self) -> &'static [FieldId] {
        &[]
    }

    fn matches(&self, _row: &Note) -> bool {
        false
    }
}

fn make_store() -> Store<Note> {
    let schema = SchemaBuilder::new()
        .field(FieldDef::comparing(TEXT, |row: &Note| &row.text))
        .filter(All)
        .filter(Never)
        .build()
        .expect("test schema");
    Store::new(schema)
}

fn insert(store: &Store<Note>, key: u32) -> RowId {
    store
        .insert(Note {
            key,
            text: format!("note {key}"),
        })
        .expect("insert")
        .0
}

fn ids(store: &Store<Note>) -> Vec<RowId> {
    store.ids()
}

#[test]
fn a_row_set_is_one_pointer_wide() {
    assert_eq!(std::mem::size_of::<RowSet>(), 8);
    assert_eq!(std::mem::size_of::<Option<RowSet>>(), 8);
}

#[test]
fn a_store_rowset_uses_store_order_and_deduplicates() {
    let store = make_store();
    let first = insert(&store, 1);
    let second = insert(&store, 2);
    let third = insert(&store, 3);

    let set = store.rowset([third, first, first]);
    assert_eq!(set.store(), store.id());
    assert_eq!(
        set.ids(),
        [first, third],
        "input order is ignored and duplicates are removed"
    );
    assert_eq!(set.len(), 2);
    assert_eq!(set.iter().collect::<Vec<_>>(), vec![first, third]);

    let subset = store.rowset([second]);
    assert_eq!(subset.ids(), [second]);
    assert!(store.rowset(std::iter::empty()).is_empty());
}

#[test]
fn a_store_rowset_rejects_foreign_and_deleted_rows() {
    let store = make_store();
    let other = make_store();
    let foreign = insert(&other, 9);
    assert!(
        std::panic::catch_unwind(AssertUnwindSafe(|| store.rowset([foreign]))).is_err(),
        "a row from another store is rejected"
    );

    let live = insert(&store, 1);
    store.remove(live).expect("remove");
    assert!(
        std::panic::catch_unwind(AssertUnwindSafe(|| store.rowset([live]))).is_err(),
        "a deleted row is rejected"
    );
}

#[test]
fn rowset_equality_includes_store_and_order() {
    let store = make_store();
    let first = insert(&store, 1);
    let second = insert(&store, 2);
    let third = insert(&store, 3);
    let order = ids(&store);

    let forward = store.rowset(order.iter().copied());
    let again = store.rowset(order.iter().copied());
    assert_eq!(forward, again, "same store and ids compare equal");
    assert!(
        !forward.shares_with(&again),
        "separate Store::rowset calls are deep-equal, not shared"
    );
    assert_eq!(
        store.rowset(order.iter().copied().rev()),
        forward,
        "input order never overrides Store order"
    );

    store
        .reorder(&[third, second, first])
        .expect("reverse the store");
    let reversed = store.rowset(store.ids());
    assert_ne!(forward, reversed, "a pure reorder changes equality");
    assert_eq!(reversed.ids(), ids(&store));
    assert_eq!(reversed.ids(), [third, second, first]);

    let other = make_store();
    let foreign_empty = other.rowset(std::iter::empty());
    assert_ne!(
        store.rowset(std::iter::empty()),
        foreign_empty,
        "empty sets from different stores are not equal"
    );
}

#[test]
fn filter_projection_rowsets_share_until_membership_or_order_moves() {
    let store = make_store();
    let first = insert(&store, 1);
    let second = insert(&store, 2);

    let first_read = store.rowset_from_filter(ALL).expect("filter");
    let second_read = store.rowset_from_filter(ALL).expect("filter");
    assert!(
        first_read.shares_with(&second_read),
        "the same revision reuses one projection Arc"
    );
    assert_eq!(first_read.ids(), [first, second]);

    store
        .update(first, |row| row.text = String::from("renamed"))
        .expect("field change");
    let after_field = store.rowset_from_filter(ALL).expect("filter");
    assert!(
        first_read.shares_with(&after_field),
        "a field-only change keeps the projection"
    );

    store.reorder(&[second, first]).expect("reorder");
    let after_order = store.rowset_from_filter(ALL).expect("filter");
    assert!(
        !first_read.shares_with(&after_order),
        "an order change rebuilds the projection"
    );
    assert_eq!(after_order.ids(), [second, first]);
    assert_eq!(
        store
            .rowset_from_filter(FilterId::new(99))
            .expect_err("unknown filter"),
        StoreError::UnknownFilter(FilterId::new(99))
    );
}

#[test]
fn an_empty_filter_projection_is_reused_across_order_changes() {
    let store = make_store();
    insert(&store, 1);
    let first = store.rowset_from_filter(NEVER).expect("filter");
    assert!(first.is_empty());

    // 插入活跃行会推进 order revision；空投影与顺序无关，不重建也不扫描。
    insert(&store, 2);
    let second = store.rowset_from_filter(NEVER).expect("filter");
    assert!(second.is_empty());
    assert!(
        first.shares_with(&second),
        "an empty projection survives an order revision unchanged"
    );

    let reversed: Vec<RowId> = store.ids().into_iter().rev().collect();
    store.reorder(&reversed).expect("reverse the store");
    assert!(
        first.shares_with(&store.rowset_from_filter(NEVER).expect("filter")),
        "reordering remains irrelevant to an empty membership"
    );
}

#[test]
fn the_core_root_names_the_model_rowset() {
    let store = make_store();
    insert(&store, 1);
    // `zenui_core::RowSet` 是模型 newtype；旧业务键集合保留为 `LegacyRowSet`。
    let set: zenui_core::RowSet = store.rowset(store.ids());
    assert_eq!(set.len(), 1);
    let legacy = zenui_core::LegacyRowSet::empty();
    assert!(legacy.is_empty());
}
