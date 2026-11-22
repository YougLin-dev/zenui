#![allow(dead_code, unused_braces)]

use zenui::prelude::*;
use zenui_demo::*;
use zenui_render::{
    DecorationKind, Point, Scene, SceneCache, SceneElement, SceneEnv, SceneError, Viewport,
};

fn runtime() -> Runtime {
    let mut app = Runtime::new();
    open_todo_window(&mut app).expect("open window");
    app
}

fn add(app: &mut Runtime, title: &str) {
    app.input("What needs to be done?", title).expect("input");
    app.submit("Add task").expect("submit");
}

fn viewport(width: f32) -> Viewport {
    Viewport::new(width, 600.0, 1.0).expect("viewport")
}

fn build_scene(snapshot: &PresentationSnapshot, width: f32) -> Scene {
    Scene::build(snapshot, viewport(width)).expect("scene")
}

fn by_text(snapshot: &PresentationSnapshot, tag: &str, text: &str) -> NodeId {
    snapshot
        .elements()
        .find(|element| element.tag() == tag && element.text() == text)
        .map(SnapshotElement::id)
        .unwrap_or_else(|| panic!("no {tag} with text {text:?}"))
}

fn background_by_label(
    snapshot: &PresentationSnapshot,
    scene: &Scene,
    label: &str,
) -> Option<Color> {
    scene
        .node(snapshot.find_label(label).expect("labelled element").id())
        .expect("scene element")
        .resolved_style()
        .background()
}

fn background_by_text(snapshot: &PresentationSnapshot, scene: &Scene, text: &str) -> Option<Color> {
    scene
        .node(by_text(snapshot, "button", text))
        .expect("button")
        .resolved_style()
        .background()
}

#[test]
fn long_toolbar_labels_stay_inside_their_buttons() {
    let app = runtime();
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 800.0);

    for label in ["Load sample", "Import file"] {
        let element = scene
            .node(by_text(&snapshot, "button", label))
            .expect("toolbar button");
        let glyphs: Vec<_> = element.text().iter().flat_map(|run| run.glyphs()).collect();
        let baseline = glyphs.first().expect("a label glyph").y;
        assert!(
            glyphs.iter().all(|glyph| (glyph.y - baseline).abs() < 0.01),
            "{label:?} wraps onto a second line"
        );
        let rect = element.rect();
        assert!(
            baseline < rect.y + rect.height,
            "{label:?} sits inside its button"
        );
    }
}

#[test]
fn a_long_value_stays_inside_its_input() {
    let mut app = runtime();
    let value = "word ".repeat(40);
    app.input("What needs to be done?", &value).expect("input");
    app.focus("What needs to be done?").expect("focus");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 220.0);
    let node = snapshot
        .find_label("What needs to be done?")
        .expect("draft input");
    let input = scene.node(node.id()).expect("scene input");
    let rect = input.rect();
    let glyphs: Vec<_> = input.text().iter().flat_map(|run| run.glyphs()).collect();
    assert!(!glyphs.is_empty(), "the value is laid out");
    assert!(
        glyphs.iter().all(|glyph| glyph.y <= rect.y + rect.height),
        "every line stays inside the input"
    );
    let caret = scene.caret(node.id()).expect("caret");
    assert!(
        caret.y + caret.height <= rect.y + rect.height + 0.5,
        "the caret stays inside the input: {caret:?} vs {rect:?}"
    );
}

#[test]
fn the_todo_snapshot_lays_out_non_zero_ordered_boxes() {
    let mut app = runtime();
    add(&mut app, "Write tests");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 800.0);

    let expected: Vec<NodeId> = snapshot.elements().map(SnapshotElement::id).collect();
    let actual: Vec<NodeId> = scene.elements().iter().map(SceneElement::id).collect();
    assert_eq!(actual, expected, "the scene keeps committed pre-order");

    for element in scene.elements() {
        let rect = element.rect();
        assert!(
            rect.width > 0.0 && rect.height > 0.0,
            "{} {:?} has zero geometry: {rect:?}",
            element.tag(),
            element.id()
        );
    }

    let root = scene.elements().first().expect("root view");
    assert_eq!(root.tag(), "view");
    assert_eq!(root.rect().width, 800.0, "the root fills the viewport");

    let all = scene
        .node(by_text(&snapshot, "button", "All"))
        .expect("All button");
    let active = scene
        .node(by_text(&snapshot, "button", "Active"))
        .expect("Active button");
    let done = scene
        .node(by_text(&snapshot, "button", "Done"))
        .expect("Done button");
    assert!(
        all.rect().x < active.rect().x && active.rect().x < done.rect().x,
        "filter buttons are ordered left to right"
    );
    assert!((all.rect().y - active.rect().y).abs() < 0.01);

    let checkbox = scene
        .node(
            snapshot
                .find_label("Complete Write tests")
                .expect("checkbox")
                .id(),
        )
        .expect("scene checkbox");
    let editor = scene
        .node(
            snapshot
                .find_label("Edit Write tests")
                .expect("editor")
                .id(),
        )
        .expect("scene editor");
    let delete = scene
        .node(
            snapshot
                .find_label("Delete Write tests")
                .expect("delete")
                .id(),
        )
        .expect("scene delete");
    assert!(
        checkbox.rect().x < editor.rect().x && editor.rect().x < delete.rect().x,
        "row controls are ordered left to right"
    );
    assert!(checkbox.rect().width <= 16.0, "the checkbox keeps its box");
    assert!(editor.rect().width > editor.rect().height);
}

#[test]
fn text_shaping_produces_latin_and_cjk_glyph_runs() {
    let mut app = runtime();
    add(&mut app, "Write tests");
    add(&mut app, "中文任务");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 800.0);

    for text in ["Write tests", "中文任务"] {
        let id = by_text(&snapshot, "text", text);
        let runs = scene.node(id).expect("scene text").text();
        assert!(!runs.is_empty(), "{text:?} produces a run");
        let glyphs: Vec<_> = runs.iter().flat_map(|run| run.glyphs()).collect();
        assert!(!glyphs.is_empty(), "{text:?} produces glyphs");
        assert!(
            glyphs
                .iter()
                .all(|glyph| glyph.x.is_finite() && glyph.y.is_finite()),
            "{text:?} has finite glyph positions"
        );
        assert!(
            runs.iter().all(|run| run.font_size() > 0.0),
            "{text:?} has a resolved font size"
        );
    }
}

#[test]
fn hit_testing_returns_the_topmost_control() {
    let mut app = runtime();
    add(&mut app, "Write tests");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 800.0);

    let targets = [
        by_text(&snapshot, "button", "All"),
        snapshot
            .find_label("Complete Write tests")
            .expect("checkbox")
            .id(),
        snapshot
            .find_label("Edit Write tests")
            .expect("editor")
            .id(),
    ];
    for id in targets {
        let element = scene.node(id).expect("scene element");
        let rect = element.rect();
        let center = Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
        assert_eq!(
            scene.hit_test(center),
            Some(id),
            "topmost hit for {}",
            element.tag()
        );
    }
    assert_eq!(scene.hit_test(Point::new(-1.0, -1.0)), None);
    assert_eq!(scene.hit_test(Point::new(10_000.0, 10_000.0)), None);
}

#[test]
fn caret_and_selection_geometry_follow_local_edit_state() {
    let mut app = runtime();
    add(&mut app, "Alpha");
    app.input("Edit Alpha", "Alpha").expect("seed the editor");

    let idle = app.presentation().expect("snapshot");
    let idle_id = idle.find_label("Edit Alpha").expect("editor").id();
    assert!(
        build_scene(&idle, 800.0).caret(idle_id).is_none(),
        "an unfocused input draws no caret"
    );

    app.focus("Edit Alpha").expect("focus");
    assert!(app.set_caret("Edit Alpha", 1), "place the caret");

    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 800.0);
    let id = snapshot.find_label("Edit Alpha").expect("editor").id();
    let input = scene.node(id).expect("scene editor").rect();
    let first = scene.caret(id).expect("caret geometry");
    assert!(
        first.x >= input.x && first.right() <= input.right() + 0.01,
        "the caret stays inside the input"
    );
    assert!(first.height > 0.0);
    assert!(
        scene
            .decorations()
            .iter()
            .all(|decoration| decoration.node() != id
                || decoration.kind() != DecorationKind::Selection),
        "a collapsed selection paints no highlight"
    );

    assert!(app.set_caret("Edit Alpha", 4), "move the caret");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 800.0);
    let later = scene.caret(id).expect("caret geometry");
    assert!(later.x > first.x, "the caret follows the character offset");

    app.compose_start("Edit Alpha").expect("start composing");
    app.compose_update("Edit Alpha", "ni")
        .expect("update preedit");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 800.0);
    let composing = scene.caret(id).expect("caret during composition");
    assert!(composing.x > later.x, "the caret follows the preedit end");
    let underlines: Vec<_> = scene
        .decorations()
        .iter()
        .filter(|decoration| {
            decoration.node() == id && decoration.kind() == DecorationKind::PreeditUnderline
        })
        .collect();
    assert!(!underlines.is_empty(), "preedit is decorated");
    assert!(
        underlines
            .iter()
            .all(|decoration| decoration.rect().width > 0.0 && decoration.rect().height > 0.0)
    );
    assert!(
        scene
            .node(id)
            .expect("scene editor")
            .text()
            .iter()
            .any(|run| !run.glyphs().is_empty()),
        "the preedit text is shaped"
    );

    app.compose_cancel("Edit Alpha").expect("cancel composing");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 800.0);
    assert!(
        scene
            .decorations()
            .iter()
            .all(|decoration| decoration.kind() != DecorationKind::PreeditUnderline),
        "cancel removes the preedit decoration"
    );
}

fn seeded_editor() -> (Runtime, NodeId) {
    let mut app = runtime();
    add(&mut app, "Alpha");
    app.input("Edit Alpha", "Alpha").expect("seed the editor");
    app.focus("Edit Alpha").expect("focus");
    let id = app
        .presentation()
        .expect("snapshot")
        .find_label("Edit Alpha")
        .expect("editor")
        .id();
    (app, id)
}

#[test]
fn caret_clicks_snap_to_the_nearest_character_boundary() {
    let (mut app, id) = seeded_editor();
    for offset in [0, 3, 5] {
        assert!(app.set_caret("Edit Alpha", offset), "place caret {offset}");
        let snapshot = app.presentation().expect("snapshot");
        let scene = build_scene(&snapshot, 800.0);
        let caret = scene.caret(id).expect("caret geometry");
        let point = Point::new(caret.x, caret.y + caret.height / 2.0);
        assert_eq!(
            scene.node(id).expect("editor").caret_index(point),
            Some(offset),
            "clicking the caret {offset} boundary selects offset {offset}"
        );
    }
}

#[test]
fn caret_clicks_past_the_text_clamp_and_non_text_elements_have_no_index() {
    let (app, id) = seeded_editor();
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 800.0);
    let editor = scene.node(id).expect("editor");
    let rect = editor.rect();
    assert_eq!(
        editor.caret_index(Point::new(rect.right() + 40.0, rect.y + rect.height / 2.0)),
        Some(5),
        "clicking past the end clamps to the last boundary"
    );

    let all = scene
        .node(by_text(&snapshot, "button", "All"))
        .expect("All button");
    assert_eq!(
        all.caret_index(Point::new(all.rect().x, all.rect().y)),
        None,
        "non-text elements have no caret index"
    );
}

#[test]
fn caret_clicks_map_a_longer_preedit_back_to_the_value() {
    let (mut app, id) = seeded_editor();
    app.set_selection_node(id, 1, 3)
        .expect("select for composition");
    app.compose_start("Edit Alpha").expect("start composing");
    app.compose_update("Edit Alpha", "XYZ")
        .expect("update preedit");

    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 800.0);
    let editor = scene.node(id).expect("editor");
    let caret = scene.caret(id).expect("composition caret");
    let caret_point = Point::new(caret.x, caret.y + caret.height / 2.0);
    let glyphs: Vec<_> = editor.text().iter().flat_map(|run| run.glyphs()).collect();

    assert_eq!(
        editor.caret_index(caret_point),
        Some(3),
        "the preedit end maps to the end of the replaced range"
    );

    let preedit = glyphs.get(1).expect("the first preedit glyph");
    let inside = Point::new(
        preedit.x + preedit.advance / 2.0,
        caret.y + caret.height / 2.0,
    );
    assert_eq!(
        editor.caret_index(inside),
        Some(1),
        "a click inside the preedit clamps to the composition base"
    );

    let after = glyphs.get(4).expect("the glyph after the preedit");
    let shifted = Point::new(after.x + after.advance * 0.9, caret.y + caret.height / 2.0);
    assert_eq!(
        editor.caret_index(shifted),
        Some(4),
        "clicks after the preedit shift by the replaced range"
    );
}

#[test]
fn an_empty_preedit_click_clamps_to_the_composition_base() {
    let (mut app, id) = seeded_editor();
    app.set_selection_node(id, 1, 3)
        .expect("select for composition");
    app.compose_start("Edit Alpha").expect("start composing");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 800.0);
    let editor = scene.node(id).expect("editor");
    let caret = scene.caret(id).expect("composition caret");
    let glyphs: Vec<_> = editor.text().iter().flat_map(|run| run.glyphs()).collect();
    let first_remaining = glyphs.get(1).expect("the first glyph after the base");
    let point = Point::new(first_remaining.x + 1.0, caret.y + caret.height / 2.0);
    assert_eq!(
        editor.caret_index(point),
        Some(1),
        "an empty preedit clamps to its insertion point"
    );
}

#[test]
fn a_viewport_resize_relayouts_while_node_ids_stay_stable() {
    let mut app = runtime();
    add(
        &mut app,
        "A rather long task title that needs more room when the window is narrow",
    );
    let snapshot = app.presentation().expect("snapshot");
    let narrow = build_scene(&snapshot, 420.0);
    let wide = build_scene(&snapshot, 900.0);

    let narrow_ids: Vec<NodeId> = narrow.elements().iter().map(SceneElement::id).collect();
    let wide_ids: Vec<NodeId> = wide.elements().iter().map(SceneElement::id).collect();
    assert_eq!(narrow_ids, wide_ids, "a resize keeps every Node ID");

    assert_eq!(narrow.elements()[0].rect().width, 420.0);
    assert_eq!(wide.elements()[0].rect().width, 900.0);

    let title = by_text(
        &snapshot,
        "text",
        "A rather long task title that needs more room when the window is narrow",
    );
    let narrow_title = narrow.node(title).expect("narrow title").rect();
    let wide_title = wide.node(title).expect("wide title").rect();
    assert!(
        narrow_title.width < wide_title.width,
        "narrow={narrow_title:?} wide={wide_title:?}"
    );

    let scaled = Scene::build(
        &snapshot,
        Viewport::new(900.0, 600.0, 2.0).expect("viewport"),
    )
    .expect("scene");
    assert_eq!(scaled.viewport().scale(), 2.0);
    assert_eq!(
        scaled.elements()[0].rect().width,
        900.0,
        "geometry stays in logical pixels"
    );

    let short = Scene::build(
        &snapshot,
        Viewport::new(900.0, 300.0, 1.0).expect("viewport"),
    )
    .expect("scene");
    let tall = Scene::build(
        &snapshot,
        Viewport::new(900.0, 900.0, 1.0).expect("viewport"),
    )
    .expect("scene");
    assert_eq!(short.elements()[0].rect().height, 300.0);
    assert_eq!(tall.elements()[0].rect().height, 900.0);
    assert_eq!(short.elements()[0].id(), tall.elements()[0].id());
}

#[test]
fn unsupported_tags_and_attributes_are_explicit_errors() {
    let mut app = Runtime::new();
    app.open_window("main", |_| {
        Ok(MountPlan::new(vec![zenui::view::element(
            "canvas",
            Vec::new(),
            Vec::new(),
        )]))
    })
    .expect("window");
    let snapshot = app.presentation().expect("snapshot");
    let error = Scene::build(&snapshot, viewport(800.0)).expect_err("unsupported tag");
    match error {
        SceneError::UnsupportedTag { tag, .. } => assert_eq!(tag, "canvas"),
        other => panic!("expected UnsupportedTag, got {other:?}"),
    }

    let mut app = Runtime::new();
    app.open_window("main", |_| {
        Ok(MountPlan::new(vec![zenui::view::element(
            "view",
            vec![zenui::view::attr("style:width", "12px")],
            Vec::new(),
        )]))
    })
    .expect("window");
    let snapshot = app.presentation().expect("snapshot");
    let error = Scene::build(&snapshot, viewport(800.0)).expect_err("unsupported attribute");
    match error {
        SceneError::UnsupportedAttribute { name, .. } => assert_eq!(name, "style:width"),
        other => panic!("expected UnsupportedAttribute, got {other:?}"),
    }

    let mut app = Runtime::new();
    app.open_window("main", |_| {
        Ok(MountPlan::new(vec![zenui::view::element(
            "text-input",
            vec![zenui::view::attr("placeholder", "type here")],
            Vec::new(),
        )]))
    })
    .expect("window");
    let snapshot = app.presentation().expect("snapshot");
    match Scene::build(&snapshot, viewport(800.0)) {
        Err(SceneError::UnsupportedAttribute { name, .. }) => assert_eq!(name, "placeholder"),
        other => panic!("expected UnsupportedAttribute for placeholder, got {other:?}"),
    }

    let mut app = Runtime::new();
    app.open_window("main", |_| {
        Ok(MountPlan::new(vec![zenui::view::element(
            "view",
            vec![zenui::view::attr("class", "px-2")],
            Vec::new(),
        )]))
    })
    .expect("window");
    let snapshot = app.presentation().expect("snapshot");
    match Scene::build(&snapshot, viewport(800.0)) {
        Err(SceneError::UnsupportedAttribute { name, .. }) => assert_eq!(name, "class"),
        other => panic!("expected UnsupportedAttribute for class, got {other:?}"),
    }

    assert!(matches!(
        Viewport::new(f32::NAN, 600.0, 1.0),
        Err(SceneError::InvalidViewport { .. })
    ));
    assert!(matches!(
        Viewport::new(800.0, 600.0, 0.0),
        Err(SceneError::InvalidViewport { .. })
    ));
}

#[test]
fn leaf_intrinsics_and_bare_root_text_are_explicit_errors() {
    let mut app = Runtime::new();
    app.open_window("main", |_| {
        Ok(MountPlan::new(vec![zenui::view::PlanNode::Text(
            zenui::view::TextSlot::Static("bare"),
        )]))
    })
    .expect("window");
    let snapshot = app.presentation().expect("snapshot");
    assert!(
        matches!(
            Scene::build(&snapshot, viewport(800.0)),
            Err(SceneError::BareRootText)
        ),
        "top-level text has no Node ID"
    );

    for tag in ["text-input", "checkbox"] {
        let tag: &'static str = tag;
        let mut app = Runtime::new();
        app.open_window("main", move |_| {
            Ok(MountPlan::new(vec![zenui::view::element(
                tag,
                Vec::new(),
                vec![zenui::view::element("text", Vec::new(), Vec::new())],
            )]))
        })
        .expect("window");
        let snapshot = app.presentation().expect("snapshot");
        match Scene::build(&snapshot, viewport(800.0)) {
            Err(SceneError::UnsupportedContent { reason, .. }) => {
                assert!(reason.contains("leaf"), "reason: {reason}");
            }
            other => panic!("expected UnsupportedContent for {tag}, got {other:?}"),
        }
    }
}

#[test]
fn a_range_selection_produces_highlight_rects() {
    let mut app = runtime();
    add(&mut app, "Alpha");
    app.input("Edit Alpha", "Alpha").expect("seed the editor");
    let snapshot = app.presentation().expect("snapshot");
    let id = snapshot.find_label("Edit Alpha").expect("editor").id();
    app.set_selection_node(id, 1, 3).expect("select a range");

    let snapshot = app.presentation().expect("snapshot");
    let edit = snapshot
        .find_label("Edit Alpha")
        .expect("editor")
        .edit()
        .expect("edit state");
    assert_eq!(edit.selection(), Some((1, 3)));
    assert_eq!(edit.composition(), None);

    let scene = build_scene(&snapshot, 800.0);
    let rects: Vec<_> = scene
        .decorations()
        .iter()
        .filter(|decoration| {
            decoration.node() == id && decoration.kind() == DecorationKind::Selection
        })
        .map(|decoration| decoration.rect())
        .collect();
    assert!(!rects.is_empty(), "a range selection is highlighted");
    let input = scene.node(id).expect("scene editor").rect();
    for rect in rects {
        assert!(rect.width > 0.0 && rect.height > 0.0);
        assert!(
            rect.x >= input.x && rect.right() <= input.right() + 0.01,
            "selection stays inside the input"
        );
    }
}

#[test]
fn composition_replaces_the_selected_range_in_the_scene() {
    let mut app = runtime();
    add(&mut app, "Alpha");
    app.input("Edit Alpha", "Alpha").expect("seed the editor");
    let snapshot = app.presentation().expect("snapshot");
    let id = snapshot.find_label("Edit Alpha").expect("editor").id();
    app.set_selection_node(id, 1, 3).expect("select a range");
    app.compose_start("Edit Alpha").expect("start composing");
    app.compose_update("Edit Alpha", "ni")
        .expect("update preedit");

    let snapshot = app.presentation().expect("snapshot");
    let edit = snapshot
        .find_label("Edit Alpha")
        .expect("editor")
        .edit()
        .expect("edit state");
    assert_eq!(edit.composition(), Some((1, 3)));
    assert_eq!(edit.caret(), 3, "the caret follows the preedit end");
    let scene = build_scene(&snapshot, 800.0);
    let composed: usize = scene
        .node(id)
        .expect("scene editor")
        .text()
        .iter()
        .map(|run| run.glyphs().len())
        .sum();

    // Shape the expected display text through the same pipeline.
    app.compose_cancel("Edit Alpha").expect("cancel composing");
    app.input("Edit Alpha", "Aniha")
        .expect("write the display text");
    let expected_snapshot = app.presentation().expect("snapshot");
    let expected_id = expected_snapshot
        .find_label("Edit Alpha")
        .expect("editor")
        .id();
    let expected: usize = build_scene(&expected_snapshot, 800.0)
        .node(expected_id)
        .expect("scene editor")
        .text()
        .iter()
        .map(|run| run.glyphs().len())
        .sum();
    assert!(composed > 0);
    assert_eq!(
        composed, expected,
        "preedit replaces the selected range instead of only inserting"
    );
}

#[test]
fn class_padding_and_gap_change_the_row_geometry() {
    let mut app = Runtime::new();
    app.mount(|| {
        view! {
            <view class="flex-row items-center gap-3">
                <button class="px-4 py-2">"Padded"</button>
                <button>"Plain"</button>
            </view>
        }
    })
    .expect("mount");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 400.0);
    let padded = scene
        .node(by_text(&snapshot, "button", "Padded"))
        .expect("padded button")
        .rect();
    let plain = scene
        .node(by_text(&snapshot, "button", "Plain"))
        .expect("plain button")
        .rect();

    assert!(
        (padded.y + padded.height / 2.0 - (plain.y + plain.height / 2.0)).abs() < 0.01,
        "items-center aligns both buttons on one line: padded={padded:?} plain={plain:?}"
    );
    let gap = plain.x - (padded.x + padded.width);
    assert!((gap - 12.0).abs() < 0.5, "gap-3 leaves 12 points: {gap}");
    assert!(
        padded.width > plain.width + 4.0,
        "px-4 widens the padded button: padded={padded:?} plain={plain:?}"
    );
    assert!(
        padded.height > plain.height + 4.0,
        "py-2 makes the padded button taller: padded={padded:?} plain={plain:?}"
    );
}

#[test]
fn flex_grow_and_justify_between_use_the_remaining_space() {
    let mut app = Runtime::new();
    app.mount(|| {
        view! {
            <view class="flex-row items-center justify-between">
                <button class="flex-1">"Grow"</button>
                <button>"Fixed"</button>
            </view>
        }
    })
    .expect("mount");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 400.0);
    let grow = scene
        .node(by_text(&snapshot, "button", "Grow"))
        .expect("grow button")
        .rect();
    let fixed = scene
        .node(by_text(&snapshot, "button", "Fixed"))
        .expect("fixed button")
        .rect();

    assert!(
        grow.width > fixed.width,
        "flex-1 takes the free space: grow={grow:?} fixed={fixed:?}"
    );
    assert!(
        (fixed.x + fixed.width - 400.0).abs() < 0.5,
        "justify-between pins the last button to the row end: fixed={fixed:?}"
    );
}

#[test]
fn paint_classes_resolve_colors_radius_and_opacity() {
    let mut app = Runtime::new();
    app.mount(|| {
        view! {
            <view class="bg-surface rounded-lg">
                <button class="bg-accent text-inverse border border-accent rounded-md opacity-50">"Accent"</button>
                <text class="text-muted">"Muted"</text>
            </view>
        }
    })
    .expect("mount");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 400.0);

    let root = scene
        .node(snapshot.roots()[0].id().expect("root element"))
        .expect("scene root");
    let style = root.resolved_style();
    assert_eq!(
        style.background(),
        Some(Color::Role(ColorRole::Surface)),
        "bg-surface resolves to its semantic role"
    );
    assert_eq!(style.radius(), 10.0, "rounded-lg is a 10-point radius");
    assert_eq!(style.opacity(), 1.0, "no opacity class means fully opaque");

    let button = scene
        .node(by_text(&snapshot, "button", "Accent"))
        .expect("accent button")
        .resolved_style();
    assert_eq!(
        button.background(),
        Some(Color::Role(ColorRole::Accent)),
        "bg-accent stays a role until the palette resolves it"
    );
    assert_eq!(
        button.text_color(),
        Some(Color::Role(ColorRole::TextInverse))
    );
    assert_eq!(
        button.border_color(),
        Some(Color::Role(ColorRole::BorderAccent))
    );
    assert_eq!(button.border_width(), Some(1.0), "border is one point");
    assert_eq!(button.radius(), 6.0, "rounded-md is a 6-point radius");
    assert_eq!(button.opacity(), 0.5, "opacity-50 halves the element paint");

    let text = scene
        .node(by_text(&snapshot, "text", "Muted"))
        .expect("muted text")
        .resolved_style();
    assert_eq!(text.text_color(), Some(Color::Role(ColorRole::TextMuted)));
}

#[test]
fn radius_affects_paint_only_not_geometry_or_hit_testing() {
    fn mounted(rounded: bool) -> (NodeId, Scene) {
        let mut app = Runtime::new();
        if rounded {
            app.mount(|| {
                view! { <text-input aria:label="padded" class="px-4 rounded-lg" value="abc" /> }
            })
        } else {
            app.mount(|| view! { <text-input aria:label="padded" class="px-4" value="abc" /> })
        }
        .expect("mount");
        app.focus("padded").expect("focus");
        let snapshot = app.presentation().expect("snapshot");
        let id = snapshot.find_label("padded").expect("input").id();
        (id, build_scene(&snapshot, 400.0))
    }

    let (plain_id, plain) = mounted(false);
    let (rounded_id, rounded) = mounted(true);
    let plain_input = plain.node(plain_id).expect("plain input");
    let rounded_input = rounded.node(rounded_id).expect("rounded input");

    assert_eq!(
        rounded_input.rect(),
        plain_input.rect(),
        "radius changes no layout"
    );
    assert_eq!(
        rounded.caret(rounded_id).expect("rounded caret"),
        plain.caret(plain_id).expect("plain caret"),
        "radius changes no caret geometry"
    );
    assert_eq!(
        rounded_input.resolved_style().radius(),
        10.0,
        "the rounded twin really carries a radius"
    );
    let corner = Point::new(rounded_input.rect().x + 0.5, rounded_input.rect().y + 0.5);
    assert_eq!(
        rounded.hit_test(corner),
        Some(rounded_id),
        "a click in the rounded-away corner still hits the input"
    );
}

#[test]
fn literal_colors_bypass_the_palette() {
    const STYLE: &[StyleDeclaration] = &[StyleDeclaration::new(
        Property::Background,
        StyleValue::Color(Color::Literal([12, 34, 56, 255])),
        Condition::Base,
    )];
    let mut app = Runtime::new();
    app.open_window("main", |_| {
        Ok(MountPlan::new(vec![zenui::view::styled_element(
            "view",
            Vec::new(),
            STYLE,
            Vec::new(),
            Vec::new(),
        )]))
    })
    .expect("window");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 400.0);
    let root = scene
        .node(snapshot.roots()[0].id().expect("root element"))
        .expect("scene root");
    assert_eq!(
        root.resolved_style().background(),
        Some(Color::Literal([12, 34, 56, 255])),
        "a literal color reaches the scene unchanged"
    );
}

#[test]
fn semantic_conditions_select_declarations_from_committed_state() {
    let mut app = Runtime::new();
    app.mount(|| {
        view! {
            <view>
                <button aria:label="dead" disabled={true} class="bg-control disabled:bg-danger">"Dead"</button>
                <button aria:label="toggle" aria:pressed={true} class="bg-control aria-pressed:bg-accent">"On"</button>
                <checkbox aria:label="checked" checked={true} class="bg-control checked:bg-accent" />
                <text-input aria:label="editor" class="bg-control focus:bg-accent" />
            </view>
        }
    })
    .expect("mount");
    app.focus("editor").expect("focus the editor");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 400.0);

    assert_eq!(
        background_by_label(&snapshot, &scene, "dead"),
        Some(Color::Role(ColorRole::Danger)),
        "disabled: applies while the button is disabled"
    );
    assert_eq!(
        background_by_label(&snapshot, &scene, "toggle"),
        Some(Color::Role(ColorRole::Accent)),
        "aria-pressed: follows the semantic toggle"
    );
    assert_eq!(
        background_by_label(&snapshot, &scene, "checked"),
        Some(Color::Role(ColorRole::Accent)),
        "checked: follows the committed checkbox state"
    );
    assert_eq!(
        background_by_label(&snapshot, &scene, "editor"),
        Some(Color::Role(ColorRole::Accent)),
        "focus: follows the committed focus"
    );

    app.blur().expect("clear focus");
    let unfocused = app.presentation().expect("snapshot");
    let scene = build_scene(&unfocused, 400.0);
    assert_eq!(
        scene
            .node(unfocused.find_label("editor").expect("editor").id())
            .expect("editor")
            .resolved_style()
            .background(),
        Some(Color::Role(ColorRole::Control)),
        "the base declaration returns once focus is gone"
    );
}

#[test]
fn condition_rank_and_source_order_break_ties() {
    let mut app = Runtime::new();
    app.mount(|| {
        view! {
            <view>
                <button
                    aria:label="ranked"
                    aria:pressed={true}
                    disabled={true}
                    class="aria-pressed:bg-accent disabled:bg-danger"
                >"Ranked"</button>
                <button
                    aria:label="tied"
                    aria:pressed={true}
                    class="aria-pressed:bg-control aria-pressed:bg-danger"
                >"Tied"</button>
            </view>
        }
    })
    .expect("mount");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 400.0);

    assert_eq!(
        background_by_label(&snapshot, &scene, "ranked"),
        Some(Color::Role(ColorRole::Accent)),
        "aria-pressed outranks disabled even when it appears earlier"
    );
    assert_eq!(
        background_by_label(&snapshot, &scene, "tied"),
        Some(Color::Role(ColorRole::Danger)),
        "equal conditions use the later source order"
    );
}

#[test]
fn the_demo_filters_show_their_selected_style_and_still_filter() {
    let mut app = runtime();
    add(&mut app, "A");
    app.change("Complete A", true).expect("complete A");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 800.0);
    assert_eq!(
        background_by_text(&snapshot, &scene, "All"),
        Some(Color::Role(ColorRole::Accent)),
        "the selected filter uses the accent role"
    );
    assert_eq!(
        background_by_text(&snapshot, &scene, "Active"),
        Some(Color::Role(ColorRole::Control)),
        "an unselected filter uses the control role"
    );

    app.click_text("Active").expect("filter active");
    let filtered = app.presentation().expect("snapshot");
    assert!(
        filtered.find_label("Complete A").is_none(),
        "aria-pressed styling leaves the toggle semantics intact"
    );
    let scene = build_scene(&filtered, 800.0);
    assert_eq!(
        scene
            .node(by_text(&filtered, "button", "Active"))
            .expect("Active")
            .resolved_style()
            .background(),
        Some(Color::Role(ColorRole::Accent)),
        "the newly selected filter gains the accent role"
    );
}

#[test]
fn inline_styles_beat_class_base_and_state_variants() {
    let mut app = Runtime::new();
    app.mount(|| {
        view! {
            <button
                aria:label="styled"
                disabled={true}
                class="bg-control disabled:bg-danger"
                style:background={Color::Role(ColorRole::Accent)}
                style:opacity={Opacity::percent(75)}
            >"Styled"</button>
        }
    })
    .expect("mount");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 400.0);
    let style = scene
        .node(snapshot.find_label("styled").expect("button").id())
        .expect("button")
        .resolved_style();
    assert_eq!(
        style.background(),
        Some(Color::Role(ColorRole::Accent)),
        "an inline declaration beats a higher-rank class variant"
    );
    assert_eq!(style.opacity(), 0.75, "inline opacity resolves in place");
}

#[test]
fn px_and_em_widths_produce_expected_geometry() {
    let mut app = Runtime::new();
    app.mount(|| {
        view! {
            <view class="flex-row">
                <view aria:label="px" style:width={px(100.0)} />
                <view aria:label="em" style:width={em(2.0)} />
            </view>
        }
    })
    .expect("mount");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 400.0);
    let width = |label: &str| {
        scene
            .node(snapshot.find_label(label).expect(label).id())
            .expect(label)
            .rect()
            .width
    };
    assert!((width("px") - 100.0).abs() < 0.01, "px is logical points");
    assert!(
        (width("em") - 28.0).abs() < 0.01,
        "em resolves against the 14-point font size"
    );
}

#[test]
fn the_demo_completed_row_text_follows_a_memo_driven_style() {
    let mut app = runtime();
    add(&mut app, "A");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 800.0);
    let tint = |scene: &Scene, snapshot: &PresentationSnapshot| {
        scene
            .node(by_text(snapshot, "text", "A"))
            .expect("row text")
            .resolved_style()
            .text_color()
    };
    assert_eq!(
        tint(&scene, &snapshot),
        Some(Color::Role(ColorRole::TextDefault)),
        "an active row uses the default text role"
    );

    app.change("Complete A", true).expect("complete A");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 800.0);
    assert_eq!(
        tint(&scene, &snapshot),
        Some(Color::Role(ColorRole::TextMuted)),
        "the memo-driven style follows the completed state"
    );
}

#[test]
fn text_classes_change_glyph_size_height_and_caret_geometry() {
    let mut app = Runtime::new();
    app.mount(|| {
        view! {
            <view>
                <text aria:label="small" class="text-sm">"Small"</text>
                <text aria:label="large" class="text-lg">"Large"</text>
                <text aria:label="base-text" class="text-lg text-base">"Base"</text>
                <button aria:label="bold" class="font-semibold">"Bold"</button>
                <button aria:label="normal" class="font-semibold font-normal">"Normal"</button>
                <text-input aria:label="base" value="abc" />
                <text-input aria:label="large-input" class="text-lg" value="abc" />
                <view aria:label="em" class="text-lg" style:width={em(2.0)} />
            </view>
        }
    })
    .expect("mount");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 400.0);
    let element = |label: &str| {
        scene
            .node(snapshot.find_label(label).expect(label).id())
            .expect(label)
    };
    let font_size = |label: &str| element(label).text()[0].font_size();

    assert_eq!(font_size("small"), 12.0, "text-sm shapes at 12 points");
    assert_eq!(font_size("large"), 16.0, "text-lg shapes at 16 points");
    assert_eq!(
        font_size("base-text"),
        14.0,
        "a later text-base overrides text-lg"
    );
    assert!(
        element("large").rect().height > element("small").rect().height,
        "measurement follows the larger font"
    );
    assert_eq!(
        element("bold").resolved_style().font_weight(),
        600,
        "font-semibold sets the weight"
    );
    assert_eq!(
        element("normal").resolved_style().font_weight(),
        400,
        "a later font-normal overrides font-semibold"
    );
    assert!(
        element("bold")
            .text()
            .first()
            .is_some_and(|run| !run.glyphs().is_empty()),
        "the weighted label still shapes glyphs"
    );
    assert!(
        (element("em").rect().width - 32.0).abs() < 0.01,
        "em resolves against the element's 16-point font"
    );

    let caret_height = |app: &mut Runtime, label: &str| {
        app.focus(label).expect("focus");
        let snapshot = app.presentation().expect("snapshot");
        let scene = build_scene(&snapshot, 400.0);
        let id = snapshot.find_label(label).expect("input").id();
        scene.caret(id).expect("caret").height
    };
    let base = caret_height(&mut app, "base");
    let large = caret_height(&mut app, "large-input");
    assert!(
        large > base,
        "the caret grows with the font: base={base} large={large}"
    );
}

#[test]
fn the_demo_count_and_add_button_use_text_utilities() {
    let app = runtime();
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 800.0);
    let count = scene
        .node(by_text(&snapshot, "text", "0 remaining"))
        .expect("count label");
    assert_eq!(
        count.resolved_style().font_size(),
        12.0,
        "the remaining count uses text-sm"
    );
    let add = scene
        .node(by_text(&snapshot, "button", "Add"))
        .expect("Add button");
    assert_eq!(
        add.resolved_style().font_weight(),
        600,
        "the primary button uses font-semibold"
    );
}

#[test]
fn pointer_and_modality_conditions_select_from_the_scene_environment() {
    let mut app = Runtime::new();
    app.mount(|| {
        view! {
            <view>
                <button
                    aria:label="feedback"
                    class="bg-control hover:bg-control-hover active:bg-control-pressed"
                >"Hover"</button>
                <text-input aria:label="editor" class="bg-control focus-visible:bg-accent" />
            </view>
        }
    })
    .expect("mount");
    app.focus("editor").expect("focus the editor");
    let snapshot = app.presentation().expect("snapshot");
    let viewport = viewport(400.0);
    let button = snapshot.find_label("feedback").expect("button").id();
    let editor = snapshot.find_label("editor").expect("editor").id();
    let background = |scene: &Scene, node| {
        scene
            .node(node)
            .expect("scene element")
            .resolved_style()
            .background()
    };

    let plain = Scene::build_with(&snapshot, viewport, &SceneEnv::default()).expect("scene");
    assert_eq!(
        background(&plain, button),
        Some(Color::Role(ColorRole::Control))
    );
    assert_eq!(
        background(&plain, editor),
        Some(Color::Role(ColorRole::Control)),
        "focus alone is not focus-visible"
    );

    let hovered = Scene::build_with(
        &snapshot,
        viewport,
        &SceneEnv::default().with_hovered(Some(button)),
    )
    .expect("scene");
    assert_eq!(
        background(&hovered, button),
        Some(Color::Role(ColorRole::ControlHover))
    );
    assert!(hovered.node(button).expect("button").hovered());
    assert!(!hovered.node(button).expect("button").pointer_pressed());

    let pressed = Scene::build_with(
        &snapshot,
        viewport,
        &SceneEnv::default()
            .with_hovered(Some(button))
            .with_pressed(Some(button)),
    )
    .expect("scene");
    assert_eq!(
        background(&pressed, button),
        Some(Color::Role(ColorRole::ControlPressed)),
        "active outranks hover"
    );
    assert!(pressed.node(button).expect("button").pointer_pressed());

    let keyboard = Scene::build_with(
        &snapshot,
        viewport,
        &SceneEnv::default().with_focus_visible(true),
    )
    .expect("scene");
    assert_eq!(
        background(&keyboard, editor),
        Some(Color::Role(ColorRole::Accent)),
        "focus-visible follows the host modality"
    );
}

#[test]
fn dark_conditions_follow_the_snapshot_appearance() {
    let mut app = Runtime::new();
    app.mount(|| view! { <view aria:label="themed" class="bg-surface dark:bg-accent" /> })
        .expect("mount");
    let snapshot = app.presentation().expect("snapshot");
    let viewport = viewport(400.0);
    let id = snapshot.find_label("themed").expect("themed view").id();
    let light = Scene::build_with(&snapshot, viewport, &SceneEnv::default()).expect("scene");
    assert_eq!(
        light
            .node(id)
            .expect("element")
            .resolved_style()
            .background(),
        Some(Color::Role(ColorRole::Surface))
    );

    app.set_appearance(Appearance::Dark).expect("switch theme");
    let dark = app.presentation().expect("snapshot");
    assert_eq!(dark.appearance(), Appearance::Dark);
    let scene = Scene::build_with(&dark, viewport, &SceneEnv::default()).expect("scene");
    assert_eq!(
        scene
            .node(id)
            .expect("element")
            .resolved_style()
            .background(),
        Some(Color::Role(ColorRole::Accent)),
        "dark: declarations follow the committed appearance"
    );
}

#[test]
fn a_mismatched_style_declaration_is_an_explicit_error() {
    const STYLE: &[StyleDeclaration] = &[StyleDeclaration::new(
        Property::Gap,
        StyleValue::Row,
        Condition::Base,
    )];
    let mut app = Runtime::new();
    app.open_window("main", |_| {
        Ok(MountPlan::new(vec![zenui::view::styled_element(
            "view",
            Vec::new(),
            STYLE,
            Vec::new(),
            Vec::new(),
        )]))
    })
    .expect("window");
    let snapshot = app.presentation().expect("snapshot");
    let node = snapshot.roots()[0].id().expect("root element");
    match Scene::build(&snapshot, viewport(800.0)) {
        Err(SceneError::UnsupportedStyle {
            node: failed,
            property,
            value,
        }) => {
            assert_eq!(failed, node);
            assert_eq!((property, value), (Property::Gap, StyleValue::Row));
        }
        other => panic!("expected UnsupportedStyle, got {other:?}"),
    }
}

#[test]
fn a_state_variant_cannot_set_a_layout_property() {
    const STYLE: &[StyleDeclaration] = &[StyleDeclaration::new(
        Property::PaddingX,
        StyleValue::Points(8),
        Condition::Focus,
    )];
    let mut app = Runtime::new();
    app.open_window("main", |_| {
        Ok(MountPlan::new(vec![zenui::view::styled_element(
            "view",
            Vec::new(),
            STYLE,
            Vec::new(),
            Vec::new(),
        )]))
    })
    .expect("window");
    let snapshot = app.presentation().expect("snapshot");
    let node = snapshot.roots()[0].id().expect("root element");
    // 失效的条件同样被拒绝：错误不能取决于当前瞬态。
    match Scene::build(&snapshot, viewport(800.0)) {
        Err(SceneError::UnsupportedStyle {
            node: failed,
            property,
            value,
        }) => {
            assert_eq!(failed, node);
            assert_eq!(
                (property, value),
                (Property::PaddingX, StyleValue::Points(8))
            );
        }
        other => panic!("expected UnsupportedStyle, got {other:?}"),
    }
}

#[test]
fn class_padding_moves_the_text_origin_but_keeps_the_hit_area() {
    let mut app = Runtime::new();
    app.mount(|| view! { <text-input aria:label="padded" class="px-4" value="abc" /> })
        .expect("mount");
    app.focus("padded").expect("focus");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 400.0);
    let id = snapshot.find_label("padded").expect("input").id();
    let input = scene.node(id).expect("scene input");
    let rect = input.rect();
    let origin = rect.x + 1.0 + 16.0;
    let caret = scene.caret(id).expect("caret");
    assert!(
        (caret.x - origin).abs() < 0.5,
        "px-4 offsets the caret past border and padding: {caret:?} rect={rect:?}"
    );
    let glyph_x = input
        .text()
        .iter()
        .flat_map(|run| run.glyphs())
        .map(|glyph| glyph.x)
        .fold(f32::INFINITY, f32::min);
    assert!(
        (glyph_x - origin).abs() < 1.0,
        "the value starts after the padding: {glyph_x} vs {origin}"
    );

    let in_padding = Point::new(rect.x + 4.0, rect.y + rect.height / 2.0);
    assert_eq!(
        scene.hit_test(in_padding),
        Some(id),
        "the padded border box stays clickable"
    );
    assert_eq!(
        input.caret_index(in_padding),
        Some(0),
        "a click in the left padding clamps to the value start"
    );
}

#[test]
fn full_width_spans_the_row() {
    let mut app = Runtime::new();
    app.mount(|| {
        view! {
            <view class="flex-row items-center">
                <button class="w-full">"Wide"</button>
            </view>
        }
    })
    .expect("mount");
    let snapshot = app.presentation().expect("snapshot");
    let scene = build_scene(&snapshot, 400.0);
    let wide = scene
        .node(by_text(&snapshot, "button", "Wide"))
        .expect("wide button")
        .rect();

    assert!(
        (wide.width - 400.0).abs() < 0.5,
        "w-full spans the row: {wide:?}"
    );
}

fn scene_fingerprint(scene: &Scene) -> String {
    let mut out = format!(
        "viewport={:?} caret={}\n",
        scene.viewport(),
        scene.caret_visible()
    );
    for element in scene.elements() {
        out.push_str(&format!(
            "element {:?} {:?} rect={:?} visual={:?} style={:?} disabled={}\n",
            element.id(),
            element.tag(),
            element.rect(),
            element.visual(),
            element.resolved_style(),
            element.disabled()
        ));
        for run in element.text() {
            out.push_str(&format!(
                "run size={} coords={:?}\n",
                run.font_size(),
                run.normalized_coords()
            ));
            for glyph in run.glyphs() {
                out.push_str(&format!(
                    "glyph {} {} {} {}\n",
                    glyph.id, glyph.x, glyph.y, glyph.advance
                ));
            }
        }
    }
    for decoration in scene.decorations() {
        out.push_str(&format!("decoration {decoration:?}\n"));
    }
    out
}

#[test]
fn a_scene_cache_reuses_layouts_without_changing_the_scene() {
    let mut app = runtime();
    add(&mut app, "Write tests");
    add(&mut app, "Read docs");
    app.input("What needs to be done?", "draft").expect("draft");
    let snapshot = app.presentation().expect("snapshot");
    let base_viewport = viewport(800.0);

    let fresh = Scene::build(&snapshot, base_viewport).expect("fresh scene");
    let mut cache = SceneCache::new();
    let first = Scene::build_cached(&snapshot, base_viewport, &SceneEnv::default(), &mut cache)
        .expect("cached scene");
    let second = Scene::build_cached(&snapshot, base_viewport, &SceneEnv::default(), &mut cache)
        .expect("cached scene again");
    let fingerprint = scene_fingerprint(&fresh);
    assert_eq!(fingerprint, scene_fingerprint(&first));
    assert_eq!(fingerprint, scene_fingerprint(&second));

    app.input("What needs to be done?", "changed")
        .expect("changed draft");
    let changed = app.presentation().expect("snapshot");
    let cached = Scene::build_cached(&changed, base_viewport, &SceneEnv::default(), &mut cache)
        .expect("cached changed scene");
    let fresh = Scene::build(&changed, base_viewport).expect("fresh changed scene");
    let changed_fingerprint = scene_fingerprint(&fresh);
    assert_eq!(changed_fingerprint, scene_fingerprint(&cached));
    assert_ne!(
        fingerprint, changed_fingerprint,
        "the changed draft produces different glyphs"
    );

    let narrow = viewport(500.0);
    let cached = Scene::build_cached(&changed, narrow, &SceneEnv::default(), &mut cache)
        .expect("cached narrow scene");
    let fresh = Scene::build(&changed, narrow).expect("fresh narrow scene");
    assert_eq!(
        scene_fingerprint(&fresh),
        scene_fingerprint(&cached),
        "a different width hits a different shaped layout"
    );
}

#[test]
fn offscreen_rows_keep_geometry_but_skip_glyphs() {
    let mut app = runtime();
    for index in 0..60 {
        add(&mut app, &format!("row-{index:02}"));
    }
    let snapshot = app.presentation().expect("snapshot");
    let short = Scene::build(
        &snapshot,
        Viewport::new(800.0, 300.0, 1.0).expect("short viewport"),
    )
    .expect("short scene");
    let tall = Scene::build(
        &snapshot,
        Viewport::new(800.0, 4000.0, 1.0).expect("tall viewport"),
    )
    .expect("tall scene");
    let id = by_text(&snapshot, "text", "row-59");

    let offscreen = short.node(id).expect("offscreen geometry is kept");
    assert!(offscreen.text().is_empty(), "offscreen glyphs are skipped");
    let visible = tall.node(id).expect("visible element");
    assert!(
        !visible.text().is_empty(),
        "a taller viewport shapes the row"
    );
    assert_eq!(
        offscreen.rect(),
        visible.rect(),
        "culling does not move the element"
    );
}
