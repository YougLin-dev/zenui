use std::any::Any;
use std::collections::BTreeSet;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use crate::context::{DepKey, Phase, release_read_only, with_state, with_state_of};

pub struct Signal<T> {
    runtime: u32,
    id: u32,
    _marker: PhantomData<fn() -> T>,
    _not_send: PhantomData<*const ()>,
}

impl<T> Copy for Signal<T> {}
impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Clone + PartialEq + 'static> Signal<T> {
    pub fn get(self) -> T {
        with_state_of(self.runtime, |state| state.read_signal(self.id))
    }

    pub fn set(self, value: T) {
        with_state_of(self.runtime, |state| state.write_signal(self.id, value));
    }

    /// 只读作用域：内部的 setter 与模型事务在改动前被拒绝。
    pub fn with<R>(self, f: impl FnOnce(&T) -> R) -> R {
        struct Guard {
            runtime: u32,
        }
        impl Drop for Guard {
            fn drop(&mut self) {
                release_read_only(self.runtime);
            }
        }

        let _guard = Guard {
            runtime: self.runtime,
        };
        let value = with_state_of(self.runtime, |state| {
            state.read_only += 1;
            state.read_signal(self.id)
        });
        f(&value)
    }
}

impl<T> fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Signal")
            .field("runtime", &self.runtime)
            .field("id", &self.id)
            .finish()
    }
}

pub struct Memo<T> {
    runtime: u32,
    id: u32,
    _marker: PhantomData<fn() -> T>,
    _not_send: PhantomData<*const ()>,
}

impl<T> Copy for Memo<T> {}
impl<T> Clone for Memo<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Clone + PartialEq + 'static> Memo<T> {
    pub fn get(self) -> T {
        let ready = with_state_of(self.runtime, |state| {
            let value = state.memo_ready::<T>(self.id);
            if value.is_some() && state.phase == Phase::Memo {
                state.note_dep(DepKey::Memo(self.id));
            }
            value
        });
        if let Some(value) = ready {
            return value;
        }
        let (compute, previous) = with_state_of(self.runtime, |state| state.begin_memo(self.id));
        let value = compute();
        with_state_of(self.runtime, |state| {
            state.finish_memo::<T>(self.id, value, previous, compute)
        });
        with_state_of(self.runtime, |state| {
            if state.phase == Phase::Memo {
                state.note_dep(DepKey::Memo(self.id));
            }
            state.memo_value(self.id)
        })
    }
}

impl<T> fmt::Debug for Memo<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Memo")
            .field("runtime", &self.runtime)
            .field("id", &self.id)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Once<T> {
    value: T,
}

impl<T> Once<T> {
    pub fn into_inner(self) -> T {
        self.value
    }
}

pub fn signal<T: Clone + PartialEq + 'static>(value: T) -> Signal<T> {
    let (runtime, id) = with_state(|state| {
        let id = state.create_signal(Box::new(value));
        (state.id, id)
    });
    Signal {
        runtime,
        id,
        _marker: PhantomData,
        _not_send: PhantomData,
    }
}

pub fn memo<T: Clone + PartialEq + 'static>(compute: impl Fn() -> T + 'static) -> Memo<T> {
    let (runtime, id) = with_state(|state| {
        let id = state.create_memo(Box::new(move || Box::new(compute()) as Box<dyn Any>));
        (state.id, id)
    });
    Memo {
        runtime,
        id,
        _marker: PhantomData,
        _not_send: PhantomData,
    }
}

pub fn once<T>(value: T) -> Once<T> {
    Once { value }
}

pub(crate) struct SignalCell {
    pub(crate) value: Rc<dyn Any>,
    pub(crate) disposed: bool,
}

pub(crate) struct MemoCell {
    pub(crate) compute: Option<Box<dyn Fn() -> Box<dyn Any>>>,
    pub(crate) value: Option<Rc<dyn Any>>,
    pub(crate) deps: Vec<DepKey>,
    pub(crate) dirty: bool,
    pub(crate) computing: bool,
    pub(crate) disposed: bool,
}

impl MemoCell {
    pub(crate) fn new(compute: Box<dyn Fn() -> Box<dyn Any>>) -> Self {
        Self {
            compute: Some(compute),
            value: None,
            deps: Vec::new(),
            dirty: true,
            computing: false,
            disposed: false,
        }
    }
}

impl crate::runtime::State {
    fn signal_index(&self, id: u32) -> usize {
        id as usize
    }

    fn memo_index(&self, id: u32) -> usize {
        id as usize
    }

    pub(crate) fn create_signal(&mut self, value: Box<dyn Any>) -> u32 {
        let id = u32::try_from(self.signals.len()).expect("signal arena exhausted");
        self.signals.push(SignalCell {
            value: Rc::from(value),
            disposed: false,
        });
        self.attach_to_owner(crate::runtime::Resource::Signal(id));
        id
    }

    pub(crate) fn read_signal<T: Clone + PartialEq + 'static>(&mut self, id: u32) -> T {
        let cell = &self.signals[self.signal_index(id)];
        assert!(!cell.disposed, "signal belongs to a disposed owner");
        let value = cell
            .value
            .clone()
            .downcast::<T>()
            .unwrap_or_else(|_| panic!("signal {id} has a different value type"));
        if self.phase == Phase::Memo {
            self.note_dep(DepKey::Signal(id));
        }
        (*value).clone()
    }

    pub(crate) fn write_signal<T: Clone + PartialEq + 'static>(&mut self, id: u32, value: T) {
        if !matches!(self.phase, Phase::Event | Phase::Delivery) || self.read_only > 0 {
            self.record_fault("Signal::set", self.phase);
            return;
        }
        let index = self.signal_index(id);
        if self.signals[index].disposed {
            self.record_fault("Signal::set", self.phase);
            return;
        }
        let cell = &self.signals[index];
        let current = cell
            .value
            .clone()
            .downcast::<T>()
            .unwrap_or_else(|_| panic!("signal {id} has a different value type"));
        if *current == value {
            return;
        }
        let index = self.signal_index(id);
        self.signals[index].value = Rc::new(value);
        self.bump(DepKey::Signal(id));
    }

    pub(crate) fn create_memo(&mut self, compute: Box<dyn Fn() -> Box<dyn Any>>) -> u32 {
        let id = u32::try_from(self.memos.len()).expect("memo arena exhausted");
        self.memos.push(MemoCell::new(compute));
        self.attach_to_owner(crate::runtime::Resource::Memo(id));
        id
    }

    pub(crate) fn memo_ready<T: Clone + PartialEq + 'static>(&mut self, id: u32) -> Option<T> {
        let cell = &self.memos[self.memo_index(id)];
        assert!(!cell.disposed, "memo belongs to a disposed owner");
        if cell.computing {
            panic!("memo {id} was read while computing (cycle)");
        }
        if cell.dirty {
            return None;
        }
        cell.value
            .as_ref()
            .map(|value| (*value.clone().downcast::<T>().expect("memo value type")).clone())
    }

    pub(crate) fn memo_value<T: Clone + PartialEq + 'static>(&mut self, id: u32) -> T {
        let cell = &self.memos[self.memo_index(id)];
        let value = cell.value.as_ref().expect("memo was not computed").clone();
        (*value.downcast::<T>().expect("memo value type")).clone()
    }

    pub(crate) fn begin_memo(&mut self, id: u32) -> (Box<dyn Fn() -> Box<dyn Any>>, Phase) {
        let index = self.memo_index(id);
        let cell = &mut self.memos[index];
        assert!(!cell.disposed, "memo belongs to a disposed owner");
        assert!(!cell.computing, "memo {id} is already computing");
        let compute = cell
            .compute
            .take()
            .expect("memo compute closure is present");
        cell.computing = true;
        let previous = self.phase;
        self.phase = Phase::Memo;
        self.computing.push((id, Vec::new()));
        (compute, previous)
    }

    /// 本轮依赖集整体替换旧依赖；只有值变化才推进 Memo revision 并使下游失效。
    /// 依赖边用于定向标脏，避免无关节点重新求值。
    pub(crate) fn finish_memo<T: Clone + PartialEq + 'static>(
        &mut self,
        id: u32,
        value: Box<dyn Any>,
        previous_phase: Phase,
        compute: Box<dyn Fn() -> Box<dyn Any>>,
    ) {
        self.phase = previous_phase;
        let (_, deps) = self.computing.pop().expect("memo compute stack");
        let index = self.memo_index(id);
        let cell = &mut self.memos[index];
        cell.computing = false;
        cell.dirty = false;
        cell.compute = Some(compute);
        let value: T = *value.downcast::<T>().expect("memo value type");
        let changed = match &cell.value {
            Some(current) => *current.clone().downcast::<T>().expect("memo value type") != value,
            None => true,
        };
        cell.value = Some(Rc::new(value));

        for key in &cell.deps {
            if let Some(dependents) = self.reverse.get_mut(key) {
                dependents.remove(&id);
            }
        }
        for key in &deps {
            self.reverse.entry(*key).or_default().insert(id);
        }
        cell.deps = deps;
        if changed {
            self.invalidate_dependents(DepKey::Memo(id));
        }
    }

    pub(crate) fn note_dep(&mut self, key: DepKey) {
        if let Some((_, deps)) = self.computing.last_mut() {
            deps.push(key);
        }
    }

    pub(crate) fn bump(&mut self, key: DepKey) {
        self.invalidate_dependents(key);
    }

    fn invalidate_dependents(&mut self, key: DepKey) {
        let dependents: BTreeSet<u32> = self.reverse.get(&key).cloned().unwrap_or_default();
        for memo in dependents {
            let index = self.memo_index(memo);
            if self.memos[index].disposed || self.memos[index].dirty {
                continue;
            }
            if self.memos[index].computing {
                continue;
            }
            self.memos[index].dirty = true;
            self.invalidate_dependents(DepKey::Memo(memo));
        }
    }

    pub(crate) fn record_fault(&mut self, operation: &'static str, phase: Phase) {
        if self.fault.is_none() {
            self.fault = Some(crate::context::ContextFault { operation, phase });
        }
    }
}
