//! Reactive source release: unmounting an owner drops signal values and removes
//! memo dependencies/reverse edges instead of retaining them with the slot.

#![allow(unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;

/// A dropped payload proves the signal's value allocation was released, not
/// merely marked disposed.
#[derive(Clone)]
struct Probe(Rc<Cell<u32>>);

impl PartialEq for Probe {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[component]
fn Branch(drops: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let probe = signal(Probe(drops));
    let trigger = signal(0_u32);
    let derived = memo(move || {
        let _ = trigger.get();
        String::from("live")
    });
    // Keep the probe signal inside this owner even though the view only reads
    // the memo; it must still be released with the Show branch.
    let _keep = probe;
    view! { <text>{derived}</text> }
}

#[component]
fn ShowHost(when: Memo<bool>, then: ChildrenFactory) -> MountResult<MountPlan> {
    Ok(MountPlan::new(vec![zenui::view::show(when, then, None)]))
}

#[component]
fn Churn(drops: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let open = signal(true);
    let branch: ChildrenFactory = {
        let drops = drops.clone();
        ChildrenFactory::from(move || {
            let drops = drops.clone();
            view! { <Branch drops={drops} /> }
        })
    };
    view! {
        <view>
            <button aria:label="toggle" on:click={move || open.set(!open.get())}>
                "Toggle"
            </button>
            <ShowHost when={memo(move || open.get())} then={branch} />
        </view>
    }
}

#[component]
fn Creator() -> MountResult<MountPlan> {
    view! {
        <button
            aria:label="make"
            on:click={move || {
                let _leak = signal(1_u32);
            }}
        >"Make"</button>
    }
}

#[test]
fn signal_creation_outside_an_owner_is_a_contained_handler_panic() {
    let mut app = Runtime::new();
    app.mount(|| view! { <Creator /> }).expect("mount");
    let before = app.live_signals();
    let report = app
        .click_label("make")
        .expect("the creation panic is contained in the turn");
    assert!(
        matches!(
            report.errors(),
            [TurnError::HandlerPanicked(panic)] if panic.message().contains("must run under an owner")
        ),
        "unexpected errors: {:?}",
        report.errors()
    );
    assert_eq!(
        app.live_signals(),
        before,
        "the rejected signal never becomes an unowned resource"
    );
}

#[test]
fn unmounting_a_branch_drops_signal_values_and_memo_edges() {
    let drops = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let drops = drops.clone();
        move || view! { <Churn drops={drops} /> }
    })
    .expect("mount");
    assert_eq!(drops.get(), 0);
    assert_eq!(
        app.live_signals(),
        3,
        "Show driver, probe and trigger are live"
    );
    assert_eq!(app.live_memos(), 2, "Show driver and derived memo are live");

    let mounted = app.storage_report();
    assert!(
        mounted.memo_deps_live >= 2,
        "both memos recorded their dependencies"
    );

    app.click_label("toggle").expect("unmount the branch");
    assert_eq!(drops.get(), 1, "the signal value is dropped on unmount");
    assert_eq!(app.live_signals(), 1, "only the Show driver remains");
    assert_eq!(app.live_memos(), 1, "the Show driver memo remains");

    let released = app.storage_report();
    assert_eq!(
        released.memo_deps_live, 1,
        "the branch memo's dependency is cleared"
    );
    assert!(
        released.memo_deps_live < mounted.memo_deps_live,
        "unmounting removes dependency entries"
    );
    assert_eq!(
        released.dep_entries, released.memo_deps_live,
        "reverse edges match the remaining live dependencies"
    );
    assert_eq!(released.consumer_edges, released.memo_deps_live);

    for expected in 2..=4 {
        app.click_label("toggle").expect("mount the branch again");
        app.click_label("toggle").expect("unmount the branch again");
        assert_eq!(
            drops.get(),
            expected,
            "every branch mount releases exactly one value"
        );
        let report = app.storage_report();
        assert_eq!(report.memo_deps_live, released.memo_deps_live);
        assert_eq!(
            report.memo_deps_storage_bytes, released.memo_deps_storage_bytes,
            "dependency capacity does not grow with churn"
        );
        assert_eq!(report.dep_entries, released.dep_entries);
        assert_eq!(report.consumer_edges, released.consumer_edges);
    }
}
