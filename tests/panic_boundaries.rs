#![allow(unused_braces)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use zenui::prelude::*;

fn panic_message(error: MountError) -> String {
    match error {
        MountError::Panic(panic) | MountError::FactoryPanicked(panic) => panic.message().to_owned(),
        other => panic!("expected a contained panic, got {other:?}"),
    }
}

fn report_panic_message(report: &TurnReport) -> String {
    match report.errors() {
        [TurnError::FactoryPanicked(panic) | TurnError::HandlerPanicked(panic)] => {
            panic.message().to_owned()
        }
        other => panic!("expected a contained panic, got {other:?}"),
    }
}

#[test]
fn a_panicking_click_handler_leaves_the_event_slot_reusable() {
    #[component]
    fn Targets(armed: Rc<Cell<bool>>, clicks: Rc<Cell<u32>>) -> MountResult<MountPlan> {
        let click_armed = armed.clone();
        let click_count = clicks.clone();
        let other_count = clicks.clone();
        view! {
            <view>
                <button on:click={move || {
                    click_count.set(click_count.get() + 1);
                    if click_armed.replace(false) {
                        panic!("click boom");
                    }
                }}>"Target"</button>
                <button on:click={move || other_count.set(other_count.get() + 10)}>"Other"</button>
            </view>
        }
    }

    let armed = Rc::new(Cell::new(true));
    let clicks = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let armed = armed.clone();
        let clicks = clicks.clone();
        move || view! { <Targets armed={armed} clicks={clicks} /> }
    })
    .expect("mount");

    let report = app
        .click_text("Target")
        .expect("the click turn contains the handler panic");
    assert!(
        matches!(
            report.errors(),
            [TurnError::HandlerPanicked(panic)] if panic.message() == "click boom"
        ),
        "the handler panic is reported in the turn"
    );
    assert_eq!(clicks.get(), 1, "writes before the panic are preserved");

    app.click_text("Target")
        .expect("the event slot is reusable after a panic");
    assert_eq!(clicks.get(), 2);
    app.click_text("Other")
        .expect("other event slots still work");
    assert_eq!(clicks.get(), 12);
}

#[test]
fn a_panicking_memo_keeps_its_last_value_until_a_dependency_changes() {
    #[component]
    fn Derived(armed: Rc<Cell<bool>>) -> MountResult<MountPlan> {
        let source = signal(1_u32);
        let shown = signal(String::from("clean"));
        let compute_armed = armed.clone();
        let derived = memo(move || {
            let value = source.get();
            if compute_armed.get() {
                panic!("memo boom");
            }
            format!("value={value}")
        });
        let break_armed = armed.clone();
        let heal_armed = armed.clone();
        view! {
            <view>
                <text>{derived}</text>
                <button on:click={move || {
                    break_armed.set(true);
                    source.set(source.get() + 1);
                }}>"Break"</button>
                <button on:click={move || {
                    heal_armed.set(false);
                    source.set(source.get() + 1);
                }}>"Heal"</button>
                <button on:click={move || {
                    shown.set(match derived.last_panic() {
                        Some(panic) => panic.message().to_owned(),
                        None => String::from("clean"),
                    });
                }}>"Probe"</button>
                <text>{shown}</text>
            </view>
        }
    }

    let armed = Rc::new(Cell::new(false));
    let mut app = Runtime::new();
    app.mount({
        let armed = armed.clone();
        move || view! { <Derived armed={armed} /> }
    })
    .expect("mount");
    assert!(app.texts().iter().any(|text| text == "value=1"));

    app.click_text("Break")
        .expect("the poisoned memo is not fatal");
    assert!(
        app.texts().iter().any(|text| text == "value=1"),
        "the last committed value stays visible"
    );
    let report = app.refresh().expect("refresh reports the poison fault");
    assert!(
        report.faults().contains(&ReadFault::ComputedPanicked),
        "a poisoned memo with a last value is still a typed binding fault: {:?}",
        report.faults()
    );

    app.click_text("Probe").expect("probe");
    assert!(
        app.texts().iter().any(|text| text == "memo boom"),
        "the poison is exposed as a typed diagnostic"
    );

    app.click_text("Heal")
        .expect("dependency change heals the memo");
    assert!(
        app.texts().iter().any(|text| text == "value=3"),
        "a dependency change recomputes the poisoned memo"
    );
    app.click_text("Probe").expect("probe after recovery");
    assert!(app.texts().iter().any(|text| text == "clean"));
}

#[test]
fn a_panicking_show_factory_keeps_the_old_branch_and_releases_the_owner() {
    #[component]
    fn ShowHost(when: Memo<bool>, then: ChildrenFactory) -> MountResult<MountPlan> {
        Ok(MountPlan::new(vec![zenui::view::show(when, then, None)]))
    }

    #[component]
    fn Switch(armed: Rc<Cell<bool>>) -> MountResult<MountPlan> {
        let expanded = signal(false);
        let when = memo(move || expanded.get());
        let then: ChildrenFactory = ChildrenFactory::from({
            let factory_armed = armed.clone();
            move || {
                if factory_armed.get() {
                    let _probe = signal(1);
                    panic!("show factory boom");
                }
                view! { <text>"open"</text> }
            }
        });
        view! {
            <view>
                <ShowHost when={when} then={then} />
                <button on:click={move || expanded.set(!expanded.get())}>"Toggle"</button>
                <button on:click={move || armed.set(false)}>"Disarm"</button>
            </view>
        }
    }

    let armed = Rc::new(Cell::new(true));
    let mut app = Runtime::new();
    app.mount({
        let armed = armed.clone();
        move || view! { <Switch armed={armed} /> }
    })
    .expect("mount");
    let signals = app.live_signals();
    assert!(!app.texts().iter().any(|text| text == "open"));

    let report = app
        .click_text("Toggle")
        .expect("the show factory panic is contained");
    assert_eq!(report_panic_message(&report), "show factory boom");
    assert!(
        !app.texts().iter().any(|text| text == "open"),
        "the old branch stays mounted"
    );
    assert_eq!(
        app.live_signals(),
        signals,
        "the proposed branch owner released its resources"
    );

    app.click_text("Disarm").expect("the runtime stays usable");
    app.click_text("Toggle").expect("toggle back");
    app.click_text("Toggle")
        .expect("the disarmed factory mounts");
    assert!(app.texts().iter().any(|text| text == "open"));
}

#[test]
fn a_panicking_for_factory_keeps_old_rows_and_releases_the_proposed_owner() {
    #[component]
    fn Rows(armed: Rc<Cell<bool>>) -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        let item_armed = armed.clone();
        view! {
            <view>
                <button on:click={move || {
                    let _ = Transaction::new().write(store.insert("A")).finish();
                }}>"Seed"</button>
                <button on:click={move || armed.set(false)}>"Disarm"</button>
                <For each={rows} store={store} key={|row: &TodoRow| row.id}>
                    {move |row: RowView<TodoRow>| {
                        if item_armed.get() {
                            let _probe = signal(1);
                            panic!("for factory boom");
                        }
                        view! { <text>{once(format!("row {}", row.key().raw()))}</text> }
                    }}
                </For>
            </view>
        }
    }

    let armed = Rc::new(Cell::new(true));
    let mut app = Runtime::new();
    app.mount({
        let armed = armed.clone();
        move || view! { <Rows armed={armed} /> }
    })
    .expect("mount");
    let signals = app.live_signals();

    let report = app
        .click_text("Seed")
        .expect("the item factory panic is contained");
    assert_eq!(report_panic_message(&report), "for factory boom");
    assert!(app.for_items().is_empty(), "no partial row was mounted");
    assert_eq!(
        app.live_signals(),
        signals,
        "the proposed row owner released its resources"
    );

    app.click_text("Disarm")
        .expect("the runtime stays usable after the panic");
    assert_eq!(
        app.for_items().len(),
        1,
        "the next refresh mounts the pending row"
    );
}

#[test]
fn a_panicking_component_setup_releases_resources_and_mount_can_retry() {
    #[component]
    fn Bomb() -> MountResult<MountPlan> {
        let _probe = signal(1);
        panic!("component setup boom");
    }

    #[component]
    fn Calm() -> MountResult<MountPlan> {
        view! { <text>"calm"</text> }
    }

    let mut app = Runtime::new();
    assert_eq!(
        panic_message(
            app.mount(|| view! { <Bomb /> })
                .expect_err("component setup panic is contained")
        ),
        "component setup boom"
    );
    assert_eq!(
        app.live_signals(),
        0,
        "the failed setup released its signal"
    );

    app.mount(|| view! { <Calm /> })
        .expect("the runtime can mount after a failed setup");
    assert!(app.texts().iter().any(|text| text == "calm"));
}

#[test]
fn a_memo_cycle_becomes_a_graph_cycle_binding_fault() {
    #[component]
    fn Cycle() -> MountResult<MountPlan> {
        let a_slot: Rc<RefCell<Option<Memo<String>>>> = Rc::new(RefCell::new(None));
        let b_slot: Rc<RefCell<Option<Memo<String>>>> = Rc::new(RefCell::new(None));
        let a = {
            let b_slot = b_slot.clone();
            memo(move || b_slot.borrow().expect("b").get())
        };
        let b = {
            let a_slot = a_slot.clone();
            memo(move || a_slot.borrow().expect("a").get())
        };
        *a_slot.borrow_mut() = Some(a);
        *b_slot.borrow_mut() = Some(b);
        view! { <text>{a}</text> }
    }

    let mut app = Runtime::new();
    app.mount(|| view! { <Cycle /> }).expect("mount");
    let report = app.refresh().expect("the cycle is a binding fault");
    assert!(
        report.faults().contains(&ReadFault::GraphCycle),
        "the cycle is reported as GraphCycle, not a panic: {:?}",
        report.faults()
    );
    assert!(
        app.texts().iter().all(|text| !text.contains("cycle")),
        "no poisoned content is rendered"
    );
}

#[test]
fn a_virgin_memo_panic_becomes_a_binding_fault_instead_of_a_mount_panic() {
    #[component]
    fn ShowHost(when: Memo<bool>, then: ChildrenFactory) -> MountResult<MountPlan> {
        Ok(MountPlan::new(vec![zenui::view::show(when, then, None)]))
    }

    #[component]
    fn Host() -> MountResult<MountPlan> {
        let expanded = signal(false);
        let when = memo(move || expanded.get());
        let failing: Memo<String> = memo(|| panic!("virgin memo boom"));
        let then = ChildrenFactory::from(move || view! { <text>{failing}</text> });
        view! {
            <view>
                <ShowHost when={when} then={then} />
                <button on:click={move || expanded.set(!expanded.get())}>"Toggle"</button>
            </view>
        }
    }

    let mut app = Runtime::new();
    app.mount(|| view! { <Host /> }).expect("mount");

    app.click_text("Toggle")
        .expect("the memo fault stays inside the binding");
    let report = app.refresh().expect("refresh reports the fault");
    assert!(
        report.faults().contains(&ReadFault::ComputedPanicked),
        "the poisoned virgin memo is a typed binding fault: {:?}",
        report.faults()
    );
    assert!(
        app.texts()
            .iter()
            .all(|text| !text.contains("virgin memo boom")),
        "the poison message is not rendered as content"
    );
    app.click_text("Toggle")
        .expect("the runtime stays usable after the memo fault");
}

#[test]
fn a_panicking_window_factory_is_typed_and_the_activation_can_retry() {
    let armed = Rc::new(Cell::new(true));
    let mut app = Runtime::new();
    {
        let armed = armed.clone();
        app.declare_windows(move |catalog| {
            catalog.window(WindowSpec::new("main").primary(), move |_| {
                if armed.get() {
                    let _probe = signal(1);
                    panic!("window factory boom");
                }
                view! { <text>"main"</text> }
            })?;
            Ok(())
        })
        .expect("declare the window");
    }

    let error = app
        .show("main")
        .expect_err("the factory panic is contained");
    match error {
        WindowError::Mount(MountError::Panic(panic)) => {
            assert_eq!(panic.message(), "window factory boom");
        }
        other => panic!("expected a contained window panic, got {other:?}"),
    }
    assert_eq!(
        app.live_signals(),
        0,
        "the aborted activation released its factory resources"
    );

    armed.set(false);
    app.show("main")
        .expect("the next activation retries the factory");
    assert!(app.texts().iter().any(|text| text == "main"));
}
