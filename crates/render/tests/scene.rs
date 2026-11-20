#![allow(dead_code, unused_braces)]

#[path = "../../../examples/minimal_todo.rs"]
mod todo;

use todo::*;
use zenui::prelude::*;
use zenui_render::{DecorationKind, Point, Scene, SceneElement, SceneError, Viewport};

fn runtime() -> Runtime {
    let mut app = Runtime::new();
    app.open_window("main", |window| view! { <TodoApp window={window} /> })
        .expect("open window");
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
        build_scene(&idle, 800.0).caret(idle_id).is_some(),
        "caret geometry exists without focus; drawing it is the backend's choice"
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
