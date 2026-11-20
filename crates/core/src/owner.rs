use std::collections::BTreeMap;
use std::fmt;

use crate::handle::OwnerId;

type Cleanup = Box<dyn FnOnce()>;

#[derive(Debug)]
pub enum OwnerError {
    UnknownOwner(OwnerId),
    AlreadyDisposed(OwnerId),

    MountDuringDispose(OwnerId),

    CleanupDuringDispose(OwnerId),
}

impl fmt::Display for OwnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OwnerError::UnknownOwner(owner) => write!(f, "unknown owner {owner:?}"),
            OwnerError::AlreadyDisposed(owner) => write!(f, "owner {owner:?} is disposed"),
            OwnerError::MountDuringDispose(owner) => {
                write!(f, "owner {owner:?} is disposing; mount is rejected")
            }

            OwnerError::CleanupDuringDispose(owner) => {
                write!(
                    f,
                    "owner {owner:?} is disposing; cleanup registration is rejected"
                )
            }
        }
    }
}

impl std::error::Error for OwnerError {}

struct OwnerNode {
    parent: Option<OwnerId>,
    children: Vec<OwnerId>,
    cleanups: Vec<Cleanup>,
    disposing: bool,
}

pub struct OwnerTree {
    nodes: BTreeMap<OwnerId, OwnerNode>,
    root: OwnerId,
    next: u32,
    live: usize,
    cleanup_runs: u64,
}

impl OwnerTree {
    pub fn new() -> Self {
        let root = OwnerId::from_index(0);
        let mut nodes = BTreeMap::new();
        nodes.insert(
            root,
            OwnerNode {
                parent: None,
                children: Vec::new(),
                cleanups: Vec::new(),
                disposing: false,
            },
        );
        Self {
            nodes,
            root,
            next: 1,
            live: 1,
            cleanup_runs: 0,
        }
    }

    pub const fn root(&self) -> OwnerId {
        self.root
    }

    pub(crate) const fn set_next(&mut self, next: u32) {
        self.next = next;
    }

    pub(crate) fn live_ids(&self) -> Vec<crate::handle::OwnerId> {
        self.nodes.keys().copied().collect()
    }

    pub(crate) const fn next_id(&self) -> u32 {
        self.next
    }

    pub(crate) fn insert_staged(&mut self, id: OwnerId, parent: OwnerId) -> bool {
        if self.nodes.contains_key(&id) || !self.nodes.contains_key(&parent) {
            return false;
        }
        self.nodes.insert(
            id,
            OwnerNode {
                parent: Some(parent),
                children: Vec::new(),
                cleanups: Vec::new(),
                disposing: false,
            },
        );
        if let Some(parent_node) = self.nodes.get_mut(&parent) {
            parent_node.children.push(id);
        }
        self.live += 1;
        true
    }

    pub fn child_of(&mut self, parent: OwnerId) -> Result<OwnerId, OwnerError> {
        let Some(node) = self.nodes.get(&parent) else {
            return Err(self.missing_owner_error(parent));
        };
        if node.disposing {
            return Err(OwnerError::MountDuringDispose(parent));
        }

        let id = OwnerId::from_index(self.next);
        self.next += 1;
        self.nodes.insert(
            id,
            OwnerNode {
                parent: Some(parent),
                children: Vec::new(),
                cleanups: Vec::new(),
                disposing: false,
            },
        );

        if let Some(parent_node) = self.nodes.get_mut(&parent) {
            parent_node.children.push(id);
        }
        self.live += 1;
        Ok(id)
    }

    pub fn on_cleanup(&mut self, owner: OwnerId, cleanup: Cleanup) -> Result<(), OwnerError> {
        let Some(node) = self.nodes.get_mut(&owner) else {
            return Err(self.missing_owner_error(owner));
        };
        if node.disposing {
            return Err(OwnerError::CleanupDuringDispose(owner));
        }
        node.cleanups.push(cleanup);
        Ok(())
    }

    pub fn dispose(&mut self, owner: OwnerId) {
        let already_gone = self.nodes.get(&owner).is_none_or(|node| node.disposing);
        if already_gone {
            return;
        }
        self.dispose_subtree(owner);
    }

    /// 释放使用独立栈，父级在全部后代退出前保持未释放状态。
    fn dispose_subtree(&mut self, owner: OwnerId) {
        let mut pending = vec![owner];
        while let Some(current) = pending.pop() {
            let Some(node) = self.nodes.get_mut(&current) else {
                continue;
            };
            if node.disposing {
                let mut cleanups = std::mem::take(&mut node.cleanups);
                cleanups.reverse();
                for cleanup in cleanups {
                    cleanup();
                    self.cleanup_runs += 1;
                }

                let parent = self.nodes.get(&current).and_then(|node| node.parent);
                if let Some(parent) = parent
                    && let Some(parent_node) = self.nodes.get_mut(&parent)
                {
                    parent_node.children.retain(|child| *child != current);
                }
                // 已释放的 Owner 只保留在 issued 水位之下，不继续占用节点存储。
                self.nodes.remove(&current);
                self.live = self.live.saturating_sub(1);
                continue;
            }
            node.disposing = true;
            pending.push(current);
            for child in std::mem::take(&mut node.children).into_iter().rev() {
                pending.push(child);
            }
        }
    }

    /// 已保留并释放的 Owner 为 disposed；水位以上从未保留的 id 不混为 disposed。
    pub fn is_disposed(&self, owner: OwnerId) -> bool {
        !self.nodes.contains_key(&owner) && self.was_reserved(owner)
    }

    /// 水位以下代表该编号已经被保留；它可能从未发布就随失败计划消失。
    const fn was_reserved(&self, owner: OwnerId) -> bool {
        owner.index() < self.next
    }

    fn missing_owner_error(&self, owner: OwnerId) -> OwnerError {
        if self.was_reserved(owner) {
            OwnerError::AlreadyDisposed(owner)
        } else {
            OwnerError::UnknownOwner(owner)
        }
    }

    pub const fn live(&self) -> usize {
        self.live
    }

    /// 已分配节点存储；释放后节点应离开该存储，而不是只改变状态位。
    pub fn storage_len(&self) -> usize {
        self.nodes.len()
    }

    pub const fn cleanup_runs(&self) -> u64 {
        self.cleanup_runs
    }
}

impl Default for OwnerTree {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for OwnerTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnerTree")
            .field("live", &self.live)
            .field("cleanup_runs", &self.cleanup_runs)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn disposal_is_post_order_and_lifo() {
        let mut owners = OwnerTree::new();
        let log = Rc::new(RefCell::new(Vec::new()));

        let root = owners.root();
        let parent = owners.child_of(root).expect("child");
        let child = owners.child_of(parent).expect("child");

        let child_log = Rc::clone(&log);
        owners
            .on_cleanup(
                child,
                Box::new(move || child_log.borrow_mut().push("child")),
            )
            .unwrap();

        let first = Rc::clone(&log);
        owners
            .on_cleanup(
                parent,
                Box::new(move || first.borrow_mut().push("parent-first")),
            )
            .unwrap();
        let second = Rc::clone(&log);
        owners
            .on_cleanup(
                parent,
                Box::new(move || second.borrow_mut().push("parent-second")),
            )
            .unwrap();

        owners.dispose(parent);

        assert_eq!(
            *log.borrow(),
            vec!["child", "parent-second", "parent-first"],
            "subtree first, then LIFO within one owner"
        );
        assert!(owners.is_disposed(child));
        assert!(owners.is_disposed(parent));
        assert_eq!(owners.live(), 1, "root survives");
    }

    #[test]
    fn dispose_is_idempotent() {
        let mut owners = OwnerTree::new();
        let scope = owners.child_of(owners.root()).expect("child");
        let runs = Rc::new(RefCell::new(0));
        let counter = Rc::clone(&runs);
        owners
            .on_cleanup(scope, Box::new(move || *counter.borrow_mut() += 1))
            .unwrap();

        owners.dispose(scope);
        owners.dispose(scope);
        assert_eq!(*runs.borrow(), 1);
    }

    #[test]
    fn disposed_nodes_leave_storage_but_keep_their_identity() {
        let mut owners = OwnerTree::new();
        let scope = owners.child_of(owners.root()).expect("child");
        let child = owners.child_of(scope).expect("grandchild");
        assert_eq!(owners.storage_len(), 3, "root, scope, child");

        owners.dispose(scope);
        assert_eq!(owners.live(), 1, "root survives");
        assert_eq!(owners.storage_len(), 1, "disposed nodes are not retained");
        assert!(owners.is_disposed(scope));
        assert!(owners.is_disposed(child));

        let never_issued = OwnerId::from_index(4096);
        assert!(!owners.is_disposed(never_issued));
        assert!(matches!(
            owners.child_of(scope),
            Err(OwnerError::AlreadyDisposed(id)) if id == scope
        ));
        assert!(matches!(
            owners.child_of(never_issued),
            Err(OwnerError::UnknownOwner(id)) if id == never_issued
        ));

        for _ in 0..1000 {
            let scope = owners.child_of(owners.root()).expect("churn child");
            owners.dispose(scope);
        }
        assert_eq!(owners.live(), 1);
        assert_eq!(owners.storage_len(), owners.live());
    }

    #[test]
    fn cleanup_registration_rejects_disposing_and_disposed_owners() {
        let mut owners = OwnerTree::new();
        let unknown = OwnerId::from_index(4096);
        assert!(matches!(
            owners.on_cleanup(unknown, Box::new(|| {})),
            Err(OwnerError::UnknownOwner(id)) if id == unknown
        ));
        let scope = owners.child_of(owners.root()).expect("child");
        owners.nodes.get_mut(&scope).expect("scope").disposing = true;
        let dropped = Rc::new(());
        let capture = Rc::clone(&dropped);
        assert!(matches!(
            owners.on_cleanup(scope, Box::new(move || drop(capture))),
            Err(OwnerError::CleanupDuringDispose(id)) if id == scope
        ));
        assert_eq!(
            Rc::strong_count(&dropped),
            1,
            "a rejected callback is dropped immediately"
        );

        owners.nodes.get_mut(&scope).expect("scope").disposing = false;
        owners.dispose(scope);
        assert!(matches!(
            owners.on_cleanup(scope, Box::new(|| {})),
            Err(OwnerError::AlreadyDisposed(id)) if id == scope
        ));
        assert_eq!(owners.cleanup_runs(), 0);
    }

    #[test]
    fn disposing_flag_rejects_child_creation_white_box() {
        let mut owners = OwnerTree::new();
        let scope = owners.child_of(owners.root()).expect("child");

        owners.nodes.get_mut(&scope).expect("scope").disposing = true;
        assert!(matches!(
            owners.child_of(scope),
            Err(OwnerError::MountDuringDispose(_))
        ));

        owners.nodes.get_mut(&scope).expect("scope").disposing = false;
        owners.dispose(scope);
        assert!(matches!(
            owners.child_of(scope),
            Err(OwnerError::AlreadyDisposed(_))
        ));
    }

    #[test]
    fn a_deep_owner_chain_is_disposed_without_call_stack_recursion() {
        const DEPTH: usize = 10_000;
        let mut owners = OwnerTree::new();
        let root = owners.root();
        let deepest = (0..DEPTH).fold(root, |parent, _| owners.child_of(parent).unwrap());
        let log = Rc::new(RefCell::new(Vec::new()));
        let child_log = Rc::clone(&log);
        owners
            .on_cleanup(
                deepest,
                Box::new(move || child_log.borrow_mut().push("child")),
            )
            .unwrap();
        let parent_log = Rc::clone(&log);
        owners
            .on_cleanup(
                root,
                Box::new(move || parent_log.borrow_mut().push("parent")),
            )
            .unwrap();
        owners.dispose(root);
        assert_eq!(*log.borrow(), vec!["child", "parent"]);
        assert_eq!(owners.live(), 0);
        assert!(owners.is_disposed(deepest));
    }
}
