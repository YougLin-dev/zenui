use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::{Rc, Weak};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Patch {
    Text { node: u32, value: String },
    Paint { node: u32, value: bool },
}

#[derive(Clone)]
pub struct Signal<T> {
    id: usize,
    value: Rc<RefCell<T>>,
    valid: Weak<Cell<bool>>,
    graph: Weak<RefCell<Graph>>,
}

impl<T: 'static> Signal<T> {
    pub fn with<R>(&self, get: impl FnOnce(&T) -> R) -> R {
        if self.valid.upgrade().is_some_and(|live| live.get())
            && let Some(graph) = self.graph.upgrade()
        {
            let mut graph = graph.borrow_mut();
            if let Some(observer) = graph.current {
                graph
                    .subscribers
                    .entry(self.id)
                    .or_default()
                    .insert(observer);
                graph
                    .dependencies
                    .entry(observer)
                    .or_default()
                    .insert(self.id);
            }
        }
        get(&self.value.borrow())
    }

    pub fn set(&self, value: T) {
        if !self.valid.upgrade().is_some_and(|live| live.get()) {
            return;
        }
        *self.value.borrow_mut() = value;
        if let Some(graph) = self.graph.upgrade() {
            graph.borrow_mut().invalidate(self.id);
        }
    }
}

#[derive(Default)]
struct Graph {
    current: Option<usize>,
    sinks: BTreeMap<usize, Box<dyn FnMut() -> Patch>>,
    subscribers: BTreeMap<usize, BTreeSet<usize>>,
    dependencies: BTreeMap<usize, BTreeSet<usize>>,
    pending: BTreeSet<usize>,
    disposed: bool,
}

impl Graph {
    fn invalidate(&mut self, source: usize) {
        if let Some(observers) = self.subscribers.get(&source) {
            self.pending.extend(observers);
        }
    }

    fn detach(&mut self, observer: usize) {
        if let Some(sources) = self.dependencies.remove(&observer) {
            for source in sources {
                if let Some(observers) = self.subscribers.get_mut(&source) {
                    observers.remove(&observer);
                }
            }
        }
    }
}

pub struct Owner {
    graph: Rc<RefCell<Graph>>,
    next_source: usize,
    sinks: Vec<usize>,
    liveness: Vec<Rc<Cell<bool>>>,
}

impl Owner {
    pub fn new() -> Self {
        Self {
            graph: Rc::default(),
            next_source: 0,
            sinks: vec![],
            liveness: vec![],
        }
    }

    pub fn signal<T>(&mut self, value: T) -> Signal<T> {
        let id = self.next_source;
        self.next_source += 1;
        let valid = Rc::new(Cell::new(true));
        self.liveness.push(valid.clone());
        Signal {
            id,
            value: Rc::new(RefCell::new(value)),
            valid: Rc::downgrade(&valid),
            graph: Rc::downgrade(&self.graph),
        }
    }

    pub fn bind(&mut self, evaluate: impl FnMut() -> Patch + 'static) -> usize {
        let id = self.next_source;
        self.next_source += 1;
        self.sinks.push(id);
        self.graph.borrow_mut().sinks.insert(id, Box::new(evaluate));
        self.graph.borrow_mut().pending.insert(id);
        self.flush();
        id
    }

    /// 调用用户 binding 前先移出图节点和旧依赖；可重入读取只登记新依赖。
    pub fn flush(&self) -> Vec<Patch> {
        let mut updates = Vec::new();
        loop {
            let id = {
                let mut graph = self.graph.borrow_mut();
                if graph.disposed {
                    break;
                }
                match graph.pending.pop_first() {
                    Some(id) => id,
                    None => break,
                }
            };
            let Some(mut sink) = self.graph.borrow_mut().sinks.remove(&id) else {
                continue;
            };
            let previous = {
                let mut graph = self.graph.borrow_mut();
                graph.detach(id);
                graph.current.replace(id)
            };

            let patch = sink();
            {
                let mut graph = self.graph.borrow_mut();
                graph.current = previous;
                if !graph.disposed {
                    graph.sinks.insert(id, sink);
                    updates.push(patch);
                }
            }
        }
        updates
    }

    pub fn batch(&self, update: impl FnOnce()) -> Vec<Patch> {
        update();

        self.flush()
    }

    /// 先撤销调用与订阅资格，再释放 signal 的存活标记；此原型忽略迟到的 set。
    pub fn dispose(&mut self) {
        self.graph.borrow_mut().disposed = true;
        self.graph.borrow_mut().pending.clear();
        self.graph.borrow_mut().sinks.clear();
        self.graph.borrow_mut().subscribers.clear();
        self.graph.borrow_mut().dependencies.clear();
        self.sinks.clear();
        for valid in self.liveness.drain(..) {
            valid.set(false);
        }
    }

    pub fn subscriber_count(&self) -> usize {
        self.graph.borrow().subscribers.len()
    }
    pub fn dependencies(&self, sink: usize) -> Vec<usize> {
        self.graph
            .borrow()
            .dependencies
            .get(&sink)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect()
    }
}
impl Default for Owner {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.dispose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_update_runs_only_matching_sink() {
        let mut owner = Owner::new();
        let first = owner.signal(false);
        let second = owner.signal(false);
        let calls_first = Rc::new(Cell::new(0));
        let calls_second = Rc::new(Cell::new(0));
        owner.bind({
            let first = first.clone();
            let calls = calls_first.clone();
            move || {
                calls.set(calls.get() + 1);
                Patch::Text {
                    node: 1,
                    value: first.with(|v| *v).to_string(),
                }
            }
        });
        owner.bind({
            let second = second.clone();
            let calls = calls_second.clone();
            move || {
                calls.set(calls.get() + 1);
                Patch::Text {
                    node: 2,
                    value: second.with(|v| *v).to_string(),
                }
            }
        });
        first.set(true);
        let updates = owner.flush();
        assert_eq!(
            updates,
            vec![Patch::Text {
                node: 1,
                value: "true".into()
            }]
        );
        assert_eq!(
            calls_first.get(),
            2,
            "initial binding and exactly one reactive recomputation"
        );
        assert_eq!(calls_second.get(), 1, "sibling only runs setup");
    }

    #[test]
    fn changing_dependencies_removes_inactive_edges() {
        let mut owner = Owner::new();
        let switch = owner.signal(false);
        let a = owner.signal("a".to_owned());
        let b = owner.signal("b".to_owned());
        let sink = owner.bind({
            let switch = switch.clone();
            let a = a.clone();
            let b = b.clone();
            move || Patch::Text {
                node: 1,
                value: if switch.with(|v| *v) {
                    a.with(Clone::clone)
                } else {
                    b.with(Clone::clone)
                },
            }
        });
        assert_eq!(
            owner.dependencies(sink),
            vec![0, 2],
            "current={:?} deps={:?} subscribers={:?} pending={:?}",
            owner.graph.borrow().current,
            owner.graph.borrow().dependencies,
            owner.graph.borrow().subscribers,
            owner.graph.borrow().pending
        );
        switch.set(true);
        owner.flush();
        assert_eq!(owner.dependencies(sink), vec![0, 1]);
        b.set("inactive".to_owned());
        assert!(owner.flush().is_empty());
    }

    #[test]
    fn multiple_writes_in_one_per_frame_batch_deduplicate_the_sink() {
        let mut owner = Owner::new();
        let a = owner.signal(false);
        let b = owner.signal(false);
        owner.bind({
            let a = a.clone();
            let b = b.clone();
            move || Patch::Paint {
                node: 1,
                value: a.with(|v| *v) && b.with(|v| *v),
            }
        });
        let updates = owner.batch(|| {
            a.set(true);
            b.set(true);
        });
        assert_eq!(
            updates,
            vec![Patch::Paint {
                node: 1,
                value: true
            }]
        );
    }

    #[test]
    fn disposing_owner_drops_sinks_and_disables_future_updates() {
        let mut owner = Owner::new();
        let selected = owner.signal(false);
        owner.bind({
            let selected = selected.clone();
            move || Patch::Paint {
                node: 1,
                value: selected.with(|v| *v),
            }
        });
        assert_eq!(owner.subscriber_count(), 1);
        owner.dispose();
        selected.set(true);
        assert!(owner.flush().is_empty());
        assert_eq!(owner.subscriber_count(), 0);
        owner.dispose();
    }
}
