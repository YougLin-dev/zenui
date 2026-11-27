//! 通用行模型的门禁：第二个非 Todo 模型只经公开 `zenui_core::model` API 运行。
//!
//! 覆盖 ticket #337 的验收：第二个模型、RowId 身份、精确 ChangeSet、候选行
//! 过滤投影、相等写入与删除后重建。

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use zenui_core::model::{
    ChangeSet, FieldDef, FieldId, FilterId, FilterSpec, Row, RowId, RowSetData, Schema,
    SchemaBuilder, Store, StoreError, TransactionError, Tx,
};

#[derive(Clone, Debug, PartialEq)]
struct Note {
    key: u32,
    text: String,
    pinned: bool,
    score: u8,
    metadata: u32,
}

impl Row for Note {
    type Key = u32;

    fn key(&self) -> u32 {
        self.key
    }
}

const TEXT: FieldId = FieldId::new(0);
const PINNED: FieldId = FieldId::new(1);
const SCORE: FieldId = FieldId::new(2);
const ALL: FilterId = FilterId::new(0);
const PINNED_FILTER: FilterId = FilterId::new(1);
const HIGH_SCORE: FilterId = FilterId::new(2);

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

struct Pinned;

impl FilterSpec<Note> for Pinned {
    fn id(&self) -> FilterId {
        PINNED_FILTER
    }

    fn watched_fields(&self) -> &'static [FieldId] {
        &[PINNED]
    }

    fn matches(&self, row: &Note) -> bool {
        row.pinned
    }
}

struct HighScore;

impl FilterSpec<Note> for HighScore {
    fn id(&self) -> FilterId {
        HIGH_SCORE
    }

    fn watched_fields(&self) -> &'static [FieldId] {
        &[SCORE]
    }

    fn matches(&self, row: &Note) -> bool {
        row.score >= 10
    }
}

fn schema() -> Schema<Note> {
    SchemaBuilder::new()
        .field(FieldDef::comparing(TEXT, |row: &Note| &row.text))
        .field(FieldDef::comparing(PINNED, |row: &Note| &row.pinned))
        .field(FieldDef::comparing(SCORE, |row: &Note| &row.score))
        .filter(All)
        .filter(Pinned)
        .filter(HighScore)
        .build()
        .expect("test schema")
}

fn store() -> Store<Note> {
    Store::new(schema())
}

fn note(key: u32) -> Note {
    Note {
        key,
        text: format!("note {key}"),
        pinned: false,
        score: 0,
        metadata: 0,
    }
}

fn insert(store: &Store<Note>, key: u32) -> RowId {
    store.insert(note(key)).expect("insert").0
}

fn keys(store: &Store<Note>, filter: FilterId) -> Vec<u32> {
    store
        .projection(filter)
        .expect("registered filter")
        .ids()
        .iter()
        .map(|id| store.key_of(*id).expect("live row"))
        .collect()
}

fn projection(store: &Store<Note>, filter: FilterId) -> Arc<RowSetData> {
    store.projection(filter).expect("registered filter")
}

fn in_transaction<T, E>(
    store: &Store<Note>,
    f: impl FnOnce(&mut Tx<'_, Note>) -> Result<T, E>,
) -> Result<(T, ChangeSet), TransactionError<E>> {
    store
        .transaction(f)
        .map(|(value, commit)| (value, commit.into_changes()))
}

#[test]
fn a_second_model_runs_through_the_public_model_api() {
    let store = store();
    let first = insert(&store, 1);
    let second = insert(&store, 2);
    assert_eq!(store.len(), 2);
    assert_eq!(store.get(first).expect("live").text, "note 1");
    assert_eq!(store.key_of(second), Some(2));
    assert_eq!(store.id_of_key(1), Some(first));
    assert!(store.contains(second));
    assert_eq!(store.ids(), vec![first, second]);
    assert_eq!(keys(&store, ALL), vec![1, 2]);
    assert_eq!(
        store
            .rows(All)
            .expect("rows")
            .snapshot()
            .expect("snapshot")
            .len(),
        2
    );

    store
        .update(first, |row| row.text = String::from("renamed"))
        .expect("rename");
    assert_eq!(store.get(first).expect("live").text, "renamed");
}

#[test]
fn row_ids_are_arena_identities_and_reinsertion_is_fresh() {
    assert_eq!(std::mem::size_of::<RowId>(), 8);
    let store = store();
    let old = insert(&store, 9);
    store.remove(old).expect("remove");
    let new = insert(&store, 9);
    assert_ne!(old, new, "the same business key gets a new identity");
    assert_eq!(old.slot(), new.slot(), "the slot is recycled");
    assert_ne!(old.generation(), new.generation());
    assert!(store.get(old).is_none());
    assert!(!store.contains(old));
    assert_eq!(store.key_of(new), Some(9));
    assert_eq!(store.ids(), vec![new]);
    assert_eq!(
        store
            .update(old, |row| row.text.clear())
            .expect_err("stale id"),
        StoreError::UnknownRow(old)
    );
}

#[test]
fn foreign_and_removed_ids_never_resolve() {
    let first = store();
    let second = store();
    let id = insert(&first, 1);
    assert!(!second.contains(id));
    assert!(second.get(id).is_none());
    assert_eq!(
        second.remove(id).expect_err("foreign id"),
        StoreError::UnknownRow(id)
    );

    first.remove(id).expect("remove");
    assert!(!first.contains(id));
    assert!(first.get(id).is_none());
}

#[test]
fn change_set_records_identity_fields_membership_and_order() {
    let store = store();
    let first = insert(&store, 1);
    let second = insert(&store, 2);
    let third = insert(&store, 3);
    store.update(third, |row| row.pinned = true).expect("pin");

    let ((), changes) = in_transaction(&store, |tx| {
        tx.update(first, |row| row.score = 12)?;
        tx.update(second, |row| row.metadata += 1)?;
        tx.reorder(&[third, second, first])?;
        Ok::<(), StoreError>(())
    })
    .expect("transaction");

    assert!(changes.inserted.is_empty() && changes.removed.is_empty());
    assert_eq!(
        changes.fields,
        vec![(first, SCORE)],
        "only registered field changes are recorded"
    );
    assert_eq!(changes.membership, vec![HIGH_SCORE]);
    assert!(changes.order);
    assert_eq!(keys(&store, ALL), vec![3, 2, 1]);
}

#[test]
fn a_failed_transaction_commits_nothing() {
    let store = store();
    let id = insert(&store, 1);
    let error = in_transaction(&store, |tx| {
        tx.insert(note(2))?;
        tx.update(id, |row| row.text = String::from("changed"))?;
        Err::<(), _>(StoreError::IncompleteOrder)
    })
    .expect_err("aborted");
    assert!(matches!(
        error,
        TransactionError::Aborted(StoreError::IncompleteOrder)
    ));
    assert_eq!(store.len(), 1);
    assert_eq!(store.get(id).expect("live").text, "note 1");
    assert_eq!(store.id_of_key(2), None);
    let recovered = insert(&store, 2);
    assert_eq!(store.len(), 2);
    assert!(store.get(recovered).is_some());
}

#[test]
fn remove_and_reinsert_in_one_transaction_keeps_both_identities() {
    let store = store();
    let old = insert(&store, 5);
    let (new, changes) = in_transaction(&store, |tx| -> Result<RowId, StoreError> {
        tx.remove(old)?;
        tx.insert(note(5))
    })
    .expect("transaction");
    assert_ne!(old, new);
    assert_eq!(changes.removed, vec![old]);
    assert_eq!(changes.inserted, vec![new]);
    assert_eq!(store.id_of_key(5), Some(new));
    assert!(store.get(old).is_none());
    assert_eq!(store.get(new).expect("live").key, 5);
}

#[test]
fn insert_then_remove_is_net_zero() {
    let store = store();
    let ((), changes) = in_transaction(&store, |tx| {
        let id = tx.insert(note(1))?;
        tx.remove(id)?;
        Ok::<(), StoreError>(())
    })
    .expect("transaction");
    assert!(changes.is_empty());
    assert_eq!(store.len(), 0);
    assert!(projection(&store, ALL).is_empty());
}

#[test]
fn duplicate_keys_are_rejected() {
    let store = store();
    insert(&store, 7);
    assert_eq!(
        store.insert(note(7)).expect_err("duplicate"),
        StoreError::DuplicateKey
    );
    assert_eq!(store.len(), 1);
}

#[test]
fn updates_cannot_change_the_business_key() {
    let store = store();
    let id = insert(&store, 1);
    assert_eq!(
        store.update(id, |row| row.key = 2).expect_err("key change"),
        StoreError::KeyChanged(id)
    );
    assert_eq!(store.key_of(id), Some(1));
}

#[test]
fn updating_a_staged_insert_cannot_change_its_key() {
    let store = store();
    let committed = insert(&store, 1);

    let error = store
        .transaction(|tx| -> Result<(), StoreError> {
            let staged = tx.insert(note(2))?;
            tx.update(staged, |row| row.key = 3)?;
            Ok(())
        })
        .expect_err("key change aborts the transaction");
    let TransactionError::Aborted(StoreError::KeyChanged(_)) = error else {
        panic!("unexpected: {error:?}");
    };
    assert_eq!(store.len(), 1);
    assert_eq!(store.id_of_key(2), None);

    // 被拒绝的写入不改变草稿键索引：原键仍归这行，新键仍可插入。
    let ((), changes) = in_transaction(&store, |tx| {
        let staged = tx.insert(note(2))?;
        assert_eq!(
            tx.update(staged, |row| row.key = 1),
            Err(StoreError::KeyChanged(staged))
        );
        assert_eq!(tx.key_of(staged), Some(2));
        assert_eq!(tx.id_of_key(1), Some(committed));
        assert_eq!(tx.id_of_key(3), None);
        assert_eq!(tx.insert(note(2)), Err(StoreError::DuplicateKey));
        tx.remove(staged)
    })
    .expect("transaction");
    assert!(changes.is_empty());
    assert_eq!(store.len(), 1);
}

#[test]
fn reorder_validates_the_full_permutation() {
    let store = store();
    let first = insert(&store, 1);
    let second = insert(&store, 2);
    assert_eq!(
        store.reorder(&[first, first]).expect_err("duplicate"),
        StoreError::DuplicateRow(first)
    );
    assert_eq!(
        store.reorder(&[first]).expect_err("incomplete"),
        StoreError::IncompleteOrder
    );
    assert!(
        store
            .reorder(&[first, second])
            .expect("same order")
            .is_empty()
    );
    let changes = store.reorder(&[second, first]).expect("swap");
    assert!(changes.order);
    assert!(changes.membership.is_empty());
    assert_eq!(keys(&store, ALL), vec![2, 1]);
}

#[test]
fn repeated_reorders_compose_to_the_last_request() {
    let store = store();
    let first = insert(&store, 1);
    let second = insert(&store, 2);

    // 同一事务里重复同一个重排不能抵消更早的请求。
    let ((), changes) = in_transaction(&store, |tx| {
        tx.reorder(&[second, first])?;
        tx.reorder(&[second, first])?;
        Ok::<(), StoreError>(())
    })
    .expect("transaction");
    assert!(changes.order);
    assert_eq!(keys(&store, ALL), vec![2, 1]);

    // 插入后再次声明完整顺序：最终顺序来自最后一次请求；新行由
    // inserted 表达，存活行的相对顺序没有变化。
    let (third, changes) = in_transaction(&store, |tx| -> Result<RowId, StoreError> {
        let third = tx.insert(note(3))?;
        tx.reorder(&[third, second, first])?;
        tx.reorder(&[third, second, first])?;
        Ok(third)
    })
    .expect("transaction");
    assert_eq!(changes.inserted, vec![third]);
    assert!(!changes.order, "moving a new row is not a survivor reorder");
    assert_eq!(keys(&store, ALL), vec![3, 2, 1]);
}

#[test]
fn a_reorder_with_no_net_effect_is_not_reported() {
    let store = store();
    let first = insert(&store, 1);
    let second = insert(&store, 2);
    let third = insert(&store, 3);
    let order = store.order_revision();

    // 事务内换回原顺序：净变化为空，顺序版本也不推进。
    let ((), changes) = in_transaction(&store, |tx| {
        tx.reorder(&[second, first, third])?;
        tx.reorder(&[first, second, third])?;
        Ok::<(), StoreError>(())
    })
    .expect("transaction");
    assert!(changes.is_empty(), "unexpected: {changes:?}");
    assert_eq!(store.ids(), vec![first, second, third]);
    assert_eq!(store.order_revision(), order);

    // 被重排的行随后删除：存活行相对顺序没有变化。
    let ((), changes) = in_transaction(&store, |tx| {
        tx.reorder(&[third, first, second])?;
        tx.remove(third)
    })
    .expect("transaction");
    assert_eq!(changes.removed, vec![third]);
    assert!(
        !changes.order,
        "removing the moved row explains the final order"
    );
    assert_eq!(keys(&store, ALL), vec![1, 2]);
}

#[test]
fn equal_writes_do_not_mark_fields_or_rows_dirty() {
    let store = store();
    let id = insert(&store, 1);
    let membership = store.membership_revision(ALL).expect("filter");
    let order = store.order_revision();

    let changes = store
        .update(id, |row| row.text = String::from("note 1"))
        .expect("equal text");
    assert!(changes.is_empty());
    let changes = store
        .update(id, |row| row.metadata += 1)
        .expect("unregistered field");
    assert!(changes.is_empty(), "unregistered fields are not reactive");
    assert_eq!(
        store.get(id).expect("live").metadata,
        1,
        "the write is still persisted"
    );
    assert_eq!(store.membership_revision(ALL), Ok(membership));
    assert_eq!(store.order_revision(), order);
}

#[test]
fn candidate_rows_join_a_filter_after_a_field_change() {
    let store = store();
    let id = insert(&store, 1);
    let pinned = insert(&store, 2);
    assert!(keys(&store, HIGH_SCORE).is_empty());

    let changes = store.update(id, |row| row.score = 20).expect("raise");
    assert_eq!(changes.fields, vec![(id, SCORE)]);
    assert_eq!(changes.membership, vec![HIGH_SCORE]);
    assert_eq!(
        keys(&store, HIGH_SCORE),
        vec![1],
        "a row outside the filter is re-evaluated after its field change"
    );

    let changes = store.update(id, |row| row.score = 1).expect("lower");
    assert_eq!(changes.membership, vec![HIGH_SCORE]);
    assert!(keys(&store, HIGH_SCORE).is_empty());

    let changes = store.update(pinned, |row| row.pinned = true).expect("pin");
    assert_eq!(changes.membership, vec![PINNED_FILTER]);
    assert_eq!(keys(&store, PINNED_FILTER), vec![2]);
}

#[test]
fn inserted_and_removed_rows_move_every_affected_membership() {
    let store = store();
    let (id, changes) = store.insert(note(1)).expect("insert");
    assert_eq!(changes.membership, vec![ALL]);

    store.update(id, |row| row.pinned = true).expect("pin");
    let changes = store.remove(id).expect("remove");
    assert_eq!(changes.removed, vec![id]);
    assert_eq!(changes.membership, vec![ALL, PINNED_FILTER]);
    assert!(!changes.order);
}

#[test]
fn projections_are_shared_until_membership_or_order_moves() {
    let store = store();
    let first = insert(&store, 1);
    insert(&store, 2);
    let before = projection(&store, ALL);
    let again = projection(&store, ALL);
    assert!(Arc::ptr_eq(&before, &again), "the same revision shares");

    let all_membership = store.membership_revision(ALL).expect("filter");
    let high_membership = store.membership_revision(HIGH_SCORE).expect("filter");
    store.update(first, |row| row.score = 30).expect("qualify");
    assert!(store.membership_revision(HIGH_SCORE).expect("filter") > high_membership);
    assert_eq!(
        store.membership_revision(ALL),
        Ok(all_membership),
        "the other filter did not move"
    );
    let after_field = projection(&store, ALL);
    assert!(
        Arc::ptr_eq(&before, &after_field),
        "a field change that leaves the filter alone keeps its projection"
    );

    let order = store.order_revision();
    let ids = store.ids();
    store.reorder(&[ids[1], ids[0]]).expect("swap");
    assert!(store.order_revision() > order);
    let rebuilt = projection(&store, ALL);
    assert!(
        !Arc::ptr_eq(&before, &rebuilt),
        "an order move invalidates every projection"
    );
    assert_eq!(keys(&store, ALL), vec![2, 1]);
}

#[test]
fn filter_is_empty_reads_membership() {
    let store = store();
    let id = insert(&store, 1);
    assert!(store.filter_is_empty(HIGH_SCORE).expect("filter"));
    store.update(id, |row| row.score = 11).expect("qualify");
    assert!(!store.filter_is_empty(HIGH_SCORE).expect("filter"));
    assert_eq!(
        store
            .filter_is_empty(FilterId::new(99))
            .expect_err("unknown"),
        StoreError::UnknownFilter(FilterId::new(99))
    );
}

#[test]
fn rows_expose_the_store_handle_and_revisions() {
    let store = store();
    insert(&store, 1);
    let rows = store.rows(All).expect("rows");
    assert_eq!(rows.store().id(), store.id());
    assert_eq!(rows.filter(), ALL);
    assert!(!rows.is_empty().expect("membership"));
    assert_eq!(rows.membership_revision().expect("revision"), 1);
    assert_eq!(rows.order_revision(), store.order_revision());
    assert_eq!(rows.ids().expect("ids").len(), 1);
}

struct Fragile;

impl FilterSpec<Note> for Fragile {
    fn id(&self) -> FilterId {
        FilterId::new(3)
    }

    fn watched_fields(&self) -> &'static [FieldId] {
        &[SCORE]
    }

    fn matches(&self, row: &Note) -> bool {
        assert!(row.metadata != 77, "filter panic");
        row.score > 5
    }
}

#[test]
fn a_panicking_filter_during_commit_rolls_back_and_keeps_the_store_usable() {
    let schema = SchemaBuilder::new()
        .field(FieldDef::comparing(SCORE, |row: &Note| &row.score))
        .filter(All)
        .filter(Fragile)
        .build()
        .expect("schema");
    let store = Store::new(schema);
    let id = insert(&store, 1);

    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        store.update(id, |row| {
            row.score = 9;
            row.metadata = 77;
        })
    }));
    assert!(outcome.is_err(), "the filter panic escapes the commit");
    assert_eq!(store.get(id).expect("live").score, 0);
    assert_eq!(store.get(id).expect("live").metadata, 0);
    assert_eq!(store.len(), 1);

    store.update(id, |row| row.score = 9).expect("recovered");
    assert_eq!(store.get(id).expect("live").score, 9);
}

#[test]
fn a_panicking_callback_rolls_back_and_leaves_the_store_usable() {
    let store = store();
    let id = insert(&store, 1);
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        store.transaction(|tx| tx.update(id, |_row| panic!("user panic")))
    }));
    assert!(outcome.is_err(), "the panic escapes the transaction");
    assert_eq!(store.get(id).expect("live").text, "note 1");
    assert_eq!(insert(&store, 2), store.id_of_key(2).expect("recovered"));
    assert_eq!(store.len(), 2);
}

#[test]
fn nested_transactions_report_busy_and_do_not_poison_the_store() {
    let store = store();
    let nested = store.clone();
    let error = store
        .transaction(|_tx| nested.transaction(|inner| inner.insert(note(1))))
        .expect_err("nested transaction");
    assert!(
        matches!(
            error,
            TransactionError::Aborted(TransactionError::Store(StoreError::Busy))
        ),
        "unexpected: {error:?}"
    );
    assert_eq!(store.len(), 0);
    insert(&store, 2);
    assert_eq!(store.len(), 1);
}
