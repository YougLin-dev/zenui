//! Signal/Memo 槽位复用、旧代际句柄诊断与 owner cleanup 边界。

#![allow(unused_braces)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use zenui::RuntimeDiagnostic;
use zenui::prelude::*;

#[component]
fn ShowSlot(when: Memo<bool>, branch: ChildrenFactory) -> MountResult<MountPlan> {
    Ok(MountPlan::new(vec![zenui::view::show(when, branch, None)]))
}

#[component]
fn StaleBranch(stale: Rc<RefCell<Option<Signal<String>>>>) -> MountResult<MountPlan> {
    let value = signal(String::from("payload"));
    // 只保留第一次挂载的句柄；重挂载会复用槽位并递增代际。
    let missing = stale.borrow().is_none();
    if missing {
        *stale.borrow_mut() = Some(value);
    }
    view! { <text>{value}</text> }
}

#[component]
fn StaleHost(stale: Rc<RefCell<Option<Signal<String>>>>) -> MountResult<MountPlan> {
    let open = signal(true);
    let probe = Rc::clone(&stale);
    let branch: ChildrenFactory = {
        let stale = Rc::clone(&stale);
        ChildrenFactory::from(move || {
            let stale = Rc::clone(&stale);
            view! { <StaleBranch stale={stale} /> }
        })
    };
    view! {
        <view>
            <button aria:label="hide" on:click={move || open.set(false)}>"Hide"</button>
            <button aria:label="show" on:click={move || open.set(true)}>"Show"</button>
            <button
                aria:label="probe"
                on:click={move || {
                    if let Some(handle) = *probe.borrow() {
                        let _ = handle.get();
                    }
                }}
            >"Probe"</button>
            <ShowSlot when={memo(move || open.get())} branch={branch} />
        </view>
    }
}

#[test]
fn a_stale_signal_handle_is_rejected_and_reported() {
    let stale: Rc<RefCell<Option<Signal<String>>>> = Rc::new(RefCell::new(None));
    let mut app = Runtime::new();
    app.mount({
        let stale = Rc::clone(&stale);
        move || view! { <StaleHost stale={stale} /> }
    })
    .expect("mount");
    assert!(
        stale.borrow().is_some(),
        "the mounted payload exposes a handle"
    );

    // 卸载后槽位进入 free list；重新挂载会复用同一槽位并递增代际。
    app.click_label("hide").expect("unmount the payload");
    app.click_label("show").expect("remount the payload");

    let report = app.click_label("probe").expect("the panic is contained");
    assert!(
        matches!(
            report.errors(),
            [TurnError::HandlerPanicked(panic)] if panic.message() == "signal belongs to a disposed owner"
        ),
        "a stale read is a contained panic: {:?}",
        report.errors()
    );

    let refresh = app.refresh().expect("refresh after the stale read");
    assert!(
        refresh
            .diagnostics()
            .iter()
            .any(|diagnostic| matches!(diagnostic, RuntimeDiagnostic::StaleSource { .. })),
        "the stale generation is a development diagnostic: {:?}",
        refresh.diagnostics()
    );
}

#[component]
fn CleanupBranch(log: Rc<RefCell<Vec<String>>>) -> MountResult<MountPlan> {
    let alive = signal(String::from("alive"));
    let first = Rc::clone(&log);
    let second = Rc::clone(&log);
    let seen = Rc::clone(&log);
    on_cleanup(move || first.borrow_mut().push(String::from("first")));
    on_cleanup(move || {
        second.borrow_mut().push(String::from("second"));
        panic!("cleanup boom");
    });
    // cleanup 在 Signal 释放前运行，仍可读取本 owner 的存活状态。
    on_cleanup(move || seen.borrow_mut().push(alive.get()));
    view! { <text>{alive}</text> }
}

#[component]
fn CleanupHost(log: Rc<RefCell<Vec<String>>>) -> MountResult<MountPlan> {
    let open = signal(true);
    let branch: ChildrenFactory = {
        let log = Rc::clone(&log);
        ChildrenFactory::from(move || {
            let log = Rc::clone(&log);
            view! { <CleanupBranch log={log} /> }
        })
    };
    view! {
        <view>
            <button aria:label="close" on:click={move || open.set(false)}>"Close"</button>
            <ShowSlot when={memo(move || open.get())} branch={branch} />
        </view>
    }
}

#[test]
fn cleanups_run_in_reverse_order_and_contain_panics() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let mut app = Runtime::new();
    app.mount({
        let log = Rc::clone(&log);
        move || view! { <CleanupHost log={log} /> }
    })
    .expect("mount");

    app.click_label("close").expect("close the payload");
    assert_eq!(
        *log.borrow(),
        ["alive", "second", "first"],
        "reverse registration order; the panic does not stop later cleanups"
    );

    let refresh = app.refresh().expect("refresh after cleanup panic");
    assert!(
        refresh.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic,
            RuntimeDiagnostic::CleanupPanicked(panic) if panic.message() == "cleanup boom"
        )),
        "cleanup panics are contained diagnostics: {:?}",
        refresh.diagnostics()
    );
}

#[component]
fn CleanupWriteBranch(value: Signal<i32>) -> MountResult<MountPlan> {
    on_cleanup(move || value.set(42));
    view! { <text>"payload"</text> }
}

#[component]
fn CleanupWriteHost(observed: Rc<Cell<i32>>) -> MountResult<MountPlan> {
    let value = signal(0);
    let open = signal(true);
    view! {
        <view>
            <button aria:label="close" on:click={move || open.set(false)}>"Close"</button>
            <button
                aria:label="probe"
                on:click={move || observed.set(value.get())}
            >"Probe"</button>
            <Show when={open}><CleanupWriteBranch value={value} /></Show>
        </view>
    }
}

#[test]
fn a_cleanup_write_is_a_typed_context_error() {
    let observed = Rc::new(Cell::new(-1));
    let mut app = Runtime::new();
    app.mount({
        let observed = Rc::clone(&observed);
        move || view! { <CleanupWriteHost observed={observed} /> }
    })
    .expect("mount");

    let error = app
        .click_label("close")
        .expect_err("a cleanup write is rejected");
    assert!(
        matches!(
            error,
            MountError::Context {
                operation: "Signal::set",
                ..
            }
        ),
        "unexpected error: {error:?}"
    );

    app.click_label("probe").expect("the runtime stays usable");
    assert_eq!(
        observed.get(),
        0,
        "the rejected cleanup write does not change the signal"
    );
}
