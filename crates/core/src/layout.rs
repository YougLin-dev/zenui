use std::collections::BTreeMap;

use crate::handle::{LayoutNodeId, ViewId};

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

    pub fn pairs(&self) -> Vec<(ViewId, LayoutNodeId)> {
        self.entries.iter().map(|(view, id)| (*view, *id)).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) const fn next_id(&self) -> u32 {
        self.next
    }

    pub(crate) fn set_next(&mut self, next: u32) {
        self.next = next;
    }

    pub(crate) fn plan(&mut self, view: ViewId) {
        if self.get(view).is_some() {
            return;
        }

        let id = LayoutNodeId::from_index(self.next);
        self.next = self.next.saturating_add(1);
        self.planned.insert(view, id);
    }

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

    pub(crate) fn commit(&mut self, unmounts: &[ViewId]) {
        for view in unmounts {
            self.entries.remove(view);
            self.planned.remove(view);
        }
        let planned = std::mem::take(&mut self.planned);
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
