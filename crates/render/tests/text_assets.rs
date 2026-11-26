//! 文本资产契约：注册固定字体后，场景使用资产字体而不是系统字体；空字节、
//! 非字体容器、未注册家族在缓存构造期给出 typed error。

#![allow(dead_code, unused_braces)]

use zenui::prelude::*;
use zenui_demo::*;
use zenui_render::{
    FontAsset, Point, Scene, SceneCache, SceneElement, SceneEnv, TextAssetError, TextAssets,
    Viewport,
};

/// 与原生原型共用的公有领域测试字体；不走系统字体集合。
const TUFFY: &[u8] = include_bytes!("../../../examples/native_poc/assets/Tuffy.ttf");

fn fixed_assets() -> TextAssets {
    TextAssets::new("Tuffy").with_font(FontAsset::new(TUFFY))
}

fn runtime() -> Runtime {
    let mut app = Runtime::new();
    open_todo_window(&mut app).expect("open window");
    app
}

fn viewport(width: f32) -> Viewport {
    Viewport::new(width, 600.0, 1.0).expect("viewport")
}

fn build(cache: &mut SceneCache, snapshot: &PresentationSnapshot, width: f32) -> Scene {
    Scene::build_cached(snapshot, viewport(width), &SceneEnv::default(), cache).expect("scene")
}

fn by_text<'a>(snapshot: &'a PresentationSnapshot, tag: &str, text: &str) -> &'a SnapshotElement {
    snapshot
        .elements()
        .find(|element| element.tag() == tag && element.text() == text)
        .unwrap_or_else(|| panic!("no {tag} with text {text:?}"))
}

fn run_fonts(element: &SceneElement) -> Vec<&[u8]> {
    element
        .text()
        .iter()
        .map(|run| run.font().data.as_ref())
        .collect()
}

#[test]
fn configured_assets_shape_covered_text_with_the_fixed_font() {
    let app = runtime();
    let snapshot = app.presentation().expect("snapshot");
    let mut cache = SceneCache::with_text_assets(fixed_assets()).expect("text assets");
    let scene = build(&mut cache, &snapshot, 800.0);

    let node = by_text(&snapshot, "button", "Load sample").id();
    let fonts = run_fonts(scene.node(node).expect("toolbar button"));
    assert!(!fonts.is_empty(), "the toolbar label carries glyph runs");
    for font in fonts {
        assert_eq!(font, TUFFY, "no system family substitutes for covered text");
    }
}

#[test]
fn the_asset_cache_keeps_metrics_and_caret_geometry_stable() {
    let mut app = runtime();
    seed(&mut app).expect("seed rows");
    app.focus("Edit Write tests").expect("focus the row input");
    let snapshot = app.presentation().expect("snapshot");
    let mut cache = SceneCache::with_text_assets(fixed_assets()).expect("text assets");
    let node = snapshot
        .find_label("Edit Write tests")
        .expect("row input")
        .id();

    let first = build(&mut cache, &snapshot, 900.0);
    let second = build(&mut cache, &snapshot, 900.0);
    let first_element = first.node(node).expect("row input in the first scene");
    let second_element = second.node(node).expect("row input in the second scene");
    assert_eq!(first_element.rect(), second_element.rect());
    assert_eq!(first.caret(node), second.caret(node));
    let caret = first
        .caret(node)
        .expect("the focused input exposes a caret");
    assert_eq!(
        first.hit_test(Point::new(caret.x + 0.5, caret.y + 0.5)),
        Some(node),
        "the caret point hits the focused input"
    );
    assert!(!first_element.text().is_empty());
    for run in first_element.text() {
        assert_eq!(run.font().data.as_ref(), TUFFY);
    }
}

#[test]
fn assets_without_bytes_fail_at_construction() {
    let error = SceneCache::with_text_assets(TextAssets::new("Tuffy"))
        .err()
        .expect("no fonts is an error");
    assert_eq!(error, TextAssetError::NoFonts);

    let error =
        SceneCache::with_text_assets(TextAssets::new("Tuffy").with_font(FontAsset::new([])))
            .err()
            .expect("empty bytes is an error");
    assert_eq!(error, TextAssetError::MissingFont { index: 0 });
}

#[test]
fn assets_that_are_not_fonts_fail_at_construction() {
    let error =
        SceneCache::with_text_assets(TextAssets::new("Tuffy").with_font(FontAsset::new("notes")))
            .err()
            .expect("garbage bytes are an error");
    assert_eq!(error, TextAssetError::UnsupportedFont { index: 0 });

    let error = SceneCache::with_text_assets(
        TextAssets::new("Tuffy").with_font(FontAsset::new(b"OTTO with no tables")),
    )
    .err()
    .expect("a container without families is an error");
    assert_eq!(error, TextAssetError::NoUsableFamily { index: 0 });
}

#[test]
fn unregistered_families_fail_at_construction() {
    let error =
        SceneCache::with_text_assets(TextAssets::new("Missing").with_font(FontAsset::new(TUFFY)))
            .err()
            .expect("an unknown primary family is an error");
    assert_eq!(
        error,
        TextAssetError::UnknownPrimaryFamily {
            family: String::from("Missing"),
        }
    );

    let error = SceneCache::with_text_assets(
        TextAssets::new("Tuffy")
            .with_fallback_family("Missing")
            .with_font(FontAsset::new(TUFFY)),
    )
    .err()
    .expect("an unknown fallback family is an error");
    assert_eq!(
        error,
        TextAssetError::UnknownFallbackFamily {
            family: String::from("Missing"),
        }
    );
}

#[test]
fn system_families_are_not_registered_without_the_flag() {
    // 系统字体默认不进入集合；即使 Windows 上存在 Segoe UI，它也不是已注册家族。
    let error =
        SceneCache::with_text_assets(TextAssets::new("Segoe UI").with_font(FontAsset::new(TUFFY)))
            .err()
            .expect("a system family is not registered");
    assert!(matches!(error, TextAssetError::UnknownPrimaryFamily { .. }));
}

#[cfg(windows)]
#[test]
fn system_fallback_registers_system_families() {
    let assets = TextAssets::new("Segoe UI")
        .with_system_fallback(true)
        .with_font(FontAsset::new(TUFFY));
    let mut cache = SceneCache::with_text_assets(assets).expect("Segoe UI is a system family");
    let app = runtime();
    let snapshot = app.presentation().expect("snapshot");
    let scene = build(&mut cache, &snapshot, 800.0);

    let node = by_text(&snapshot, "button", "Load sample").id();
    let fonts = run_fonts(scene.node(node).expect("toolbar button"));
    assert!(!fonts.is_empty());
    for font in fonts {
        assert_ne!(font, TUFFY, "the configured system family shapes the label");
    }
}

#[cfg(windows)]
#[test]
fn the_primary_family_wins_over_the_fallback_chain() {
    let app = runtime();
    let snapshot = app.presentation().expect("snapshot");
    let node = by_text(&snapshot, "button", "Load sample").id();

    let mut fixed_first = SceneCache::with_text_assets(
        TextAssets::new("Tuffy")
            .with_fallback_family("Segoe UI")
            .with_system_fallback(true)
            .with_font(FontAsset::new(TUFFY)),
    )
    .expect("both families are registered");
    let fixed_scene = build(&mut fixed_first, &snapshot, 800.0);
    for font in run_fonts(fixed_scene.node(node).expect("toolbar button")) {
        assert_eq!(font, TUFFY, "the primary family wins for covered text");
    }

    let mut system_first = SceneCache::with_text_assets(
        TextAssets::new("Segoe UI")
            .with_fallback_family("Tuffy")
            .with_system_fallback(true)
            .with_font(FontAsset::new(TUFFY)),
    )
    .expect("both families are registered");
    let system_scene = build(&mut system_first, &snapshot, 800.0);
    for font in run_fonts(system_scene.node(node).expect("toolbar button")) {
        assert_ne!(
            font, TUFFY,
            "the primary family wins when it is a system family"
        );
    }
}
