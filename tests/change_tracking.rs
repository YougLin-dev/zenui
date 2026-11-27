#![allow(unused_braces)]

use std::cell::Cell;
use std::rc::Rc;

use zenui::prelude::*;

#[test]
fn an_empty_click_does_not_bump_the_tree_revision() {
    #[component]
    fn Panel(clicks: Rc<Cell<u32>>) -> MountResult<MountPlan> {
        let count = clicks.clone();
        view! {
            <view>
                <button on:click={move || count.set(count.get() + 1)}>"Noop"</button>
                <text>{once(String::from("stable"))}</text>
            </view>
        }
    }

    let clicks = Rc::new(Cell::new(0));
    let mut app = Runtime::new();
    app.mount({
        let clicks = clicks.clone();
        move || view! { <Panel clicks={clicks} /> }
    })
    .expect("mount");
    let _ = app.refresh().expect("drain the mount report");
    let revision = app.presentation_revision();
    let builds = app.presentation_builds();
    let before = app.presentation().expect("snapshot");

    app.click_text("Noop").expect("empty click");
    assert_eq!(clicks.get(), 1, "the handler ran");
    assert_eq!(
        app.presentation_revision(),
        revision,
        "an empty click does not advance the tree revision"
    );
    let report = app.refresh().expect("report the empty turn");
    assert!(!report.changed, "no tree changed");
    assert!(report.changed_windows().is_empty());
    assert_eq!(
        app.presentation().expect("snapshot"),
        before,
        "the committed snapshot is unchanged"
    );
    assert_eq!(
        app.presentation_builds(),
        builds + 1,
        "the first read built the root; the second shared it"
    );
}

#[test]
fn an_equal_signal_write_does_not_bump_the_tree_revision() {
    #[component]
    fn EqualWrite() -> MountResult<MountPlan> {
        let value = signal(1_u32);
        let shown = memo(move || format!("v={}", value.get()));
        view! {
            <view>
                <text>{shown}</text>
                <button on:click={move || value.set(1)}>"Equal"</button>
                <button on:click={move || value.set(2)}>"Change"</button>
            </view>
        }
    }

    let mut app = Runtime::new();
    app.mount(|| view! { <EqualWrite /> }).expect("mount");
    let _ = app.refresh().expect("drain the mount report");
    let revision = app.presentation_revision();
    assert!(app.texts().iter().any(|text| text == "v=1"));

    app.click_text("Equal").expect("equal write");
    assert_eq!(
        app.presentation_revision(),
        revision,
        "an equal write does not advance the tree revision"
    );
    assert!(!app.refresh().expect("report the equal turn").changed);

    app.click_text("Change").expect("visible write");
    assert_ne!(
        app.presentation_revision(),
        revision,
        "a visible write advances the tree revision"
    );
    assert!(app.texts().iter().any(|text| text == "v=2"));
    let report = app.refresh().expect("report the visible turn");
    assert!(report.changed);
    assert!(report.root_changed);
}

#[test]
fn a_visible_change_bumps_only_the_owning_window_tree() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("a").primary(), |_| {
            let value = signal(0_u32);
            let shown = memo(move || format!("a={}", value.get()));
            view! {
                <view>
                    <text>{shown}</text>
                    <button aria:label="BumpA" on:click={move || value.set(value.get() + 1)}>"BumpA"</button>
                </view>
            }
        })?;
        catalog.window(WindowSpec::new("b"), |_| {
            let value = signal(0_u32);
            let shown = memo(move || format!("b={}", value.get()));
            view! {
                <view>
                    <text>{shown}</text>
                    <button aria:label="BumpB" on:click={move || value.set(value.get() + 1)}>"BumpB"</button>
                </view>
            }
        })?;
        Ok(())
    })
    .expect("declare windows");
    let a = app.show("a").expect("show a");
    let b = app.show("b").expect("show b");
    let _ = app.refresh().expect("drain the open report");
    let revision_a = app.presentation_revision_of(a).expect("a revision");
    let revision_b = app.presentation_revision_of(b).expect("b revision");
    let active = app.presentation_revision();
    assert_eq!(active, revision_b, "b is the focused window");

    let bump_a = app
        .presentation_of(a)
        .expect("a snapshot")
        .find_label("BumpA")
        .expect("the a button")
        .id();
    app.click_node(bump_a).expect("bump the a value");
    assert_ne!(
        app.presentation_revision_of(a).expect("a revision"),
        revision_a,
        "the owning tree advances"
    );
    assert_eq!(
        app.presentation_revision_of(b).expect("b revision"),
        revision_b,
        "the other window tree is untouched"
    );
    assert_eq!(
        app.presentation_revision(),
        revision_b,
        "the active window revision is not polluted by another tree"
    );
    let report = app.refresh().expect("report the a change");
    assert!(report.changed);
    assert_eq!(report.changed_windows(), ["a"]);
    let revision_a_after = app.presentation_revision_of(a).expect("a revision");

    let bump_b = app
        .presentation_of(b)
        .expect("b snapshot")
        .find_label("BumpB")
        .expect("the b button")
        .id();
    app.click_node(bump_b).expect("bump the b value");
    assert_ne!(
        app.presentation_revision_of(b).expect("b revision"),
        revision_b,
        "b advances after its own change"
    );
    assert_eq!(
        app.presentation_revision_of(a).expect("a revision"),
        revision_a_after,
        "b's change leaves a's revision untouched"
    );
    let report = app.refresh().expect("report the b change");
    assert_eq!(report.changed_windows(), ["b"]);
}

#[test]
fn appearance_bumps_every_window_tree() {
    let mut app = Runtime::new();
    app.declare_windows(|catalog| {
        catalog.window(WindowSpec::new("a").primary(), |_| {
            view! { <text>"a"</text> }
        })?;
        catalog.window(WindowSpec::new("b"), |_| {
            view! { <text>"b"</text> }
        })?;
        Ok(())
    })
    .expect("declare windows");
    let a = app.show("a").expect("show a");
    let b = app.show("b").expect("show b");
    let _ = app.refresh().expect("drain the open report");
    let revision_a = app.presentation_revision_of(a).expect("a revision");
    let revision_b = app.presentation_revision_of(b).expect("b revision");

    app.set_appearance(Appearance::Dark)
        .expect("set the appearance");
    assert_ne!(
        app.presentation_revision_of(a).expect("a revision"),
        revision_a
    );
    assert_ne!(
        app.presentation_revision_of(b).expect("b revision"),
        revision_b
    );
    let report = app.refresh().expect("report the appearance change");
    assert_eq!(report.changed_windows(), ["a", "b"]);
}
