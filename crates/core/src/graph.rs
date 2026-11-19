use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::metrics::Counters;

pub type NodeId = u32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Count(u32),
    Flag(bool),
}

impl Value {
    const fn kind(&self) -> &'static str {
        match self {
            Value::Count(_) => "count",
            Value::Flag(_) => "flag",
        }
    }
}

#[derive(Debug)]
pub enum GraphError {
    UnknownNode(NodeId),

    Uncomputed(NodeId),

    Cycle {
        node: NodeId,
    },

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
            GraphError::Cycle { node } => {
                write!(f, "graph node {node} is part of a cycle")
            }
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

pub struct DepReader<'a> {
    values: &'a [Option<Value>],
    nodes: &'a [Node],
    scratch: Option<&'a GraphScratch>,
    deps: Vec<NodeId>,
    cycle: Option<NodeId>,
}

impl<'a> DepReader<'a> {
    pub fn read(&mut self, node: NodeId) -> Option<&'a Value> {
        let values = self.values;
        let scratch = self.scratch;
        let index = node as usize;
        if index >= values.len() {
            return None;
        }
        if let Some(scratch) = self.scratch
            && scratch.computing.contains(&node)
        {
            self.cycle = Some(node);
            return None;
        }
        if !self.deps.contains(&node) {
            self.deps.push(node);
        }
        match scratch {
            Some(scratch) => scratch.values.get(&node).or(values[index].as_ref()),
            None => values[index].as_ref(),
        }
    }

    pub fn is_dirty(&self, node: NodeId) -> bool {
        let staged = self
            .scratch
            .is_some_and(|scratch| scratch.dirty.contains(&node));
        staged
            || self
                .nodes
                .get(node as usize)
                .is_some_and(|state| state.dirty)
    }

    pub fn deps(&self) -> &[NodeId] {
        &self.deps
    }
}

#[derive(Debug, Default)]
/// Scratch 只记录本轮计算与暂存写入，失败时直接丢弃而不改动已提交图。
///
/// 一次 Memo 求值优先读取同轮新值；共享 source 的多个依赖用同一轮数值汇合。
pub(crate) struct GraphScratch {
    values: BTreeMap<NodeId, Value>,
    revisions: BTreeMap<NodeId, u64>,

    observed: BTreeMap<NodeId, BTreeMap<NodeId, u64>>,

    observed_at: BTreeMap<NodeId, u64>,

    source_rev: BTreeMap<NodeId, u64>,
    dirty: BTreeSet<NodeId>,

    computing: Vec<NodeId>,

    cycle: Option<NodeId>,
}

impl GraphScratch {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn begin_computing(&mut self, node: NodeId) {
        self.computing.push(node);
    }

    #[cfg(test)]
    pub(crate) fn mark_dirty(&mut self, node: NodeId) {
        self.dirty.insert(node);
    }

    #[cfg(test)]
    pub(crate) const fn cycle(&self) -> Option<NodeId> {
        self.cycle
    }
}

pub struct MemoGraph {
    nodes: Vec<Node>,
    values: Vec<Option<Value>>,

    forward: BTreeMap<NodeId, BTreeSet<NodeId>>,

    reverse: BTreeMap<NodeId, BTreeSet<NodeId>>,

    observed: Vec<BTreeMap<NodeId, u64>>,
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
            memo_recomputes: 0,
        }
    }

    pub fn source(&mut self, label: &'static str) -> NodeId {
        self.push_node(label, false)
    }

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

    pub fn bump_source(&mut self, node: NodeId) {
        if let Some(state) = self.nodes.get_mut(node as usize) {
            state.revision += 1;
        }
        self.mark_dependents(node);
    }

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
            self.nodes[index].dirty = false;
            return self
                .values
                .get(index)
                .and_then(Option::clone)
                .ok_or(GraphError::Uncomputed(node));
        }

        let (deps, value, cycle) = {
            let mut stack = GraphScratch::new();
            stack.computing.push(node);
            let mut reader = DepReader {
                values: &self.values,
                nodes: &self.nodes,
                scratch: Some(&stack),
                deps: Vec::new(),
                cycle: None,
            };
            let value = f(&mut reader);
            (reader.deps, value, reader.cycle)
        };
        if let Some(node) = cycle {
            return Err(GraphError::Cycle { node });
        }

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

    pub fn peek(&self, node: NodeId) -> Option<&Value> {
        self.values.get(node as usize)?.as_ref()
    }

    pub fn edge_count(&self) -> usize {
        self.forward.values().map(BTreeSet::len).sum()
    }

    pub fn active_source_count(&self) -> usize {
        self.forward.values().filter(|set| !set.is_empty()).count()
    }

    pub fn counters(&self) -> Counters {
        Counters {
            memo_recomputes: self.memo_recomputes,
            ..Counters::default()
        }
    }

    pub(crate) fn effective_revision(&self, scratch: &GraphScratch, node: NodeId) -> u64 {
        scratch
            .source_rev
            .get(&node)
            .copied()
            .or_else(|| scratch.revisions.get(&node).copied())
            .unwrap_or_else(|| {
                self.nodes
                    .get(node as usize)
                    .map_or(0, |state| state.revision)
            })
    }

    fn staged_value(&self, scratch: &GraphScratch, node: NodeId) -> Option<Value> {
        scratch
            .values
            .get(&node)
            .cloned()
            .or_else(|| self.values.get(node as usize).and_then(Option::clone))
    }

    pub(crate) fn bump_source_staged(&self, scratch: &mut GraphScratch, node: NodeId) {
        let next = self.effective_revision(scratch, node).saturating_add(1);
        scratch.source_rev.insert(node, next);
        self.mark_dependents_staged(scratch, node);
    }

    fn mark_dependents_staged(&self, scratch: &mut GraphScratch, node: NodeId) {
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            let Some(dependents) = self.forward.get(&current) else {
                continue;
            };
            for dependent in dependents {
                if scratch.dirty.insert(*dependent) {
                    stack.push(*dependent);
                }
            }
        }
    }

    pub(crate) fn compute_staged<F>(
        &mut self,
        node: NodeId,
        scratch: &mut GraphScratch,
        staged_rev: u64,
        f: F,
    ) -> Result<(Value, bool), GraphError>
    where
        F: FnOnce(&mut DepReader<'_>) -> Value,
    {
        let index = node as usize;
        let Some(state) = self.nodes.get(index) else {
            return Err(GraphError::UnknownNode(node));
        };

        let dirty = state.dirty || scratch.dirty.contains(&node);
        if !dirty {
            return self
                .staged_value(scratch, node)
                .map(|value| (value, false))
                .ok_or(GraphError::Uncomputed(node));
        }

        let observed = scratch
            .observed
            .get(&node)
            .cloned()
            .or_else(|| self.observed.get(index).cloned());
        if let Some(observed) = observed
            && !observed.is_empty()
            && observed
                .iter()
                .all(|(dep, rev)| self.effective_revision(scratch, *dep) == *rev)
        {
            scratch.dirty.remove(&node);
            return self
                .staged_value(scratch, node)
                .map(|value| (value, false))
                .ok_or(GraphError::Uncomputed(node));
        }

        scratch.computing.push(node);
        let (deps, value, cycle) = {
            let mut reader = DepReader {
                values: &self.values,
                nodes: &self.nodes,
                scratch: Some(&*scratch),
                deps: Vec::new(),
                cycle: None,
            };
            let value = f(&mut reader);
            (reader.deps, value, reader.cycle)
        };
        scratch.computing.pop();
        if let Some(node) = cycle {
            scratch.cycle = Some(node);
            return Err(GraphError::Cycle { node });
        }

        if let Some(previous) = self.staged_value(scratch, node)
            && previous.kind() != value.kind()
        {
            return Err(GraphError::ValueKindChanged {
                node,
                expected: previous.kind(),
            });
        }

        self.memo_recomputes += 1;
        let previous = self.staged_value(scratch, node);
        let changed = previous.as_ref() != Some(&value);
        let revision = self
            .effective_revision(scratch, node)
            .saturating_add(u64::from(changed));
        let dep_revs: BTreeMap<NodeId, u64> = deps
            .iter()
            .map(|dep| (*dep, self.effective_revision(scratch, *dep)))
            .collect();

        scratch.values.insert(node, value.clone());
        scratch.revisions.insert(node, revision);
        scratch.observed.insert(node, dep_revs);
        scratch.observed_at.insert(node, staged_rev);
        scratch.dirty.remove(&node);
        if changed {
            self.mark_dependents_staged(scratch, node);
        }
        Ok((value, true))
    }

    pub(crate) fn commit_scratch(&mut self, scratch: GraphScratch, final_staged_rev: u64) {
        let GraphScratch {
            values,
            revisions,
            observed,
            observed_at,
            source_rev,
            dirty,
            ..
        } = scratch;
        let mut installed: std::collections::BTreeSet<NodeId> = std::collections::BTreeSet::new();

        for (node, revision) in source_rev {
            if let Some(state) = self.nodes.get_mut(node as usize) {
                state.revision = revision;
            }
        }

        for (node, value) in values {
            if observed_at.get(&node) != Some(&final_staged_rev) {
                continue;
            }
            let index = node as usize;
            if self.values.get(index).is_none() {
                continue;
            }
            self.values[index] = Some(value);
            installed.insert(node);
            if let Some(revision) = revisions.get(&node)
                && let Some(state) = self.nodes.get_mut(index)
            {
                state.revision = *revision;
                state.dirty = false;
            }
            if let Some(deps) = observed.get(&node) {
                self.record_dependencies(node, deps.keys().copied().collect());
            }
        }

        for node in dirty {
            if installed.contains(&node) {
                continue;
            }
            if let Some(state) = self.nodes.get_mut(node as usize) {
                state.dirty = true;
            }
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
                .is_some_and(|state| !state.dirty && state.revision == *revision)
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
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let _ = graph
            .compute(upstream, |reader| {
                reader.read(source);
                Value::Count(1)
            })
            .expect("recompute");

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

        graph.bump_source(left);
        let _ = graph
            .compute(memo, |reader| {
                reader.read(right);
                Value::Count(2)
            })
            .expect("recompute");
        assert_eq!(graph.edge_count(), 1, "one edge, not two");

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
    fn the_compute_stack_reports_a_read_into_a_cycle() {
        let mut graph = MemoGraph::new();
        let first = graph.memo("first");
        let second = graph.memo("second");
        let mut scratch = GraphScratch::new();

        scratch.begin_computing(first);
        let error = graph
            .compute_staged(second, &mut scratch, 1, |reader| {
                let _ = reader.read(first);
                Value::Count(1)
            })
            .expect_err("reading a computing node is a cycle");

        assert!(
            matches!(error, GraphError::Cycle { node } if node == first),
            "expected Cycle({first}), got {error:?}"
        );
    }

    #[test]
    fn a_self_reading_memo_is_a_cycle() {
        let mut graph = MemoGraph::new();
        let node = graph.memo("loop");
        let mut scratch = GraphScratch::new();

        let error = graph
            .compute_staged(node, &mut scratch, 1, |reader| {
                let _ = reader.read(node);
                Value::Count(1)
            })
            .expect_err("a self-reading memo must not converge");

        assert!(
            matches!(error, GraphError::Cycle { node: reported } if reported == node),
            "expected Cycle({node}), got {error:?}"
        );
        assert_eq!(scratch.cycle(), Some(node));
    }

    #[test]
    fn the_public_compute_path_reports_a_self_read_cycle() {
        let mut graph = MemoGraph::new();
        let node = graph.memo("loop");

        let error = graph
            .compute(node, |reader| {
                let _ = reader.read(node);
                Value::Count(1)
            })
            .expect_err("a self-reading memo must not converge");

        assert!(
            matches!(error, GraphError::Cycle { node: reported } if reported == node),
            "expected Cycle({node}), got {error:?}"
        );
        assert!(
            graph.peek(node).is_none(),
            "the refused body must not install a value"
        );
    }

    #[test]
    fn a_write_only_frame_leaves_the_memo_dirty() {
        let mut graph = MemoGraph::new();
        let source = graph.source("store");
        let memo = graph.memo("remaining");

        let first = graph
            .compute(memo, |reader| {
                reader.read(source);
                Value::Count(1)
            })
            .expect("first compute");
        assert_eq!(first, Value::Count(1));

        let mut scratch = GraphScratch::new();
        graph.bump_source_staged(&mut scratch, source);
        graph.commit_scratch(scratch, 1);

        let after = graph
            .compute(memo, |reader| {
                reader.read(source);
                Value::Count(2)
            })
            .expect("the memo must recompute after the write-only frame");
        assert_eq!(
            after,
            Value::Count(2),
            "a stale value survived a write-only frame"
        );
    }

    #[test]
    fn a_dirty_dependency_is_not_treated_as_unchanged() {
        let mut graph = MemoGraph::new();
        let source = graph.source("store");
        let middle = graph.memo("middle");
        let top = graph.memo("top");

        let _ = graph
            .compute(middle, |reader| {
                reader.read(source);
                Value::Count(1)
            })
            .expect("middle");
        let _ = graph
            .compute(top, |reader| {
                reader.read(middle);
                Value::Count(10)
            })
            .expect("top");

        graph.bump_source(source);

        let top_value = graph
            .compute(top, |reader| {
                reader.read(middle);
                Value::Count(20)
            })
            .expect("top must not serve a value built on a dirty dependency");
        assert_eq!(top_value, Value::Count(20));
    }

    #[test]
    fn a_diamond_never_observes_two_epochs() {
        let mut graph = MemoGraph::new();
        let source = graph.source("store");
        let left = graph.memo("left");
        let right = graph.memo("right");
        let join = graph.memo("join");
        let mut scratch = GraphScratch::new();
        let mut revision = 0u64;

        let read_count = |value: Option<&Value>| match value {
            Some(Value::Count(count)) => *count,
            _ => 0,
        };

        for epoch in 1..=3u32 {
            revision += 1;
            graph.bump_source_staged(&mut scratch, source);
            let _ = graph
                .compute_staged(left, &mut scratch, revision, |reader| {
                    reader.read(source);
                    Value::Count(epoch)
                })
                .expect("left");
            let _ = graph
                .compute_staged(right, &mut scratch, revision, |reader| {
                    reader.read(source);
                    Value::Count(epoch)
                })
                .expect("right");
            let (joined, recomputed) = graph
                .compute_staged(join, &mut scratch, revision, |reader| {
                    let left_value = read_count(reader.read(left));
                    let right_value = read_count(reader.read(right));
                    assert_eq!(
                        left_value, right_value,
                        "a diamond must not mix epochs: {left_value} vs {right_value}"
                    );
                    Value::Count(left_value + right_value)
                })
                .expect("join");
            assert_eq!(joined, Value::Count(epoch * 2));
            assert!(
                recomputed,
                "epoch {epoch}: the join must recompute exactly once"
            );

            let (cached, recomputed_again) = graph
                .compute_staged(join, &mut scratch, revision, |_| Value::Count(0))
                .expect("join again");
            assert_eq!(cached, Value::Count(epoch * 2));
            assert!(
                !recomputed_again,
                "epoch {epoch}: a second observation must hit the stage, not recompute"
            );
        }
    }

    #[test]
    fn a_long_chain_keeps_the_final_value_consistent() {
        let mut graph = MemoGraph::new();
        let source = graph.source("store");
        let mut nodes = vec![graph.memo("chain")];
        for _ in 1..8 {
            nodes.push(graph.memo("chain"));
        }

        let mut scratch = GraphScratch::new();
        graph.bump_source_staged(&mut scratch, source);

        for (index, node) in nodes.iter().copied().enumerate() {
            let staged = match index {
                0 => graph
                    .compute_staged(node, &mut scratch, 1, |reader| {
                        reader.read(source);
                        Value::Count(1)
                    })
                    .expect("chain head"),
                _ => {
                    let previous = nodes[index - 1];
                    graph
                        .compute_staged(node, &mut scratch, 1, |reader| {
                            let previous_value = match reader.read(previous) {
                                Some(Value::Count(value)) => *value,
                                _ => 0,
                            };
                            Value::Count(previous_value + 1)
                        })
                        .expect("chain node")
                }
            };
            assert_eq!(
                staged.0,
                Value::Count(index as u32 + 1),
                "chain node {index} saw a stale predecessor"
            );
        }
    }
}
