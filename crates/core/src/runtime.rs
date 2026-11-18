// crates/core/src/runtime.rs
//! The runtime authority.
//!
//! A `Runtime` is the only place that mints a `RuntimeId`, and therefore the
//! only owner of the store, the reactive graph, the retained view tree and the
//! owner scopes. `SPEC-RT-005`'s cross-runtime rejection is a comparison between
//! a handle's `runtime_id` and the *resolver's* own identity, so resolution has
//! to live on the authority, not on the handle.
//!
//! TODO(A0-06): the frame transaction and its journals are not wired in yet;
//! P2 grows this type into the single commit point.

use std::fmt;

use crate::handle::RuntimeId;
use crate::query::QueryCache;
use crate::store::Store;

/// Runtime identity plus the authorities it owns.
pub struct Runtime {
    id: RuntimeId,
    store: Store,
    query: QueryCache,
}

impl Runtime {
    /// Mint a fresh runtime identity and build the authorities around it.
    pub fn new() -> Self {
        let id = RuntimeId::allocate();
        Self {
            id,
            store: Store::with_runtime(id),
            query: QueryCache::new(),
        }
    }

    pub const fn id(&self) -> RuntimeId {
        self.id
    }

    pub const fn store(&self) -> &Store {
        &self.store
    }

    pub const fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    pub const fn query_mut(&mut self) -> &mut QueryCache {
        &mut self.query
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `RuntimeId` renders as `RuntimeId(..)`; the value never leaves here.
        f.debug_struct("Runtime")
            .field("id", &self.id)
            .field("store", &self.store)
            .finish()
    }
}
