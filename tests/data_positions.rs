//! 数据位置的隐式 memo：闭包在挂载时降为 memo，并随读取的句柄更新。

#![allow(unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;

fn text_of(app: &Runtime, label: &str) -> String {
    app.presentation()
        .expect("snapshot")
        .find_label(label)
        .unwrap_or_else(|| panic!("label {label}"))
        .text()
}

#[test]
fn text_and_aria_closures_follow_their_sources() {
    #[component]
    fn Counter() -> MountResult<MountPlan> {
        let count = signal(0_u32);
        view! {
            <view>
                <text aria:label="count">{|| format!("{} left", count.get())}</text>
                <view aria:label={|| format!("badge-{}", count.get())} />
                <button aria:label="bump" on:click={move || count.set(count.get() + 1)}>
                    "Bump"
                </button>
            </view>
        }
    }

    let mut app = Runtime::new();
    app.mount(|| view! { <Counter /> }).expect("mount");
    assert_eq!(text_of(&app, "count"), "0 left");
    assert!(
        app.presentation()
            .expect("snapshot")
            .find_label("badge-0")
            .is_some(),
        "the aria:label closure runs at mount"
    );

    app.click_label("bump").expect("bump the count");
    assert_eq!(text_of(&app, "count"), "1 left");
    assert!(
        app.presentation()
            .expect("snapshot")
            .find_label("badge-1")
            .is_some(),
        "the implicit memo updates the attribute binding"
    );
}

#[test]
fn disabled_show_and_branch_closures_follow_their_sources() {
    #[component]
    fn Gated(clicks: Rc<Cell<u32>>) -> MountResult<MountPlan> {
        let busy = signal(false);
        let open = signal(false);
        let clicked = clicks.clone();
        view! {
            <view>
                <button
                    aria:label="act"
                    disabled={|| busy.get()}
                    on:click={move || clicked.set(clicked.get() + 1)}
                >
                    "Act"
                </button>
                <button aria:label="busy" on:click={move || busy.set(!busy.get())}>
                    "Busy"
                </button>
                <Show when={|| open.get()}>
                    <text aria:label="branch">{|| format!("open={}", open.get())}</text>
                </Show>
                <button aria:label="open" on:click={move || open.set(!open.get())}>
                    "Open"
                </button>
            </view>
        }
    }

    let clicks = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let clicks = clicks.clone();
        move || view! { <Gated clicks={clicks} /> }
    })
    .expect("mount");

    app.click_label("act").expect("an enabled click");
    assert_eq!(clicks.get(), 1);

    app.click_label("busy").expect("disable the button");
    assert!(
        app.presentation()
            .expect("snapshot")
            .find_label("act")
            .expect("act")
            .disabled(),
        "the disabled closure binds the new value"
    );
    let report = app.click_label("act").expect("a disabled click is a no-op");
    assert!(!report.changed());
    assert_eq!(clicks.get(), 1, "the disabled handler never runs");

    app.click_label("open").expect("open the branch");
    assert_eq!(text_of(&app, "branch"), "open=true");
    app.click_label("open").expect("close the branch");
    assert!(
        app.presentation()
            .expect("snapshot")
            .find_label("branch")
            .is_none(),
        "the branch unmounts with its implicit memo"
    );
}
