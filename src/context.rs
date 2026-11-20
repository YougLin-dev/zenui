use std::cell::RefCell;
use std::rc::Rc;

use zenui_core::Filter;

use crate::runtime::State;
use crate::store::TodoId;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Idle,
    Setup,
    Binding,
    Memo,
    Event,
    Delivery,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum FieldKind {
    Title,
    Completed,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum DepKey {
    Signal(u32),
    Memo(u32),
    Field(TodoId, FieldKind),
    Rows(u8),
}

pub(crate) const fn filter_slot(filter: Filter) -> u8 {
    match filter {
        Filter::All => 0,
        Filter::Active => 1,
        Filter::Completed => 2,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ContextFault {
    pub operation: &'static str,
    pub phase: Phase,
}

thread_local! {
    static CURRENT: RefCell<Vec<Rc<RefCell<State>>>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn install<R>(state: &Rc<RefCell<State>>, f: impl FnOnce() -> R) -> R {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            CURRENT.with(|current| {
                current.borrow_mut().pop();
            });
        }
    }

    CURRENT.with(|current| current.borrow_mut().push(Rc::clone(state)));
    let guard = Guard;
    let result = f();
    drop(guard);
    result
}

pub(crate) fn installed_runtime() -> Option<u32> {
    let state = CURRENT.with(|current| current.borrow().last().cloned());
    state.and_then(|state| state.try_borrow().ok().map(|state| state.id))
}

pub(crate) fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let state = CURRENT.with(|current| current.borrow().last().cloned());
    let state = state.expect("state access requires an installed ZenUI runtime");
    let mut state = state.borrow_mut();
    f(&mut state)
}

pub(crate) fn with_state_of<R>(runtime: u32, f: impl FnOnce(&mut State) -> R) -> R {
    let state = CURRENT.with(|current| current.borrow().last().cloned());
    let state = state.expect("state access requires an installed ZenUI runtime");
    let mut state = state.borrow_mut();
    assert_eq!(
        state.id, runtime,
        "a handle from another ZenUI Runtime must not access this one"
    );
    f(&mut state)
}

pub(crate) fn release_read_only(runtime: u32) {
    let state = CURRENT.with(|current| current.borrow().last().cloned());
    if let Some(state) = state {
        let mut state = state.borrow_mut();
        if state.id == runtime {
            state.read_only = state.read_only.saturating_sub(1);
        }
    }
}
