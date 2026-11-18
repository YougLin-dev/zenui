// crates/core/src/handle.rs
//! Identity domains and generation-checked handles.
//!
//! `SPEC-RT-002` requires one newtype per identity domain rather than a bare
//! `u64` shared between them; `SPEC-RT-005` requires the field handle to be a
//! small `Copy` struct of `{runtime_id, slot, generation}`.
//!
//! P1 scope: the shape is in place and the store rejects stale or foreign
//! handles. The handle-side `get`/`with` API of `SPEC-RT-005`, plus the
//! slot-reuse and cross-runtime policy that `SPEC-GATE-A0-03` verifies, are
//! owned by P2 and are intentionally not claimed here.

use std::fmt;
use std::marker::PhantomData;

macro_rules! identity_newtype {
    ($($(#[$meta:meta])* $name:ident),+ $(,)?) => {
        $(
            $(#[$meta])*
            #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
            pub struct $name(u32);

            impl $name {
                /// Only runtime code mints identities (`SPEC-RT-002`).
                ///
                /// Every domain gets the constructor now so later phases do not
                /// have to add it; some domains are not constructed in P1 yet.
                #[allow(dead_code)]
                pub(crate) const fn from_index(index: u32) -> Self {
                    Self(index)
                }

                /// Stable, address-free index. Safe to print in a trace.
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

identity_newtype!(
    /// Retained view identity. Stable across reorder, resize and field updates;
    /// only unmount/remount produces a new `ViewGeneration` (`SPEC-RT-003`).
    ViewId,
    /// Scope owner identity used for cleanup and subscription teardown.
    OwnerId,
    /// Reactive source identity.
    SignalId,
    /// Derived reactive node identity.
    MemoId,
    /// Query cache identity.
    QueryId,
    /// Layout adapter node identity. The `taffy::NodeId` mapping stays private
    /// to the adapter (`SPEC-RT-002`).
    LayoutNodeId,
);

/// Runtime instance identity. Handles carry it so a handle minted by one
/// runtime cannot be resolved by another (`SPEC-RT-005`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeId(u64);

impl RuntimeId {
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for RuntimeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RuntimeId({})", self.0)
    }
}

/// Index into a generational slot table.
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

/// Generation counter for a reused slot or a remounted view.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Generation(u32);

impl Generation {
    /// Generation assigned to a never-reused slot.
    pub(crate) const FIRST: Generation = Generation(0);

    /// Next generation for a recycled slot. Wrapping is deliberate: reaching
    /// `u32::MAX` reuses would require >4e9 mount/unmount cycles inside one
    /// runtime, and a wrapped value still cannot resurrect a live handle
    /// because recycling also frees the slot first.
    pub(crate) fn next(self) -> Generation {
        Generation(self.0.wrapping_add(1))
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Stable business key for a store row.
///
/// `RowSet` carries these (or handles), never owned titles (`SPEC-RT-045`).
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

/// Why a handle failed to resolve.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HandleError {
    /// Handle belongs to a different runtime instance.
    ForeignRuntime {
        expected: RuntimeId,
        found: RuntimeId,
    },
    /// Slot was never allocated or has been freed.
    Vacant { slot: SlotIndex },
    /// Slot was recycled: the handle is older than the live occupant.
    StaleGeneration {
        slot: SlotIndex,
        current: Generation,
        handle: Generation,
    },
}

/// Typed failure returned by every checked read/write (`SPEC-RT-005`,
/// `SPEC-RT-006`). Never a default value, never a silent no-op.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InvalidHandle {
    pub reason: HandleError,
}

impl InvalidHandle {
    /// Stable classification for trace output. Carries no slot index, so it is
    /// safe to print in a golden trace.
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
            HandleError::ForeignRuntime { expected, found } => write!(
                f,
                "handle from foreign runtime: expected {expected:?}, found {found:?}"
            ),
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

/// Generation-checked handle to a value owned by a store.
///
/// `Copy` does not mean "always valid" and does not make access free
/// (`SPEC-RT-005`).
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
    fn identity_newtypes_do_not_convert_from_each_other() {
        // Compile-time property: distinct types. This test documents the
        // intent; the actual proof is that no `From<ViewId> for OwnerId` impl
        // exists anywhere in the crate.
        let view = ViewId::from_index(3);
        assert_eq!(view.index(), 3);
        assert_eq!(format!("{view:?}"), "ViewId(3)");
    }

    #[test]
    fn generation_wraps_without_aliasing_a_live_slot() {
        let generation = Generation(u32::MAX).next();
        assert_eq!(generation, Generation::FIRST);
    }
}
