// crates/core/src/owner.rs
//! Owner scope tree and teardown order (`SPEC-RT-020`, `SPEC-RT-021`).
//!
//! Disposal is post-order (children before parent) and, inside one owner,
//! LIFO over registered cleanups. `dispose` is idempotent. Mounting into a
//! scope that is currently disposing is rejected rather than silently growing a
//! tree during teardown.
//!
//! TODO(A0-11): P1 counts live owners but does not verify that every edge and
//! subscription bucket returns to baseline after churn. Empty subscriber buckets
//! are not yet modelled at all.

use std::collections::BTreeMap;
use std::fmt;

use crate::handle::OwnerId;

type Cleanup = Box<dyn FnOnce()>;

#[derive(Debug)]
pub enum OwnerError {
    UnknownOwner(OwnerId),
    AlreadyDisposed(OwnerId),
    /// `SPEC-RT-021`: cleanup must not mount.
    MountDuringDispose(OwnerId),
}

impl fmt::Display for OwnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OwnerError::UnknownOwner(owner) => write!(f, "unknown owner {owner:?}"),
            OwnerError::AlreadyDisposed(owner) => write!(f, "owner {owner:?} is disposed"),
            OwnerError::MountDuringDispose(owner) => {
                write!(f, "owner {owner:?} is disposing; mount is rejected")
            }
        }
    }
}

impl std::error::Error for OwnerError {}

struct OwnerNode {
    parent: Option<OwnerId>,
    children: Vec<OwnerId>,
    cleanups: Vec<Cleanup>,
    disposed: bool,
    disposing: bool,
}

/// Arena of owner scopes. The root is created by [`OwnerTree::new`].
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
                disposed: false,
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

    pub fn child_of(&mut self, parent: OwnerId) -> Result<OwnerId, OwnerError> {
        let Some(node) = self.nodes.get(&parent) else {
            return Err(OwnerError::UnknownOwner(parent));
        };
        if node.disposed {
            return Err(OwnerError::AlreadyDisposed(parent));
        }
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
                disposed: false,
                disposing: false,
            },
        );
        // The parent was checked above, so the entry exists.
        if let Some(parent_node) = self.nodes.get_mut(&parent) {
            parent_node.children.push(id);
        }
        self.live += 1;
        Ok(id)
    }

    pub fn on_cleanup(&mut self, owner: OwnerId, cleanup: Cleanup) {
        if let Some(node) = self.nodes.get_mut(&owner) {
            node.cleanups.push(cleanup);
        }
    }

    /// Dispose a scope and its subtree. Idempotent.
    pub fn dispose(&mut self, owner: OwnerId) {
        let already_gone = self
            .nodes
            .get(&owner)
            .is_none_or(|node| node.disposed || node.disposing);
        if already_gone {
            return;
        }
        self.dispose_recursive(owner);
    }

    fn dispose_recursive(&mut self, owner: OwnerId) {
        // Post-order: release the newest child scope first.
        let children = {
            let Some(node) = self.nodes.get_mut(&owner) else {
                return;
            };
            node.disposing = true;
            let mut children = std::mem::take(&mut node.children);
            children.reverse();
            children
        };
        for child in children {
            self.dispose_recursive(child);
        }

        let cleanups = {
            let Some(node) = self.nodes.get_mut(&owner) else {
                return;
            };
            let mut cleanups = std::mem::take(&mut node.cleanups);
            cleanups.reverse();
            cleanups
        };
        for cleanup in cleanups {
            cleanup();
            self.cleanup_runs += 1;
        }

        let parent = self.nodes.get_mut(&owner).and_then(|node| {
            node.disposed = true;
            node.disposing = false;
            node.parent
        });
        if let Some(parent) = parent
            && let Some(parent_node) = self.nodes.get_mut(&parent)
        {
            parent_node.children.retain(|child| *child != owner);
        }
        self.live = self.live.saturating_sub(1);
    }

    pub fn is_disposed(&self, owner: OwnerId) -> bool {
        self.nodes.get(&owner).is_none_or(|node| node.disposed)
    }

    pub const fn live(&self) -> usize {
        self.live
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
        owners.on_cleanup(
            child,
            Box::new(move || child_log.borrow_mut().push("child")),
        );

        let first = Rc::clone(&log);
        owners.on_cleanup(
            parent,
            Box::new(move || first.borrow_mut().push("parent-first")),
        );
        let second = Rc::clone(&log);
        owners.on_cleanup(
            parent,
            Box::new(move || second.borrow_mut().push("parent-second")),
        );

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
        owners.on_cleanup(scope, Box::new(move || *counter.borrow_mut() += 1));

        owners.dispose(scope);
        owners.dispose(scope);
        assert_eq!(*runs.borrow(), 1);
    }

    #[test]
    fn mount_into_a_disposing_scope_is_rejected() {
        let mut owners = OwnerTree::new();
        let scope = owners.child_of(owners.root()).expect("child");

        // `dispose` sets `disposing` before it runs cleanups, so a cleanup that
        // tries to mount hits this branch. Drive the flag directly to keep the
        // test free of a self-referential cleanup handle.
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
}
