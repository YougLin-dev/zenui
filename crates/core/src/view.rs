use std::collections::BTreeMap;
use std::fmt;

use crate::handle::{Generation, OwnerId, RowKey, ViewId};
use crate::metrics::Counters;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewKind {
    Root,
    Row,
}

#[derive(Debug)]
pub struct ViewNode {
    pub id: ViewId,
    pub generation: Generation,
    pub kind: ViewKind,
    pub owner: OwnerId,
    pub row: Option<RowKey>,

    pub title_len: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FocusIntent {
    pub next: Option<ViewId>,
    pub previous: Option<ViewId>,
}

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

    pub(crate) const fn next_id(&self) -> u32 {
        self.next
    }

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

    pub fn mount_row(&mut self, row: RowKey, owner: OwnerId, title_len: u32) -> ViewId {
        let id = ViewId::from_index(self.next);
        self.next += 1;
        self.mount_row_with_id(id, row, owner, title_len)
    }

    /// 同时解除焦点关联和 caret；row 的业务生命周期不随控件卸载而改变。
    pub fn unmount(&mut self, view: ViewId) -> Option<ViewNode> {
        let node = self.nodes.remove(&view)?;
        self.order.retain(|candidate| *candidate != view);
        self.carets.remove(&view);

        self.unmounts += 1;
        self.geometry_epoch += 1;
        Some(node)
    }

    pub fn reorder(&mut self, order: &[ViewId]) {
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

    pub(crate) fn claiming_view(&self, row: RowKey) -> Option<ViewId> {
        self.nodes
            .values()
            .find(|node| node.row == Some(row))
            .map(|node| node.id)
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
    fn a_reorder_preserves_every_generation() {
        let mut tree = tree();
        let a = tree.mount_row(key(1), owner(), 1);
        let b = tree.mount_row(key(2), owner(), 1);
        let c = tree.mount_row(key(3), owner(), 1);
        let generations = |tree: &ViewTree, ids: [ViewId; 3]| -> BTreeMap<ViewId, u32> {
            ids.into_iter()
                .map(|id| {
                    let node = tree.node(id).expect("mounted");
                    (id, node.generation.raw())
                })
                .collect()
        };
        let before = generations(&tree, [a, b, c]);

        tree.reorder(&[c, a, b]);

        assert_eq!(
            tree.order(),
            &[c, a, b],
            "the fixture must reorder for real"
        );
        assert_eq!(
            generations(&tree, [a, b, c]),
            before,
            "a reorder must not change any view generation"
        );
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

        assert_eq!(tree.focused(), Some(a));
        assert!(!tree.is_live(a));
        assert_eq!(tree.caret(a), None);
    }

    #[test]
    fn a_reorder_preserves_each_views_owner() {
        let mut tree = tree();
        let a = tree.mount_row(key(1), OwnerId::from_index(1), 1);
        let b = tree.mount_row(key(2), OwnerId::from_index(2), 1);
        let c = tree.mount_row(key(3), OwnerId::from_index(3), 1);
        tree.reorder(&[a, b, c]);

        let owners_before: Vec<(ViewId, OwnerId)> = tree
            .row_views()
            .into_iter()
            .map(|(view, _)| (view, tree.node(view).expect("node").owner))
            .collect();

        tree.reorder(&[c, b, a]);

        for (view, owner) in &owners_before {
            assert_eq!(
                tree.node(*view).expect("node").owner,
                *owner,
                "a same-owner reorder must not reassign ownership"
            );
        }
        assert_eq!(tree.order(), &[c, b, a]);
    }
}
