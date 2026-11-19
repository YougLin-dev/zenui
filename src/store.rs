use std::fmt;
use std::marker::PhantomData;

use zenui_core::{Fatal, Filter, RowKey, RowSet, Tx, ValueError};

use crate::context::{DepKey, FieldKind, Phase, filter_slot, with_state, with_state_of};

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

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Store {
    runtime: u32,
}

impl Store {
    pub fn new() -> Self {
        Self {
            runtime: with_state(|state| state.id),
        }
    }

    pub fn rows(self, filter: Filter) -> RowSet {
        with_state_of(self.runtime, |state| state.store_rows(filter))
    }

    pub fn is_empty(self, filter: Filter) -> bool {
        with_state_of(self.runtime, |state| state.store_is_empty(filter))
    }

    pub fn title(self, id: TodoId) -> Field<String> {
        Field::new(self.runtime, id, FieldKind::Title)
    }

    pub fn completed(self, id: TodoId) -> Field<bool> {
        Field::new(self.runtime, id, FieldKind::Completed)
    }

    pub fn insert(self, title: impl Into<String>) -> WriteOp<TodoId> {
        let title = title.into();
        WriteOp::new(self.runtime, move |tx| {
            let handle = tx.insert(title.clone()).map_err(WriteError::Fatal)?;
            let key = tx.key_of(handle).map_err(WriteError::Rejected)?;
            let id = TodoId(key);
            Ok((id, vec![Touch::Insert(id)]))
        })
    }

    pub fn set_completed(self, id: TodoId, done: bool) -> WriteOp<()> {
        WriteOp::new(self.runtime, move |tx| {
            let handle = tx.handle_for(id.0).ok_or(WriteError::Missing(id))?;
            tx.set_completed(handle, done)
                .map_err(WriteError::Rejected)?;
            Ok(((), vec![Touch::Completed(id)]))
        })
    }

    pub fn set_title(self, id: TodoId, title: impl Into<String>) -> WriteOp<()> {
        let title = title.into();
        WriteOp::new(self.runtime, move |tx| {
            let handle = tx.handle_for(id.0).ok_or(WriteError::Missing(id))?;
            tx.set_title(handle, title.clone())
                .map_err(WriteError::Rejected)?;
            Ok(((), vec![Touch::Title(id)]))
        })
    }

    pub fn remove(self, id: TodoId) -> WriteOp<()> {
        WriteOp::new(self.runtime, move |tx| {
            let handle = tx.handle_for(id.0).ok_or(WriteError::Missing(id))?;
            tx.remove(handle).map_err(WriteError::Rejected)?;
            Ok(((), vec![Touch::Removed(id)]))
        })
    }

    pub fn clear_completed(self) -> WriteOp<()> {
        WriteOp::new(self.runtime, move |tx| {
            let keys = tx.clear_completed().map_err(WriteError::Fatal)?;
            Ok((
                (),
                keys.into_iter()
                    .map(|key| Touch::Removed(TodoId(key)))
                    .collect(),
            ))
        })
    }

    pub fn reorder(self, ids: &[TodoId]) -> WriteOp<()> {
        let keys: Vec<RowKey> = ids.iter().map(|id| id.0).collect();
        WriteOp::new(self.runtime, move |tx| {
            tx.reorder(&keys).map_err(WriteError::Rejected)?;
            Ok(((), vec![Touch::Order]))
        })
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store")
            .field("runtime", &self.runtime)
            .finish()
    }
}

pub struct Field<T> {
    runtime: u32,
    id: TodoId,
    kind: FieldKind,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Field<T> {
    const fn new(runtime: u32, id: TodoId, kind: FieldKind) -> Self {
        Self {
            runtime,
            id,
            kind,
            _marker: PhantomData,
        }
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
        f.debug_struct("Field")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .finish()
    }
}

impl Field<String> {
    pub fn get(self) -> Result<String, FieldError> {
        with_state_of(self.runtime, |state| state.read_title(self.id, false))
    }

    pub fn read_once(self) -> Result<String, FieldError> {
        with_state_of(self.runtime, |state| state.read_title(self.id, true))
    }
}

impl Field<bool> {
    pub fn get(self) -> Result<bool, FieldError> {
        with_state_of(self.runtime, |state| state.read_completed(self.id, false))
    }

    pub fn read_once(self) -> Result<bool, FieldError> {
        with_state_of(self.runtime, |state| state.read_completed(self.id, true))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldError {
    Missing(TodoId),
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(id) => write!(f, "row {} is no longer live", id.raw()),
        }
    }
}

impl std::error::Error for FieldError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WriteError {
    Fatal(Fatal),
    Rejected(ValueError),
    Missing(TodoId),
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

type WriteResult<T> = Result<(T, Vec<Touch>), WriteError>;
type OpResult = Result<Vec<Touch>, WriteError>;
type Op = Box<dyn for<'a, 'b> FnOnce(&'a mut Tx<'b>) -> OpResult>;
type WriteAction<T> = Box<dyn for<'a, 'b> FnOnce(&'a mut Tx<'b>) -> WriteResult<T>>;

pub struct WriteOp<T> {
    runtime: u32,
    action: WriteAction<T>,
}

impl<T> WriteOp<T> {
    fn new(
        runtime: u32,
        action: impl for<'a, 'b> FnOnce(&'a mut Tx<'b>) -> WriteResult<T> + 'static,
    ) -> Self {
        Self {
            runtime,
            action: Box::new(action),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Touch {
    Insert(TodoId),
    Completed(TodoId),
    Title(TodoId),
    Removed(TodoId),
    Order,
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
        self.ops.push(Box::new(move |tx| {
            (op.action)(tx).map(|(_, touches)| touches)
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
    /// 读取共享的已提交 RowSet；同一 revision 的消费者拿到同一份投影。
    pub(crate) fn store_rows(&mut self, filter: Filter) -> RowSet {
        let rows = self.core.rows(filter);
        if self.phase == Phase::Memo {
            self.note_dep(DepKey::Rows(filter_slot(filter)));
        }
        rows
    }

    pub(crate) fn store_is_empty(&mut self, filter: Filter) -> bool {
        let empty = self.core.committed().is_empty_for(filter);
        if self.phase == Phase::Memo {
            self.note_dep(DepKey::Rows(filter_slot(filter)));
        }
        empty
    }

    pub(crate) fn read_title(&mut self, id: TodoId, once: bool) -> Result<String, FieldError> {
        if !once && self.phase == Phase::Memo {
            self.note_dep(DepKey::Field(id, FieldKind::Title));
        }
        if once {
            self.once_reads += 1;
        }
        self.core
            .committed()
            .rows()
            .find(|row| row.key() == id.0)
            .map(|row| row.title().to_owned())
            .ok_or(FieldError::Missing(id))
    }

    pub(crate) fn read_completed(&mut self, id: TodoId, once: bool) -> Result<bool, FieldError> {
        if !once && self.phase == Phase::Memo {
            self.note_dep(DepKey::Field(id, FieldKind::Completed));
        }
        if once {
            self.once_reads += 1;
        }
        self.core
            .committed()
            .rows()
            .find(|row| row.key() == id.0)
            .map(|row| row.completed())
            .ok_or(FieldError::Missing(id))
    }

    /// 事务只在 Event command turn 中执行；失败不改变模型，但会记录到命令边界，
    /// 即使调用方忽略返回值，dispatch 仍会报告这个错误。
    pub(crate) fn finish_transaction(&mut self, ops: Vec<Op>) -> Result<(), WriteError> {
        if self.phase != Phase::Event || self.read_only > 0 {
            let error = WriteError::Context {
                operation: "Transaction::finish",
                phase: self.phase,
            };
            self.write_error = Some(error);
            return Err(error);
        }

        let mut error = None;
        let mut touches = Vec::new();
        let result = self.core.frame(|tx| {
            for op in ops {
                match op(tx) {
                    Ok(mut touched) => touches.append(&mut touched),
                    Err(failure) => {
                        error = Some(failure);
                        return Err(Fatal::Aborted);
                    }
                }
            }
            Ok(())
        });

        if let Some(error) = error {
            self.write_error = Some(error);
            return Err(error);
        }
        if let Err(error) = result {
            let error = WriteError::Fatal(error);
            self.write_error = Some(error);
            return Err(error);
        }

        for touch in touches {
            self.note_touch(touch);
        }
        Ok(())
    }

    fn note_touch(&mut self, touch: Touch) {
        match touch {
            Touch::Insert(id) | Touch::Removed(id) => {
                self.bump(DepKey::Field(id, FieldKind::Title));
                self.bump(DepKey::Field(id, FieldKind::Completed));
                self.bump_rows();
            }
            Touch::Completed(id) => {
                self.bump(DepKey::Field(id, FieldKind::Completed));
                self.bump(DepKey::Rows(filter_slot(Filter::Active)));
                self.bump(DepKey::Rows(filter_slot(Filter::Completed)));
            }
            Touch::Title(id) => self.bump(DepKey::Field(id, FieldKind::Title)),
            Touch::Order => self.bump_rows(),
        }
    }

    fn bump_rows(&mut self) {
        for filter in [Filter::All, Filter::Active, Filter::Completed] {
            self.bump(DepKey::Rows(filter_slot(filter)));
        }
    }
}
