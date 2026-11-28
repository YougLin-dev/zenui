use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::Hasher;
use std::rc::Rc;

use zenui_core::PanicInfo;
use zenui_core::model::{
    FilterId, FilterSpec, Row, RowId, RowKey, RowSet, Rows, Store as ModelStore, StoreError,
    StoreId,
};

use zenui_style::{Color, Length, Opacity, Property, StyleDeclaration};

use crate::context::Phase;
use crate::node::{BindingRef, BindingTable, BindingValue, ReadSlot};
use crate::signal::{Memo, Once, ReactiveValue, Signal, once};
use crate::store::{Field, FieldError, WriteError};

pub type MountResult<T> = Result<T, MountError>;

#[cfg(test)]
thread_local! {
    /// 测试用：本轮动态绑定求值次数；用于证明同一 binding 每轮最多求值一次。
    static BINDING_EVALUATIONS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn take_binding_evaluations() -> u64 {
    BINDING_EVALUATIONS.with(|count| count.replace(0))
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MountError {
    MissingProp(&'static str),
    Context {
        operation: &'static str,
        phase: Phase,
    },
    Field(FieldError),
    /// 组件 setup 的 `Read::read_once()?` 读取失败。
    Read(ReadFault),
    DuplicateKey,
    Write(WriteError),
    /// Store 拒绝的读取或投影。
    Store(StoreError),
    /// 行集来自另一个 Store；`ForSource` 的 store 与它不匹配。
    WrongStore {
        expected: StoreId,
        found: StoreId,
    },
    /// 行 factory 返回 `Err`；拟议行已释放，旧行映射保留。
    FactoryFailed(Box<MountError>),
    /// 行 factory panic；拟议行已释放，旧行映射保留。
    FactoryPanicked(PanicInfo),
    Delivery(&'static str),
    Component(&'static str),
    Setup(&'static str),
    /// 用户闭包 panic 已被 contain；运行时状态已恢复，调用方可继续。
    ///
    /// containment 依赖 `panic = "unwind"` 构建。
    Panic(PanicInfo),
}

impl MountError {
    pub const fn missing_prop(name: &'static str) -> Self {
        Self::MissingProp(name)
    }
}

impl fmt::Display for MountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingProp(name) => write!(f, "missing component prop `{name}`"),
            Self::Context { operation, phase } => {
                write!(f, "{operation} is not allowed during {phase:?}")
            }
            Self::Field(error) => write!(f, "{error}"),
            Self::Read(fault) => write!(f, "{fault}"),
            Self::DuplicateKey => f.write_str("duplicate For key before interface publication"),
            Self::Write(error) => write!(f, "{error}"),
            Self::Store(error) => write!(f, "{error}"),
            Self::WrongStore { expected, found } => write!(
                f,
                "For rows belong to store {found:?}, expected {expected:?}"
            ),
            Self::FactoryFailed(error) => write!(f, "For row factory failed: {error}"),
            Self::FactoryPanicked(panic) => write!(f, "For row factory panicked: {panic}"),
            Self::Delivery(message) => f.write_str(message),
            Self::Component(label) => write!(f, "component {label} rejected its mount"),
            Self::Setup(message) => f.write_str(message),
            Self::Panic(panic) => write!(f, "{panic}"),
        }
    }
}

impl std::error::Error for MountError {}

impl From<FieldError> for MountError {
    fn from(error: FieldError) -> Self {
        Self::Field(error)
    }
}

impl From<ReadFault> for MountError {
    fn from(fault: ReadFault) -> Self {
        Self::Read(fault)
    }
}

impl From<WriteError> for MountError {
    fn from(error: WriteError) -> Self {
        Self::Write(error)
    }
}

impl From<StoreError> for MountError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

pub struct MountPlan {
    pub(crate) nodes: Vec<PlanNode>,
}

impl MountPlan {
    pub fn new(nodes: Vec<PlanNode>) -> Self {
        Self { nodes }
    }
}

pub enum PlanNode {
    Text(Read<String>),
    Children(Children),
    Factory(ChildrenFactory),
    Element(ElementPlan),
    Component(ComponentPlan),
    Show(ShowPlan),
    For(ForPlan),
}

pub struct ElementPlan {
    pub(crate) tag: &'static str,
    pub(crate) attrs: Vec<Attr>,
    pub(crate) class_styles: &'static [StyleDeclaration],
    pub(crate) inline_styles: Vec<InlineStyle>,
    pub(crate) children: Vec<PlanNode>,
}

/// 数据位置与组件 prop 的统一读源。
///
/// `Static` 只由字面量构造；`Once` 是显式挂载快照；`Signal`/`Memo` 是可观察
/// 句柄；`Field` 的读取可能失败。裸值不会自动变成 `Once`，因此“写了值却永远
/// 不更新”不能靠隐式转换通过编译。
#[derive(Clone, Debug)]
pub enum Read<T> {
    /// 字面量或 `&'static str` 静态值；挂载时写入绑定当前值，没有依赖。
    ///
    /// 条件选择的文本也属于静态值；需要随后更新时写 `once(...)`、signal 或
    /// memo，否则它会永远停留在挂载时的值。
    Static(T),
    /// 显式一次性快照；保留来源诊断，不注册依赖。
    Once(Once<T>),
    Signal(Signal<T>),
    Memo(Memo<T>),
    /// 行字段；读取可能返回 [`ReadFault::FieldMissing`]。
    Field(Field<T>),
}

/// 一次读取的结果；失败与最后已提交值并列，不把失败伪装成默认值。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ReadValue<T> {
    Value(T),
    Fault(ReadFault),
}

/// 数据位置读取失败的 typed 诊断。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ReadFault {
    /// 行已经不存在；绑定保留最后一次成功值。
    FieldMissing(FieldError),
    /// memo 计算 panic 且没有最后已提交值。
    ComputedPanicked,
    /// memo 读取形成环。
    GraphCycle,
    /// 资源随 owner 释放后仍被读取。
    Disposed,
}

impl fmt::Display for ReadFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FieldMissing(error) => write!(f, "field read failed: {error}"),
            Self::ComputedPanicked => f.write_str("memo compute panicked"),
            Self::GraphCycle => f.write_str("memo graph cycle"),
            Self::Disposed => f.write_str("reactive source was disposed"),
        }
    }
}

impl std::error::Error for ReadFault {}

impl From<FieldError> for ReadFault {
    fn from(error: FieldError) -> Self {
        Self::FieldMissing(error)
    }
}

/// 行字段的类型化读取入口；`Field<T>` 目前覆盖文本与布尔字段。
pub(crate) trait FieldSource<T>: Copy {
    /// 在绑定/刷新阶段读取；在 memo 计算中注册字段依赖。
    fn read_field(self) -> Result<T, FieldError>;
    /// 组件 setup 的一次性读取；不注册依赖。
    fn read_field_once(self) -> Result<T, FieldError>;
}

impl<T> FieldSource<T> for Field<T>
where
    T: ReactiveValue,
{
    fn read_field(self) -> Result<T, FieldError> {
        self.read_value(false)
    }

    fn read_field_once(self) -> Result<T, FieldError> {
        self.read_value(true)
    }
}

impl<T> From<Once<T>> for Read<T>
where
    T: ReactiveValue,
{
    fn from(value: Once<T>) -> Self {
        Self::Once(value)
    }
}

impl<T> From<Signal<T>> for Read<T>
where
    T: ReactiveValue,
{
    fn from(value: Signal<T>) -> Self {
        Self::Signal(value)
    }
}

impl<T> From<Memo<T>> for Read<T>
where
    T: ReactiveValue,
{
    fn from(value: Memo<T>) -> Self {
        Self::Memo(value)
    }
}

impl<T: ReactiveValue> From<Field<T>> for Read<T> {
    fn from(value: Field<T>) -> Self {
        Self::Field(value)
    }
}

impl From<&'static str> for Read<String> {
    fn from(value: &'static str) -> Self {
        Self::Static(value.to_owned())
    }
}

impl From<Once<&'static str>> for Read<String> {
    fn from(value: Once<&'static str>) -> Self {
        Self::Once(crate::signal::once(value.into_inner().to_owned()))
    }
}

/// 一个 `RowSet` 是 Store 构造的不可变快照，因此可以显式成为读源；
/// 裸 `RowSet` 仍不会自动出现在 `each` 位置——那需要同时给出 `store`。
impl From<RowSet> for Read<RowSet> {
    fn from(value: RowSet) -> Self {
        Self::Once(once(value))
    }
}

impl<T: ReactiveValue> Read<T> {
    /// 组件 setup 的一次性读取（`Field` 走 `read_once`，不注册依赖）。
    ///
    /// 失败实现 `From<ReadFault> for MountError`，因此组件里可以直接写
    /// `read.read_once()?` 把 typed fault 交给挂载错误通道。
    pub fn read_once(self) -> Result<T, ReadFault> {
        match self {
            Self::Static(value) => Ok(value),
            Self::Once(value) => Ok(value.into_inner()),
            Self::Signal(signal) => signal.try_get_once(),
            Self::Memo(memo) => memo.try_get_once(),
            Self::Field(field) => field.read_field_once().map_err(ReadFault::FieldMissing),
        }
    }

    /// [`Read::read_once`] 的别名，强调读取可能失败。
    pub fn try_get(self) -> Result<T, ReadFault> {
        self.read_once()
    }

    /// 挂载期把公开读源转成绑定槽。
    ///
    /// `Static`/`Once` 的值在这里写入 `BindingSlot.current`，之后不再求值；
    /// 动态源只保存带 generation 的引用。静态值没有依赖，也不注册 `DepKey`。
    pub(crate) fn into_binding(self, table: &mut BindingTable) -> BindingRef
    where
        T: BindingValue,
    {
        match self {
            Self::Static(value) => table.insert::<T>(ReadSlot::Static, Some(value), None),
            Self::Once(value) => {
                let origin = table.once_origin();
                table.insert::<T>(ReadSlot::Once(origin), Some(value.into_inner()), None)
            }
            Self::Signal(signal) => {
                table.insert::<T>(ReadSlot::Signal(signal.source()), None, None)
            }
            Self::Memo(memo) => table.insert::<T>(ReadSlot::Memo(memo.source()), None, None),
            Self::Field(field) => table.insert::<T>(ReadSlot::Field(field.field_key()), None, None),
        }
    }
}

impl ReadSlot {
    /// 求值一个动态绑定源；`Static`/`Once` 的值只在挂载时写入，调用方跳过。
    ///
    /// `Field` 缺失与已释放的源返回 typed fault；memo 计算 panic 仍按
    /// `Memo::get` 的既有契约向上传播，由刷新边界 contain。
    pub(crate) fn evaluate<T>(self) -> ReadValue<T>
    where
        T: BindingValue,
    {
        #[cfg(test)]
        BINDING_EVALUATIONS.with(|count| count.set(count.get() + 1));
        match self {
            Self::Static | Self::Once(_) => {
                unreachable!("a static binding slot is written once at mount")
            }
            Self::Signal(source) => match Signal::<T>::from_source(source).try_get_once() {
                Ok(value) => ReadValue::Value(value),
                Err(fault) => ReadValue::Fault(fault),
            },
            Self::Memo(source) => match Memo::<T>::from_source(source).try_get_once() {
                Ok(value) => ReadValue::Value(value),
                Err(ReadFault::Disposed) => ReadValue::Fault(ReadFault::Disposed),
                Err(_) => ReadValue::Value(Memo::<T>::from_source(source).get()),
            },
            Self::Field(key) => match Field::<T>::from_key(key).read_field() {
                Ok(value) => ReadValue::Value(value),
                Err(error) => ReadValue::Fault(ReadFault::FieldMissing(error)),
            },
        }
    }

    /// 值绑定的写入尝试代数；静态来源没有写入路径。
    ///
    /// Signal/Field 的相等写入也推进它，文本编辑器据此辨认“处理器处理过
    /// 这次编辑”；普通绑定用 [`ReadSlot::change_epoch`] 跳过相等写入。
    pub(crate) fn epoch(self, state: &crate::runtime::State) -> Option<u64> {
        match self {
            Self::Static | Self::Once(_) => None,
            Self::Signal(source) => Some(state.source_write_epoch(source)),
            Self::Memo(source) => Some(state.source_memo_revision(source)),
            Self::Field(key) => Some(state.field_write_epoch(key.id, key.kind)),
        }
    }

    /// 来源 memo 是否仍有未结算的脏状态。
    ///
    /// 值变化代数只在 memo 完成计算后推进；dirty 但尚未结算的 memo 必须
    /// 通过绑定求值触发的读取路径补救，不能按代数相等跳过。
    pub(crate) fn is_dirty(self, state: &crate::runtime::State) -> bool {
        let Self::Memo(source) = self else {
            return false;
        };
        state
            .memos
            .get(source.id as usize)
            .is_none_or(|cell| cell.dirty || cell.computing)
            || state.memo_deps_unsettled(source.id)
    }

    /// 来源是否已经释放；释放不推进值变化代数，跳过求值前必须单独检查。
    pub(crate) fn is_disposed(self, state: &crate::runtime::State) -> bool {
        match self {
            Self::Static | Self::Once(_) => false,
            Self::Signal(source) => state
                .signals
                .get(source.id as usize)
                .is_some_and(|cell| cell.disposed),
            Self::Memo(source) => state
                .memos
                .get(source.id as usize)
                .is_some_and(|cell| cell.disposed),
            Self::Field(_) => false,
        }
    }

    /// 值变化代数：只有值真的改变才推进。
    ///
    /// 绑定刷新用它判断能否跳过求值；memo 的相等重算不推进 revision，
    /// 因此下游绑定也不需要重算。
    pub(crate) fn change_epoch(self, state: &crate::runtime::State) -> Option<u64> {
        match self {
            Self::Static | Self::Once(_) => None,
            Self::Signal(source) => Some(
                state
                    .write_epochs
                    .get(&crate::context::DepKey::Signal(source.id))
                    .copied()
                    .unwrap_or(0),
            ),
            Self::Memo(source) => Some(state.source_memo_revision(source)),
            Self::Field(key) => Some(
                state
                    .write_epochs
                    .get(&crate::context::DepKey::Field(key.id, key.kind))
                    .copied()
                    .unwrap_or(0),
            ),
        }
    }
}

/// 一条 inline `style:` 声明的类型化读源。
pub enum StyleSlot {
    Color(Read<Color>),
    Length(Read<Length>),
    Opacity(Read<Opacity>),
}

/// `ElementPlan` 里的一条 inline 声明。
pub struct InlineStyle {
    pub(crate) property: Property,
    pub(crate) slot: StyleSlot,
}

/// `style:background` 的类型化值。
pub fn style_background(value: impl Into<Read<Color>>) -> InlineStyle {
    InlineStyle {
        property: Property::Background,
        slot: StyleSlot::Color(value.into()),
    }
}

/// `style:color` 的类型化值。
pub fn style_color(value: impl Into<Read<Color>>) -> InlineStyle {
    InlineStyle {
        property: Property::TextColor,
        slot: StyleSlot::Color(value.into()),
    }
}

/// `style:border-color` 的类型化值。
pub fn style_border_color(value: impl Into<Read<Color>>) -> InlineStyle {
    InlineStyle {
        property: Property::BorderColor,
        slot: StyleSlot::Color(value.into()),
    }
}

/// `style:opacity` 的类型化值。
pub fn style_opacity(value: impl Into<Read<Opacity>>) -> InlineStyle {
    InlineStyle {
        property: Property::Opacity,
        slot: StyleSlot::Opacity(value.into()),
    }
}

/// `style:width` 的类型化值。
pub fn style_width(value: impl Into<Read<Length>>) -> InlineStyle {
    InlineStyle {
        property: Property::Width,
        slot: StyleSlot::Length(value.into()),
    }
}

/// `style:height` 的类型化值。
pub fn style_height(value: impl Into<Read<Length>>) -> InlineStyle {
    InlineStyle {
        property: Property::Height,
        slot: StyleSlot::Length(value.into()),
    }
}

pub struct ComponentPlan {
    pub(crate) label: &'static str,
    pub(crate) mount: Box<dyn FnOnce() -> MountResult<MountPlan>>,
}

pub struct ShowPlan {
    pub(crate) when: Read<bool>,
    pub(crate) then: ChildrenFactory,
    pub(crate) fallback: Option<ChildrenFactory>,
}

/// `For` 的行集来源：显式读源，或跟随 Store 当前投影的 `Rows`。
pub(crate) enum ForRows {
    Read(Read<RowSet>),
    Projection(FilterId),
}

/// `For` 的唯一数据入口：行 Store 与行集来源。
///
/// `ForSource::new` 接受任意 `Read<RowSet>`（Signal/Memo/Once/Static）；
/// [`Rows<R, F>`] 自带 store，经 `From` 转为跟随投影的来源，无需安装 runtime。
pub struct ForSource<R: Row> {
    store: ModelStore<R>,
    rows: ForRows,
}

impl<R: Row> ForSource<R> {
    pub fn new(store: ModelStore<R>, rows: impl Into<Read<RowSet>>) -> Self {
        Self {
            store,
            rows: ForRows::Read(rows.into()),
        }
    }
}

impl<R: Row, F: FilterSpec<R>> From<Rows<R, F>> for ForSource<R> {
    fn from(rows: Rows<R, F>) -> Self {
        Self {
            store: rows.store(),
            rows: ForRows::Projection(rows.filter()),
        }
    }
}

/// 一个已解析行的挂载快照：`id()` 是行身份，`key()` 是业务键，`get()` 是行数据。
///
/// 快照不随后续 Store 写入变化；响应式字段读取由后续版本的行字段句柄承担。
#[derive(Clone)]
pub struct RowView<R: Row> {
    id: RowId,
    snapshot: R,
}

impl<R: Row> RowView<R> {
    pub(crate) fn new(id: RowId, snapshot: R) -> Self {
        Self { id, snapshot }
    }

    pub const fn id(&self) -> RowId {
        self.id
    }

    pub fn key(&self) -> R::Key {
        self.snapshot.key()
    }

    pub fn get(&self) -> &R {
        &self.snapshot
    }
}

/// `store` 属性可接受的行 Store：`ModelStore<R>` 自身，或应用对它的封装。
pub trait ForStore {
    type Row: Row;

    fn for_source(&self, rows: impl Into<Read<RowSet>>) -> ForSource<Self::Row>;
}

impl<R: Row> ForStore for ModelStore<R> {
    type Row = R;

    fn for_source(&self, rows: impl Into<Read<RowSet>>) -> ForSource<R> {
        ForSource::new(self.clone(), rows)
    }
}

/// `For` 的输入：`ForSource` 本身，或自带 store 的 [`Rows`]。
///
/// 裸 `RowSet`/`Memo<RowSet>` 不会实现本 trait，因此必须配 `store` 属性。
#[diagnostic::on_unimplemented(
    message = "`For` needs a store to resolve row data",
    label = "this row source has no Store handle",
    note = "pass the `store` attribute alongside `each`, or use a `Rows` source that carries its Store"
)]
pub trait IntoForSource {
    type Row: Row;

    fn into_for_source(self) -> ForSource<Self::Row>;
}

impl<R: Row> IntoForSource for ForSource<R> {
    type Row = R;

    fn into_for_source(self) -> ForSource<R> {
        self
    }
}

impl<R: Row, F: FilterSpec<R>> IntoForSource for Rows<R, F> {
    type Row = R;

    fn into_for_source(self) -> ForSource<R> {
        self.into()
    }
}

/// 泛型 `For`：行集来源 + 业务键选择 + 行 factory + 可选统一行高。
///
/// 键选择器接收已解析的行快照；投影变化时会重跑，因此必须是纯函数。
/// 挂载时被擦除为 [`ForPlan`]；`R`/`K` 只存在于驱动对象内部。
pub struct For<R: Row, K: RowKey> {
    source: ForSource<R>,
    key: Rc<dyn Fn(&R) -> K>,
    item: Rc<dyn Fn(RowView<R>) -> MountResult<MountPlan>>,
    row_height: Option<Length>,
}

impl<R: Row, K: RowKey> For<R, K> {
    pub fn new(
        source: impl IntoForSource<Row = R>,
        key: impl Fn(&R) -> K + 'static,
        item: impl Fn(RowView<R>) -> MountResult<MountPlan> + 'static,
    ) -> Self {
        Self {
            source: source.into_for_source(),
            key: Rc::new(key),
            item: Rc::new(item),
            row_height: None,
        }
    }

    /// 窗口化 For 的统一行高；`None` 保持全量挂载（ADR-0010）。
    pub fn row_height(mut self, height: Length) -> Self {
        self.row_height = Some(height);
        self
    }

    fn into_plan(self) -> ForPlan {
        let Self {
            source,
            key,
            item,
            row_height,
        } = self;
        let store = source.store.clone();
        let store_id = store.id();
        let attach_store = store.clone();
        // 观察者必须在挂载期安装：`ForSource` 可以在 runtime 外构造，
        // 但只有真正挂载的行才需要进入 `PendingChanges`。
        let attach: Rc<dyn Fn()> =
            Rc::new(move || crate::runtime::observe_store_commits(&attach_store));
        let driver: Rc<dyn ForDriver> = Rc::new(TypedForDriver { store, key, item });
        ForPlan {
            store: store_id,
            rows: source.rows,
            driver,
            row_height,
            attach,
        }
    }
}

impl<R: Row, K: RowKey> IntoNode for For<R, K> {
    fn into_node(self) -> PlanNode {
        PlanNode::For(self.into_plan())
    }
}

/// 擦除后的 `For` 计划；`NodeArena` 只保存它，不接触 `R`/`K`。
pub struct ForPlan {
    pub(crate) store: StoreId,
    pub(crate) rows: ForRows,
    pub(crate) driver: Rc<dyn ForDriver>,
    pub(crate) row_height: Option<Length>,
    /// 挂载期把 Store 提交接入当前 runtime 的 `PendingChanges`。
    pub(crate) attach: Rc<dyn Fn()>,
}

/// 行的 store 归属；行身份本身已在本 runtime 内全局唯一。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ErasedStore {
    pub(crate) store: StoreId,
}

/// 交给驱动的行挂载请求；`row` 是拥有所有权的行快照。
pub(crate) struct ErasedRowView {
    pub(crate) id: RowId,
    pub(crate) store: ErasedStore,
    pub(crate) row: Rc<dyn Any>,
}

/// 唯一持有 `R`/`K` 的边界：键映射、行快照与行 factory 全在 typed driver 内。
pub(crate) trait ForDriver {
    fn row_snapshot(&self, id: RowId) -> Option<Rc<dyn Any>>;

    /// 行是否仍存活；静态快照可能早于删除。
    fn row_live(&self, id: RowId) -> bool;

    fn key_hash(&self, id: RowId) -> Option<u64>;

    fn key_eq(&self, a: RowId, b: RowId) -> bool;

    fn projection_rowset(&self, filter: FilterId) -> Result<RowSet, StoreError>;

    fn mount_row(&self, view: ErasedRowView) -> MountResult<MountPlan>;
}

struct TypedForDriver<R: Row, K: RowKey> {
    store: ModelStore<R>,
    key: Rc<dyn Fn(&R) -> K>,
    item: Rc<dyn Fn(RowView<R>) -> MountResult<MountPlan>>,
}

impl<R: Row, K: RowKey> ForDriver for TypedForDriver<R, K> {
    fn row_snapshot(&self, id: RowId) -> Option<Rc<dyn Any>> {
        self.store.get(id).map(|row| Rc::new(row) as Rc<dyn Any>)
    }

    fn row_live(&self, id: RowId) -> bool {
        self.store.contains(id)
    }

    /// 先克隆行再运行用户选择器，避免在 Store 的 `RefCell` 借用内重入读取。
    fn key_hash(&self, id: RowId) -> Option<u64> {
        let row = self.store.get(id)?;
        let mut hasher = DefaultHasher::new();
        (self.key)(&row).hash(&mut hasher);
        Some(hasher.finish())
    }

    fn key_eq(&self, a: RowId, b: RowId) -> bool {
        let (Some(a), Some(b)) = (self.store.get(a), self.store.get(b)) else {
            return false;
        };
        (self.key)(&a) == (self.key)(&b)
    }

    fn projection_rowset(&self, filter: FilterId) -> Result<RowSet, StoreError> {
        self.store.rowset_from_filter(filter)
    }

    fn mount_row(&self, view: ErasedRowView) -> MountResult<MountPlan> {
        debug_assert_eq!(
            view.store.store,
            self.store.id(),
            "a For driver only mounts rows from its own store"
        );
        let row = view
            .row
            .downcast::<R>()
            .expect("a For driver only receives its own row type");
        let snapshot = Rc::try_unwrap(row).unwrap_or_else(|row| (*row).clone());
        (self.item)(RowView::new(view.id, snapshot))
    }
}

pub struct Children {
    mount: Box<dyn FnOnce() -> MountResult<MountPlan>>,
}

impl Children {
    pub(crate) fn mount(self) -> MountResult<MountPlan> {
        (self.mount)()
    }
}

impl<F> From<F> for Children
where
    F: FnOnce() -> MountResult<MountPlan> + 'static,
{
    fn from(mount: F) -> Self {
        Self {
            mount: Box::new(mount),
        }
    }
}

pub struct ChildrenFactory {
    mount: Box<dyn Fn() -> MountResult<MountPlan>>,
}

impl ChildrenFactory {
    pub(crate) fn mount(&self) -> MountResult<MountPlan> {
        (self.mount)()
    }
}

impl<F> From<F> for ChildrenFactory
where
    F: Fn() -> MountResult<MountPlan> + 'static,
{
    fn from(mount: F) -> Self {
        Self {
            mount: Box::new(mount),
        }
    }
}

/// 一条属性值：文本或布尔，二者共用 [`Read`] 的读取与失败契约。
pub enum AttrValue {
    Text(Read<String>),
    Bool(Read<bool>),
}

pub enum Attr {
    Value {
        name: &'static str,
        value: AttrValue,
    },
    Event(Event),
}

/// 用户闭包返回的错误；在 turn 边界进入 [`crate::TurnReport::errors`]。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HandlerError {
    message: String,
}

impl HandlerError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for HandlerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HandlerError {}

impl From<WriteError> for HandlerError {
    fn from(error: WriteError) -> Self {
        Self::new(error.to_string())
    }
}

impl From<&str> for HandlerError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

impl From<String> for HandlerError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

/// 事件槽的返回值；`Err` 停止本事件剩余 handler 与默认动作。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HandlerOutcome {
    Ok,
    Err(HandlerError),
}

impl HandlerOutcome {
    pub const fn is_err(&self) -> bool {
        matches!(self, Self::Err(_))
    }

    pub const fn error(&self) -> Option<&HandlerError> {
        match self {
            Self::Ok => None,
            Self::Err(error) => Some(error),
        }
    }
}

/// 把 `()`、`Result<(), E>` 或 [`HandlerOutcome`] 归一为事件槽结果。
pub trait IntoHandlerOutcome {
    fn into_handler_outcome(self) -> HandlerOutcome;
}

impl IntoHandlerOutcome for HandlerOutcome {
    fn into_handler_outcome(self) -> HandlerOutcome {
        self
    }
}

impl IntoHandlerOutcome for () {
    fn into_handler_outcome(self) -> HandlerOutcome {
        HandlerOutcome::Ok
    }
}

impl<E: Into<HandlerError>> IntoHandlerOutcome for Result<(), E> {
    fn into_handler_outcome(self) -> HandlerOutcome {
        match self {
            Ok(()) => HandlerOutcome::Ok,
            Err(error) => HandlerOutcome::Err(error.into()),
        }
    }
}

pub enum Event {
    Click(Box<dyn FnMut() -> HandlerOutcome>),
    Input(Box<dyn FnMut(String) -> HandlerOutcome>),
    Change(Box<dyn FnMut(bool)>),
    Submit(Box<dyn FnMut(&mut SubmitEvent) -> HandlerOutcome>),
    KeyDown(Box<dyn FnMut(&mut KeyEvent) -> HandlerOutcome>),
    KeyDownCapture(Box<dyn FnMut(&mut KeyEvent) -> HandlerOutcome>),
    KeyDownBubble(Box<dyn FnMut(&mut KeyEvent) -> HandlerOutcome>),
    Focus(Box<dyn FnMut(FocusEvent)>),
    Blur(Box<dyn FnMut(BlurEvent)>),
    CompositionStart(Box<dyn FnMut(CompositionEvent)>),
    CompositionUpdate(Box<dyn FnMut(CompositionEvent)>),
    CompositionEnd(Box<dyn FnMut(CompositionEvent)>),
    CompositionCancel(Box<dyn FnMut(CompositionEvent)>),
}

/// 文本数据位置：接受字面量、显式 once 与读句柄；裸 `String` 不再隐式成为 once。
pub trait IntoText {
    fn into_text(self) -> Read<String>;
}

impl IntoText for &'static str {
    fn into_text(self) -> Read<String> {
        Read::Static(self.to_owned())
    }
}

impl IntoText for Once<String> {
    fn into_text(self) -> Read<String> {
        Read::Once(self)
    }
}

impl IntoText for Once<&'static str> {
    fn into_text(self) -> Read<String> {
        Read::from(self)
    }
}

impl IntoText for Signal<String> {
    fn into_text(self) -> Read<String> {
        Read::Signal(self)
    }
}

impl IntoText for Memo<String> {
    fn into_text(self) -> Read<String> {
        Read::Memo(self)
    }
}

impl IntoText for Field<String> {
    fn into_text(self) -> Read<String> {
        Read::Field(self)
    }
}

impl IntoText for Read<String> {
    fn into_text(self) -> Read<String> {
        self
    }
}

/// 属性数据位置：字面量、显式 once 与读句柄；裸 `String`/`bool` 不再隐式成为静态值。
pub trait IntoAttr {
    fn into_attr(self) -> AttrValue;
}

impl IntoAttr for &'static str {
    fn into_attr(self) -> AttrValue {
        AttrValue::Text(Read::Static(self.to_owned()))
    }
}

impl IntoAttr for Once<String> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Text(Read::Once(self))
    }
}

impl IntoAttr for Once<&'static str> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Text(Read::from(self))
    }
}

impl IntoAttr for Once<bool> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Bool(Read::Once(self))
    }
}

impl IntoAttr for Signal<String> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Text(Read::Signal(self))
    }
}

impl IntoAttr for Memo<String> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Text(Read::Memo(self))
    }
}

impl IntoAttr for Signal<bool> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Bool(Read::Signal(self))
    }
}

impl IntoAttr for Memo<bool> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Bool(Read::Memo(self))
    }
}

impl IntoAttr for Field<String> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Text(Read::Field(self))
    }
}

impl IntoAttr for Field<bool> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Bool(Read::Field(self))
    }
}

impl IntoAttr for Read<String> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Text(self)
    }
}

impl IntoAttr for Read<bool> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Bool(self)
    }
}

pub fn text_static(value: &'static str) -> PlanNode {
    PlanNode::Text(Read::Static(value.to_owned()))
}

pub fn child(value: impl IntoNode) -> PlanNode {
    value.into_node()
}

pub trait IntoNode {
    fn into_node(self) -> PlanNode;
}

impl IntoNode for &'static str {
    fn into_node(self) -> PlanNode {
        PlanNode::Text(self.into_text())
    }
}

impl IntoNode for Once<String> {
    fn into_node(self) -> PlanNode {
        PlanNode::Text(self.into_text())
    }
}

impl IntoNode for Once<&'static str> {
    fn into_node(self) -> PlanNode {
        PlanNode::Text(self.into_text())
    }
}

impl IntoNode for Signal<String> {
    fn into_node(self) -> PlanNode {
        PlanNode::Text(self.into_text())
    }
}

impl IntoNode for Memo<String> {
    fn into_node(self) -> PlanNode {
        PlanNode::Text(self.into_text())
    }
}

impl IntoNode for Field<String> {
    fn into_node(self) -> PlanNode {
        PlanNode::Text(self.into_text())
    }
}

impl IntoNode for Read<String> {
    fn into_node(self) -> PlanNode {
        PlanNode::Text(self.into_text())
    }
}

impl IntoNode for Children {
    fn into_node(self) -> PlanNode {
        PlanNode::Children(self)
    }
}

impl IntoNode for ChildrenFactory {
    fn into_node(self) -> PlanNode {
        PlanNode::Factory(self)
    }
}

pub fn attr(name: &'static str, value: impl IntoAttr) -> Attr {
    Attr::Value {
        name,
        value: value.into_attr(),
    }
}

pub fn element(tag: &'static str, attrs: Vec<Attr>, children: Vec<PlanNode>) -> PlanNode {
    PlanNode::Element(ElementPlan {
        tag,
        attrs,
        class_styles: &[],
        inline_styles: Vec::new(),
        children,
    })
}

/// 带 type-safe 样式的元素；class 声明来自展开期，inline 声明来自 `style:` 属性。
pub fn styled_element(
    tag: &'static str,
    attrs: Vec<Attr>,
    class_styles: &'static [StyleDeclaration],
    inline_styles: Vec<InlineStyle>,
    children: Vec<PlanNode>,
) -> PlanNode {
    PlanNode::Element(ElementPlan {
        tag,
        attrs,
        class_styles,
        inline_styles,
        children,
    })
}

pub fn component(
    label: &'static str,
    mount: impl FnOnce() -> MountResult<MountPlan> + 'static,
) -> PlanNode {
    PlanNode::Component(ComponentPlan {
        label,
        mount: Box::new(mount),
    })
}

pub fn show(
    when: impl Into<Read<bool>>,
    then: impl Into<ChildrenFactory>,
    fallback: Option<ChildrenFactory>,
) -> PlanNode {
    PlanNode::Show(ShowPlan {
        when: when.into(),
        then: then.into(),
        fallback,
    })
}

pub fn for_each<R: Row, K: RowKey>(
    source: impl IntoForSource<Row = R>,
    key: impl Fn(&R) -> K + 'static,
    item: impl Fn(RowView<R>) -> MountResult<MountPlan> + 'static,
) -> PlanNode {
    For::new(source, key, item).into_node()
}

/// 把 `store` 属性与 `each` 读源组合成唯一的 `For` 数据入口。
///
/// 只借用 Store，因此同一份 Store 句柄仍可在同一声明里的其他节点使用。
pub fn for_source<S: ForStore>(store: &S, rows: impl Into<Read<RowSet>>) -> ForSource<S::Row> {
    store.for_source(rows)
}

/// 窗口化 `For`：只挂载宿主声明的可见行窗口，行槽高由 `row_height` 声明。
///
/// 行高是每行的外层高度，父容器必须是零间距的纵向布局；For 用上下两个
/// spacer 保持 `总行数 × row_height` 的声明范围（ADR-0011）。
pub fn for_each_windowed<R: Row, K: RowKey>(
    source: impl IntoForSource<Row = R>,
    key: impl Fn(&R) -> K + 'static,
    row_height: Length,
    item: impl Fn(RowView<R>) -> MountResult<MountPlan> + 'static,
) -> PlanNode {
    For::new(source, key, item)
        .row_height(row_height)
        .into_node()
}

pub fn on_click<R>(mut handler: impl FnMut() -> R + 'static) -> Attr
where
    R: IntoHandlerOutcome,
{
    Attr::Event(Event::Click(Box::new(move || {
        handler().into_handler_outcome()
    })))
}

pub fn on_input<R>(mut handler: impl FnMut(String) -> R + 'static) -> Attr
where
    R: IntoHandlerOutcome,
{
    Attr::Event(Event::Input(Box::new(move |value| {
        handler(value).into_handler_outcome()
    })))
}

pub fn on_change(handler: impl FnMut(bool) + 'static) -> Attr {
    Attr::Event(Event::Change(Box::new(handler)))
}

pub fn on_submit<R>(mut handler: impl FnMut(&mut SubmitEvent) -> R + 'static) -> Attr
where
    R: IntoHandlerOutcome,
{
    Attr::Event(Event::Submit(Box::new(move |event| {
        handler(event).into_handler_outcome()
    })))
}

pub fn on_keydown<R>(mut handler: impl FnMut(&mut KeyEvent) -> R + 'static) -> Attr
where
    R: IntoHandlerOutcome,
{
    Attr::Event(Event::KeyDown(Box::new(move |event| {
        handler(event).into_handler_outcome()
    })))
}

pub fn on_keydown_capture<R>(mut handler: impl FnMut(&mut KeyEvent) -> R + 'static) -> Attr
where
    R: IntoHandlerOutcome,
{
    Attr::Event(Event::KeyDownCapture(Box::new(move |event| {
        handler(event).into_handler_outcome()
    })))
}

pub fn on_keydown_bubble<R>(mut handler: impl FnMut(&mut KeyEvent) -> R + 'static) -> Attr
where
    R: IntoHandlerOutcome,
{
    Attr::Event(Event::KeyDownBubble(Box::new(move |event| {
        handler(event).into_handler_outcome()
    })))
}

pub fn on_focus(handler: impl FnMut(FocusEvent) + 'static) -> Attr {
    Attr::Event(Event::Focus(Box::new(handler)))
}

pub fn on_blur(handler: impl FnMut(BlurEvent) + 'static) -> Attr {
    Attr::Event(Event::Blur(Box::new(handler)))
}

pub fn on_composition_start(handler: impl FnMut(CompositionEvent) + 'static) -> Attr {
    Attr::Event(Event::CompositionStart(Box::new(handler)))
}

pub fn on_composition_update(handler: impl FnMut(CompositionEvent) + 'static) -> Attr {
    Attr::Event(Event::CompositionUpdate(Box::new(handler)))
}

pub fn on_composition_end(handler: impl FnMut(CompositionEvent) + 'static) -> Attr {
    Attr::Event(Event::CompositionEnd(Box::new(handler)))
}

pub fn on_composition_cancel(handler: impl FnMut(CompositionEvent) + 'static) -> Attr {
    Attr::Event(Event::CompositionCancel(Box::new(handler)))
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct SubmitEvent {
    consumed: bool,
    prevented: bool,
}

impl SubmitEvent {
    /// 停止后续处理；submit 本身没有后续默认动作，consume 用于统一的 turn flow。
    pub fn consume(&mut self) {
        self.consumed = true;
    }

    /// 文档化的 no-op：submit 没有可以被跳过的后续默认动作。
    pub fn prevent_default(&mut self) {
        self.prevented = true;
    }

    pub const fn is_consumed(self) -> bool {
        self.consumed
    }

    pub const fn is_prevented(self) -> bool {
        self.prevented
    }
}

/// 焦点移动的来源：显式命令，或旧目标离开可见集合后的自动结算。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FocusCause {
    Command,
    Settlement,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FocusEvent {
    cause: FocusCause,
}

impl FocusEvent {
    pub(crate) const fn new(cause: FocusCause) -> Self {
        Self { cause }
    }

    pub const fn cause(self) -> FocusCause {
        self.cause
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BlurEvent {
    cause: FocusCause,
}

impl BlurEvent {
    pub(crate) const fn new(cause: FocusCause) -> Self {
        Self { cause }
    }

    pub const fn cause(self) -> FocusCause {
        self.cause
    }
}

/// 一次本地 composition 的阶段；候选确认是 `End`，取消恢复是 `Cancel`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompositionPhase {
    Start,
    Update,
    End,
    Cancel,
}

/// 编辑器本地 composition 的 typed 载荷；`text` 在 Start/Update 是 preedit，
/// 在 End 是确认的候选文本，在 Cancel 是被丢弃的 preedit。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CompositionEvent {
    phase: CompositionPhase,
    text: String,
    caret: usize,
    selection: Option<(usize, usize)>,
}

impl CompositionEvent {
    pub(crate) fn new(
        phase: CompositionPhase,
        text: String,
        caret: usize,
        selection: Option<(usize, usize)>,
    ) -> Self {
        Self {
            phase,
            text,
            caret,
            selection,
        }
    }

    pub const fn phase(&self) -> CompositionPhase {
        self.phase
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub const fn caret(&self) -> usize {
        self.caret
    }

    pub const fn selection(&self) -> Option<(usize, usize)> {
        self.selection
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Escape,
    Enter,
    Tab,
    Space,
    Backspace,
    Delete,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Home,
    End,
    PageUp,
    PageDown,
    Other,
}

/// 一次 keydown 的载荷；处理器可消费事件或只跳过默认动作。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyEvent {
    key: Key,
    consumed: bool,
    prevent_default: bool,
}

impl KeyEvent {
    pub const fn new(key: Key) -> Self {
        Self {
            key,
            consumed: false,
            prevent_default: false,
        }
    }

    pub const fn key(self) -> Key {
        self.key
    }

    /// 停止后续捕获/冒泡阶段与全部默认动作。
    pub fn consume(&mut self) {
        self.consumed = true;
    }

    /// 只跳过默认动作，传播继续。
    pub fn prevent_default(&mut self) {
        self.prevent_default = true;
    }

    pub const fn is_consumed(self) -> bool {
        self.consumed
    }

    pub const fn prevents_default(self) -> bool {
        self.prevent_default
    }
}
