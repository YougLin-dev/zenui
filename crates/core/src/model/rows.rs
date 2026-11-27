use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use super::id::{FilterId, RowId, StoreId};
use super::row::{FilterSpec, Row};
use super::store::{Store, StoreError};

/// 一个过滤投影的共享快照：来源 store 与按 Store 当前顺序排列的行身份。
///
/// `Arc` 共享让同一 revision 的消费者拿到同一份投影；[`RowSet`] 是它对外
/// 的 newtype，本类型不做 UI/绑定依赖。
#[derive(Clone, PartialEq, Eq)]
pub struct RowSetData {
    store: StoreId,
    ids: Box<[RowId]>,
}

impl RowSetData {
    pub(crate) fn new(store: StoreId, ids: Box<[RowId]>) -> Self {
        Self { store, ids }
    }

    pub const fn store(&self) -> StoreId {
        self.store
    }

    pub fn ids(&self) -> &[RowId] {
        &self.ids
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn shares_with(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl fmt::Debug for RowSetData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RowSetData")
            .field("store", &self.store)
            .field("ids", &self.ids)
            .finish()
    }
}

/// 一个 Store 拥有的行身份快照；`ids` 顺序恒为 Store 当前用户顺序。
///
/// 不是 `Arc<RowSetData>` 的类型别名：newtype 才能提供 [`RowSet::shares_with`]
/// 与顺序敏感的自定义 `PartialEq`，同时继续命中 `ReactiveValue` 的 blanket impl。
/// 只能由 [`super::Store::rowset`] 或 [`super::Store::rowset_from_filter`] 构造，
/// 因此集合总带有真实来源与合法顺序。
#[repr(transparent)]
#[derive(Clone)]
pub struct RowSet(Arc<RowSetData>);

impl RowSet {
    pub(crate) fn from_data(data: Arc<RowSetData>) -> Self {
        Self(data)
    }

    pub fn store(&self) -> StoreId {
        self.0.store()
    }

    pub fn ids(&self) -> &[RowId] {
        self.0.ids()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = RowId> + '_ {
        self.0.ids().iter().copied()
    }

    /// 同一份共享投影（`Arc` 指针相同）时返回真；O(1)。
    pub fn shares_with(&self, other: &Self) -> bool {
        self.0.shares_with(&other.0)
    }
}

/// 先比较共享身份，再比较来源 store 与**顺序**；纯重排因此改变相等性。
impl PartialEq for RowSet {
    fn eq(&self, other: &Self) -> bool {
        self.shares_with(other) || (self.store() == other.store() && self.ids() == other.ids())
    }
}

impl Eq for RowSet {}

impl fmt::Debug for RowSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RowSet")
            .field("store", &self.store())
            .field("ids", &self.ids())
            .finish()
    }
}

/// 一个 filter 在某个 Store 上的读投影。
///
/// 只借用 Store 的共享句柄；同一 `(membership_revision, order_revision)` 的
/// [`Rows::snapshot`] 返回共享的 `Arc<RowSetData>`，不会被复制。
pub struct Rows<R: Row, F: FilterSpec<R>> {
    store: Store<R>,
    filter: FilterId,
    _spec: PhantomData<fn() -> F>,
}

impl<R: Row, F: FilterSpec<R>> Rows<R, F> {
    pub(crate) fn new(store: Store<R>, filter: FilterId) -> Self {
        Self {
            store,
            filter,
            _spec: PhantomData,
        }
    }

    pub fn store(&self) -> Store<R> {
        self.store.clone()
    }

    pub const fn filter(&self) -> FilterId {
        self.filter
    }

    pub fn snapshot(&self) -> Result<Arc<RowSetData>, StoreError> {
        self.store.projection(self.filter)
    }

    pub fn ids(&self) -> Result<Vec<RowId>, StoreError> {
        Ok(self.snapshot()?.ids().to_vec())
    }

    pub fn len(&self) -> Result<usize, StoreError> {
        Ok(self.snapshot()?.len())
    }

    pub fn is_empty(&self) -> Result<bool, StoreError> {
        self.store.filter_is_empty(self.filter)
    }

    pub fn membership_revision(&self) -> Result<u64, StoreError> {
        self.store.membership_revision(self.filter)
    }

    pub fn order_revision(&self) -> u64 {
        self.store.order_revision()
    }
}

impl<R: Row, F: FilterSpec<R>> Clone for Rows<R, F> {
    fn clone(&self) -> Self {
        Self::new(self.store.clone(), self.filter)
    }
}

impl<R: Row, F: FilterSpec<R>> fmt::Debug for Rows<R, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Rows")
            .field("store", &self.store.id())
            .field("filter", &self.filter)
            .finish()
    }
}
