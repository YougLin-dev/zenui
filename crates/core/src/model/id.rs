use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::handle::Generation;

/// 8 字节 arena 行身份：32-bit slot + 32-bit generation，与 `NodeRef` 同构。
///
/// 业务键不在身份里；删除后以同一业务键重建行会分配新的 slot 或 generation，
/// 旧 `RowId` 不会命中新行。generation 来自进程内单调分配，因此不同 Store
/// 同时存活时身份也不会相撞。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RowId {
    slot: u32,
    generation: Generation,
}

impl RowId {
    pub(crate) const fn new(slot: u32, generation: Generation) -> Self {
        Self { slot, generation }
    }

    pub const fn slot(self) -> u32 {
        self.slot
    }

    pub const fn generation(self) -> Generation {
        self.generation
    }
}

impl fmt::Debug for RowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RowId({}:{})", self.slot, self.generation.raw())
    }
}

static NEXT_ROW_GENERATION: AtomicU32 = AtomicU32::new(1);

/// 分配一个进程内唯一的行代际。
///
/// arena 只在 slot 复用时比较代际，所以跨 Store 的唯一性来自这里是单调的。
/// 进程生命周期内的行分配上限是 2^32（与既有 arena 的代际预算同级）。
pub(crate) fn next_generation() -> Generation {
    let raw = NEXT_ROW_GENERATION.fetch_add(1, Ordering::Relaxed);
    if raw == 0 {
        Generation::from_raw(1)
    } else {
        Generation::from_raw(raw)
    }
}

static NEXT_STORE: AtomicU32 = AtomicU32::new(1);

/// runtime 内的 store 槽；只用于路由与 `RowSetData` 归属，不是行身份。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct StoreId(u32);

impl StoreId {
    pub(crate) fn allocate() -> Self {
        Self(NEXT_STORE.fetch_add(1, Ordering::Relaxed).max(1))
    }

    pub const fn index(self) -> u32 {
        self.0
    }
}

/// 模型声明的字段标识；`ChangeSet.fields` 与过滤观察都用它。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FieldId(u16);

impl FieldId {
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn index(self) -> u16 {
        self.0
    }
}

/// 模型声明的过滤标识；由 schema 注册，不能退化成 Todo 专用的三值枚举。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FilterId(u16);

impl FilterId {
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    pub const fn index(self) -> u16 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_id_is_eight_bytes_and_keeps_slot_and_generation() {
        assert_eq!(std::mem::size_of::<RowId>(), 8);
        let first = RowId::new(3, next_generation());
        let second = RowId::new(3, next_generation());
        assert_eq!(first.slot(), 3);
        assert_ne!(first.generation(), second.generation());
        assert_ne!(first, second);
    }

    #[test]
    fn generations_are_unique_across_allocations() {
        let ids: Vec<Generation> = (0..64).map(|_| next_generation()).collect();
        let unique: std::collections::BTreeSet<u32> = ids.iter().map(|id| id.raw()).collect();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn store_ids_are_unique() {
        assert_ne!(StoreId::allocate(), StoreId::allocate());
    }
}
