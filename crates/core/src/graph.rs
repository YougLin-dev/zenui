// crates/core/src/graph.rs
//! Minimal tracked memo graph (`SPEC-RT-030`, `SPEC-RT-031`, `SPEC-RT-032`).
//!
//! Nodes are either sources (bumped by writes) or memos (pulled on read).
//! Dependencies are recorded by [`DepReader`] during a compute, so a memo only
//! tracks what its body actually read, mirroring `SPEC-RT-042`'s "actual read"
//! rule for queries.
//!
//! P1 scope: dirty propagation, lazy recompute, equality suppression at the memo
//! boundary, and edge counting. Glitch-free topological publication, cycle
//! detection and `WriteDuringCompute` as a domain error are `SPEC-GATE-A0-05`
//! and remain TODO — see the notes on [`MemoGraph::compute`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::metrics::Counters;
use crate::query::RowSet;
use crate::transaction::TransactionState;

/// Dense internal index of a graph node. Not an identity domain: it is never
/// exposed outside the runtime and never printed in a trace.
pub type NodeId = u32;

/// A memo value. Equality drives `SPEC-RT-032` suppression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Rows(RowSet),
    Count(u32),
    Flag(bool),
}

impl Value {
    const fn kind(&self) -> &'static str {
        match self {
            Value::Rows(_) => "rows",
            Value::Count(_) => "count",
            Value::Flag(_) => "flag",
        }
    }
}

#[derive(Debug)]
pub enum GraphError {
    UnknownNode(NodeId),
    /// A memo was declared but never computed before being read.
    Uncomputed(NodeId),
    /// The memo body returned a value of a different shape than before.
    ValueKindChanged {
        node: NodeId,
        expected: &'static str,
    },
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphError::UnknownNode(node) => write!(f, "unknown graph node {node}"),
            GraphError::Uncomputed(node) => write!(f, "graph node {node} has no value"),
            GraphError::ValueKindChanged { node, expected } => {
                write!(f, "graph node {node} no longer yields a {expected} value")
            }
        }
    }
}

impl std::error::Error for GraphError {}

#[derive(Debug)]
struct Node {
    label: &'static str,
    revision: u64,
    dirty: bool,
}

/// Read-only view handed to a memo body.
///
/// It can only read; there is no publish path, which is why P1 cannot express
/// `WriteDuringCompute`. The real phase guard lands with `SPEC-GATE-A0-05`.
pub struct DepReader<'a> {
    values: &'a [Option<Value>],
    nodes: &'a [Node],
    deps: Vec<NodeId>,
}

impl<'a> DepReader<'a> {
    /// Read a dependency and record the edge. Returns `None` for sources that
    /// carry no value yet, but still records the dependency.
    pub fn read(&mut self, node: NodeId) -> Option<&'a Value> {
        let values = self.values;
        let index = node as usize;
        if index >= values.len() {
            return None;
        }
        if !self.deps.contains(&node) {
            self.deps.push(node);
        }
        values[index].as_ref()
    }

    pub fn is_dirty(&self, node: NodeId) -> bool {
        self.nodes
            .get(node as usize)
            .is_some_and(|state| state.dirty)
    }

    pub fn deps(&self) -> &[NodeId] {
        &self.deps
    }
}

/// Source/memo graph with deterministic iteration (no `HashMap`).
pub struct MemoGraph {
    nodes: Vec<Node>,
    values: Vec<Option<Value>>,
    /// node -> dependents
    forward: BTreeMap<NodeId, BTreeSet<NodeId>>,
    /// node -> dependencies
    reverse: BTreeMap<NodeId, BTreeSet<NodeId>>,
    /// node -> the dependency revisions observed at its last compute. A dirty
    /// memo whose dependencies all still carry these revisions can serve its
    /// cached value instead of recomputing (`SPEC-RT-032` suppression).
    observed: Vec<BTreeMap<NodeId, u64>>,
    tx: TransactionState,
    memo_recomputes: u64,
}

impl MemoGraph {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            values: Vec::new(),
            forward: BTreeMap::new(),
            reverse: BTreeMap::new(),
            observed: Vec::new(),
            tx: TransactionState::new(),
            memo_recomputes: 0,
        }
    }

    /// Declare a source node. Sources carry a revision, not a value.
    pub fn source(&mut self, label: &'static str) -> NodeId {
        self.push_node(label, false)
    }

    /// Declare a memo node. Memos start dirty and compute on first read.
    pub fn memo(&mut self, label: &'static str) -> NodeId {
        self.push_node(label, true)
    }

    fn push_node(&mut self, label: &'static str, dirty: bool) -> NodeId {
        let id = u32::try_from(self.nodes.len()).expect("graph exhausted");
        self.nodes.push(Node {
            label,
            revision: 0,
            dirty,
        });
        self.values.push(None);
        self.observed.push(BTreeMap::new());
        id
    }

    pub fn label(&self, node: NodeId) -> Option<&'static str> {
        self.nodes.get(node as usize).map(|state| state.label)
    }

    /// Bump a source revision and mark everything downstream dirty.
    pub fn bump_source(&mut self, node: NodeId) {
        if let Some(state) = self.nodes.get_mut(node as usize) {
            state.revision += 1;
        }
        self.mark_dependents(node);
        self.tx.note_dirty(node);
    }

    /// Recompute `node` if dirty; otherwise return the cached value.
    ///
    /// TODO(A0-05): there is no topological scheduler. A memo is evaluated on
    /// demand, and a diamond may recompute a shared dependency more than once
    /// per epoch. Values published here are consistent because reads are lazy,
    /// but the "stable value before publication + observable-count per epoch"
    /// contract is unverified, as are cycle detection and `WriteDuringCompute`.
    pub fn compute<F>(&mut self, node: NodeId, f: F) -> Result<Value, GraphError>
    where
        F: FnOnce(&mut DepReader<'_>) -> Value,
    {
        let index = node as usize;
        let Some(state) = self.nodes.get(index) else {
            return Err(GraphError::UnknownNode(node));
        };
        if !state.dirty {
            return self
                .values
                .get(index)
                .and_then(Option::clone)
                .ok_or(GraphError::Uncomputed(node));
        }
        if self.dependencies_unchanged(index) {
            // Dirty was only a hint: no dependency revision moved, so the
            // cached value is still the stable one.
            self.nodes[index].dirty = false;
            return self
                .values
                .get(index)
                .and_then(Option::clone)
                .ok_or(GraphError::Uncomputed(node));
        }

        let (deps, value) = {
            let mut reader = DepReader {
                values: &self.values,
                nodes: &self.nodes,
                deps: Vec::new(),
            };
            let value = f(&mut reader);
            (reader.deps, value)
        };

        if let Some(previous) = &self.values[index]
            && previous.kind() != value.kind()
        {
            return Err(GraphError::ValueKindChanged {
                node,
                expected: previous.kind(),
            });
        }

        self.memo_recomputes += 1;
        let changed = self.values[index].as_ref() != Some(&value);
        self.values[index] = Some(value.clone());
        self.record_dependencies(node, deps);
        self.nodes[index].dirty = false;
        if changed {
            self.nodes[index].revision += 1;
            self.mark_dependents(node);
        }
        Ok(value)
    }

    /// Cached value without recomputation.
    pub fn peek(&self, node: NodeId) -> Option<&Value> {
        self.values.get(node as usize)?.as_ref()
    }

    /// Number of live dependency edges.
    pub fn edge_count(&self) -> usize {
        self.forward.values().map(BTreeSet::len).sum()
    }

    /// Number of nodes that currently hold at least one dependent.
    pub fn active_source_count(&self) -> usize {
        self.forward.values().filter(|set| !set.is_empty()).count()
    }

    pub fn begin_batch(&mut self) {
        self.tx.begin();
    }

    pub fn end_batch(&mut self) -> Option<Vec<NodeId>> {
        self.tx.end()
    }

    pub fn commits(&self) -> u64 {
        self.tx.commits()
    }

    pub fn transaction_depth(&self) -> u32 {
        self.tx.depth()
    }

    /// True while a frame transaction is open.
    pub fn in_transaction(&self) -> bool {
        self.tx.is_open()
    }

    pub fn counters(&self) -> Counters {
        Counters {
            memo_recomputes: self.memo_recomputes,
            commits: self.tx.commits(),
            ..Counters::default()
        }
    }

    fn record_dependencies(&mut self, node: NodeId, deps: Vec<NodeId>) {
        if let Some(previous) = self.reverse.remove(&node) {
            let mut empty = Vec::new();
            for dependency in &previous {
                if let Some(dependents) = self.forward.get_mut(dependency) {
                    dependents.remove(&node);
                    if dependents.is_empty() {
                        empty.push(*dependency);
                    }
                }
            }
            for dependency in empty {
                self.forward.remove(&dependency);
            }
        }

        let mut set = BTreeSet::new();
        let mut revisions = BTreeMap::new();
        for dependency in deps {
            if dependency as usize >= self.nodes.len() {
                continue;
            }
            set.insert(dependency);
            revisions.insert(dependency, self.nodes[dependency as usize].revision);
            self.forward.entry(dependency).or_default().insert(node);
        }
        self.reverse.insert(node, set);
        self.observed[node as usize] = revisions;
    }

    fn dependencies_unchanged(&self, index: usize) -> bool {
        let Some(observed) = self.observed.get(index) else {
            return false;
        };
        if observed.is_empty() {
            return false;
        }
        observed.iter().all(|(dependency, revision)| {
            self.nodes
                .get(*dependency as usize)
                .is_some_and(|state| state.revision == *revision)
        })
    }

    fn mark_dependents(&mut self, node: NodeId) {
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            let Some(dependents) = self.forward.get(&current) else {
                continue;
            };
            for dependent in dependents {
                let index = *dependent as usize;
                if index < self.nodes.len() && !self.nodes[index].dirty {
                    self.nodes[index].dirty = true;
                    stack.push(*dependent);
                }
            }
        }
    }
}

impl Default for MemoGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for MemoGraph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoGraph")
            .field("nodes", &self.nodes.len())
            .field("edges", &self.edge_count())
            .field("commits", &self.tx.commits())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(order: &[u64]) -> Value {
        Value::Rows(RowSet::from_keys(
            order
                .iter()
                .map(|raw| crate::handle::RowKey::from_raw(*raw))
                .collect(),
        ))
    }

    #[test]
    fn memo_recomputes_only_when_dirty() {
        let mut graph = MemoGraph::new();
        let source = graph.source("source");
        let memo = graph.memo("memo");

        let first = graph
            .compute(memo, |reader| {
                reader.read(source);
                Value::Count(1)
            })
            .expect("compute");
        assert_eq!(first, Value::Count(1));

        let second = graph
            .compute(memo, |_| panic!("must not recompute"))
            .expect("cached");
        assert_eq!(second, Value::Count(1));
        assert_eq!(graph.counters().memo_recomputes, 1);
    }

    #[test]
    fn equal_value_suppresses_downstream_notification() {
        let mut graph = MemoGraph::new();
        let source = graph.source("source");
        let upstream = graph.memo("upstream");
        let downstream = graph.memo("downstream");

        let _ = graph
            .compute(upstream, |reader| {
                reader.read(source);
                Value::Count(1)
            })
            .expect("compute");
        let _ = graph
            .compute(downstream, |reader| {
                reader.read(upstream);
                Value::Count(10)
            })
            .expect("compute");
        assert_eq!(graph.edge_count(), 2);

        graph.bump_source(source);
        // Upstream recomputes to the same value.
        let _ = graph
            .compute(upstream, |reader| {
                reader.read(source);
                Value::Count(1)
            })
            .expect("recompute");
        // Downstream was not invalidated by an equal value.
        let cached = graph
            .compute(downstream, |_| panic!("downstream must stay cached"))
            .expect("cached");
        assert_eq!(cached, Value::Count(10));
        assert_eq!(
            graph.counters().memo_recomputes,
            3,
            "two first computes plus one upstream recompute"
        );
    }

    #[test]
    fn dependency_edges_are_replaced_not_accumulated() {
        let mut graph = MemoGraph::new();
        let left = graph.source("left");
        let right = graph.source("right");
        let memo = graph.memo("memo");

        let _ = graph
            .compute(memo, |reader| {
                reader.read(left);
                Value::Count(1)
            })
            .expect("compute");
        assert_eq!(graph.edge_count(), 1);

        // Bump a real dependency so the second body is allowed to run; a
        // clean memo would be served from cache and the body would never run.
        graph.bump_source(left);
        let _ = graph
            .compute(memo, |reader| {
                reader.read(right);
                Value::Count(2)
            })
            .expect("recompute");
        assert_eq!(graph.edge_count(), 1, "one edge, not two");

        // The `left` edge is gone: bumping `left` must no longer invalidate
        // the memo, which only a removed edge can prove.
        graph.bump_source(left);
        let cached = graph
            .compute(memo, |_| panic!("memo must not be invalidated by `left`"))
            .expect("cached");
        assert_eq!(cached, Value::Count(2));
    }

    #[test]
    fn dirty_propagates_transitively() {
        let mut graph = MemoGraph::new();
        let source = graph.source("source");
        let middle = graph.memo("middle");
        let leaf = graph.memo("leaf");

        let _ = graph
            .compute(middle, |reader| {
                reader.read(source);
                Value::Count(1)
            })
            .expect("middle");
        let _ = graph
            .compute(leaf, |reader| {
                reader.read(middle);
                Value::Count(2)
            })
            .expect("leaf");

        graph.bump_source(source);
        // A changed middle must dirty the leaf two hops away. If propagation
        // stopped at direct dependents, the leaf would serve its cached value.
        let _ = graph
            .compute(middle, |reader| {
                reader.read(source);
                Value::Count(9)
            })
            .expect("middle changed");
        let leaf_value = graph
            .compute(leaf, |reader| {
                reader.read(middle);
                Value::Count(3)
            })
            .expect("leaf recomputed");
        assert_eq!(leaf_value, Value::Count(3));
    }

    #[test]
    fn row_values_compare_by_content() {
        assert_eq!(rows(&[1, 2]), rows(&[1, 2]));
        assert_ne!(rows(&[1, 2]), rows(&[2, 1]));
    }
}
