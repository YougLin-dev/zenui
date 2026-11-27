use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use crate::handle::Generation;

use super::change::ChangeSet;
use super::id::{FieldId, FilterId, RowId, StoreId, next_generation};
use super::row::{FieldDef, FilterDef, FilterSpec, Row, Schema};
use super::rows::{RowSet, RowSetData, Rows};

/// 通用行模型的写事务错误。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StoreError {
    /// 业务键已经有一行存活（或在同一事务中已插入）。
    DuplicateKey,
    /// 行身份不存在、已删除或属于另一个 Store。
    UnknownRow(RowId),
    /// reorder 列表中有重复行身份。
    DuplicateRow(RowId),
    /// reorder 列表没有覆盖全部存活行。
    IncompleteOrder,
    /// filter 未在 schema 注册。
    UnknownFilter(FilterId),
    /// 更新不能改变业务键。
    KeyChanged(RowId),
    /// 同一 Store 已有活动事务。
    Busy,
    /// arena 无法再分配行身份。
    Exhausted,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateKey => f.write_str("row key is already live"),
            Self::UnknownRow(id) => write!(f, "row {id:?} is not live in this store"),
            Self::DuplicateRow(id) => write!(f, "row {id:?} appears twice in the order"),
            Self::IncompleteOrder => f.write_str("reorder must list every live row"),
            Self::UnknownFilter(filter) => {
                write!(
                    f,
                    "filter {} is not registered by the schema",
                    filter.index()
                )
            }
            Self::KeyChanged(id) => write!(f, "update changed the business key of {id:?}"),
            Self::Busy => f.write_str("the store already has an active transaction"),
            Self::Exhausted => f.write_str("the row arena is exhausted"),
        }
    }
}

impl std::error::Error for StoreError {}

/// 事务闭包错误或 Store 自身拒绝（`Busy`、提交校验）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TransactionError<E> {
    /// 闭包返回的错误；事务已回滚。
    Aborted(E),
    /// Store 拒绝了事务（嵌套事务、提交校验失败）。
    Store(StoreError),
}

impl<E: fmt::Display> fmt::Display for TransactionError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Aborted(error) => write!(f, "{error}"),
            Self::Store(error) => write!(f, "{error}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for TransactionError<E> {}

/// 提交结果：非泛型变化集 + 受影响行身份的业务键。
///
/// `key_of` 在提交后仍能解析被删除行的业务键，供上层把字段依赖映射回业务键。
pub struct Commit<R: Row> {
    changes: ChangeSet,
    keys: BTreeMap<RowId, R::Key>,
}

impl<R: Row> Commit<R> {
    pub const fn changes(&self) -> &ChangeSet {
        &self.changes
    }

    pub fn key_of(&self, id: RowId) -> Option<R::Key> {
        self.keys.get(&id).copied()
    }

    pub fn into_changes(self) -> ChangeSet {
        self.changes
    }
}

impl<R: Row> fmt::Debug for Commit<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Commit")
            .field("changes", &self.changes)
            .finish()
    }
}

/// 通用行 Store：共享句柄，数据由同一 runtime 的消费者共同持有。
///
/// 句柄是 `Clone` 而不是 `Copy`；这是跨 Store 读取、`ForSource` 共享与
/// 无上下文 `get`/`key_of` 的折中。Store 不是 `Send`，与单线程 UI runtime
/// 一致；FilterSpec/FieldDef 回调必须是纯函数，不能重入同一 Store 的写事务。
pub struct Store<R: Row> {
    inner: Rc<StoreInner<R>>,
}

struct StoreInner<R: Row> {
    data: RefCell<StoreData<R>>,
    active: Cell<bool>,
}

struct StoreData<R: Row> {
    id: StoreId,
    rows: RowSlots<R>,
    by_key: BTreeMap<R::Key, RowId>,
    order: Vec<RowId>,
    fields: BTreeMap<FieldId, FieldDef<R>>,
    filters: BTreeMap<FilterId, FilterEntry<R>>,
    /// `FieldId → [FilterId]` 候选反向索引；字段变化只重新求值相关过滤。
    field_filters: BTreeMap<FieldId, Vec<FilterId>>,
    projections: BTreeMap<FilterId, ProjectionCache>,
    order_revision: u64,
}

struct FilterEntry<R: Row> {
    spec: Box<dyn FilterSpec<R>>,
    members: BTreeSet<RowId>,
    membership_revision: u64,
}

struct ProjectionCache {
    membership_revision: u64,
    order_revision: u64,
    data: Arc<RowSetData>,
}

impl<R: Row> Clone for Store<R> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl<R: Row> fmt::Debug for Store<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let data = self.inner.data.borrow();
        f.debug_struct("Store")
            .field("id", &data.id)
            .field("rows", &data.order.len())
            .field("order_revision", &data.order_revision)
            .finish()
    }
}

impl<R: Row> Store<R> {
    pub fn new(schema: Schema<R>) -> Self {
        let (fields, filter_defs) = schema.into_parts();
        let mut filters = BTreeMap::new();
        let mut field_filters: BTreeMap<FieldId, Vec<FilterId>> = BTreeMap::new();
        for (id, FilterDef { spec, watched }) in filter_defs {
            for field in &watched {
                field_filters.entry(*field).or_default().push(id);
            }
            filters.insert(
                id,
                FilterEntry {
                    spec,
                    members: BTreeSet::new(),
                    membership_revision: 0,
                },
            );
        }
        Self {
            inner: Rc::new(StoreInner {
                data: RefCell::new(StoreData {
                    id: StoreId::allocate(),
                    rows: RowSlots::new(),
                    by_key: BTreeMap::new(),
                    order: Vec::new(),
                    fields,
                    filters,
                    field_filters,
                    projections: BTreeMap::new(),
                    order_revision: 0,
                }),
                active: Cell::new(false),
            }),
        }
    }

    pub fn id(&self) -> StoreId {
        self.inner.data.borrow().id
    }

    /// 一次原子事务：闭包返回 `Ok` 时提交，返回 `Err` 或 panic 时回滚。
    pub fn transaction<T, E>(
        &self,
        f: impl FnOnce(&mut Tx<'_, R>) -> Result<T, E>,
    ) -> Result<(T, Commit<R>), TransactionError<E>> {
        if self.inner.active.replace(true) {
            return Err(TransactionError::Store(StoreError::Busy));
        }
        let mut tx = Tx {
            store: self,
            draft: Draft::default(),
            finished: false,
            plan_changes: ChangeSet::default(),
            plan_keys: BTreeMap::new(),
            plan_order: Vec::new(),
            plan_order_moved: false,
            plan_membership: BTreeMap::new(),
            plan_updates: BTreeMap::new(),
        };
        match f(&mut tx) {
            Ok(value) => match tx.finish() {
                Ok(commit) => Ok((value, commit)),
                Err(error) => Err(TransactionError::Store(error)),
            },
            Err(error) => Err(TransactionError::Aborted(error)),
        }
    }

    pub fn insert(&self, row: R) -> Result<(RowId, ChangeSet), StoreError> {
        self.transaction(|tx| tx.insert(row))
            .map(|(id, commit)| (id, commit.into_changes()))
            .map_err(TransactionError::into_store_error)
    }

    pub fn update(&self, id: RowId, change: impl FnOnce(&mut R)) -> Result<ChangeSet, StoreError> {
        self.transaction(|tx| tx.update(id, change))
            .map(|((), commit)| commit.into_changes())
            .map_err(TransactionError::into_store_error)
    }

    pub fn remove(&self, id: RowId) -> Result<ChangeSet, StoreError> {
        self.transaction(|tx| tx.remove(id))
            .map(|((), commit)| commit.into_changes())
            .map_err(TransactionError::into_store_error)
    }

    pub fn reorder(&self, order: &[RowId]) -> Result<ChangeSet, StoreError> {
        self.transaction(|tx| tx.reorder(order))
            .map(|((), commit)| commit.into_changes())
            .map_err(TransactionError::into_store_error)
    }

    pub fn get(&self, id: RowId) -> Option<R> {
        self.inner.data.borrow().rows.get(id).cloned()
    }

    pub fn with_row<T>(&self, id: RowId, f: impl FnOnce(&R) -> T) -> Option<T> {
        let data = self.inner.data.borrow();
        data.rows.get(id).map(f)
    }

    pub fn key_of(&self, id: RowId) -> Option<R::Key> {
        self.inner.data.borrow().rows.get(id).map(Row::key)
    }

    pub fn id_of_key(&self, key: R::Key) -> Option<RowId> {
        self.inner.data.borrow().by_key.get(&key).copied()
    }

    pub fn contains(&self, id: RowId) -> bool {
        self.inner.data.borrow().rows.get(id).is_some()
    }

    pub fn len(&self) -> usize {
        self.inner.data.borrow().order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.data.borrow().order.is_empty()
    }

    /// 存活行身份，按 Store 当前用户顺序排列。
    pub fn ids(&self) -> Vec<RowId> {
        self.inner.data.borrow().order.clone()
    }

    pub fn keys(&self) -> Vec<R::Key> {
        let data = self.inner.data.borrow();
        data.order
            .iter()
            .filter_map(|id| data.rows.get(*id).map(Row::key))
            .collect()
    }

    pub fn rows<F: FilterSpec<R>>(&self, filter: F) -> Result<Rows<R, F>, StoreError> {
        let id = filter.id();
        if !self.inner.data.borrow().filters.contains_key(&id) {
            return Err(StoreError::UnknownFilter(id));
        }
        Ok(Rows::new(self.clone(), id))
    }

    /// 从一个行身份集合构造 Store 拥有的快照；输出顺序恒为 Store 当前用户顺序。
    ///
    /// `ids` 是成员集合：输入顺序不生效，重复 id 被去掉；不在本 Store 存活的行
    /// 是调用方错误（`#[track_caller]` panic）。空集合返回空的 Store 快照。
    #[track_caller]
    pub fn rowset(&self, ids: impl IntoIterator<Item = RowId>) -> RowSet {
        let wanted: BTreeSet<RowId> = ids.into_iter().collect();
        let data = self.inner.data.borrow();
        let store = data.id;
        if wanted.is_empty() {
            return RowSet::from_data(Arc::new(RowSetData::new(store, Box::new([]))));
        }
        for id in &wanted {
            assert!(
                data.rows.get(*id).is_some(),
                "row {id:?} is not live in this store"
            );
        }
        let selected: Box<[RowId]> = data
            .order
            .iter()
            .copied()
            .filter(|id| wanted.contains(id))
            .collect();
        debug_assert_eq!(
            selected.len(),
            wanted.len(),
            "every live row appears once in Store order"
        );
        RowSet::from_data(Arc::new(RowSetData::new(store, selected)))
    }

    /// 过滤投影的 [`RowSet`] 包装；同一 `(membership, order)` revision 复用同一 `Arc`。
    pub fn rowset_from_filter(&self, filter: FilterId) -> Result<RowSet, StoreError> {
        self.projection(filter).map(RowSet::from_data)
    }

    /// 过滤投影；同一 `(membership_revision, order_revision)` 复用共享快照。
    ///
    /// 成员为空的 filter 与 order 无关：命中同 membership revision 的缓存即可
    /// 复用，且从不扫描 Store order。
    pub fn projection(&self, filter: FilterId) -> Result<Arc<RowSetData>, StoreError> {
        let mut data = self.inner.data.borrow_mut();
        let entry = data
            .filters
            .get(&filter)
            .ok_or(StoreError::UnknownFilter(filter))?;
        let membership_revision = entry.membership_revision;
        let membership_empty = entry.members.is_empty();
        let order_revision = data.order_revision;
        if let Some(cached) = data.projections.get(&filter)
            && cached.membership_revision == membership_revision
            && (membership_empty || cached.order_revision == order_revision)
        {
            return Ok(Arc::clone(&cached.data));
        }
        let ids: Box<[RowId]> = if membership_empty {
            Box::new([])
        } else {
            data.order
                .iter()
                .copied()
                .filter(|id| data.filters[&filter].members.contains(id))
                .collect()
        };
        let data_arc = Arc::new(RowSetData::new(data.id, ids));
        data.projections.insert(
            filter,
            ProjectionCache {
                membership_revision,
                order_revision,
                data: Arc::clone(&data_arc),
            },
        );
        Ok(data_arc)
    }

    pub fn membership_revision(&self, filter: FilterId) -> Result<u64, StoreError> {
        self.inner
            .data
            .borrow()
            .filters
            .get(&filter)
            .map(|entry| entry.membership_revision)
            .ok_or(StoreError::UnknownFilter(filter))
    }

    pub fn order_revision(&self) -> u64 {
        self.inner.data.borrow().order_revision
    }

    pub fn filter_is_empty(&self, filter: FilterId) -> Result<bool, StoreError> {
        self.inner
            .data
            .borrow()
            .filters
            .get(&filter)
            .map(|entry| entry.members.is_empty())
            .ok_or(StoreError::UnknownFilter(filter))
    }
}

impl<E> TransactionError<E> {
    fn into_store_error(self) -> StoreError
    where
        E: Into<StoreError>,
    {
        match self {
            Self::Aborted(error) => error.into(),
            Self::Store(error) => error,
        }
    }
}

/// 一个按 Store 顺序排列、带代际的行 arena。
struct RowSlots<R> {
    values: Vec<Option<R>>,
    generations: Vec<Generation>,
    free: Vec<u32>,
}

impl<R> RowSlots<R> {
    const fn new() -> Self {
        Self {
            values: Vec::new(),
            generations: Vec::new(),
            free: Vec::new(),
        }
    }

    /// 预留一个未填充的槽位；代际在预留时分配，因此同一行身份永不复用。
    fn reserve(&mut self) -> Option<RowId> {
        let slot = match self.free.pop() {
            Some(slot) => slot,
            None => {
                let slot = u32::try_from(self.values.len()).ok()?;
                self.values.push(None);
                self.generations.push(Generation::FIRST);
                slot
            }
        };
        let generation = next_generation();
        self.generations[slot as usize] = generation;
        Some(RowId::new(slot, generation))
    }

    fn fill(&mut self, id: RowId, row: R) -> bool {
        let slot = id.slot() as usize;
        match self.values.get_mut(slot) {
            Some(entry @ None) if self.generations[slot] == id.generation() => {
                *entry = Some(row);
                true
            }
            _ => false,
        }
    }

    /// 归还一个从未填充的预留槽位；已填充的槽位必须经 [`Self::remove`]。
    fn release(&mut self, id: RowId) {
        let slot = id.slot() as usize;
        if self.generations.get(slot) != Some(&id.generation()) {
            return;
        }
        if self.values.get(slot).is_some_and(Option::is_none) {
            self.free.push(id.slot());
        }
    }

    fn remove(&mut self, id: RowId) -> Option<R> {
        let slot = id.slot() as usize;
        if self.generations.get(slot) != Some(&id.generation()) {
            return None;
        }
        let value = self.values.get_mut(slot)?.take()?;
        self.free.push(id.slot());
        Some(value)
    }

    fn get(&self, id: RowId) -> Option<&R> {
        let slot = id.slot() as usize;
        if self.generations.get(slot) != Some(&id.generation()) {
            return None;
        }
        self.values.get(slot)?.as_ref()
    }

    fn get_mut(&mut self, id: RowId) -> Option<&mut R> {
        let slot = id.slot() as usize;
        if self.generations.get(slot) != Some(&id.generation()) {
            return None;
        }
        self.values.get_mut(slot)?.as_mut()
    }
}

struct Draft<R: Row> {
    inserts: BTreeMap<RowId, R>,
    insert_keys: BTreeMap<R::Key, RowId>,
    inserted_order: Vec<RowId>,
    updates: BTreeMap<RowId, R>,
    removed: BTreeSet<RowId>,
    removed_order: Vec<RowId>,
    order: Option<Vec<RowId>>,
    reserved: Vec<RowId>,
}

impl<R: Row> Default for Draft<R> {
    fn default() -> Self {
        Self {
            inserts: BTreeMap::new(),
            insert_keys: BTreeMap::new(),
            inserted_order: Vec::new(),
            updates: BTreeMap::new(),
            removed: BTreeSet::new(),
            removed_order: Vec::new(),
            order: None,
            reserved: Vec::new(),
        }
    }
}

/// 一个 Store 的写事务视图；提交前所有修改都在草稿中。
pub struct Tx<'a, R: Row> {
    store: &'a Store<R>,
    draft: Draft<R>,
    finished: bool,
    plan_changes: ChangeSet,
    plan_keys: BTreeMap<RowId, R::Key>,
    plan_order: Vec<RowId>,
    plan_order_moved: bool,
    plan_membership: BTreeMap<FilterId, Vec<(RowId, bool)>>,
    plan_updates: BTreeMap<RowId, R>,
}

impl<R: Row> Tx<'_, R> {
    pub fn insert(&mut self, row: R) -> Result<RowId, StoreError> {
        let key = row.key();
        if self.draft.insert_keys.contains_key(&key) {
            return Err(StoreError::DuplicateKey);
        }
        {
            let data = self.store.inner.data.borrow();
            if let Some(existing) = data.by_key.get(&key)
                && !self.draft.removed.contains(existing)
            {
                return Err(StoreError::DuplicateKey);
            }
        }
        let id = self
            .store
            .inner
            .data
            .borrow_mut()
            .rows
            .reserve()
            .ok_or(StoreError::Exhausted)?;
        self.draft.insert_keys.insert(key, id);
        self.draft.inserts.insert(id, row);
        self.draft.inserted_order.push(id);
        self.draft.reserved.push(id);
        Ok(id)
    }

    pub fn update(&mut self, id: RowId, change: impl FnOnce(&mut R)) -> Result<(), StoreError> {
        let mut row = self.get(id).ok_or(StoreError::UnknownRow(id))?;
        let key = row.key();
        change(&mut row);
        // 业务键是 `by_key` 与草稿键索引的索引列；在任何分支写回前拒绝改键，
        // 否则同一事务里新建的行会绕过 `plan` 对已提交行的检查。
        if row.key() != key {
            return Err(StoreError::KeyChanged(id));
        }
        if let Some(staged) = self.draft.inserts.get_mut(&id) {
            *staged = row;
        } else {
            self.draft.updates.insert(id, row);
        }
        Ok(())
    }

    pub fn remove(&mut self, id: RowId) -> Result<(), StoreError> {
        if self.draft.removed.contains(&id) {
            return Ok(());
        }
        if let Some(row) = self.draft.inserts.remove(&id) {
            self.draft.insert_keys.remove(&row.key());
            self.draft
                .inserted_order
                .retain(|candidate| *candidate != id);
            if let Some(order) = &mut self.draft.order {
                order.retain(|candidate| *candidate != id);
            }
            self.release_reserved(id);
            return Ok(());
        }
        if self.store.get(id).is_none() {
            return Err(StoreError::UnknownRow(id));
        }
        self.draft.updates.remove(&id);
        self.draft.removed.insert(id);
        self.draft.removed_order.push(id);
        Ok(())
    }

    pub fn reorder(&mut self, order: &[RowId]) -> Result<(), StoreError> {
        let effective = self.effective_order();
        let live: BTreeSet<RowId> = effective.iter().copied().collect();
        let mut seen = BTreeSet::new();
        for id in order {
            if !seen.insert(*id) {
                return Err(StoreError::DuplicateRow(*id));
            }
            if !live.contains(id) {
                return Err(StoreError::UnknownRow(*id));
            }
        }
        if seen != live {
            return Err(StoreError::IncompleteOrder);
        }
        // 请求总是覆盖草稿顺序：与当前草稿视图相等时也不能清空草稿，因为
        // 事务中可能已经有一个更早的重排；清空会让提交退回已提交顺序。
        self.draft.order = Some(order.to_vec());
        Ok(())
    }

    /// 草稿视角的行快照：插入与更新可见，已删除行不可见。
    pub fn get(&self, id: RowId) -> Option<R> {
        if self.draft.removed.contains(&id) {
            return None;
        }
        if let Some(row) = self.draft.inserts.get(&id) {
            return Some(row.clone());
        }
        if let Some(row) = self.draft.updates.get(&id) {
            return Some(row.clone());
        }
        self.store.inner.data.borrow().rows.get(id).cloned()
    }

    pub fn key_of(&self, id: RowId) -> Option<R::Key> {
        if self.draft.removed.contains(&id) {
            return None;
        }
        if let Some(row) = self.draft.inserts.get(&id) {
            return Some(row.key());
        }
        if let Some(row) = self.draft.updates.get(&id) {
            return Some(row.key());
        }
        self.store.inner.data.borrow().rows.get(id).map(Row::key)
    }

    pub fn id_of_key(&self, key: R::Key) -> Option<RowId> {
        if let Some(id) = self.draft.insert_keys.get(&key) {
            return Some(*id);
        }
        let data = self.store.inner.data.borrow();
        let id = *data.by_key.get(&key)?;
        if self.draft.removed.contains(&id) {
            None
        } else {
            Some(id)
        }
    }

    pub fn contains(&self, id: RowId) -> bool {
        self.get(id).is_some()
    }

    /// 草稿视角的存活行身份，按当前顺序排列。
    pub fn ids(&self) -> Vec<RowId> {
        self.effective_order()
    }

    pub fn len(&self) -> usize {
        self.effective_order().len()
    }

    pub fn is_empty(&self) -> bool {
        self.effective_order().is_empty()
    }

    fn effective_order(&self) -> Vec<RowId> {
        let base = match &self.draft.order {
            Some(order) => order.clone(),
            None => self.store.inner.data.borrow().order.clone(),
        };
        let mut order: Vec<RowId> = base
            .into_iter()
            .filter(|id| !self.draft.removed.contains(id))
            .collect();
        let mut seen: BTreeSet<RowId> = order.iter().copied().collect();
        for id in &self.draft.inserted_order {
            if seen.insert(*id) {
                order.push(*id);
            }
        }
        order
    }

    fn release_reserved(&mut self, id: RowId) {
        if let Some(index) = self
            .draft
            .reserved
            .iter()
            .position(|candidate| *candidate == id)
        {
            self.draft.reserved.swap_remove(index);
        }
        if let Ok(mut data) = self.store.inner.data.try_borrow_mut() {
            data.rows.release(id);
        }
    }

    fn finish(&mut self) -> Result<Commit<R>, StoreError> {
        {
            let data = self.store.inner.data.borrow();
            self.plan(&data)?;
        }
        // 应用阶段不再运行用户代码，因此不会中途 panic；先标记完成，
        // Drop 不再把已填充的槽位当作预留归还。
        self.finished = true;
        self.draft.reserved.clear();
        self.apply();
        Ok(Commit {
            changes: std::mem::take(&mut self.plan_changes),
            keys: std::mem::take(&mut self.plan_keys),
        })
    }

    fn plan(&mut self, data: &StoreData<R>) -> Result<(), StoreError> {
        let mut final_order: Vec<RowId> = Vec::with_capacity(data.order.len());
        let mut seen: BTreeSet<RowId> = BTreeSet::new();
        let base = self.draft.order.as_deref().unwrap_or(&data.order);
        for id in base {
            if self.draft.removed.contains(id) {
                continue;
            }
            if !seen.insert(*id) {
                return Err(StoreError::DuplicateRow(*id));
            }
            final_order.push(*id);
        }
        for id in &self.draft.inserted_order {
            if seen.insert(*id) {
                final_order.push(*id);
            }
        }
        for id in &final_order {
            let live = self.draft.inserts.contains_key(id)
                || (!self.draft.removed.contains(id) && data.rows.get(*id).is_some());
            if !live {
                return Err(StoreError::UnknownRow(*id));
            }
        }
        let order_moved = data.order.as_slice() != final_order.as_slice();
        // `order` 只记录存活行相对顺序的变化：插入/删除由身份与成员迁移表达，
        // 虽然两者都会移动序列并推进 `order_revision`。事务内被抵消的重排，
        // 或随后被删除的移动行，都不改变存活行相对顺序。
        let inserted_ids: BTreeSet<RowId> = self.draft.inserted_order.iter().copied().collect();
        let surviving_before = data
            .order
            .iter()
            .copied()
            .filter(|id| !self.draft.removed.contains(id));
        let surviving_after = final_order
            .iter()
            .copied()
            .filter(|id| !inserted_ids.contains(id));
        let order_changed = !surviving_before.eq(surviving_after);

        let mut fields = BTreeSet::new();
        for (id, new_row) in &self.draft.updates {
            if self.draft.removed.contains(id) {
                continue;
            }
            let old = data.rows.get(*id).ok_or(StoreError::UnknownRow(*id))?;
            if old.key() != new_row.key() {
                return Err(StoreError::KeyChanged(*id));
            }
            for (field, def) in &data.fields {
                if def.differs(old, new_row) {
                    fields.insert((*id, *field));
                }
            }
        }

        let mut transitions: BTreeMap<FilterId, (Vec<RowId>, Vec<RowId>)> = BTreeMap::new();
        for id in &self.draft.inserted_order {
            let row = &self.draft.inserts[id];
            for (filter_id, entry) in &data.filters {
                if entry.spec.matches(row) {
                    transitions.entry(*filter_id).or_default().0.push(*id);
                }
            }
        }
        for id in &self.draft.removed_order {
            for (filter_id, entry) in &data.filters {
                if entry.members.contains(id) {
                    transitions.entry(*filter_id).or_default().1.push(*id);
                }
            }
        }
        // 字段变化通过反向索引找到候选 filter，并重新求值候选行本身；
        // 当前不在结果内的行因此也能进入 membership 迁移。
        let mut candidates: BTreeSet<(FilterId, RowId)> = BTreeSet::new();
        for (row, field) in &fields {
            if let Some(filters) = data.field_filters.get(field) {
                for filter in filters {
                    candidates.insert((*filter, *row));
                }
            }
        }
        for (filter, row) in candidates {
            let Some(entry) = data.filters.get(&filter) else {
                continue;
            };
            let Some(new_row) = self.draft.updates.get(&row) else {
                continue;
            };
            if self.draft.removed.contains(&row) {
                continue;
            }
            let wanted = entry.spec.matches(new_row);
            if wanted != entry.members.contains(&row) {
                let pair = transitions.entry(filter).or_default();
                if wanted {
                    pair.0.push(row);
                } else {
                    pair.1.push(row);
                }
            }
        }
        let mut membership = Vec::new();
        let mut membership_ops: BTreeMap<FilterId, Vec<(RowId, bool)>> = BTreeMap::new();
        for (filter, (adds, removes)) in transitions {
            if adds.is_empty() && removes.is_empty() {
                continue;
            }
            membership.push(filter);
            let mut ops: Vec<(RowId, bool)> = adds.into_iter().map(|id| (id, true)).collect();
            ops.extend(removes.into_iter().map(|id| (id, false)));
            membership_ops.insert(filter, ops);
        }

        let inserted: Vec<RowId> = self
            .draft
            .inserted_order
            .iter()
            .copied()
            .filter(|id| !self.draft.removed.contains(id))
            .collect();
        let removed = self.draft.removed_order.clone();
        let mut keys = BTreeMap::new();
        for id in &inserted {
            if let Some(row) = self.draft.inserts.get(id) {
                keys.insert(*id, row.key());
            }
        }
        for id in &removed {
            if let Some(row) = data.rows.get(*id) {
                keys.insert(*id, row.key());
            }
        }
        self.plan_changes = ChangeSet {
            inserted,
            removed,
            fields: fields.into_iter().collect(),
            membership,
            order: order_changed,
        };
        self.plan_keys = keys;
        self.plan_order_moved = order_moved;
        self.plan_membership = membership_ops;
        self.plan_updates = self
            .draft
            .updates
            .iter()
            .filter(|(id, _)| !self.draft.removed.contains(id))
            .map(|(id, row)| (*id, row.clone()))
            .collect();
        self.plan_order = final_order;
        Ok(())
    }

    fn apply(&mut self) {
        let mut data = self.store.inner.data.borrow_mut();
        for id in &self.plan_changes.removed {
            if let Some(row) = data.rows.remove(*id) {
                data.by_key.remove(&row.key());
            }
        }
        for (id, row) in std::mem::take(&mut self.plan_updates) {
            if let Some(slot) = data.rows.get_mut(id) {
                *slot = row;
            }
        }
        for id in &self.plan_changes.inserted {
            if let Some(row) = self.draft.inserts.remove(id) {
                data.by_key.insert(row.key(), *id);
                let filled = data.rows.fill(*id, row);
                debug_assert!(filled, "a reserved row slot must be empty");
            }
        }
        data.order = std::mem::take(&mut self.plan_order);
        for (filter, ops) in std::mem::take(&mut self.plan_membership) {
            if let Some(entry) = data.filters.get_mut(&filter) {
                for (id, add) in ops {
                    if add {
                        entry.members.insert(id);
                    } else {
                        entry.members.remove(&id);
                    }
                }
                entry.membership_revision = entry.membership_revision.wrapping_add(1);
            }
        }
        if self.plan_order_moved {
            data.order_revision = data.order_revision.wrapping_add(1);
        }
    }
}

impl<R: Row> Drop for Tx<'_, R> {
    fn drop(&mut self) {
        self.store.inner.active.set(false);
        if self.finished {
            return;
        }
        let reserved = std::mem::take(&mut self.draft.reserved);
        if let Ok(mut data) = self.store.inner.data.try_borrow_mut() {
            for id in reserved {
                data.rows.release(id);
            }
        }
    }
}
