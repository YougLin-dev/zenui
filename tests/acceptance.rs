#![allow(dead_code, unused_braces)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use zenui::prelude::*;

fn mount(root: impl FnOnce() -> MountResult<MountPlan> + 'static) -> Runtime {
    let mut app = Runtime::new();
    app.mount(root).expect("mount");
    app
}

#[test]
fn show_and_children_factories_stay_deferred_and_repeatable() {
    #[component]
    fn Marked() -> MountResult<MountPlan> {
        view! { <text>"open"</text> }
    }

    #[component]
    fn Host(builds: Rc<Cell<u32>>) -> MountResult<MountPlan> {
        let expanded = signal(false);
        let fallback_builds = builds.clone();

        view! {
            <view>
                <Show
                    when={memo(move || expanded.get())}
                    fallback={move || {
                        fallback_builds.set(fallback_builds.get() + 1);
                        view! { <text>"closed"</text> }
                    }}
                >
                    <Marked />
                </Show>
                <button on:click={move || expanded.set(!expanded.get())}>"Toggle"</button>
            </view>
        }
    }

    let builds = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    assert_eq!(builds.get(), 0, "factories are not evaluated before mount");
    app.mount({
        let builds = builds.clone();
        move || view! { <Host builds={builds} /> }
    })
    .expect("mount");
    assert_eq!(
        builds.get(),
        1,
        "the fallback factory builds on first mount"
    );
    assert!(app.texts().iter().any(|text| text == "closed"));

    app.click_text("Toggle").expect("expand");
    assert!(app.texts().iter().any(|text| text == "open"));
    assert_eq!(app.setup_count("Marked"), 1, "the then branch mounts once");

    let unmounts = app.unmounts();
    app.click_text("Toggle").expect("collapse");
    assert!(app.texts().iter().any(|text| text == "closed"));
    assert_eq!(builds.get(), 2, "the fallback factory re-enters");
    assert!(app.unmounts() > unmounts, "the then branch is unmounted");

    app.click_text("Toggle").expect("expand again");
    assert_eq!(
        app.setup_count("Marked"),
        2,
        "re-entering Show mounts a fresh component"
    );
}

#[test]
fn a_failing_component_keeps_committed_siblings_and_releases_setup_state() {
    fn guard(armed: bool) -> MountResult<()> {
        if armed {
            return Err(MountError::Component("bomb"));
        }
        Ok(())
    }

    #[component]
    fn Bomb(armed: bool) -> MountResult<MountPlan> {
        let setup_signal = signal(1);
        let _setup_memo = memo(move || format!("{}", setup_signal.get()));
        guard(armed)?;
        view! { <text>"armed"</text> }
    }

    #[component]
    fn Host() -> MountResult<MountPlan> {
        let broken = signal(false);
        let when = memo(move || broken.get());

        view! {
            <view>
                <text>"stable"</text>
                <Show when={when}>
                    <Bomb armed={true} />
                </Show>
                <button on:click={move || broken.set(true)}>"Break"</button>
                <button on:click={move || broken.set(false)}>"Fix"</button>
            </view>
        }
    }

    let mut app = mount(|| view! { <Host /> });
    let baseline_signals = app.live_signals();
    let baseline_memos = app.live_memos();
    assert!(app.texts().iter().any(|text| text == "stable"));

    let failure = app.click_text("Break").expect_err("the branch mount fails");
    assert_eq!(failure, MountError::Component("bomb"));
    assert_eq!(
        app.live_signals(),
        baseline_signals,
        "the failed component releases its setup signal"
    );
    assert_eq!(
        app.live_memos(),
        baseline_memos,
        "the failed component releases its setup memo"
    );
    assert!(app.texts().iter().any(|text| text == "stable"));
    assert!(
        !app.texts().iter().any(|text| text == "armed"),
        "the proposed branch is not published"
    );

    app.click_text("Fix")
        .expect("the next legal command recovers");
    assert!(app.texts().iter().any(|text| text == "stable"));
}

#[test]
fn illegal_context_writes_fail_before_state_changes_and_recover() {
    #[component]
    fn Probe() -> MountResult<MountPlan> {
        let store = Store::new();
        let value = signal(0);
        let shown = memo(move || format!("v={}", value.get()));
        let row_count = memo(move || store.rows(Filter::All).len());
        let rows = memo(move || format!("n={}", row_count.get()));
        let bad_signal = memo(move || {
            value.set(99);
            0
        });
        let bad_model = memo(move || {
            let _ = Transaction::new().write(store.insert("ghost")).finish();
            0
        });

        view! {
            <view>
                <text>{shown}</text>
                <text>{rows}</text>
                <button on:click={move || {
                    let _ = bad_signal.get();
                }}>"BadSignal"</button>
                <button on:click={move || {
                    let _ = bad_model.get();
                }}>"BadModel"</button>
                <button on:click={move || value.set(value.get() + 1)}>"Good"</button>
            </view>
        }
    }

    let mut app = mount(|| view! { <Probe /> });
    assert!(app.texts().iter().any(|text| text == "v=0"));
    assert!(app.texts().iter().any(|text| text == "n=0"));

    let failure = app
        .click_text("BadSignal")
        .expect_err("a memo write is rejected");
    assert!(matches!(
        failure,
        MountError::Context {
            operation: "Signal::set",
            ..
        }
    ));
    assert!(
        app.texts().iter().any(|text| text == "v=0"),
        "the rejected write did not change the signal"
    );

    app.click_text("Good").expect("a legal event still runs");
    assert!(app.texts().iter().any(|text| text == "v=1"));

    let failure = app
        .click_text("BadModel")
        .expect_err("the rejected model write reaches the command boundary");
    assert!(matches!(
        failure,
        MountError::Write(WriteError::Context {
            operation: "Transaction::finish",
            ..
        })
    ));
    assert!(
        app.texts().iter().any(|text| text == "n=0"),
        "no model row is published from a memo"
    );

    app.click_text("Good").expect("later legal recovery");
    assert!(app.texts().iter().any(|text| text == "v=2"));
}

#[test]
fn duplicate_for_keys_fail_before_publication() {
    #[component]
    fn BadList() -> MountResult<MountPlan> {
        let store = Store::new();
        let rows = memo(move || store.rows(Filter::All));
        let first = Cell::new(None);

        view! {
            <view>
                <button on:click={move || {
                    let _ = Transaction::new().write(store.insert("A")).finish();
                    let _ = Transaction::new().write(store.insert("B")).finish();
                }}>"Seed"</button>
                <For
                    each={rows}
                    key={move |id: TodoId| {
                        if first.get().is_none() {
                            first.set(Some(id));
                        }
                        first.get().expect("a first key")
                    }}
                >
                    {move |id: TodoId| view! { <text>{once(id.raw().to_string())}</text> }}
                </For>
            </view>
        }
    }

    let mut app = mount(|| view! { <BadList /> });
    let before = app.texts();

    let failure = app
        .click_text("Seed")
        .expect_err("duplicate keys reject the frame");
    assert_eq!(failure, MountError::DuplicateKey);
    assert_eq!(app.texts(), before, "the committed interface is untouched");
    assert!(app.for_items().is_empty(), "no duplicate row was mounted");
}

#[test]
fn shared_rowsets_come_from_the_target_query_cache() {
    #[component]
    fn Shared() -> MountResult<MountPlan> {
        let store = Store::new();
        let left = memo(move || store.rows(Filter::All));
        let right = memo(move || store.rows(Filter::All));
        let shared = memo(move || left.get().shares_with(&right.get()));
        let count = memo(move || store.rows(Filter::All).len());
        let label = memo(move || String::from(if shared.get() { "shared" } else { "split" }));
        let rows = memo(move || format!("n={}", count.get()));

        view! {
            <view>
                <text>{label}</text>
                <text>{rows}</text>
                <button on:click={move || {
                    let _ = Transaction::new().write(store.insert("x")).finish();
                }}>"Insert"</button>
            </view>
        }
    }

    let mut app = mount(|| view! { <Shared /> });
    assert!(app.texts().iter().any(|text| text == "shared"));
    assert!(app.texts().iter().any(|text| text == "n=0"));
    let evals = app.counters().query_evals;
    app.click_text("Insert").expect("insert");
    assert!(app.counters().query_evals > evals, "the cache rebuilds");
    assert!(app.texts().iter().any(|text| text == "n=1"));
    assert!(
        app.texts().iter().any(|text| text == "shared"),
        "two consumers keep one RowSet projection"
    );
}

#[test]
fn row_visits_counts_the_rows_a_filter_rebuild_scanned() {
    #[component]
    fn ScanHost() -> MountResult<MountPlan> {
        let store = Store::new();
        let active = memo(move || store.rows(Filter::Active));
        let label = memo(move || format!("active={}", active.get().len()));

        view! {
            <view>
                <button aria:label="seed" on:click={move || -> Result<(), WriteError> {
                    Transaction::new()
                        .write(store.insert("a"))
                        .write(store.insert("b"))
                        .finish()
                }}>"Seed"</button>
                <text>{label}</text>
                <For each={active} key={|id: TodoId| id}>
                    {move |id: TodoId| view! {
                        <button
                            aria:label="done"
                            on:click={move || -> Result<(), WriteError> {
                                Transaction::new()
                                    .write(store.set_completed(id, true))
                                    .finish()
                            }}
                        >"Done"</button>
                    }}
                </For>
            </view>
        }
    }

    let mut app = mount(|| view! { <ScanHost /> });
    assert!(app.texts().iter().any(|text| text == "active=0"));

    let before = app.counters().row_visits;
    app.click_label("seed").expect("seed two rows");
    assert_eq!(
        app.counters().row_visits - before,
        2,
        "a rebuild scans every live row, not just the matching ones"
    );
    assert!(app.texts().iter().any(|text| text == "active=2"));

    let before = app.counters().row_visits;
    app.click_label("done").expect("complete the first row");
    assert_eq!(
        app.counters().row_visits - before,
        2,
        "the remaining active row still pays for the whole scan"
    );

    let before = app.counters().row_visits;
    app.click_label("done").expect("complete the second row");
    assert_eq!(
        app.counters().row_visits,
        before,
        "an empty filter is short-circuited without scanning"
    );
    assert!(app.texts().iter().any(|text| text == "active=0"));
}

#[test]
fn typed_key_events_reach_the_target_handler() {
    #[component]
    fn Editor() -> MountResult<MountPlan> {
        let text = signal(String::from("draft"));

        view! {
            <view>
                <text>{text}</text>
                <text-input
                    value={text}
                    aria:label="Editor"
                    on:keydown={move |event: &mut KeyEvent| {
                        if event.key() == Key::Escape {
                            text.set(String::new());
                        }
                    }}
                />
            </view>
        }
    }

    let mut app = mount(|| view! { <Editor /> });
    app.keydown("Editor", Key::Escape).expect("keydown");
    assert_eq!(app.texts(), vec![String::new()]);
}

#[test]
fn signal_with_rejects_writes_without_changing_state() {
    #[component]
    fn Guarded() -> MountResult<MountPlan> {
        let value = signal(0);
        let shown = memo(move || format!("v={}", value.get()));

        view! {
            <view>
                <text>{shown}</text>
                <button on:click={move || {
                    value.with(|_| value.set(9));
                }}>"Write"</button>
                <button on:click={move || value.set(value.get() + 1)}>"Good"</button>
            </view>
        }
    }

    let mut app = mount(|| view! { <Guarded /> });
    let failure = app
        .click_text("Write")
        .expect_err("a write inside with is rejected");
    assert!(matches!(
        failure,
        MountError::Context {
            operation: "Signal::set",
            ..
        }
    ));
    assert!(app.texts().iter().any(|text| text == "v=0"));

    app.click_text("Good").expect("a legal event recovers");
    assert!(app.texts().iter().any(|text| text == "v=1"));
}

#[test]
fn a_setup_write_is_rejected_and_the_mount_can_retry() {
    #[component]
    fn BadSetup() -> MountResult<MountPlan> {
        let value = signal(0);
        value.set(1);
        let shown = memo(move || value.get().to_string());
        view! { <text>{shown}</text> }
    }

    #[component]
    fn Good() -> MountResult<MountPlan> {
        view! { <text>"ok"</text> }
    }

    let mut app = Runtime::new();
    let failure = app
        .mount(|| view! { <BadSetup /> })
        .expect_err("setup writes are rejected");
    assert!(matches!(
        failure,
        MountError::Context {
            operation: "Signal::set",
            ..
        }
    ));
    assert_eq!(app.live_signals(), 0, "the failed setup signal is released");

    app.mount(|| view! { <Good /> })
        .expect("the same Runtime accepts a later legal mount");
    assert_eq!(app.texts(), vec!["ok"]);
}

#[test]
fn a_handle_cannot_cross_runtimes() {
    #[component]
    fn Leak(slot: Rc<RefCell<Option<Signal<i32>>>>) -> MountResult<MountPlan> {
        *slot.borrow_mut() = Some(signal(1));
        view! { <text>"leak"</text> }
    }

    let slot = Rc::new(RefCell::new(None));
    let mut first = Runtime::new();
    first
        .mount({
            let slot = slot.clone();
            move || view! { <Leak slot={slot} /> }
        })
        .expect("first runtime");
    let leaked = slot.borrow().expect("a signal handle");

    let mut second = Runtime::new();
    second
        .mount(move || {
            view! {
                <view>
                    <button on:click={move || {
                        let _ = leaked.get();
                    }}>"Touch"</button>
                </view>
            }
        })
        .expect("second runtime");
    let report = second
        .click_text("Touch")
        .expect("the click turn contains the foreign read");
    assert!(matches!(
        report.errors(),
        [TurnError::HandlerPanicked(panic)] if panic.message().contains("another ZenUI Runtime")
    ));
}

#[test]
fn children_and_named_slots_take_their_own_factory_timing() {
    #[component]
    fn Panel(
        title: &'static str,
        actions: impl Into<ChildrenFactory>,
        children: impl Into<Children>,
    ) -> MountResult<MountPlan> {
        let actions = actions.into();
        let children = children.into();

        view! {
            <view>
                <text>{title}</text>
                <view>{actions}</view>
                <view>{children}</view>
            </view>
        }
    }

    let builds = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let builds = builds.clone();
        move || {
            view! {
                <Panel
                    title="P"
                    actions={move || {
                        builds.set(builds.get() + 1);
                        view! { <text>"act"</text> }
                    }}
                >
                    <text>"body"</text>
                </Panel>
            }
        }
    })
    .expect("mount");

    assert_eq!(builds.get(), 1, "the named slot factory runs once");
    assert_eq!(app.texts(), vec!["P", "act", "body"]);
}

#[test]
fn a_store_cannot_write_across_runtimes() {
    #[component]
    fn LeakStore(slot: Rc<RefCell<Option<Store>>>) -> MountResult<MountPlan> {
        *slot.borrow_mut() = Some(Store::new());
        view! { <text>"leak"</text> }
    }

    let slot = Rc::new(RefCell::new(None));
    let mut first = Runtime::new();
    first
        .mount({
            let slot = slot.clone();
            move || view! { <LeakStore slot={slot} /> }
        })
        .expect("first runtime");
    let leaked = slot.borrow().expect("a store handle");

    let mut second = Runtime::new();
    second
        .mount(move || {
            view! {
                <view>
                    <button on:click={move || {
                        let _ = Transaction::new().write(leaked.insert("x")).finish();
                    }}>"Touch"</button>
                </view>
            }
        })
        .expect("second runtime");
    let report = second
        .click_text("Touch")
        .expect("the click turn contains the foreign write");
    assert!(matches!(
        report.errors(),
        [TurnError::HandlerPanicked(panic)] if panic.message().contains("another ZenUI Runtime")
    ));
}

#[test]
fn slot_factory_resources_are_released_with_their_branch() {
    #[component]
    fn Slotted(actions: impl Into<ChildrenFactory>) -> MountResult<MountPlan> {
        let actions = actions.into();
        view! { <view>{actions}</view> }
    }

    #[component]
    fn Host() -> MountResult<MountPlan> {
        let expanded = signal(false);
        let when = memo(move || expanded.get());

        view! {
            <view>
                <Show when={when}>
                    <Slotted actions={move || {
                        let _factory_signal = signal(1);
                        let shown = memo(move || _factory_signal.get().to_string());
                        view! { <text>{shown}</text> }
                    }} />
                </Show>
                <button on:click={move || expanded.set(!expanded.get())}>"Toggle"</button>
            </view>
        }
    }

    let mut app = mount(|| view! { <Host /> });
    let signals = app.live_signals();
    let memos = app.live_memos();

    app.click_text("Toggle").expect("expand");
    assert!(
        app.live_signals() > signals,
        "the slot factory owns a signal"
    );
    app.click_text("Toggle").expect("collapse");
    assert_eq!(
        app.live_signals(),
        signals,
        "the slot signal is released with its branch"
    );
    assert_eq!(
        app.live_memos(),
        memos,
        "the slot memo is released with its branch"
    );
}

#[test]
fn stale_field_and_write_errors_are_reported() {
    #[component]
    fn StaleProbe() -> MountResult<MountPlan> {
        let store = Store::new();
        let last = Rc::new(Cell::new(None::<TodoId>));
        let shown = signal(String::from("none"));
        let item_last = last.clone();
        let delete_last = last.clone();
        let probe_last = last.clone();
        let again_last = last.clone();
        let rows = memo(move || store.rows(Filter::All));

        view! {
            <view>
                <button on:click={move || {
                    let _ = Transaction::new().write(store.insert("A")).finish();
                }}>"Seed"</button>
                <button on:click={move || {
                    if let Some(id) = delete_last.get() {
                        let _ = Transaction::new().write(store.remove(id)).finish();
                    }
                }}>"Delete"</button>
                <button on:click={move || {
                    if let Some(id) = probe_last.get() {
                        shown.set(format!("{:?}", store.title(id).get()));
                    }
                }}>"Probe"</button>
                <button on:click={move || {
                    if let Some(id) = again_last.get() {
                        let _ = Transaction::new().write(store.remove(id)).finish();
                    }
                }}>"DeleteAgain"</button>
                <text>{shown}</text>
                <For each={rows} key={|id: TodoId| id}>
                    {move |id: TodoId| {
                        item_last.set(Some(id));
                        view! { <text>{once(id.raw().to_string())}</text> }
                    }}
                </For>
            </view>
        }
    }

    let mut app = mount(|| view! { <StaleProbe /> });
    app.click_text("Seed").expect("seed");
    app.click_text("Delete").expect("delete");
    app.click_text("Probe").expect("probe");
    assert!(
        app.texts().iter().any(|text| text.contains("Missing")),
        "a removed field reads as an invalid handle"
    );

    let failure = app
        .click_text("DeleteAgain")
        .expect_err("a stale write is reported");
    assert!(matches!(failure, MountError::Write(WriteError::Missing(_))));
}

#[test]
fn a_failing_root_plan_releases_its_setup_resources() {
    let mut app = Runtime::new();
    let failure = app
        .mount(|| {
            let _root_signal = signal(1);
            Err(MountError::Component("root"))
        })
        .expect_err("the root plan fails");
    assert_eq!(failure, MountError::Component("root"));
    assert_eq!(app.live_signals(), 0, "root setup resources are released");

    #[component]
    fn Good() -> MountResult<MountPlan> {
        view! { <text>"ok"</text> }
    }

    app.mount(|| view! { <Good /> })
        .expect("the same Runtime retries");
    assert_eq!(app.texts(), vec!["ok"]);
}
