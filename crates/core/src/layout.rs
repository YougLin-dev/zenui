// crates/core/src/layout.rs
//! `ViewId → LayoutNodeId` mapping.
//!
//! `SPEC-GATE-A0-06` compares this mapping across a failed commit, and
//! `SPEC-RT-051` lists it among the things the preflight journal must carry.
//! A0 has no layout adapter, so the map is a dependency-free table owned by the
//! runtime: it participates in preflight for real rather than being stubbed out
//! and moved to another gate.

use std::collections::BTreeMap;

use crate::handle::{LayoutNodeId, ViewId};

/// The mapping the layout adapter will consume once one exists.
#[derive(Debug, Default)]
pub struct LayoutMap {
    next: u32,
    entries: BTreeMap<ViewId, LayoutNodeId>,
    planned: BTreeMap<ViewId, LayoutNodeId>,
}

impl LayoutMap {
    pub const fn new() -> Self {
        Self {
            next: 0,
            entries: BTreeMap::new(),
            planned: BTreeMap::new(),
        }
    }

    pub fn get(&self, view: ViewId) -> Option<LayoutNodeId> {
        self.planned
            .get(&view)
            .or_else(|| self.entries.get(&view))
            .copied()
    }

    /// Committed pairs, in `ViewId` order.
    pub fn pairs(&self) -> Vec<(ViewId, LayoutNodeId)> {
        self.entries.iter().map(|(view, id)| (*view, *id)).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Reserve a node id for a view the frame is about to mount.
    pub(crate) fn plan(&mut self, view: ViewId) {
        if self.get(view).is_some() {
            return;
        }
        // The allocator advances only on commit: a discarded frame must not
        // burn a layout id (`SPEC-RT-051`).
        let id = LayoutNodeId::from_index(self.next + self.planned.len() as u32);
        self.planned.insert(view, id);
    }

    /// Fail if the mapping would be left inconsistent by this commit.
    ///
    /// This is what makes the A0-06 mapping clause falsifiable: a live view that
    /// is neither remounted nor unmounted must still have a node, and a planned
    /// view must not collide with an existing one.
    pub(crate) fn preflight(&self, mounts: &[ViewId], unmounts: &[ViewId]) -> Result<(), ()> {
        let gone: std::collections::BTreeSet<ViewId> = unmounts.iter().copied().collect();
        for view in mounts {
            if gone.contains(view) {
                return Err(());
            }
            if self.entries.contains_key(view) {
                return Err(());
            }
            if !self.planned.contains_key(view) {
                return Err(());
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for id in self.planned.values() {
            if !seen.insert(*id) {
                return Err(());
            }
        }
        Ok(())
    }

    /// Install the planned nodes and drop the unmounted ones.
    pub(crate) fn commit(&mut self, unmounts: &[ViewId]) {
        for view in unmounts {
            self.entries.remove(view);
        }
        let planned = std::mem::take(&mut self.planned);
        self.next += planned.len() as u32;
        for (view, id) in planned {
            self.entries.insert(view, id);
        }
    }

    pub(crate) fn discard(&mut self) {
        self.planned.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planned_ids_are_installed_on_commit_and_dropped_on_discard() {
        let mut map = LayoutMap::new();
        let view = ViewId::from_index(1);

        map.plan(view);
        assert_eq!(map.len(), 0, "planning alone does not commit");
        assert!(map.get(view).is_some(), "but reads see the plan");

        map.discard();
        assert!(map.get(view).is_none(), "discard forgets the plan");

        map.plan(view);
        assert!(map.preflight(&[view], &[]).is_ok());
        map.commit(&[]);
        assert_eq!(map.len(), 1);
        assert!(map.get(view).is_some());
    }

    #[test]
    fn preflight_rejects_a_view_that_is_both_mounted_and_unmounted() {
        let mut map = LayoutMap::new();
        let view = ViewId::from_index(2);
        map.plan(view);
        assert!(map.preflight(&[view], &[view]).is_err());
    }

    #[test]
    fn preflight_rejects_a_mount_without_a_planned_node() {
        let map = LayoutMap::new();
        let view = ViewId::from_index(3);
        assert!(map.preflight(&[view], &[]).is_err());
    }
}
