#![allow(unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;

#[component]
fn TwoButtons(focuses: Rc<Cell<u32>>, blurs: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let a_focus = focuses.clone();
    let a_blur = blurs.clone();
    let b_focus = focuses.clone();
    let b_blur = blurs.clone();

    view! {
        <view>
            <button
                aria:label="A"
                on:focus={move |event: FocusEvent| {
                    assert_eq!(event.cause(), FocusCause::Command);
                    a_focus.set(a_focus.get() + 1);
                }}
                on:blur={move |_event: BlurEvent| a_blur.set(a_blur.get() + 1)}
            >"A"</button>
            <button
                aria:label="B"
                on:focus={move |event: FocusEvent| {
                    assert_eq!(event.cause(), FocusCause::Command);
                    b_focus.set(b_focus.get() + 1);
                }}
                on:blur={move |_event: BlurEvent| b_blur.set(b_blur.get() + 1)}
            >"B"</button>
        </view>
    }
}

fn buttons() -> (Runtime, Rc<Cell<u32>>, Rc<Cell<u32>>) {
    let focuses = Rc::new(Cell::new(0));
    let blurs = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let focuses = focuses.clone();
        let blurs = blurs.clone();
        move || view! { <TwoButtons focuses={focuses} blurs={blurs} /> }
    })
    .expect("mount");
    (app, focuses, blurs)
}

#[test]
fn moving_focus_dispatches_blur_and_focus_exactly_once() {
    let (mut app, focuses, blurs) = buttons();
    assert_eq!(app.focused(), None);

    app.focus("A").expect("focus A");
    assert_eq!(
        app.focused(),
        Some(FocusInfo {
            tag: "button",
            label: Some(String::from("A")),
        })
    );
    assert_eq!(focuses.get(), 1);
    assert_eq!(blurs.get(), 0, "the first focus has no previous target");

    app.focus("A")
        .expect("focusing the current target is a no-op");
    assert_eq!(focuses.get(), 1);
    assert_eq!(blurs.get(), 0);

    app.focus("B").expect("focus B");
    assert_eq!(focuses.get(), 2);
    assert_eq!(blurs.get(), 1, "the old target blurs exactly once");
    assert_eq!(
        app.focused().and_then(|focus| focus.label),
        Some(String::from("B"))
    );

    app.blur().expect("clear focus");
    assert_eq!(blurs.get(), 2, "the target blurs exactly once");
    assert_eq!(focuses.get(), 2, "clearing focus dispatches no focus event");
    assert_eq!(app.focused(), None);

    app.blur().expect("blur without focus is a no-op");
    assert_eq!(blurs.get(), 2);
}

#[component]
fn Writer() -> MountResult<MountPlan> {
    let status = signal(String::from("idle"));
    let text = memo(move || status.get());

    view! {
        <view>
            <text>{text}</text>
            <button
                aria:label="writer"
                on:focus={move |_event: FocusEvent| status.set(String::from("focused"))}
                on:blur={move |_event: BlurEvent| status.set(String::from("blurred"))}
            >"Writer"</button>
        </view>
    }
}

#[test]
fn focus_handlers_run_as_event_turns_and_update_bindings() {
    let mut app = Runtime::new();
    app.mount(|| view! { <Writer /> }).expect("mount");
    assert_eq!(app.texts(), vec!["idle", "Writer"]);

    app.focus("writer").expect("focus");
    assert_eq!(
        app.texts(),
        vec!["focused", "Writer"],
        "the focus handler runs in the event phase and refreshes the bound text"
    );

    app.blur().expect("blur");
    assert_eq!(app.texts(), vec!["blurred", "Writer"]);
}

#[component]
fn HandlerOnly() -> MountResult<MountPlan> {
    let touched = signal(0);
    let text = memo(move || format!("touched={}", touched.get()));

    view! {
        <view>
            <text>{text}</text>
            <view aria:label="panel" on:focus={move |_event: FocusEvent| {
                touched.set(touched.get() + 1);
            }}>
                <text>"panel"</text>
            </view>
        </view>
    }
}

#[test]
fn an_explicit_focus_handler_makes_a_non_control_focusable() {
    let mut app = Runtime::new();
    app.mount(|| view! { <HandlerOnly /> }).expect("mount");

    app.focus("panel")
        .expect("the handler makes the view focusable");
    assert_eq!(
        app.focused(),
        Some(FocusInfo {
            tag: "view",
            label: Some(String::from("panel")),
        })
    );
    assert_eq!(app.texts(), vec!["touched=1", "panel"]);
}

#[component]
fn Inner() -> MountResult<MountPlan> {
    view! { <text-input aria:label="inner" value={signal(String::new())} /> }
}

#[component]
fn OrderFixture() -> MountResult<MountPlan> {
    let store = Store::new();
    let rows = memo(move || store.rows(Filter::All));
    let expanded = signal(true);
    let shown = memo(move || expanded.get());

    view! {
        <view>
            <button aria:label="first" on:click={move || {
                let _ = Transaction::new()
                    .write(store.insert("A"))
                    .write(store.insert("B"))
                    .finish();
            }}>"Seed"</button>
            <text-input aria:label="draft" value={signal(String::new())} />
            <button aria:label="dead" disabled={true}>"Dead"</button>
            <Inner />
            <Show when={shown}>
                <button aria:label="shown">"Shown"</button>
            </Show>
            <For each={rows} key={|id: TodoId| id}>
                {move |todo: TodoId| view! {
                    <button aria:label={once(format!("row {}", todo.raw()))}>"row"</button>
                }}
            </For>
            <button aria:label="last">"Last"</button>
        </view>
    }
}

#[test]
fn tab_and_arrow_traversal_follow_the_mounted_order() {
    let mut app = Runtime::new();
    app.mount(|| view! { <OrderFixture /> }).expect("mount");
    app.click_text("Seed").expect("seed two rows");
    let ids: Vec<u64> = app.for_items().iter().map(|item| item.todo.raw()).collect();
    assert_eq!(ids.len(), 2);

    app.focus("first").expect("focus first");
    let mut seen = vec![app.focused().and_then(|focus| focus.label).unwrap()];
    for _ in 0..6 {
        app.focus_next().expect("next");
        seen.push(app.focused().and_then(|focus| focus.label).unwrap());
    }
    assert_eq!(
        seen,
        vec![
            String::from("first"),
            String::from("draft"),
            String::from("inner"),
            String::from("shown"),
            format!("row {}", ids[0]),
            format!("row {}", ids[1]),
            String::from("last"),
        ],
        "the disabled button is skipped and components/Show/For descend in mount order"
    );

    app.focus_next().expect("wrap forward");
    assert_eq!(
        app.focused().and_then(|focus| focus.label),
        Some(String::from("first"))
    );
    app.focus_previous().expect("wrap backward");
    assert_eq!(
        app.focused().and_then(|focus| focus.label),
        Some(String::from("last"))
    );
    app.focus_previous().expect("backward");
    assert_eq!(
        app.focused().and_then(|focus| focus.label),
        Some(format!("row {}", ids[1]))
    );

    app.blur().expect("no focus");
    app.focus_next().expect("next starts at the first target");
    assert_eq!(
        app.focused().and_then(|focus| focus.label),
        Some(String::from("first"))
    );
    app.blur().expect("no focus");
    app.focus_previous()
        .expect("previous starts at the last target");
    assert_eq!(
        app.focused().and_then(|focus| focus.label),
        Some(String::from("last"))
    );
}

#[test]
fn disabled_and_plain_targets_do_not_take_focus() {
    let mut app = Runtime::new();
    app.mount(|| view! { <OrderFixture /> }).expect("mount");

    assert_eq!(
        app.focus("dead")
            .expect_err("a disabled control is skipped"),
        MountError::Component("focus target is not focusable")
    );

    app.focus("first").expect("focus first");
    app.focus_next().expect("draft");
    app.focus_next().expect("inner");
    assert_eq!(
        app.focused().and_then(|focus| focus.label),
        Some(String::from("inner")),
        "the disabled control is never visited"
    );
}

#[component]
fn Rows() -> MountResult<MountPlan> {
    let store = Store::new();
    let rows = memo(move || store.rows(Filter::All));

    view! {
        <view>
            <button aria:label="seed" on:click={move || {
                let _ = Transaction::new()
                    .write(store.insert("A"))
                    .write(store.insert("B"))
                    .finish();
            }}>"Seed"</button>
            <button aria:label="reverse" on:click={move || {
                let mut ids: Vec<TodoId> =
                    rows.get().iter().map(TodoId::from_key).collect();
                ids.reverse();
                let _ = Transaction::new().write(store.reorder(&ids)).finish();
            }}>"Reverse"</button>
            <For each={rows} key={|id: TodoId| id}>
                {move |todo: TodoId| view! {
                    <text-input aria:label={once(format!("row {}", todo.raw()))} />
                }}
            </For>
        </view>
    }
}

#[test]
fn reorder_keeps_focus_on_the_same_control() {
    let mut app = Runtime::new();
    app.mount(|| view! { <Rows /> }).expect("mount");
    app.click_text("Seed").expect("seed");
    let before: Vec<u64> = app.for_items().iter().map(|item| item.todo.raw()).collect();

    app.focus(&format!("row {}", before[0])).expect("focus");
    app.click_text("Reverse").expect("reorder");

    let after: Vec<u64> = app.for_items().iter().map(|item| item.todo.raw()).collect();
    assert_eq!(after[1], before[0], "the row really moved");
    assert_eq!(
        app.focused().and_then(|focus| focus.label),
        Some(format!("row {}", before[0])),
        "the surviving control keeps focus across reorder"
    );
}

#[component]
fn DeletableRows(
    focuses: Rc<Cell<u32>>,
    blurs: Rc<Cell<u32>>,
    cause: Rc<Cell<Option<FocusCause>>>,
) -> MountResult<MountPlan> {
    let store = Store::new();
    let rows = memo(move || store.rows(Filter::All));
    let seed_focuses = focuses.clone();
    let seed_blurs = blurs.clone();

    view! {
        <view>
            <button aria:label="seed" on:focus={move |_event: FocusEvent| {
                seed_focuses.set(seed_focuses.get() + 1);
            }} on:blur={move |_event: BlurEvent| {
                seed_blurs.set(seed_blurs.get() + 1);
            }} on:click={move || {
                let _ = Transaction::new()
                    .write(store.insert("A"))
                    .write(store.insert("B"))
                    .finish();
            }}>"Seed"</button>
            <For each={rows} key={|id: TodoId| id}>
                {move |todo: TodoId| {
                    let focus_count = focuses.clone();
                    let blur_count = blurs.clone();
                    let cause_slot = cause.clone();
                    view! {
                        <view>
                            <text-input
                                aria:label={once(format!("row {}", todo.raw()))}
                                on:focus={move |event: FocusEvent| {
                                    cause_slot.set(Some(event.cause()));
                                    focus_count.set(focus_count.get() + 1);
                                }}
                                on:blur={move |_event: BlurEvent| {
                                    blur_count.set(blur_count.get() + 1);
                                }}
                            />
                            <button
                                aria:label={once(format!("delete {}", todo.raw()))}
                                on:click={move || {
                                    let _ = Transaction::new().write(store.remove(todo)).finish();
                                }}
                            >"Delete"</button>
                        </view>
                    }
                }}
            </For>
        </view>
    }
}

#[test]
fn deleting_the_focused_row_settles_once_to_a_surviving_control() {
    let focuses = Rc::new(Cell::new(0));
    let blurs = Rc::new(Cell::new(0));
    let cause = Rc::new(Cell::new(None));
    let mut app = Runtime::new();
    app.mount({
        let focuses = focuses.clone();
        let blurs = blurs.clone();
        let cause = cause.clone();
        move || view! { <DeletableRows focuses={focuses} blurs={blurs} cause={cause} /> }
    })
    .expect("mount");
    app.click_text("Seed").expect("seed");
    let ids: Vec<u64> = app.for_items().iter().map(|item| item.todo.raw()).collect();

    app.focus(&format!("row {}", ids[1])).expect("focus B");
    assert_eq!(focuses.get(), 1);
    assert_eq!(cause.get(), Some(FocusCause::Command));

    app.click_label(&format!("delete {}", ids[1]))
        .expect("delete the second row");

    assert_eq!(focuses.get(), 2, "settlement dispatches one focus");
    assert_eq!(
        cause.get(),
        Some(FocusCause::Settlement),
        "the second focus is an automatic settlement"
    );
    assert_eq!(
        blurs.get(),
        0,
        "an unmounted target cannot receive a blur event"
    );
    assert_eq!(
        app.focused().and_then(|focus| focus.label),
        Some(format!("row {}", ids[0])),
        "the previous same-tag control takes over"
    );
}

#[component]
fn Target(focuses: Signal<u32>, blurs: Signal<u32>) -> MountResult<MountPlan> {
    view! {
        <button
            aria:label="target"
            on:focus={move |_event: FocusEvent| {
                focuses.set(focuses.get() + 1);
            }}
            on:blur={move |_event: BlurEvent| {
                blurs.set(blurs.get() + 1);
            }}
        >"Target"</button>
    }
}

#[component]
fn RemountTarget(toggle_blurs: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let visible = signal(true);
    let when = memo(move || visible.get());
    let focuses = signal(0);
    let blurs = signal(0);
    let focus_text = memo(move || format!("focus={}", focuses.get()));
    let blur_text = memo(move || format!("blur={}", blurs.get()));

    view! {
        <view>
            <text>{focus_text}</text>
            <text>{blur_text}</text>
            <button aria:label="toggle" on:blur={move |_event: BlurEvent| {
                toggle_blurs.set(toggle_blurs.get() + 1);
            }} on:click={move || visible.set(!visible.get())}>"Toggle"</button>
            <Show when={when}>
                <Target focuses={focuses} blurs={blurs} />
            </Show>
        </view>
    }
}

#[test]
fn remounting_a_show_branch_never_targets_the_old_generation() {
    let toggle_blurs = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let toggle_blurs = toggle_blurs.clone();
        move || view! { <RemountTarget toggle_blurs={toggle_blurs} /> }
    })
    .expect("mount");
    assert!(app.texts().iter().any(|text| text == "focus=0"));
    assert!(app.texts().iter().any(|text| text == "blur=0"));

    app.focus("target").expect("focus the first generation");
    assert!(app.texts().iter().any(|text| text == "focus=1"));

    app.click_text("Toggle").expect("hide the branch");
    assert_eq!(
        app.focused().and_then(|focus| focus.label),
        Some(String::from("toggle")),
        "focus settles to the surviving neighbour"
    );
    assert!(
        app.texts().iter().any(|text| text == "blur=0"),
        "the unmounted target does not blur"
    );

    app.click_text("Toggle").expect("show a new generation");
    assert_eq!(
        app.focused().and_then(|focus| focus.label),
        Some(String::from("toggle")),
        "a remount does not steal focus"
    );
    assert!(
        app.texts().iter().any(|text| text == "focus=1"),
        "the new generation must not receive a focus event on mount"
    );
    assert!(
        app.texts().iter().any(|text| text == "blur=0"),
        "the old generation must not receive a blur on unmount"
    );

    app.focus("target").expect("focus the new generation");
    assert!(
        app.texts().iter().any(|text| text == "focus=2"),
        "only the new generation's handler fires"
    );
    assert_eq!(toggle_blurs.get(), 1, "the previous target blurs once");
}

#[component]
fn DisableTarget(focuses: Rc<Cell<u32>>, blurs: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    let disabled = signal(false);
    let is_disabled = memo(move || disabled.get());
    let victim_focus = focuses.clone();
    let victim_blur = blurs.clone();

    view! {
        <view>
            <button aria:label="toggle" on:click={move || disabled.set(true)}>"Disable"</button>
            <button
                aria:label="victim"
                disabled={is_disabled}
                on:focus={move |_event: FocusEvent| {
                    victim_focus.set(victim_focus.get() + 1);
                }}
                on:blur={move |_event: BlurEvent| {
                    victim_blur.set(victim_blur.get() + 1);
                }}
            >"Victim"</button>
        </view>
    }
}

#[test]
fn disabling_the_focused_control_blurs_it_and_settles() {
    let focuses = Rc::new(Cell::new(0));
    let blurs = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let focuses = focuses.clone();
        let blurs = blurs.clone();
        move || view! { <DisableTarget focuses={focuses} blurs={blurs} /> }
    })
    .expect("mount");
    app.focus("victim").expect("focus victim");
    assert_eq!(focuses.get(), 1);

    app.click_text("Disable").expect("disable the target");

    assert_eq!(blurs.get(), 1, "a mounted target blurs exactly once");
    assert_eq!(focuses.get(), 1, "the survivor was already focused");
    assert_eq!(
        app.focused().and_then(|focus| focus.label),
        Some(String::from("toggle"))
    );
}

#[component]
fn WindowButton(focuses: Rc<Cell<u32>>, blurs: Rc<Cell<u32>>) -> MountResult<MountPlan> {
    view! {
        <button
            aria:label="only"
            on:focus={move |_event: FocusEvent| {
                focuses.set(focuses.get() + 1);
            }}
            on:blur={move |_event: BlurEvent| {
                blurs.set(blurs.get() + 1);
            }}
        >"Only"</button>
    }
}

#[test]
fn reopening_a_window_starts_without_focus() {
    let focuses = Rc::new(Cell::new(0));
    let blurs = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    let first = app
        .open_window("main", {
            let focuses = focuses.clone();
            let blurs = blurs.clone();
            move |_| {
                let focuses = focuses.clone();
                let blurs = blurs.clone();
                view! { <WindowButton focuses={focuses} blurs={blurs} /> }
            }
        })
        .expect("open");
    app.focus("only").expect("focus");
    assert_eq!(focuses.get(), 1);
    app.close_window(first).expect("close");
    assert_eq!(app.focused(), None, "closing drops the focus target");
    assert_eq!(blurs.get(), 0, "closing does not dispatch a stale blur");

    let _second = app
        .open_window("main", {
            let focuses = focuses.clone();
            let blurs = blurs.clone();
            move |_| {
                let focuses = focuses.clone();
                let blurs = blurs.clone();
                view! { <WindowButton focuses={focuses} blurs={blurs} /> }
            }
        })
        .expect("reopen");
    assert_eq!(app.focused(), None, "a new activation starts unfocused");
    assert_eq!(
        focuses.get(),
        1,
        "reopening does not fire the old generation"
    );
}

#[test]
fn focus_rejects_unknown_and_non_focusable_targets() {
    #[component]
    fn Plain() -> MountResult<MountPlan> {
        view! {
            <view aria:label="plain">
                <text>"plain"</text>
            </view>
        }
    }

    let mut app = Runtime::new();
    app.mount(|| view! { <Plain /> }).expect("mount");

    assert_eq!(
        app.focus("plain")
            .expect_err("a plain view is not a focus target"),
        MountError::Component("focus target is not focusable")
    );
    assert_eq!(
        app.focus("missing").expect_err("an unknown label"),
        MountError::Component("focus target not found")
    );
    assert_eq!(
        app.focus_next().expect_err("there is no focusable control"),
        MountError::Component("no focusable target")
    );
    assert_eq!(app.focused(), None);
}
