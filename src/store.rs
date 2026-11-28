use std::fmt;
use std::marker::PhantomData;

use zenui_core::model::{
    ChangeSet, Commit, FieldDef, FieldId, FilterId, FilterSpec, Row as ModelRow, RowId, RowSet,
    SchemaBuilder, Store as ModelStore, StoreError as ModelError, StoreId, TransactionError,
    Tx as ModelTx,
};
use zenui_core::{Fatal, Filter, RowKey, ValueError};

use crate::context::{DepKey, Phase, filter_slot, with_state, with_state_of};
use crate::node::FieldKey;
use crate::runtime::ModelFieldRead;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TodoId(RowKey);

impl TodoId {
    pub const fn raw(self) -> u64 {
        self.0.raw()
    }

    pub const fn from_key(key: RowKey) -> Self {
        Self(key)
    }
}

/// Todo 模型的字段标识；`ChangeSet.fields` 与过滤观察都用它。
pub(crate) const TITLE_FIELD: FieldId = FieldId::new(0);
pub(crate) const COMPLETED_FIELD: FieldId = FieldId::new(1);

pub(crate) const ALL_FILTER: FilterId = FilterId::new(0);
pub(crate) const ACTIVE_FILTER: FilterId = FilterId::new(1);
pub(crate) const COMPLETED_ROWS: FilterId = FilterId::new(2);

pub(crate) const fn filter_id(filter: Filter) -> FilterId {
    match filter {
        Filter::All => ALL_FILTER,
        Filter::Active => ACTIVE_FILTER,
        Filter::Completed => COMPLETED_ROWS,
    }
}

pub(crate) const fn filter_kind(filter: FilterId) -> Filter {
    match filter.index() {
        1 => Filter::Active,
        2 => Filter::Completed,
        _ => Filter::All,
    }
}

/// Todo 作为通用行模型的第一个实例化：行身份是 `RowId`，业务键是 `TodoId`。
#[derive(Clone, Debug)]
pub struct TodoRow {
    pub id: TodoId,
    pub title: String,
    pub completed: bool,
}

impl ModelRow for TodoRow {
    type Key = TodoId;

    fn key(&self) -> TodoId {
        self.id
    }
}

struct AllRows;

impl FilterSpec<TodoRow> for AllRows {
    fn id(&self) -> FilterId {
        ALL_FILTER
    }

    fn watched_fields(&self) -> &'static [FieldId] {
        &[]
    }

    fn matches(&self, _row: &TodoRow) -> bool {
        true
    }
}

struct ActiveRows;

impl FilterSpec<TodoRow> for ActiveRows {
    fn id(&self) -> FilterId {
        ACTIVE_FILTER
    }

    fn watched_fields(&self) -> &'static [FieldId] {
        &[COMPLETED_FIELD]
    }

    fn matches(&self, row: &TodoRow) -> bool {
        !row.completed
    }
}

struct CompletedRows;

impl FilterSpec<TodoRow> for CompletedRows {
    fn id(&self) -> FilterId {
        COMPLETED_ROWS
    }

    fn watched_fields(&self) -> &'static [FieldId] {
        &[COMPLETED_FIELD]
    }

    fn matches(&self, row: &TodoRow) -> bool {
        row.completed
    }
}

pub(crate) fn todo_schema() -> zenui_core::model::Schema<TodoRow> {
    SchemaBuilder::new()
        .field(FieldDef::comparing(TITLE_FIELD, |row: &TodoRow| &row.title))
        .field(FieldDef::comparing(COMPLETED_FIELD, |row: &TodoRow| {
            &row.completed
        }))
        .filter(AllRows)
        .filter(ActiveRows)
        .filter(CompletedRows)
        .build()
        .expect("the Todo schema is static and valid")
}

/// 共享的旧 `RowSet` 投影缓存；revision 对未变化时同一 `Arc` 直接复用。
pub(crate) struct CachedRowSet {
    pub(crate) membership: u64,
    pub(crate) order: u64,
    pub(crate) set: RowSet,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Store {
    runtime: u32,
    _not_send: PhantomData<*const ()>,
}

impl Store {
    pub fn new() -> Self {
        Self {
            runtime: with_state(|state| state.id),
            _not_send: PhantomData,
        }
    }

    pub fn rows(self, filter: Filter) -> RowSet {
        with_state_of(self.runtime, |state| state.store_rows(filter))
    }

    /// 当前过滤结果的业务键，按 Store 顺序。
    pub fn keys(self, filter: Filter) -> Vec<TodoId> {
        with_state_of(self.runtime, |state| state.store_keys(filter))
    }

    /// 行身份对应的业务键；行已删除时返回 `None`。
    pub fn key_of(self, row: RowId) -> Option<TodoId> {
        with_state_of(self.runtime, |state| state.todo_store.key_of(row))
    }

    /// 业务键当前对应的行身份；键不在 Store 中时返回 `None`。
    pub fn row_id(self, id: TodoId) -> Option<RowId> {
        with_state_of(self.runtime, |state| state.todo_store.id_of_key(id))
    }

    /// 运行时持有的模型 Store 句柄；只在 SDK 内部用于 `For` 的行解析。
    pub(crate) fn model_store(self) -> ModelStore<TodoRow> {
        with_state_of(self.runtime, |state| state.todo_store.clone())
    }

    pub fn is_empty(self, filter: Filter) -> bool {
        with_state_of(self.runtime, |state| state.store_is_empty(filter))
    }

    /// 业务键在构造时解析为行身份；删除并以同一标题重建后，旧句柄读到 `Missing`。
    pub fn title(self, id: TodoId) -> Field<String> {
        let (store, row) = with_state_of(self.runtime, |state| {
            (state.todo_store.id(), state.todo_store.id_of_key(id))
        });
        Field::from_key(FieldKey::new(
            self.runtime,
            store,
            row,
            TITLE_FIELD,
            Some(id),
        ))
    }

    pub fn completed(self, id: TodoId) -> Field<bool> {
        let (store, row) = with_state_of(self.runtime, |state| {
            (state.todo_store.id(), state.todo_store.id_of_key(id))
        });
        Field::from_key(FieldKey::new(
            self.runtime,
            store,
            row,
            COMPLETED_FIELD,
            Some(id),
        ))
    }

    pub fn insert(self, title: impl Into<String>) -> WriteOp<TodoId> {
        let title = title.into();
        WriteOp::new(self.runtime, move |tx, next_key, log| {
            let id = TodoId(RowKey::from_raw(*next_key));
            *next_key += 1;
            tx.insert(TodoRow {
                id,
                title: title.clone(),
                completed: false,
            })
            .map_err(WriteError::Model)?;
            log.wrote();
            log.wrote_field(id, TITLE_FIELD);
            log.wrote_field(id, COMPLETED_FIELD);
            Ok(id)
        })
    }

    pub fn set_completed(self, id: TodoId, done: bool) -> WriteOp<()> {
        WriteOp::new(self.runtime, move |tx, _next_key, log| {
            let row = tx.id_of_key(id).ok_or(WriteError::Missing(id))?;
            log.wrote_field(id, COMPLETED_FIELD);
            let current = tx.get(row).ok_or(WriteError::Missing(id))?;
            if current.completed == done {
                return Ok(());
            }
            log.wrote();
            tx.update(row, |row| row.completed = done)
                .map_err(WriteError::Model)
        })
    }

    pub fn set_title(self, id: TodoId, title: impl Into<String>) -> WriteOp<()> {
        let title = title.into();
        WriteOp::new(self.runtime, move |tx, _next_key, log| {
            let row = tx.id_of_key(id).ok_or(WriteError::Missing(id))?;
            log.wrote_field(id, TITLE_FIELD);
            let current = tx.get(row).ok_or(WriteError::Missing(id))?;
            if current.title == title {
                return Ok(());
            }
            log.wrote();
            tx.update(row, |row| row.title = title.clone())
                .map_err(WriteError::Model)
        })
    }

    pub fn remove(self, id: TodoId) -> WriteOp<()> {
        WriteOp::new(self.runtime, move |tx, _next_key, log| {
            let row = tx.id_of_key(id).ok_or(WriteError::Missing(id))?;
            log.wrote_field(id, TITLE_FIELD);
            log.wrote_field(id, COMPLETED_FIELD);
            if tx.contains(row) {
                log.wrote();
            }
            tx.remove(row).map_err(WriteError::Model)
        })
    }

    pub fn clear_completed(self) -> WriteOp<()> {
        WriteOp::new(self.runtime, move |tx, _next_key, log| {
            let mut failure = None;
            let mut removed = 0;
            for row in tx.ids() {
                if !tx.get(row).is_some_and(|row| row.completed) {
                    continue;
                }
                if let Some(key) = tx.key_of(row) {
                    log.wrote_field(key, TITLE_FIELD);
                    log.wrote_field(key, COMPLETED_FIELD);
                }
                match tx.remove(row) {
                    Ok(()) => removed += 1,
                    Err(error) if failure.is_none() => failure = Some(error),
                    Err(_) => {}
                }
            }
            if removed > 0 {
                log.wrote();
            }
            match failure {
                Some(error) => Err(WriteError::Model(error)),
                None => Ok(()),
            }
        })
    }

    pub fn reorder(self, ids: &[TodoId]) -> WriteOp<()> {
        let ids = ids.to_vec();
        WriteOp::new(self.runtime, move |tx, _next_key, log| {
            let mut rows = Vec::with_capacity(ids.len());
            for id in &ids {
                rows.push(tx.id_of_key(*id).ok_or(WriteError::Missing(*id))?);
            }
            let before = tx.ids();
            tx.reorder(&rows).map_err(WriteError::Model)?;
            if before != rows {
                log.wrote();
            }
            Ok(())
        })
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

/// Todo 的 `For` 入口：`each` 读源与应用 Store 组合成 `ForSource<TodoRow>`，
/// 模型 Store 句柄不经公开 API 泄漏。
impl crate::view::ForStore for Store {
    type Row = TodoRow;

    fn for_source(
        &self,
        rows: impl Into<crate::view::Read<RowSet>>,
    ) -> crate::view::ForSource<TodoRow> {
        crate::view::ForSource::new(self.model_store(), rows)
    }
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store")
            .field("runtime", &self.runtime)
            .finish()
    }
}

/// typed、类型擦除的字段句柄：runtime + `FieldKey`。
///
/// `FieldKey` 用 `StoreId + RowId + FieldId` 标识来源；行身份自带代际，因此
/// 删除并以同一业务键重建后，旧句柄继续读到 `Missing`，不会命中新行。
pub struct Field<T> {
    key: FieldKey,
    _marker: PhantomData<fn() -> T>,
    _not_send: PhantomData<*const ()>,
}

impl<T> Field<T> {
    /// 绑定槽只保存字段键；求值时按键还原同一句柄。
    pub(crate) const fn from_key(key: FieldKey) -> Self {
        Self {
            key,
            _marker: PhantomData,
            _not_send: PhantomData,
        }
    }

    /// 绑定槽的字段键；值类型由持有槽的类型决定。
    pub(crate) const fn field_key(self) -> FieldKey {
        self.key
    }
}

impl<T> Field<T>
where
    T: crate::signal::ReactiveValue,
{
    /// 读取字段当前值并注册依赖；行已删除时返回 typed `FieldError`。
    pub fn get(self) -> Result<T, FieldError> {
        self.read_value(false)
    }

    /// setup 阶段的一次性读取：不注册依赖。
    pub fn read_once(self) -> Result<T, FieldError> {
        self.read_value(true)
    }

    /// 类型擦除读取：字段值的类型由构造字段的 accessor 决定。
    pub(crate) fn read_value(self, once: bool) -> Result<T, FieldError> {
        let value = with_state_of(self.key.runtime, |state| state.read_field(self.key, once))?;
        Ok(*value
            .downcast::<T>()
            .expect("field value type matches its accessor"))
    }
}

impl<T> Copy for Field<T> {}
impl<T> Clone for Field<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> fmt::Debug for Field<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Field").field("key", &self.key).finish()
    }
}

/// 通用行 Store 的响应式字段入口；`RowView::field` 与直接持有 Store 的代码共用。
pub trait RowStoreField<R: ModelRow> {
    /// 按行身份与已注册字段构造句柄；读取时经 runtime 注册的类型擦除读取器。
    fn field<T: crate::signal::ReactiveValue>(&self, row: RowId, field: FieldId) -> Field<T>;
}

impl<R: ModelRow> RowStoreField<R> for ModelStore<R> {
    fn field<T: crate::signal::ReactiveValue>(&self, row: RowId, field: FieldId) -> Field<T> {
        let (runtime, store, todo) = with_state(|state| {
            let store = self.id();
            state.register_store_reader(store, crate::runtime::StoreReader::of(self.clone()));
            // Todo 行经 `RowView::field` 与 `Store::title` 必须得到同一个键，
            // 否则两条 accessor 路径的写入失效互不相通。
            let todo = if store == state.todo_store.id() {
                state.todo_store.key_of(row)
            } else {
                None
            };
            (state.id, store, todo)
        });
        // 直接持有 Store 的调用方没有 `For` 的 attach 路径；字段句柄本身
        // 订阅提交，写入才能进入 PendingChanges 并推进字段代数。
        crate::runtime::observe_store_commits(self);
        Field::from_key(FieldKey::new(runtime, store, Some(row), field, todo))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldError {
    /// Todo accessor 的业务键已不存在。
    Missing(TodoId),
    /// 通用行身份已删除或代际失效。
    MissingRow(RowId),
    /// 该 Store 的 schema 未注册这个字段。
    UnknownField { store: StoreId, field: FieldId },
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(id) => write!(f, "row {} is no longer live", id.raw()),
            Self::MissingRow(row) => write!(f, "row {row:?} is no longer live"),
            Self::UnknownField { store, field } => {
                write!(f, "store {store:?} has no field {}", field.index())
            }
        }
    }
}

impl std::error::Error for FieldError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WriteError {
    Fatal(Fatal),
    Rejected(ValueError),
    Missing(TodoId),
    /// 通用行 Store 拒绝的写事务（重复键、键变更、非法顺序等）。
    Model(ModelError),
    Context {
        operation: &'static str,
        phase: Phase,
    },
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fatal(error) => write!(f, "{error}"),
            Self::Rejected(error) => write!(f, "{error}"),
            Self::Missing(id) => write!(f, "row {} is no longer live", id.raw()),
            Self::Model(error) => write!(f, "{error}"),
            Self::Context { operation, phase } => {
                write!(f, "{operation} is not allowed during {phase:?}")
            }
        }
    }
}

impl std::error::Error for WriteError {}

impl From<Fatal> for WriteError {
    fn from(error: Fatal) -> Self {
        Self::Fatal(error)
    }
}

impl From<ValueError> for WriteError {
    fn from(error: ValueError) -> Self {
        Self::Rejected(error)
    }
}

/// 写事务里 SDK 层记录的写尝试。
///
/// `writes` 复刻旧 `journal.writes`：实际改变状态的操作各计一次
/// （`clear_completed` 整批一次）。`fields` 记录应用对字段的每次写入尝试，
/// 相等写入也算——文本编辑器据此判断处理器是否处理过这次编辑。
#[derive(Default)]
struct WriteLog {
    writes: u64,
    fields: Vec<(TodoId, FieldId)>,
}

impl WriteLog {
    fn wrote(&mut self) {
        self.writes += 1;
    }

    fn wrote_field(&mut self, id: TodoId, field: FieldId) {
        self.fields.push((id, field));
    }
}

type WriteResult<T> = Result<T, WriteError>;
type WriteAction<T> =
    Box<dyn FnOnce(&mut ModelTx<'_, TodoRow>, &mut u64, &mut WriteLog) -> WriteResult<T>>;
type Op =
    Box<dyn FnOnce(&mut ModelTx<'_, TodoRow>, &mut u64, &mut WriteLog) -> Result<(), WriteError>>;

pub struct WriteOp<T> {
    runtime: u32,
    action: WriteAction<T>,
}

impl<T> WriteOp<T> {
    fn new(
        runtime: u32,
        action: impl FnOnce(&mut ModelTx<'_, TodoRow>, &mut u64, &mut WriteLog) -> WriteResult<T>
        + 'static,
    ) -> Self {
        Self {
            runtime,
            action: Box::new(action),
        }
    }
}

#[derive(Default)]
pub struct Transaction {
    runtime: Option<u32>,
    ops: Vec<Op>,
}

impl Transaction {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write<T: 'static>(mut self, op: WriteOp<T>) -> Self {
        match self.runtime {
            Some(runtime) => assert_eq!(
                runtime, op.runtime,
                "one Transaction cannot mix different Runtimes"
            ),
            None => self.runtime = Some(op.runtime),
        }
        self.ops.push(Box::new(move |tx, next_key, log| {
            (op.action)(tx, next_key, log).map(|_| ())
        }));
        self
    }

    pub fn finish(self) -> Result<(), WriteError> {
        match self.runtime {
            Some(runtime) => with_state_of(runtime, |state| state.finish_transaction(self.ops)),
            None => Ok(()),
        }
    }
}

impl crate::runtime::State {
    /// 读取共享的已提交 RowSet；同一 (membership, order) revision 复用同一份投影。
    pub(crate) fn store_rows(&mut self, filter: Filter) -> RowSet {
        let id = filter_id(filter);
        let slot = filter_slot(filter) as usize;
        let membership = self
            .todo_store
            .membership_revision(id)
            .expect("the Todo filter is registered");
        let order = self.todo_store.order_revision();
        if let Some(cached) = &self.rowset_cache[slot]
            && cached.membership == membership
            && cached.order == order
        {
            let rows = cached.set.clone();
            if self.phase == Phase::Memo {
                self.note_dep(DepKey::Rows(filter_slot(filter)));
            }
            return rows;
        }
        self.store_counters.query_evals += 1;
        // 旧 QueryCache 对空过滤不扫描，非空过滤扫描全部存活行；
        // `row_visits` 保持“本次重建访问的行数”，而不是结果行数。
        let empty = self
            .todo_store
            .filter_is_empty(id)
            .expect("the Todo filter is registered");
        let set = self
            .todo_store
            .rowset_from_filter(id)
            .expect("the Todo filter is registered");
        if !empty {
            self.store_counters.cache_builds += 1;
            self.store_counters.id_materializations += 1;
            self.store_counters.row_visits += self.todo_store.len() as u64;
        }
        self.rowset_cache[slot] = Some(CachedRowSet {
            membership,
            order,
            set: set.clone(),
        });
        if self.phase == Phase::Memo {
            self.note_dep(DepKey::Rows(filter_slot(filter)));
        }
        set
    }

    pub(crate) fn store_keys(&mut self, filter: Filter) -> Vec<TodoId> {
        self.store_rows(filter)
            .ids()
            .iter()
            .filter_map(|row| self.todo_store.key_of(*row))
            .collect()
    }

    pub(crate) fn store_is_empty(&mut self, filter: Filter) -> bool {
        let empty = self
            .todo_store
            .filter_is_empty(filter_id(filter))
            .expect("the Todo filter is registered");
        if self.phase == Phase::Memo {
            self.note_dep(DepKey::Rows(filter_slot(filter)));
        }
        empty
    }

    /// 一次字段读取的唯一实现；返回类型擦除值，由 `Field<T>` 还原。
    ///
    /// 读取通过 StoreId 对应的已注册读取器完成；Todo accessor 的旧句柄
    /// 仍返回 `FieldError::Missing(TodoId)`，通用行返回 `MissingRow(RowId)`。
    pub(crate) fn read_field(
        &mut self,
        key: FieldKey,
        once: bool,
    ) -> Result<Box<dyn std::any::Any>, FieldError> {
        if !once && self.phase == Phase::Memo {
            self.note_dep(DepKey::Field(key));
        }
        if once {
            self.once_reads += 1;
        }
        let Some(row) = key.row else {
            return Err(match key.todo {
                Some(todo) => FieldError::Missing(todo),
                None => FieldError::UnknownField {
                    store: key.store,
                    field: key.field,
                },
            });
        };
        match self.read_model_field(key.store, row, key.field) {
            ModelFieldRead::Value(value) => Ok(value),
            ModelFieldRead::MissingRow => Err(match key.todo {
                Some(todo) => FieldError::Missing(todo),
                None => FieldError::MissingRow(row),
            }),
            ModelFieldRead::UnknownField => Err(FieldError::UnknownField {
                store: key.store,
                field: key.field,
            }),
        }
    }

    /// 事务只在 Event/Delivery turn 中执行；失败不改变模型，但会记录到命令边界，
    /// 即使调用方忽略返回值，dispatch 仍会报告这个错误。
    fn finish_transaction(&mut self, ops: Vec<Op>) -> Result<(), WriteError> {
        if !matches!(self.phase, Phase::Event | Phase::Delivery)
            || self.read_only > 0
            || self.disposing > 0
        {
            let error = WriteError::Context {
                operation: "Transaction::finish",
                phase: self.phase,
            };
            self.write_error = Some(error);
            return Err(error);
        }

        let store = self.todo_store.clone();
        let mut next_key = self.next_todo_key;
        let mut log = WriteLog::default();
        let outcome = store.transaction(|tx| {
            for op in ops {
                op(tx, &mut next_key, &mut log)?;
            }
            Ok(())
        });
        match outcome {
            Ok(((), commit)) => {
                self.next_todo_key = next_key;
                // 相等写入不脏任何依赖，但仍是应用对字段的处理；
                // 文本编辑器用尝试代数区分“写了旧值”和“没有写”。
                for (id, field) in log.fields {
                    if let Some(row) = self.todo_store.id_of_key(id) {
                        self.note_field_write(self.todo_store.id(), row, field);
                    }
                }
                self.store_counters.commits += 1;
                self.store_counters.io_writes += log.writes;
                self.apply_commit(&commit);
                Ok(())
            }
            Err(TransactionError::Aborted(error)) => {
                self.write_error = Some(error);
                Err(error)
            }
            Err(TransactionError::Store(error)) => {
                let error = WriteError::Model(error);
                self.write_error = Some(error);
                Err(error)
            }
        }
    }

    /// 把通用 ChangeSet 映射为现有的响应式依赖代数。
    ///
    /// 字段变化精确到 `(行, 字段)`；成员迁移只推进真正变化的 filter。插入/
    /// 删除沿用旧的粗粒度行为（三个 filter 都标脏），让既有 memo 调度保持
    /// 不变；精确的成员列表仍保留在 ChangeSet 里供后续阶段使用。
    fn apply_commit(&mut self, commit: &Commit<TodoRow>) {
        let changes: &ChangeSet = commit.changes();
        if changes.is_empty() {
            return;
        }
        let runtime = self.id;
        let store = self.todo_store.id();
        let mut coarse_rows = false;
        for row in &changes.inserted {
            coarse_rows = true;
            let Some(key) = commit.key_of(*row) else {
                continue;
            };
            self.bump(DepKey::Field(FieldKey::new(
                runtime,
                store,
                Some(*row),
                TITLE_FIELD,
                Some(key),
            )));
            self.bump(DepKey::Field(FieldKey::new(
                runtime,
                store,
                Some(*row),
                COMPLETED_FIELD,
                Some(key),
            )));
        }
        let mut purged = Vec::new();
        for row in &changes.removed {
            coarse_rows = true;
            let Some(key) = commit.key_of(*row) else {
                continue;
            };
            // 先标脏消费者，再丢弃已删除行的写入历史：绑定本帧仍会求值
            // 并读到 FieldMissing，历史表不会随删除行数增长。
            self.bump(DepKey::Field(FieldKey::new(
                runtime,
                store,
                Some(*row),
                TITLE_FIELD,
                Some(key),
            )));
            self.bump(DepKey::Field(FieldKey::new(
                runtime,
                store,
                Some(*row),
                COMPLETED_FIELD,
                Some(key),
            )));
            purged.push(*row);
        }
        self.purge_row_history(&purged);
        for (row, field) in &changes.fields {
            if let Some(key) = self.todo_store.key_of(*row) {
                self.bump(DepKey::Field(FieldKey::new(
                    runtime,
                    store,
                    Some(*row),
                    *field,
                    Some(key),
                )));
            }
        }
        if coarse_rows || changes.order {
            for filter in [Filter::All, Filter::Active, Filter::Completed] {
                self.bump(DepKey::Rows(filter_slot(filter)));
            }
        } else {
            for filter in &changes.membership {
                self.bump(DepKey::Rows(filter_slot(filter_kind(*filter))));
            }
        }
    }
}
