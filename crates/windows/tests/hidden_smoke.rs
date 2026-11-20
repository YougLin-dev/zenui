#![cfg(windows)]

use std::collections::BTreeSet;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE, WM_LBUTTONDOWN};
use zenui::NodeId;
use zenui_render::{Point, SceneElement, Visual};
use zenui_windows::{Host, HostConfig, HostError};

fn post_click(host: &Host, point: Point, scale: f32) {
    let x = (point.x * scale).round() as i16;
    let y = (point.y * scale).round() as i16;
    let lparam = LPARAM((x as isize & 0xffff) | ((y as isize & 0xffff) << 16));
    let hwnd = host.hwnd();
    // 安全：HWND 由 Host 持有，消息只排队，不跨线程解引用。
    unsafe {
        PostMessageW(Some(hwnd), WM_LBUTTONDOWN, WPARAM(0), lparam).expect("post the click");
    }
}

fn post_close(host: &Host) {
    let hwnd = host.hwnd();
    // 安全：HWND 由 Host 持有，消息只排队，不跨线程解引用。
    unsafe {
        PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).expect("post the close");
    }
}

fn point_in(host: &Host, id: NodeId, fraction: f32) -> Point {
    let rect = host
        .scene()
        .expect("a rendered scene")
        .node(id)
        .expect("element")
        .rect();
    Point::new(rect.x + rect.width * fraction, rect.y + rect.height / 2.0)
}

fn first_text_glyph(host: &Host) -> u32 {
    host.scene()
        .expect("a rendered scene")
        .elements()
        .iter()
        .find(|element| matches!(element.visual(), Visual::Text))
        .and_then(|element| element.text().first())
        .and_then(|run| run.glyphs().first())
        .expect("a text glyph")
        .id
}

fn center_of(host: &Host, predicate: impl Fn(&SceneElement) -> bool) -> (NodeId, Point, f32) {
    let scene = host.scene().expect("a rendered scene");
    let element = scene
        .elements()
        .iter()
        .find(|element| predicate(element))
        .expect("a matching scene element");
    let rect = element.rect();
    let point = Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
    (element.id(), point, scene.viewport().scale())
}

#[test]
fn a_hidden_window_configures_a_surface_and_reads_back_a_frame() {
    let mut host =
        Host::open(HostConfig::hidden("ZenUI hidden smoke")).expect("open a hidden host");

    host.render().expect("render and present a surface frame");
    let frame = host.read_frame().expect("render and read back a frame");

    assert!(frame.width() > 0 && frame.height() > 0);
    assert_eq!(
        frame.pixels().len(),
        (frame.width() * frame.height() * 4) as usize
    );
    assert_eq!(
        frame.pixel(4, 4),
        Some([255, 255, 255, 255]),
        "the root surface paints opaque white in the default palette"
    );
    let distinct: BTreeSet<[u8; 4]> = frame.pixels().as_chunks::<4>().0.iter().copied().collect();
    assert!(
        distinct.len() > 3,
        "the frame contains control and text pixels, got {distinct:?}"
    );
}

#[test]
fn closing_the_window_ends_the_loop_and_reopening_mounts_fresh() {
    let mut host = Host::open(HostConfig::hidden("ZenUI close smoke")).expect("open a host");
    host.request_close().expect("post WM_CLOSE");
    host.run().expect("the message loop exits on close");
    drop(host);

    let mut reopened =
        Host::open(HostConfig::hidden("ZenUI reopen smoke")).expect("reopen a fresh host");
    reopened.render().expect("render a fresh activation");
}

#[test]
fn an_empty_client_area_renders_nothing_and_refuses_readback() {
    let mut host = Host::open(HostConfig {
        title: "ZenUI empty smoke",
        visible: false,
        width: 0,
        height: 0,
    })
    .expect("open a zero-sized host");

    host.render()
        .expect("rendering an empty client area is a no-op");
    assert!(
        matches!(host.read_frame(), Err(HostError::Readback(_))),
        "readback of an empty client area reports a readback error"
    );
}

#[test]
fn clicking_a_text_input_focuses_it_and_changes_the_frame() {
    let mut host = Host::open(HostConfig::hidden("ZenUI pointer focus")).expect("open a host");
    host.render().expect("render the initial frame");
    let before = host.read_frame().expect("baseline frame");

    let (id, _, scale) = center_of(&host, |element| {
        matches!(element.visual(), Visual::TextInput) && !element.text().is_empty()
    });
    let initial_caret = host.scene().expect("scene").caret(id).expect("caret").x;
    // 有内容的输入在点击靠右处会把 caret 移到非零偏移，帧因此变化。
    post_click(&host, point_in(&host, id, 0.8), scale);
    assert!(
        !host.pump().expect("process the click"),
        "no close was posted"
    );

    let focused = host.read_frame().expect("frame after the click");
    assert!(
        host.scene()
            .expect("scene after the click")
            .node(id)
            .expect("text input")
            .focused(),
        "the click focuses the text input"
    );
    assert_ne!(
        before.pixels(),
        focused.pixels(),
        "the focused caret changes the frame"
    );
    let placed_caret = host.scene().expect("scene").caret(id).expect("caret").x;
    assert!(
        placed_caret > initial_caret + 10.0,
        "the click moves the caret into the text"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn clicking_blank_space_clears_focus() {
    let mut host = Host::open(HostConfig::hidden("ZenUI pointer blur")).expect("open a host");
    host.render().expect("render the initial frame");

    let (id, _, scale) = center_of(&host, |element| {
        matches!(element.visual(), Visual::TextInput) && !element.text().is_empty()
    });
    post_click(&host, point_in(&host, id, 0.8), scale);
    host.pump().expect("process the focus click");
    assert!(
        host.scene()
            .expect("scene after focus")
            .node(id)
            .expect("text input")
            .focused(),
        "the input starts focused"
    );

    // 根容器左上角的空白不是可聚焦内容，应清除焦点。
    post_click(&host, Point::new(2.0, 2.0), scale);
    host.pump().expect("process the blur click");
    assert!(
        !host
            .scene()
            .expect("scene after blur")
            .node(id)
            .expect("text input")
            .focused(),
        "clicking non-focusable space clears focus"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn clicking_a_checkbox_toggles_its_visual_state() {
    let mut host = Host::open(HostConfig::hidden("ZenUI pointer checkbox")).expect("open a host");
    host.render().expect("render the initial frame");

    let (id, point, scale) = center_of(&host, |element| {
        matches!(element.visual(), Visual::Checkbox { .. })
    });
    assert!(
        matches!(
            host.scene()
                .expect("scene")
                .node(id)
                .expect("checkbox")
                .visual(),
            Visual::Checkbox { checked: true }
        ),
        "the first item starts completed"
    );

    let before = host.read_frame().expect("baseline frame");
    let counter = first_text_glyph(&host);
    post_click(&host, point, scale);
    assert!(
        !host.pump().expect("process the click"),
        "no close was posted"
    );

    let after = host.read_frame().expect("frame after the click");
    assert_ne!(
        before.pixels(),
        after.pixels(),
        "the toggled checkbox changes the frame"
    );
    assert!(
        matches!(
            host.scene()
                .expect("scene after click")
                .node(id)
                .expect("checkbox")
                .visual(),
            Visual::Checkbox { checked: false }
        ),
        "the checkbox click toggles the Todo"
    );
    assert_ne!(
        first_text_glyph(&host),
        counter,
        "toggling the Todo updates the remaining counter"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn clicking_the_active_filter_hides_completed_items() {
    let mut host = Host::open(HostConfig::hidden("ZenUI pointer filter")).expect("open a host");
    host.render().expect("render the initial frame");

    let (_, active, scale) = {
        let scene = host.scene().expect("scene");
        let buttons: Vec<_> = scene
            .elements()
            .iter()
            .filter(|element| matches!(element.visual(), Visual::Button { .. }))
            .collect();
        // 前四个按钮依次是 Add、All、Active、Done。
        let button = buttons.get(2).expect("the Active filter button");
        let rect = button.rect();
        (
            button.id(),
            Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0),
            scene.viewport().scale(),
        )
    };
    assert_eq!(
        host.scene()
            .expect("scene")
            .elements()
            .iter()
            .filter(|element| matches!(element.visual(), Visual::Checkbox { .. }))
            .count(),
        2,
        "the unfiltered list shows both items"
    );

    post_click(&host, active, scale);
    post_close(&host);
    host.run().expect("process the click and close");

    assert_eq!(
        host.scene()
            .expect("scene after click")
            .elements()
            .iter()
            .filter(|element| matches!(element.visual(), Visual::Checkbox { .. }))
            .count(),
        1,
        "the Active filter hides the completed item"
    );
}
