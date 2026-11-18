// crates/core/src/transaction.rs
//! Minimal batch/transaction shell for P1.
//!
//! `SPEC-RT-033` requires nesting depth so only the outermost exit commits.
//! P1 does exactly that counter bookkeeping; the parts A0-04 actually measures
//! — no intermediate value escaping a nested batch, exactly one commit per
//! interaction, no synchronous flush inside a binding — are P2 work and are
//! **not** claimed by this module.
//!
//! TODO(A0-04): a nested batch may currently publish intermediate values to any
//! reader that evaluates inside the open batch. Fixing that needs the staging
//! journal of `SPEC-RT-051` and is out of P1 scope.
//!
//! TODO(A0-06): there is no preflight journal and no staging tree. A rejected
//! operation simply leaves the store untouched; that is the observable effect
//! A0-06 wants, but not its proof, because nothing is preflighted and no
//! failure is injected.

use std::collections::BTreeSet;

use crate::graph::NodeId;

/// Depth counter and commit log for one runtime's frame transactions.
#[derive(Debug, Default)]
pub struct TransactionState {
    depth: u32,
    commits: u64,
    pending: BTreeSet<NodeId>,
}

impl TransactionState {
    pub const fn new() -> Self {
        Self {
            depth: 0,
            commits: 0,
            pending: BTreeSet::new(),
        }
    }

    pub fn begin(&mut self) {
        self.depth += 1;
    }

    /// Close one nesting level. Returns the nodes dirtied by the outermost
    /// batch, or `None` while still nested.
    ///
    /// An end without a matching begin is a bug, not a transaction: debug
    /// builds assert, and release builds refuse to record a commit.
    pub fn end(&mut self) -> Option<Vec<NodeId>> {
        debug_assert!(self.depth > 0, "unbalanced transaction end");
        if self.depth == 0 {
            return None;
        }
        self.depth -= 1;
        if self.depth == 0 {
            self.commits += 1;
            Some(std::mem::take(&mut self.pending).into_iter().collect())
        } else {
            None
        }
    }

    pub const fn is_open(&self) -> bool {
        self.depth > 0
    }

    pub const fn depth(&self) -> u32 {
        self.depth
    }

    pub const fn commits(&self) -> u64 {
        self.commits
    }

    /// Record a node dirtied by the current transaction.
    pub fn note_dirty(&mut self, node: NodeId) {
        self.pending.insert(node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_outermost_exit_commits() {
        let mut state = TransactionState::new();
        state.begin();
        state.begin();
        state.note_dirty(1);
        assert!(state.end().is_none());
        assert_eq!(state.commits(), 0);
        assert!(state.is_open());

        state.begin();
        state.note_dirty(2);
        assert!(state.end().is_none());

        let committed = state.end().expect("outermost batch commits");
        assert_eq!(committed, vec![1, 2], "deterministic node order");
        assert_eq!(state.commits(), 1);
        assert!(!state.is_open());
    }

    /// Debug builds assert on an unbalanced end; the point of the assertion is
    /// that the panic happens *before* any commit is recorded.
    #[cfg(debug_assertions)]
    #[test]
    fn unbalanced_end_panics_without_committing() {
        let mut state = TransactionState::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| state.end()));
        assert!(result.is_err(), "debug builds assert on an unbalanced end");
        assert_eq!(state.commits(), 0, "a refused end must not commit");
    }

    /// Release builds skip the assertion, so this is where the "refuse to
    /// commit" branch is actually exercised.
    #[cfg(not(debug_assertions))]
    #[test]
    fn unbalanced_end_does_not_commit() {
        let mut state = TransactionState::new();
        assert_eq!(state.end(), None);
        assert_eq!(state.commits(), 0);
    }
}
