#![allow(unused_braces)]

//! #343 验收：renderer 的 `applied_revision` 与 delta 追赶协议。
//!
//! 连续 delta 与按最新根全量重建产生同一场景；断档时丢弃 delta，按最新根
//! 重建。场景比较使用元素身份、几何与字形 run 的确定性签名。

use zenui::prelude::*;
use zenui_demo::open_todo_window;
use zenui_render::{PresentationApply, PresentationView, Rect, Scene, Viewport};

fn viewport() -> Viewport {
    Viewport::new(900.0, 700.0, 1.0).expect("viewport")
}

fn build_scene(snapshot: &PresentationSnapshot) -> Scene {
    Scene::build(snapshot, viewport()).expect("scene")
}

type SceneSignature = Vec<(
    NodeId,
    &'static str,
    Rect,
    Vec<(f32, f32, u32, f32, f32, f32)>,
)>;

fn scene_signature(scene: &Scene) -> SceneSignature {
    scene
        .elements()
        .iter()
        .map(|element| {
            let glyphs = element
                .text()
                .iter()
                .flat_map(|run| {
                    let font_size = run.font_size();
                    let baseline = run.baseline();
                    run.glyphs().iter().map(move |glyph| {
                        (
                            font_size,
                            baseline,
                            glyph.id,
                            glyph.x,
                            glyph.y,
                            glyph.advance,
                        )
                    })
                })
                .collect();
            (element.id(), element.tag(), element.rect(), glyphs)
        })
        .collect()
}

fn add(app: &mut Runtime, title: &str) {
    app.input("What needs to be done?", title).expect("input");
    app.submit("Add task").expect("submit");
}

fn contains_key(nodes: &[SnapshotNode], key: PresentationKey) -> bool {
    nodes.iter().any(|node| {
        node.key() == key
            || node
                .as_element()
                .is_some_and(|element| contains_key(element.children(), key))
    })
}

#[test]
fn a_contiguous_delta_builds_the_same_scene_as_a_full_root() {
    let mut app = Runtime::new();
    let window = open_todo_window(&mut app).expect("open window");
    let first = app
        .presentation_update_of(window)
        .expect("the first publication");
    assert!(
        first.delta().is_none(),
        "the first publication is a full root"
    );
    let mut view = PresentationView::new(first.into_snapshot());
    assert_eq!(
        view.applied_revision(),
        app.presentation_revision_of(window).expect("revision")
    );
    let repeat = app
        .presentation_update_of(window)
        .expect("a repeat publication");
    assert!(repeat.delta().is_none(), "no new revision has no delta");
    assert_eq!(
        view.apply(repeat),
        PresentationApply::Applied,
        "the same published revision needs no rebuild"
    );

    add(&mut app, "delta row");
    let update = app
        .presentation_update_of(window)
        .expect("the changed publication");
    let delta = update.delta().expect("a contiguous delta");
    assert_eq!(delta.from_revision(), view.applied_revision());
    assert_eq!(delta.to_revision(), update.snapshot().revision());
    assert!(
        !delta.inserted().is_empty(),
        "the added row publishes inserted keys"
    );
    assert!(
        delta
            .changed()
            .iter()
            .all(|key| contains_key(update.snapshot().roots(), *key)),
        "changed keys address live committed nodes"
    );

    assert_eq!(view.apply(update), PresentationApply::Applied);
    assert_eq!(
        view.applied_revision(),
        view.snapshot().revision(),
        "the renderer applied the published revision"
    );

    let latest = app.presentation_of(window).expect("the latest root");
    assert_eq!(
        view.snapshot(),
        &latest,
        "the view holds the published root"
    );
    let incremental = build_scene(view.snapshot());
    let full = build_scene(&latest);
    assert_eq!(
        scene_signature(&incremental),
        scene_signature(&full),
        "a contiguous delta and a full rebuild agree"
    );
}

#[component]
fn RootToggle() -> MountResult<MountPlan> {
    let show = signal(false);
    view! {
        <Show when={show}><text aria:label="first">"first"</text></Show>
        <text aria:label="second">"second"</text>
        <button aria:label="toggle" on:click={move || show.set(!show.get())}>"toggle"</button>
    }
}

#[test]
fn a_full_root_publication_rebuilds_the_view() {
    let mut app = Runtime::new();
    app.mount(|| view! { <RootToggle /> }).expect("mount");
    let first = app.presentation_update().expect("the first publication");
    let mut view = PresentationView::new(first.into_snapshot());
    let before = view.applied_revision();

    app.click_label("toggle")
        .expect("change the top-level root list");
    let update = app
        .presentation_update()
        .expect("the full-root publication");
    assert!(
        update.delta().is_none(),
        "a top-level root list change publishes a full root"
    );
    assert_ne!(update.snapshot().revision(), before);
    assert_eq!(view.apply(update), PresentationApply::FullRebuild);
    assert_eq!(view.applied_revision(), app.presentation_revision());

    let latest = app.presentation().expect("the latest root");
    let rebuilt = build_scene(view.snapshot());
    let full = build_scene(&latest);
    assert_eq!(scene_signature(&rebuilt), scene_signature(&full));
}

#[test]
fn a_gap_discards_the_delta_and_rebuilds_from_the_latest_root() {
    let mut app = Runtime::new();
    let window = open_todo_window(&mut app).expect("open window");
    let first = app
        .presentation_update_of(window)
        .expect("the first publication");
    let mut view = PresentationView::new(first.into_snapshot());
    let applied = view.applied_revision();

    add(&mut app, "missed row");
    let missed = app
        .presentation_update_of(window)
        .expect("the publication the host skipped");
    let missed_delta = missed.delta().expect("the skipped delta");
    assert_eq!(missed_delta.from_revision(), applied);
    let missed_to = missed_delta.to_revision();
    drop(missed);

    add(&mut app, "next row");
    let update = app
        .presentation_update_of(window)
        .expect("the publication after the gap");
    let delta = update.delta().expect("the cumulative delta");
    assert_eq!(
        delta.from_revision(),
        missed_to,
        "the runtime baseline moved past the renderer"
    );
    assert_ne!(
        delta.from_revision(),
        view.applied_revision(),
        "the update starts at a revision the view never applied"
    );
    let latest = update.snapshot().clone();
    assert_eq!(view.apply(update), PresentationApply::FullRebuild);
    assert_eq!(view.snapshot(), &latest);
    assert_eq!(view.applied_revision(), latest.revision());

    let latest = app.presentation_of(window).expect("the latest root");
    let rebuilt = build_scene(view.snapshot());
    let full = build_scene(&latest);
    assert_eq!(
        scene_signature(&rebuilt),
        scene_signature(&full),
        "a gap rebuild matches a full rebuild from the latest root"
    );
}
