use std::fmt;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

// 每种身份都有独立类型，不能将行、控件或作用域索引互换使用。
macro_rules! identity_newtype {
    ($($(#[$meta:meta])* $name:ident),+ $(,)?) => {
        $(
            $(#[$meta])*
            #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
            pub struct $name(u32);

            impl $name {

                #[allow(dead_code)]
                pub(crate) const fn from_index(index: u32) -> Self {
                    Self(index)
                }

                pub const fn index(self) -> u32 {
                    self.0
                }
            }

            impl fmt::Debug for $name {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    write!(f, "{}({})", stringify!($name), self.0)
                }
            }
        )+
    };
}

identity_newtype!(ViewId, OwnerId, SignalId, MemoId, QueryId, LayoutNodeId,);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeId(u64);

static NEXT_RUNTIME: AtomicU64 = AtomicU64::new(1);

impl RuntimeId {
    /// Runtime 分配不重复的进程内身份；句柄通过接收者匹配身份后才会解析。
    pub(crate) fn allocate() -> Self {
        Self(NEXT_RUNTIME.fetch_add(1, Ordering::Relaxed))
    }
}

impl fmt::Debug for RuntimeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RuntimeId(..)")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SlotIndex(u32);

impl SlotIndex {
    pub(crate) const fn from_index(index: u32) -> Self {
        Self(index)
    }

    pub const fn index(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Generation(u32);

impl Generation {
    pub(crate) const FIRST: Generation = Generation(0);

    /// 当前占位代际递增顺序；长生命周期接口需独立处理代际耗尽与禁止别名。
    pub(crate) fn next(self) -> Generation {
        Generation(self.0.wrapping_add(1))
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RowKey(u64);

impl RowKey {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HandleError {
    ForeignRuntime {
        expected: RuntimeId,
        found: RuntimeId,
    },

    Vacant {
        slot: SlotIndex,
    },

    StaleGeneration {
        slot: SlotIndex,
        current: Generation,
        handle: Generation,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InvalidHandle {
    pub reason: HandleError,
}

impl InvalidHandle {
    pub const fn reason_kind(&self) -> &'static str {
        match self.reason {
            HandleError::ForeignRuntime { .. } => "foreign_runtime",
            HandleError::Vacant { .. } => "vacant",
            HandleError::StaleGeneration { .. } => "stale_generation",
        }
    }
}

impl From<HandleError> for InvalidHandle {
    fn from(reason: HandleError) -> Self {
        Self { reason }
    }
}

impl fmt::Display for InvalidHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.reason {
            HandleError::ForeignRuntime { .. } => f.write_str("handle from foreign runtime"),
            HandleError::Vacant { slot } => write!(f, "handle points at vacant slot {slot:?}"),
            HandleError::StaleGeneration {
                slot,
                current,
                handle,
            } => write!(
                f,
                "stale handle at slot {slot:?}: live generation {current:?}, handle has {handle:?}"
            ),
        }
    }
}

impl std::error::Error for InvalidHandle {}

/// 可复制的是行身份，不是业务对象；由接收者检查来源和代际后才解析。
pub struct Handle<T> {
    runtime: RuntimeId,
    slot: SlotIndex,
    generation: Generation,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    pub(crate) const fn new(runtime: RuntimeId, slot: SlotIndex, generation: Generation) -> Self {
        Self {
            runtime,
            slot,
            generation,
            _marker: PhantomData,
        }
    }

    pub(crate) const fn runtime(self) -> RuntimeId {
        self.runtime
    }

    pub(crate) const fn slot(self) -> SlotIndex {
        self.slot
    }

    pub(crate) const fn generation(self) -> Generation {
        self.generation
    }
}

impl<T> Copy for Handle<T> {}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.runtime == other.runtime
            && self.slot == other.slot
            && self.generation == other.generation
    }
}

impl<T> Eq for Handle<T> {}

impl<T> std::hash::Hash for Handle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.runtime.hash(state);
        self.slot.hash(state);
        self.generation.hash(state);
    }
}

impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handle")
            .field("runtime", &self.runtime)
            .field("slot", &self.slot)
            .field("generation", &self.generation)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_id_index_is_stable_for_trace_output() {
        let view = ViewId::from_index(3);
        assert_eq!(view.index(), 3);
        assert_eq!(format!("{view:?}"), "ViewId(3)");
    }

    #[test]
    fn runtime_id_debug_hides_the_value() {
        let id = RuntimeId::allocate();
        assert_eq!(format!("{id:?}"), "RuntimeId(..)");
    }

    #[test]
    fn foreign_runtime_display_hides_the_values() {
        let error = InvalidHandle::from(HandleError::ForeignRuntime {
            expected: RuntimeId::allocate(),
            found: RuntimeId::allocate(),
        });

        assert_eq!(error.to_string(), "handle from foreign runtime");
        assert_eq!(error.reason_kind(), "foreign_runtime");
    }

    #[test]
    fn minted_runtime_ids_are_unique_across_threads() {
        let handles: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(RuntimeId::allocate))
            .collect();
        let mut ids: Vec<u64> = handles
            .into_iter()
            .map(|handle| handle.join().expect("thread").0)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 8, "the allocator must not repeat an id");
    }

    #[test]
    fn generation_wraps_to_first() {
        let generation = Generation(u32::MAX).next();
        assert_eq!(generation, Generation::FIRST);
    }

    #[test]
    fn a_handle_identity_is_the_runtime_slot_generation_triple() {
        let runtime_a = RuntimeId::allocate();
        let runtime_b = RuntimeId::allocate();
        let slot = SlotIndex::from_index(0);
        let generation = Generation::FIRST;
        let handle = Handle::<u32>::new(runtime_a, slot, generation);

        assert_eq!(handle.runtime(), runtime_a);
        assert_eq!(handle.slot(), slot);
        assert_eq!(handle.generation(), generation);
        assert_eq!(
            handle,
            Handle::<u32>::new(runtime_a, slot, generation),
            "the same triple is the same handle"
        );
        assert_ne!(
            handle,
            Handle::<u32>::new(runtime_b, slot, generation),
            "runtime_id separates handles"
        );
        assert_ne!(
            handle,
            Handle::<u32>::new(runtime_a, SlotIndex::from_index(1), generation),
            "slot separates handles"
        );
        assert_ne!(
            handle,
            Handle::<u32>::new(runtime_a, slot, generation.next()),
            "generation separates handles"
        );
    }
}
