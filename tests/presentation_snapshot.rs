#![allow(unused_braces)]

//! #342 验收：`PresentationSnapshot` 是唯一发布单位，按树 revision 共享根。
//!
//! 覆盖：同一 revision 的重复读取共享同一个快照根、revision 推进后重建、
//! 隐藏窗口在 flush 同步推进 revision 时延迟快照构建，show 后按需构建。

use zenui::prelude::*;

#[component]
fn SharedValue(shared: Signal<String>) -> MountResult<MountPlan> {
    view! {
        <view>
            <text aria:label="shared">{shared}</text>
            <button aria:label="update" on:click={move || shared.set(String::from("updated"))}>
                "Update"
            </button>
        </view>
    }
}

/// 自包含的本地 signal；headless 根不能再使用应用级 setup。
#[component]
fn LocalValue() -> MountResult<MountPlan> {
    let value = signal(String::from("initial"));
    view! {
        <view>
            <text aria:label="shared">{value}</text>
            <button aria:label="update" on:click={move || value.set(String::from("updated"))}>
                "Update"
            </button>
        </view>
    }
}

#[test]
fn reads_share_one_root_until_the_tree_revision_changes() {
    let mut app = Runtime::new();
    app.mount(|| view! { <LocalValue /> })
        .expect("mount the value");

    let revision = app.presentation_revision();
    let builds = app.presentation_builds();
    let first = app.presentation().expect("the first snapshot");
    assert_eq!(
        first.revision(),
        revision,
        "the snapshot carries its tree revision"
    );
    assert_eq!(
        app.presentation_builds(),
        builds + 1,
        "the first read builds one root"
    );

    let second = app.presentation().expect("the repeated snapshot");
    assert_eq!(
        app.presentation_builds(),
        builds + 1,
        "an unchanged tree reuses the shared root"
    );
    assert_eq!(
        second, first,
        "the repeated read presents the same snapshot"
    );

    app.click_label("update").expect("update the shared value");
    let revisions = app.presentation_revision();
    assert_ne!(
        revisions, revision,
        "the visible write advances the revision"
    );
    let builds = app.presentation_builds();
    let changed = app.presentation().expect("the changed snapshot");
    assert_eq!(
        app.presentation_builds(),
        builds + 1,
        "a revision change builds exactly one new root"
    );
    assert_eq!(changed.revision(), revisions);
    assert_ne!(changed, first, "a different revision is not equal");
    assert_eq!(
        changed.find_label("shared").expect("shared text").text(),
        "updated"
    );
}

#[test]
fn a_hidden_window_defers_its_snapshot_while_flush_stays_synchronous() {
    let mut app = Runtime::new();
    let shared = app
        .setup(|| signal(String::from("initial")))
        .expect("application setup");
    app.declare_windows(move |catalog| {
        catalog.window(WindowSpec::new("main").primary(), move |_| {
            view! { <SharedValue shared={shared} /> }
        })?;
        catalog.window(WindowSpec::new("other"), move |_| {
            view! { <SharedValue shared={shared} /> }
        })?;
        Ok(())
    })
    .expect("declare the windows");

    let main = app.show("main").expect("show main");
    let other = app.show("other").expect("show other");
    let builds = app.presentation_builds();
    let main_snapshot = app.presentation_of(main).expect("main snapshot");
    let other_snapshot = app.presentation_of(other).expect("other snapshot");
    assert_eq!(
        app.presentation_builds(),
        builds + 2,
        "each visible window builds its own root"
    );
    assert_eq!(
        main_snapshot.revision(),
        app.presentation_revision_of(main).expect("main revision")
    );
    assert_eq!(
        other_snapshot.revision(),
        app.presentation_revision_of(other).expect("other revision")
    );
    assert_ne!(
        main_snapshot.revision(),
        other_snapshot.revision(),
        "trees never share a revision"
    );

    app.hide(other).expect("hide other");
    let revision = app
        .presentation_revision_of(other)
        .expect("the hidden tree revision");
    let builds = app.presentation_builds();
    app.click_label("update")
        .expect("update the shared value from the visible window");
    assert!(
        app.presentation_revision_of(other).expect("revision") > revision,
        "the hidden tree flushes and advances its revision synchronously"
    );
    assert_eq!(
        app.presentation_builds(),
        builds,
        "a hidden window builds no snapshot during flush"
    );
    assert_eq!(
        app.presentation_of(other),
        Err(WindowError::Hidden),
        "hidden windows are not presented"
    );

    let builds = app.presentation_builds();
    let restored = app.show("other").expect("show other again");
    assert_eq!(
        app.presentation_builds(),
        builds,
        "show alone does not build the deferred snapshot"
    );
    let snapshot = app
        .presentation_of(restored)
        .expect("the deferred snapshot builds on read");
    assert_eq!(
        app.presentation_builds(),
        builds + 1,
        "the first read builds the deferred root"
    );
    assert_eq!(
        snapshot.revision(),
        app.presentation_revision_of(restored).expect("revision"),
        "the deferred root carries the flushed revision"
    );
    assert_eq!(
        snapshot.find_label("shared").expect("shared text").text(),
        "updated",
        "the deferred snapshot sees the shared write"
    );
}
