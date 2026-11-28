//! 通用行模型：应用行数据、稳定的业务键与带代际的 arena 行身份。
//!
//! 模型不依赖 UI runtime、`Read` 或窗口；`Store<R>` 只持有行、顺序与过滤成员。
//! 行身份是 [`RowId`]（slot + generation），业务键由 [`Row::key`] 提供；删除后
//! 以同一业务键重新插入会得到新的 `RowId`，旧身份不会命中新行。
//!
//! 一次事务提交产出非泛型的 [`ChangeSet`]，精确记录插入/移除的行身份、变化的
//! 字段、成员迁移与顺序变化；相等写入不产生任何变更，也不推进任何 revision。
//! 过滤规格在 schema 中声明，Store 维护 `FieldId → [FilterId]` 的候选索引，
//! 字段变化会重新求值当前不在结果内的候选行。

mod change;
mod id;
mod row;
mod rows;
mod store;

pub use change::ChangeSet;
pub use id::{FieldId, FilterId, RowId, StoreId};
pub use row::{FieldDef, FilterSpec, Row, RowKey, Schema, SchemaBuilder, SchemaError};
pub use rows::{RowSet, RowSetData, Rows};
pub use store::{Commit, CommitObserver, ErasedFieldRead, Store, StoreError, TransactionError, Tx};
