#![allow(unused_braces)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use zenui::prelude::*;

fn text_of(snapshot: &PresentationSnapshot, label: &str) -> String {
    snapshot
        .find_label(label)
        .unwrap_or_else(|| panic!("label {label}"))
        .text()
}

fn root_text(app: &Runtime, label: &str) -> String {
    text_of(&app.presentation().expect("snapshot"), label)
}

#[derive(Default)]
struct Counters {
    upstream: Cell<u32>,
    downstream: Cell<u32>,
    other: Cell<u32>,
}

fn bump(counter: &Cell<u32>) {
    counter.set(counter.get() + 1);
}

/// 自定义相等只在类型内部成立；类型擦除后 memo 必须仍使用它。
#[derive(Clone)]
struct Modulo(u32);

impl PartialEq for Modulo {
    fn eq(&self, other: &Self) -> bool {
        self.0 % 2 == other.0 % 2
    }
}

#[test]
fn type_erased_equality_suppresses_an_equal_downstream_recompute() {
    #[component]
    fn Probe(runs: Rc<Counters>) -> MountResult<MountPlan> {
        let source = signal(1_u32);
        let upstream_runs = runs.clone();
        let upstream = memo(move || {
            bump(&upstream_runs.upstream);
            Modulo(source.get())
        });
        let downstream_runs = runs.clone();
        let downstream = memo(move || {
            bump(&downstream_runs.downstream);
            format!("bucket={}", upstream.get().0 % 2)
        });
        view! {
            <view>
                <text aria:label="value">{downstream}</text>
                <button aria:label="three" on:click={move || source.set(3)}>"three"</button>
                <button aria:label="four" on:click={move || source.set(4)}>"four"</button>
            </view>
        }
    }

    let runs = Rc::new(Counters::default());
    let mut app = Runtime::new();
    app.mount({
        let runs = runs.clone();
        move || view! { <Probe runs={runs} /> }
    })
    .expect("mount");
    assert_eq!(root_text(&app, "value"), "bucket=1");
    assert_eq!(runs.upstream.get(), 1);
    assert_eq!(runs.downstream.get(), 1);
    let _ = app.refresh().expect("drain the mount report");

    app.click_label("three").expect("change the source");
    assert_eq!(
        runs.upstream.get(),
        2,
        "the upstream memo recomputes when its source changes"
    );
    assert_eq!(
        runs.downstream.get(),
        1,
        "a value-equal upstream recompute does not dirty downstream"
    );
    assert_eq!(root_text(&app, "value"), "bucket=1");
    assert!(
        !app.refresh().expect("refresh").changed(),
        "the equal recompute produces no effective binding change"
    );

    app.click_label("four").expect("change the bucket");
    assert_eq!(runs.upstream.get(), 3);
    assert_eq!(
        runs.downstream.get(),
        2,
        "a changed upstream value reaches downstream exactly once"
    );
    assert_eq!(root_text(&app, "value"), "bucket=0");
}

#[test]
fn three_level_chains_recompute_once_per_visible_change() {
    #[component]
    fn Chain(runs: Rc<Counters>) -> MountResult<MountPlan> {
        let source = signal(0_u32);
        let a_runs = runs.clone();
        let a = memo(move || {
            bump(&a_runs.upstream);
            source.get() + 1
        });
        let b_runs = runs.clone();
        let b = memo(move || {
            bump(&b_runs.downstream);
            a.get() + 1
        });
        let c_runs = runs.clone();
        let c = memo(move || {
            bump(&c_runs.other);
            format!("{}", b.get() + 1)
        });
        view! {
            <view>
                <text aria:label="value">{c}</text>
                <button aria:label="one" on:click={move || source.set(1)}>"one"</button>
            </view>
        }
    }

    let runs = Rc::new(Counters::default());
    let mut app = Runtime::new();
    app.mount({
        let runs = runs.clone();
        move || view! { <Chain runs={runs} /> }
    })
    .expect("mount");
    assert_eq!(root_text(&app, "value"), "3");
    assert_eq!(
        (runs.upstream.get(), runs.downstream.get(), runs.other.get()),
        (1, 1, 1)
    );

    app.click_label("one").expect("advance the chain");
    assert_eq!(root_text(&app, "value"), "4");
    assert_eq!(
        (runs.upstream.get(), runs.downstream.get(), runs.other.get()),
        (2, 2, 2),
        "each level recomputes exactly once"
    );

    app.click_label("one").expect("write the same source value");
    assert_eq!(
        (runs.upstream.get(), runs.downstream.get(), runs.other.get()),
        (2, 2, 2),
        "an equal source write does not enter the worklist"
    );
}

#[test]
fn a_diamond_recomputes_the_join_once_per_turn() {
    #[component]
    fn Diamond(runs: Rc<Counters>) -> MountResult<MountPlan> {
        let source = signal(1_u32);
        let left_runs = runs.clone();
        let left = memo(move || {
            bump(&left_runs.upstream);
            source.get() + 1
        });
        let right_runs = runs.clone();
        let right = memo(move || {
            bump(&right_runs.downstream);
            source.get() * 2
        });
        let sum_runs = runs.clone();
        let sum = memo(move || {
            bump(&sum_runs.other);
            format!("{}", left.get() + right.get())
        });
        view! {
            <view>
                <text aria:label="value">{sum}</text>
                <button aria:label="two" on:click={move || source.set(2)}>"two"</button>
                <button aria:label="three" on:click={move || source.set(3)}>"three"</button>
            </view>
        }
    }

    let runs = Rc::new(Counters::default());
    let mut app = Runtime::new();
    app.mount({
        let runs = runs.clone();
        move || view! { <Diamond runs={runs} /> }
    })
    .expect("mount");
    assert_eq!(root_text(&app, "value"), "4");

    app.click_label("two").expect("both branches change");
    assert_eq!(root_text(&app, "value"), "7");
    assert_eq!(
        (runs.upstream.get(), runs.downstream.get(), runs.other.get()),
        (2, 2, 2),
        "the join runs once even though both parents changed"
    );

    app.click_label("two").expect("same source value");
    assert_eq!(
        (runs.upstream.get(), runs.downstream.get(), runs.other.get()),
        (2, 2, 2)
    );

    app.click_label("three").expect("advance again");
    assert_eq!(root_text(&app, "value"), "10");
    assert_eq!(
        (runs.upstream.get(), runs.downstream.get(), runs.other.get()),
        (3, 3, 3)
    );
}

#[component]
fn WindowA(source: Signal<u32>, doubled: Memo<String>) -> MountResult<MountPlan> {
    view! {
        <view>
            <text aria:label="a-value">{doubled}</text>
            <button aria:label="inc-a" on:click={move || source.set(source.get() + 1)}>"inc"</button>
        </view>
    }
}

#[component]
fn WindowB(source: Signal<u32>, doubled: Memo<String>) -> MountResult<MountPlan> {
    view! {
        <view>
            <text aria:label="b-value">{doubled}</text>
            <button aria:label="inc-b" on:click={move || source.set(source.get() + 10)}>"inc"</button>
        </view>
    }
}

#[test]
fn a_shared_memo_publishes_to_two_windows() {
    let mut app = Runtime::new();
    // 应用 owner 的句柄跨窗口存活；两个窗口只是同一 memo 图的两个消费者。
    let (source, doubled) = app
        .setup(|| {
            let source = signal(1_u32);
            let doubled = memo(move || format!("{}", source.get() * 2));
            (source, doubled)
        })
        .expect("app setup");
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("a").primary(), move |_| {
            view! { <WindowA source={source} doubled={doubled} /> }
        })?;
        catalog.window(WindowSpec::new("b"), move |_| {
            view! { <WindowB source={source} doubled={doubled} /> }
        })?;
        Ok(())
    })
    .expect("declare windows");
    let a = app.show("a").expect("show a");
    let b = app.show("b").expect("show b");

    assert_eq!(
        text_of(&app.presentation_of(a).expect("a snapshot"), "a-value"),
        "2"
    );
    assert_eq!(
        text_of(&app.presentation_of(b).expect("b snapshot"), "b-value"),
        "2"
    );

    let inc_a = app
        .presentation_of(a)
        .expect("a snapshot")
        .find_label("inc-a")
        .expect("inc-a")
        .id();
    app.activate_node(inc_a).expect("increment from window a");
    assert_eq!(
        text_of(&app.presentation_of(a).expect("a snapshot"), "a-value"),
        "4"
    );
    assert_eq!(
        text_of(&app.presentation_of(b).expect("b snapshot"), "b-value"),
        "4",
        "the shared memo republishes in the other window"
    );

    let inc_b = app
        .presentation_of(b)
        .expect("b snapshot")
        .find_label("inc-b")
        .expect("inc-b")
        .id();
    app.activate_node(inc_b).expect("increment from window b");
    assert_eq!(
        text_of(&app.presentation_of(a).expect("a snapshot"), "a-value"),
        "24"
    );
    assert_eq!(
        text_of(&app.presentation_of(b).expect("b snapshot"), "b-value"),
        "24"
    );
}

#[test]
fn once_is_a_mount_snapshot_without_a_dependency() {
    #[component]
    fn Snapshot() -> MountResult<MountPlan> {
        let source = signal(1_u32);
        let snapshot = once(source.get().to_string());
        let live = memo(move || source.get().to_string());
        view! {
            <view>
                <text aria:label="snapshot">{snapshot}</text>
                <text aria:label="live">{live}</text>
                <button aria:label="bump" on:click={move || source.set(source.get() + 1)}>"bump"</button>
            </view>
        }
    }

    let mut app = Runtime::new();
    app.mount(|| view! { <Snapshot /> }).expect("mount");
    assert_eq!(root_text(&app, "snapshot"), "1");
    assert_eq!(root_text(&app, "live"), "1");

    app.click_label("bump").expect("advance the live source");
    assert_eq!(
        root_text(&app, "snapshot"),
        "1",
        "the once value is the mount snapshot"
    );
    assert_eq!(root_text(&app, "live"), "2");
}

#[component]
fn FieldText(
    ids: Signal<Option<TodoId>>,
    store: Store,
    counter: Signal<String>,
) -> MountResult<MountPlan> {
    let id = ids
        .get()
        .expect("the row is seeded before the branch mounts");
    let title = store.title(id);
    let completed = store.completed(id);
    view! {
        <view>
            <text aria:label="field">{title}</text>
            <text aria:label="counter">{counter}</text>
            <text-input
                aria:label="title-input"
                value={title}
                on:input={move |_value: String| {}}
            />
            <checkbox aria:label="done" checked={completed} />
        </view>
    }
}

#[test]
fn a_missing_field_keeps_the_last_value_and_reports_a_typed_fault() {
    #[component]
    fn Host(seen: Rc<Cell<Option<TodoId>>>) -> MountResult<MountPlan> {
        let store = Store::new();
        let ids = signal(Option::<TodoId>::None);
        let counter = signal(String::from("counter=0"));
        let seeded = memo(move || ids.get().is_some());
        let seed_seen = seen.clone();
        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    Transaction::new().write(store.insert("row")).finish()?;
                    let id = store
                        .keys(Filter::All)
                        .into_iter()
                        .next()
                        .expect("the inserted row");
                    ids.set(Some(id));
                    seed_seen.set(Some(id));
                    Ok(())
                }}>"Seed"</button>
                <button aria:label="complete" on:click={move || -> Result<(), WriteError> {
                    let id = ids.get().expect("seeded");
                    Transaction::new()
                        .write(store.set_completed(id, true))
                        .finish()?;
                    Ok(())
                }}>"Complete"</button>
                <button aria:label="delete" on:click={move || -> Result<(), WriteError> {
                    let id = ids.get().expect("seeded");
                    Transaction::new().write(store.remove(id)).finish()?;
                    Ok(())
                }}>"Delete"</button>
                <button aria:label="bump" on:click={move || {
                    counter.set(String::from("counter=1"));
                }}>"Bump"</button>
                <Show when={seeded}>
                    <FieldText ids={ids} store={store} counter={counter} />
                </Show>
            </view>
        }
    }

    let seen = Rc::new(Cell::new(None::<TodoId>));
    let mut app = Runtime::new();
    app.mount({
        let seen = seen.clone();
        move || view! { <Host seen={seen} /> }
    })
    .expect("mount");

    app.click_label("seed").expect("seed a row");
    assert_eq!(root_text(&app, "field"), "row");
    let input = app
        .presentation()
        .expect("snapshot")
        .find_label("title-input")
        .expect("input")
        .id();
    app.focus("title-input").expect("focus the field editor");
    app.set_caret_node(input, 3).expect("caret at the end");
    app.type_text_node(input, "X")
        .expect("the handler defers the model write");
    let snapshot = app.presentation().expect("snapshot");
    assert_eq!(
        snapshot
            .find_label("title-input")
            .expect("input")
            .text_value(),
        Some("rowX"),
        "a field binding without a write keeps the local edit"
    );
    app.click_label("complete").expect("complete the row");
    assert_eq!(app.checked("done"), Some(true));

    app.click_label("delete").expect("delete the row");
    let report = app.refresh().expect("observe the fault");
    let id = seen.get().expect("the seeded id");
    assert_eq!(
        text_of(&app.presentation().expect("snapshot"), "field"),
        "row",
        "the last committed text stays after the field disappears"
    );
    assert_eq!(
        app.checked("done"),
        Some(true),
        "the last committed boolean stays after the field disappears"
    );
    assert!(
        report
            .faults()
            .contains(&ReadFault::FieldMissing(FieldError::Missing(id))),
        "the failed read is reported as a typed fault: {:?}",
        report.faults()
    );

    app.click_label("bump")
        .expect("the retained branch still refreshes");
    assert_eq!(
        root_text(&app, "counter"),
        "counter=1",
        "a faulting Show keep its branch live instead of freezing it"
    );
}

#[test]
fn a_reported_context_fault_does_not_poison_the_next_refresh() {
    #[component]
    fn Probe() -> MountResult<MountPlan> {
        let source = signal(1_u32);
        let shown = signal(String::from("before"));
        view! {
            <view>
                <text aria:label="value">{shown}</text>
                <button aria:label="fault" on:click={move || {
                    source.with(|_| source.set(2));
                    shown.set(String::from("after"));
                }}>"Fault"</button>
            </view>
        }
    }

    let mut app = Runtime::new();
    app.mount(|| view! { <Probe /> }).expect("mount");
    let _ = app.refresh().expect("drain the mount report");

    let failure = app
        .click_label("fault")
        .expect_err("the illegal write is reported");
    assert!(matches!(
        failure,
        MountError::Context {
            operation: "Signal::set",
            ..
        }
    ));
    assert_eq!(root_text(&app, "value"), "before");

    let report = app
        .refresh()
        .expect("the stale fault does not abort the frame");
    assert!(report.changed());
    assert_eq!(
        root_text(&app, "value"),
        "after",
        "the legal write from the same turn is published by the next refresh"
    );
}

#[test]
fn a_dynamic_memo_cycle_terminates_without_spinning() {
    #[component]
    fn Cycle() -> MountResult<MountPlan> {
        let s = signal(false);
        let t = signal(false);
        let a_slot: Rc<RefCell<Option<Memo<u32>>>> = Rc::new(RefCell::new(None));
        let b_slot: Rc<RefCell<Option<Memo<u32>>>> = Rc::new(RefCell::new(None));
        let a = {
            let b_slot = b_slot.clone();
            memo(move || {
                if s.get() {
                    b_slot.borrow().expect("b is created").get() + 1
                } else {
                    0
                }
            })
        };
        let b = {
            let a_slot = a_slot.clone();
            memo(move || {
                if t.get() {
                    a_slot.borrow().expect("a is created").get() + 1
                } else {
                    100
                }
            })
        };
        *a_slot.borrow_mut() = Some(a);
        *b_slot.borrow_mut() = Some(b);
        let a_text = memo(move || format!("a={}", a.get()));
        let b_text = memo(move || format!("b={}", b.get()));
        view! {
            <view>
                <text aria:label="a">{a_text}</text>
                <text aria:label="b">{b_text}</text>
                <button aria:label="toggle-b" on:click={move || t.set(true)}>"B"</button>
                <button aria:label="toggle-a" on:click={move || s.set(true)}>"A"</button>
            </view>
        }
    }

    let mut app = Runtime::new();
    app.mount(|| view! { <Cycle /> }).expect("mount");
    let _ = app.refresh().expect("drain the mount report");
    app.click_label("toggle-b").expect("b starts reading a");
    app.click_label("toggle-a")
        .expect("the dynamic cycle is bounded, not a hang");
    // 本轮结束后运行时仍可刷新；无限结算会让这个调用永远不返回。
    app.refresh()
        .expect("the runtime stays responsive after a cycle");
}

#[test]
fn a_read_prop_setup_fault_is_a_typed_mount_error() {
    #[component]
    fn BadRead(value: Read<String>) -> MountResult<MountPlan> {
        let value = value.read_once()?;
        view! { <text>{once(value)}</text> }
    }

    #[component]
    fn BadHost() -> MountResult<MountPlan> {
        let failing: Memo<String> = memo(|| panic!("virgin memo boom"));
        view! { <BadRead value={failing} /> }
    }

    let mut app = Runtime::new();
    let failure = app
        .mount(|| view! { <BadHost /> })
        .expect_err("the setup read reports its fault");
    assert!(
        matches!(failure, MountError::Read(ReadFault::ComputedPanicked)),
        "unexpected error: {failure:?}"
    );
}

#[test]
fn a_store_write_from_a_memo_is_reported_after_settlement() {
    #[component]
    fn BadModel() -> MountResult<MountPlan> {
        let store = Store::new();
        let bad = memo(move || {
            let _ = Transaction::new().write(store.insert("ghost")).finish();
            String::from("bad")
        });
        view! { <text>{bad}</text> }
    }

    let mut app = Runtime::new();
    let failure = app
        .mount(|| view! { <BadModel /> })
        .expect_err("a memo write is rejected during settlement");
    assert!(
        matches!(
            failure,
            MountError::Write(WriteError::Context {
                operation: "Transaction::finish",
                ..
            })
        ),
        "unexpected error: {failure:?}"
    );
}

type Disposable = Rc<RefCell<Option<Signal<String>>>>;

#[component]
fn OwnerWindow(shared: Disposable) -> MountResult<MountPlan> {
    let value = signal(String::from("owned"));
    *shared.borrow_mut() = Some(value);
    view! { <text aria:label="owner">{value}</text> }
}

#[component]
fn BorrowerWindow(shared: Disposable) -> MountResult<MountPlan> {
    let value = shared.borrow().expect("the owner mounted first");
    view! { <text aria:label="borrower">{value}</text> }
}

#[test]
fn a_disposed_signal_keeps_the_last_value_and_reports_a_typed_fault() {
    let shared: Disposable = Rc::new(RefCell::new(None));
    let mut app = Runtime::new();
    app.declare_windows({
        let owner = shared.clone();
        let borrower = shared.clone();
        move |catalog| {
            catalog.window(WindowSpec::new("owner").primary(), {
                let owner = owner.clone();
                move |_| {
                    let owner = owner.clone();
                    view! { <OwnerWindow shared={owner} /> }
                }
            })?;
            catalog.window(WindowSpec::new("borrower"), {
                let borrower = borrower.clone();
                move |_| {
                    let borrower = borrower.clone();
                    view! { <BorrowerWindow shared={borrower} /> }
                }
            })?;
            Ok(())
        }
    })
    .expect("declare windows");
    let owner = app.show("owner").expect("show owner");
    let borrower = app.show("borrower").expect("show borrower");
    assert_eq!(
        text_of(
            &app.presentation_of(borrower).expect("borrower snapshot"),
            "borrower"
        ),
        "owned"
    );

    app.close_window(owner).expect("close the owner");
    let report = app.refresh().expect("the disposed read degrades");
    assert_eq!(
        text_of(
            &app.presentation_of(borrower).expect("borrower snapshot"),
            "borrower"
        ),
        "owned",
        "the borrower keeps its last committed text"
    );
    assert!(
        report.faults().contains(&ReadFault::Disposed),
        "the disposed source is a typed fault: {:?}",
        report.faults()
    );
}
