// crates/core/src/view.rs
//! Retained view tree, focus and caret state (`SPEC-RT-001`, `SPEC-RT-003`,
//! `SPEC-RT-026`).
//!
//! P1 keeps the smallest tree that can express the A0-01 scenario: a root, row
//! views keyed by `RowKey`, one focus pointer and per-row caret offsets.
//!
//! TODO(A0-09): reorder preserves `ViewId` here, but duplicate-key rejection and
//! the "no unmount/setup on reorder" accounting are P3 work.
//! TODO(A0-10): focus settlement records a `FocusIntent` (next → previous) and
//! settles it once after the structure mutation. The full order (toolbar/input
//! fallback) and cancellation by a user focus move are unimplemented.

use std::collections::BTreeMap;
use std::fmt;

use crate::handle::{Generation, OwnerId, RowKey, ViewId};
use crate::metrics::Counters;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewKind {
    Root,
    Row,
}

/// One retained view node. `generation` changes only on remount (`SPEC-RT-003`).
#[derive(Debug)]
pub struct ViewNode {
    pub id: ViewId,
    pub generation: Generation,
    pub kind: ViewKind,
    pub owner: OwnerId,
    pub row: Option<RowKey>,
    /// Cached title length used to place the caret without re-reading the title.
    pub title_len: u32,
}

/// Captured before a structural mutation, settled after the commit
/// (`SPEC-RT-026`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FocusIntent {
    pub next: Option<ViewId>,
    pub previous: Option<ViewId>,
}

/// Retained view tree with focus and caret state.
pub struct ViewTree {
    next: u32,
    nodes: BTreeMap<ViewId, ViewNode>,
    order: Vec<ViewId>,
    root: ViewId,
    focused: Option<ViewId>,
    carets: BTreeMap<ViewId, usize>,
    mounts: u64,
    unmounts: u64,
    focus_moves: u64,
    caret_ops: u64,
    geometry_epoch: u64,
}

impl ViewTree {
    /// Create the tree with its root view. The root is not counted as a mount:
    /// it exists for the lifetime of the runtime.
    pub fn new(root_owner: OwnerId) -> Self {
        let root = ViewId::from_index(0);
        let mut nodes = BTreeMap::new();
        nodes.insert(
            root,
            ViewNode {
                id: root,
                generation: Generation::FIRST,
                kind: ViewKind::Root,
                owner: root_owner,
                row: None,
                title_len: 0,
            },
        );
        Self {
            next: 1,
            nodes,
            order: Vec::new(),
            root,
            focused: None,
            carets: BTreeMap::new(),
            mounts: 0,
            unmounts: 0,
            focus_moves: 0,
            caret_ops: 0,
            geometry_epoch: 1,
        }
    }

    pub const fn root(&self) -> ViewId {
        self.root
    }

    /// Next free view id. A frame draft starts here so a rollback does not
    /// consume ids (`SPEC-RT-053` keeps the high-water mark out of the
    /// comparison set's mutation).
    pub(crate) const fn next_id(&self) -> u32 {
        self.next
    }

    /// Apply a mount whose id was reserved by a transaction draft.
    pub(crate) fn mount_row_with_id(
        &mut self,
        id: ViewId,
        row: RowKey,
        owner: OwnerId,
        title_len: u32,
    ) -> ViewId {
        self.nodes.insert(
            id,
            ViewNode {
                id,
                generation: Generation::FIRST,
                kind: ViewKind::Row,
                owner,
                row: Some(row),
                title_len,
            },
        );
        self.mounts += 1;
        self.geometry_epoch += 1;
        id
    }

    pub(crate) const fn set_next(&mut self, next: u32) {
        self.next = next;
    }

    /// Mount one row view. A remount never reuses an id in P1, so a stale
    /// `ViewId` can never point at a different row.
    ///
    /// TODO(A0-03): monotonic ids are a *stronger* guarantee than the gate
    /// requires and deliberately skip generation reuse. A0-03 also wants a
    /// remounted scope to receive a new generation while the id stays stable,
    /// plus a stale-click dispatch fixture; both are P2 work.
    pub fn mount_row(&mut self, row: RowKey, owner: OwnerId, title_len: u32) -> ViewId {
        let id = ViewId::from_index(self.next);
        self.next += 1;
        self.mount_row_with_id(id, row, owner, title_len)
    }

    pub fn unmount(&mut self, view: ViewId) -> Option<ViewNode> {
        let node = self.nodes.remove(&view)?;
        self.order.retain(|candidate| *candidate != view);
        self.carets.remove(&view);
        // Focus is not cleared here: the transaction settles the recorded
        // intent after the swap, so a removal and its focus fallback count as
        // one observable move (`SPEC-GATE-A0-10`).
        self.unmounts += 1;
        self.geometry_epoch += 1;
        Some(node)
    }

    /// Set the row order. Unknown ids are ignored; the epoch only advances when
    /// the order actually changed.
    pub fn reorder(&mut self, order: &[ViewId]) {
        // TODO(A0-09): duplicates and unknown ids must be rejected before any
        // patch is applied; P1 filters instead of failing.
        let mut next = Vec::with_capacity(order.len());
        for id in order {
            if self.nodes.contains_key(id) && !next.contains(id) {
                next.push(*id);
            }
        }
        if self.order != next {
            self.order = next;
            self.geometry_epoch += 1;
        }
    }

    /// Move focus to a row view and place its caret at the cached title length.
    pub fn focus(&mut self, view: ViewId) {
        let Some(node) = self.nodes.get(&view) else {
            return;
        };
        if self.focused != Some(view) {
            self.focused = Some(view);
            self.focus_moves += 1;
        }
        let caret = node.title_len as usize;
        self.carets.insert(view, caret);
        self.caret_ops += 1;
    }

    /// Place the caret without moving focus (`SPEC-TEXT-012`).
    pub fn set_caret(&mut self, view: ViewId, offset: usize) {
        if self.nodes.contains_key(&view) {
            self.carets.insert(view, offset);
            self.caret_ops += 1;
        }
    }

    pub fn clear_focus(&mut self) {
        if self.focused.take().is_some() {
            self.focus_moves += 1;
        }
    }

    pub const fn focused(&self) -> Option<ViewId> {
        self.focused
    }

    pub fn caret(&self, view: ViewId) -> Option<usize> {
        self.carets.get(&view).copied()
    }

    /// Neighbours that a removal of `view` would hand focus to.
    pub fn focus_intent_for(&self, view: ViewId) -> Option<FocusIntent> {
        let index = self.order.iter().position(|candidate| *candidate == view)?;
        Some(FocusIntent {
            next: self.order.get(index + 1).copied(),
            previous: index
                .checked_sub(1)
                .and_then(|previous| self.order.get(previous).copied()),
        })
    }

    pub fn is_live(&self, view: ViewId) -> bool {
        self.nodes.contains_key(&view)
    }

    pub fn order(&self) -> &[ViewId] {
        &self.order
    }

    /// Mounted row views in display order, paired with their row key.
    pub fn row_views(&self) -> Vec<(ViewId, RowKey)> {
        self.order
            .iter()
            .filter_map(|id| {
                let node = self.nodes.get(id)?;
                Some((*id, node.row?))
            })
            .collect()
    }

    pub fn view_of_row(&self, row: RowKey) -> Option<ViewId> {
        self.order
            .iter()
            .copied()
            .find(|id| self.nodes.get(id).is_some_and(|node| node.row == Some(row)))
    }

    pub fn node(&self, view: ViewId) -> Option<&ViewNode> {
        self.nodes.get(&view)
    }

    pub const fn geometry_epoch(&self) -> u64 {
        self.geometry_epoch
    }

    pub fn counters(&self) -> Counters {
        Counters {
            mounts: self.mounts,
            unmounts: self.unmounts,
            focus_moves: self.focus_moves,
            caret_ops: self.caret_ops,
            ..Counters::default()
        }
    }
}

impl fmt::Debug for ViewTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewTree")
            .field("nodes", &self.nodes.len())
            .field("order", &self.order)
            .field("focused", &self.focused)
            .field("geometry_epoch", &self.geometry_epoch)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::OwnerId;

    fn tree() -> ViewTree {
        ViewTree::new(OwnerId::from_index(0))
    }

    fn key(raw: u64) -> RowKey {
        RowKey::from_raw(raw)
    }

    fn owner() -> OwnerId {
        OwnerId::from_index(0)
    }

    #[test]
    fn reorder_preserves_view_ids() {
        let mut tree = tree();
        let a = tree.mount_row(key(1), owner(), 1);
        let b = tree.mount_row(key(2), owner(), 1);
        tree.reorder(&[a, b]);
        let epoch = tree.geometry_epoch();

        tree.reorder(&[b, a]);

        assert_eq!(tree.order(), &[b, a]);
        assert_eq!(tree.counters().mounts, 2);
        assert_eq!(tree.counters().unmounts, 0);
        assert!(tree.geometry_epoch() > epoch);
    }

    #[test]
    fn reorder_without_change_does_not_advance_geometry() {
        let mut tree = tree();
        let a = tree.mount_row(key(1), owner(), 1);
        tree.reorder(&[a]);
        let epoch = tree.geometry_epoch();
        tree.reorder(&[a]);
        assert_eq!(tree.geometry_epoch(), epoch);
    }

    #[test]
    fn focus_intent_reports_both_neighbours() {
        let mut tree = tree();
        let a = tree.mount_row(key(1), owner(), 1);
        let b = tree.mount_row(key(2), owner(), 1);
        let c = tree.mount_row(key(3), owner(), 1);
        tree.reorder(&[a, b, c]);

        let middle = tree.focus_intent_for(b).expect("mounted");
        assert_eq!(middle.next, Some(c));
        assert_eq!(middle.previous, Some(a));

        let last = tree.focus_intent_for(c).expect("mounted");
        assert_eq!(last.next, None);
        assert_eq!(last.previous, Some(b));
    }

    #[test]
    fn focus_places_caret_at_title_length() {
        let mut tree = tree();
        let a = tree.mount_row(key(1), owner(), 7);
        tree.focus(a);
        assert_eq!(tree.focused(), Some(a));
        assert_eq!(tree.caret(a), Some(7));
        assert_eq!(tree.counters().caret_ops, 1);
    }

    #[test]
    fn unmount_clears_caret_and_leaves_focus_for_the_transaction() {
        let mut tree = tree();
        let a = tree.mount_row(key(1), owner(), 1);
        tree.focus(a);
        assert!(tree.unmount(a).is_some());
        // The pointer stays until the transaction settles the recorded intent,
        // so the removal and its fallback are one observable move.
        assert_eq!(tree.focused(), Some(a));
        assert!(!tree.is_live(a));
        assert_eq!(tree.caret(a), None);
    }
}
