#![cfg(windows)]

use std::collections::BTreeSet;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_BACK, VK_DELETE, VK_END, VK_HOME, VK_RETURN, VK_RIGHT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, WM_CHAR, WM_CLOSE, WM_KEYDOWN, WM_LBUTTONDOWN,
};
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

fn post_char(host: &Host, character: char) {
    let hwnd = host.hwnd();
    // 安全：HWND 由 Host 持有，消息只排队，不跨线程解引用。
    unsafe {
        PostMessageW(Some(hwnd), WM_CHAR, WPARAM(character as usize), LPARAM(0))
            .expect("post the character");
    }
}

fn post_key(host: &Host, key: VIRTUAL_KEY) {
    let hwnd = host.hwnd();
    // 安全：HWND 由 Host 持有，消息只排队，不跨线程解引用。
    unsafe {
        PostMessageW(Some(hwnd), WM_KEYDOWN, WPARAM(key.0 as usize), LPARAM(0))
            .expect("post the key");
    }
}

fn glyphs(host: &Host, id: NodeId) -> usize {
    host.scene()
        .expect("a rendered scene")
        .node(id)
        .expect("element")
        .text()
        .iter()
        .map(|run| run.glyphs().len())
        .sum()
}

fn focused_id(host: &Host) -> Option<NodeId> {
    host.scene()
        .expect("a rendered scene")
        .elements()
        .iter()
        .find(|element| element.focused())
        .map(SceneElement::id)
}

fn post_close(host: &Host) {
    let hwnd = host.hwnd();
    // 安全：HWND 由 Host 持有，消息只排队，不跨线程解引用。
    unsafe {
        PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).expect("post the close");
    }
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

fn point_in_text(host: &Host, id: NodeId, fraction: f32) -> Point {
    let scene = host.scene().expect("a rendered scene");
    let element = scene.node(id).expect("element");
    let glyphs: Vec<_> = element.text().iter().flat_map(|run| run.glyphs()).collect();
    let start = glyphs.first().map_or(element.rect().x, |glyph| glyph.x);
    let end = glyphs.last().map_or(start, |glyph| glyph.x + glyph.advance);
    Point::new(
        start + (end - start) * fraction,
        element.rect().y + element.rect().height / 2.0,
    )
}

fn checkbox_ids(host: &Host) -> Vec<NodeId> {
    host.scene()
        .expect("a rendered scene")
        .elements()
        .iter()
        .filter(|element| matches!(element.visual(), Visual::Checkbox { .. }))
        .map(SceneElement::id)
        .collect()
}

fn button_at(host: &Host, index: usize) -> (Point, f32) {
    let scene = host.scene().expect("a rendered scene");
    let button = scene
        .elements()
        .iter()
        .filter(|element| matches!(element.visual(), Visual::Button { .. }))
        .nth(index)
        .expect("a button");
    let rect = button.rect();
    (
        Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0),
        scene.viewport().scale(),
    )
}

/// 处理排队消息直到条件成立；超出时限说明唤醒链路没有把状态送达。
fn pump_until(host: &mut Host, mut reached: impl FnMut(&Host) -> bool) {
    for _ in 0..500 {
        if reached(host) {
            return;
        }
        assert!(
            !host.pump().expect("process queued messages"),
            "no close was posted"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(reached(host), "the expected state did not arrive");
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
fn resizing_relayouts_and_reads_back_a_matching_frame() {
    let mut host = Host::open(HostConfig::hidden("ZenUI resize smoke")).expect("open a host");
    host.render().expect("render the initial frame");
    let before = host.scene().expect("scene").viewport().width();

    host.resize(640, 480).expect("resize the host");
    let scene = host.scene().expect("scene after resize");
    let scale = scene.viewport().scale();
    assert!(
        (scene.viewport().width() - 640.0).abs() < 2.0,
        "the logical viewport follows the requested size"
    );
    assert_ne!(scene.viewport().width(), before);

    let frame = host.read_frame().expect("frame after resize");
    let expected_width = (640.0 * scale).round() as u32;
    assert!(
        frame.width().abs_diff(expected_width) <= 1,
        "frame width {} matches the physical size {expected_width}",
        frame.width()
    );
    assert_eq!(
        frame.pixel(4, 4),
        Some([255, 255, 255, 255]),
        "the resized frame paints the root surface"
    );

    post_close(&host);
    host.run().expect("process the close");
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
    // 种子输入把 caret 留在文本末尾；点击靠左处应把它移回文本内部。
    let initial_caret = host.scene().expect("scene").caret(id).expect("caret").x;
    post_click(&host, point_in_text(&host, id, 0.2), scale);
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
    let placed_caret = host.scene().expect("scene").caret(id).expect("caret").x;
    assert!(
        placed_caret < initial_caret - 10.0,
        "the click moves the caret into the text"
    );
    assert_ne!(
        before.pixels(),
        focused.pixels(),
        "the focused caret changes the frame"
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
    post_click(&host, point_in_text(&host, id, 0.8), scale);
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

#[test]
fn typing_and_enter_add_a_todo() {
    let mut host = Host::open(HostConfig::hidden("ZenUI keyboard add")).expect("open a host");
    host.render().expect("render the initial frame");

    let (draft, point, scale) = center_of(&host, |element| {
        matches!(element.visual(), Visual::TextInput) && element.text().is_empty()
    });
    post_click(&host, point, scale);
    host.pump().expect("focus the draft input");
    let before = host.read_frame().expect("focused frame");

    for character in "Read".chars() {
        post_char(&host, character);
    }
    host.pump().expect("process the typed characters");
    assert_eq!(glyphs(&host, draft), 4, "typing inserts the characters");

    let typed = host.read_frame().expect("frame after typing");
    assert_ne!(
        before.pixels(),
        typed.pixels(),
        "typed text changes the frame"
    );

    post_key(&host, VK_RETURN);
    host.pump().expect("submit the draft");
    assert_eq!(checkbox_ids(&host).len(), 3, "Enter adds the typed Todo");
    assert_eq!(glyphs(&host, draft), 0, "submitting clears the draft");

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn backspace_delete_and_navigation_edit_the_focused_input() {
    let mut host = Host::open(HostConfig::hidden("ZenUI keyboard edit")).expect("open a host");
    host.render().expect("render the initial frame");

    let (editor, _, scale) = center_of(&host, |element| {
        matches!(element.visual(), Visual::TextInput) && !element.text().is_empty()
    });
    post_click(&host, point_in_text(&host, editor, 0.2), scale);
    host.pump().expect("focus the editor");
    let length = glyphs(&host, editor);
    assert!(length >= 3, "the seeded editor has text");

    post_key(&host, VK_END);
    host.pump().expect("move to the end");
    post_key(&host, VK_BACK);
    host.pump().expect("backspace");
    assert_eq!(glyphs(&host, editor), length - 1, "backspace deletes");

    post_key(&host, VK_DELETE);
    host.pump().expect("delete at the end");
    assert_eq!(
        glyphs(&host, editor),
        length - 1,
        "delete at the end changes nothing"
    );

    post_key(&host, VK_HOME);
    host.pump().expect("move to the start");
    let home = host.scene().expect("scene").caret(editor).expect("caret").x;
    post_key(&host, VK_RIGHT);
    host.pump().expect("step right");
    let stepped = host.scene().expect("scene").caret(editor).expect("caret").x;
    assert!(stepped > home, "right arrow moves the caret");
    assert_eq!(
        glyphs(&host, editor),
        length - 1,
        "navigation keeps the text"
    );

    post_key(&host, VK_DELETE);
    host.pump().expect("delete at the caret");
    assert_eq!(glyphs(&host, editor), length - 2, "delete removes a glyph");

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn tab_moves_focus_to_the_next_control() {
    let mut host = Host::open(HostConfig::hidden("ZenUI keyboard tab")).expect("open a host");
    host.render().expect("render the initial frame");
    let (editor, _, scale) = center_of(&host, |element| {
        matches!(element.visual(), Visual::TextInput) && !element.text().is_empty()
    });
    post_click(&host, point_in_text(&host, editor, 0.2), scale);
    host.pump().expect("focus the editor");
    assert_eq!(focused_id(&host), Some(editor));

    post_key(&host, VK_TAB);
    host.pump().expect("tab forward");
    let second = focused_id(&host).expect("a focused control");
    assert_ne!(second, editor, "Tab moves focus");

    post_key(&host, VK_TAB);
    host.pump().expect("tab forward again");
    assert_ne!(
        focused_id(&host),
        Some(second),
        "the next Tab moves focus again"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn reverse_reorders_the_native_rows() {
    let mut host = Host::open(HostConfig::hidden("ZenUI native reorder")).expect("open a host");
    host.render().expect("render the initial frame");
    let original = checkbox_ids(&host);
    assert_eq!(original.len(), 2);

    let (reverse, scale) = button_at(&host, 4);
    post_click(&host, reverse, scale);
    host.pump().expect("reverse the list");
    let mut expected = original;
    expected.reverse();
    assert_eq!(
        checkbox_ids(&host),
        expected,
        "Reverse reorders the native rows"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn reorder_keeps_focus_and_caret_on_the_same_row() {
    let mut host = Host::open(HostConfig::hidden("ZenUI native focus row")).expect("open a host");
    host.render().expect("render the initial frame");
    let (editor, _, scale) = center_of(&host, |element| {
        matches!(element.visual(), Visual::TextInput) && !element.text().is_empty()
    });
    post_click(&host, point_in_text(&host, editor, 0.2), scale);
    host.pump().expect("focus the editor");
    let caret = host.scene().expect("scene").caret(editor).expect("caret").x;

    let (reverse, scale) = button_at(&host, 4);
    post_click(&host, reverse, scale);
    host.pump().expect("reverse the list");

    assert_eq!(
        focused_id(&host),
        Some(editor),
        "the focused row keeps its Node ID"
    );
    assert_eq!(
        host.scene().expect("scene").caret(editor).expect("caret").x,
        caret,
        "the caret stays in the same logical row"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn a_delivered_task_wakes_the_host_and_repaints() {
    let mut host = Host::open(HostConfig::hidden("ZenUI task wake")).expect("open a host");
    host.render().expect("render the initial frame");
    let before = host.read_frame().expect("baseline frame");
    assert_eq!(checkbox_ids(&host).len(), 2);

    let (load, scale) = button_at(&host, 6);
    post_click(&host, load, scale);
    // 不轮询任务状态：只有 worker 投递的唤醒消息会触发交付与重绘。
    pump_until(&mut host, |host| checkbox_ids(host).len() == 4);

    let after = host.read_frame().expect("frame after delivery");
    assert_ne!(
        before.pixels(),
        after.pixels(),
        "the delivered rows repaint the native window"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn closing_while_a_task_runs_keeps_the_last_scene_and_reopens_fresh() {
    let mut host = Host::open(HostConfig::hidden("ZenUI task close")).expect("open a host");
    host.render().expect("render the initial frame");
    let (preview, scale) = button_at(&host, 7);
    post_click(&host, preview, scale);
    host.pump().expect("start the preview task");
    post_close(&host);
    host.run().expect("process the close");
    // run() 可能在处理 WM_CLOSE 之前交付任务；关闭后的场景才是最终基线。
    let elements = host.scene().expect("last scene").elements().len();
    // 关闭之后到达的唤醒只丢弃结果，不能重绘。
    host.pump()
        .expect("process any late wake without repainting");
    assert_eq!(
        host.scene()
            .expect("scene after the late wake")
            .elements()
            .len(),
        elements,
        "the closed window keeps its last scene"
    );
    drop(host);

    let mut reopened = Host::open(HostConfig::hidden("ZenUI task reopen")).expect("reopen");
    reopened.render().expect("render the reopened window");
    assert_eq!(
        checkbox_ids(&reopened).len(),
        2,
        "reopening mounts a fresh seeded activation"
    );
}

#[test]
fn delete_removes_a_native_row() {
    let mut host = Host::open(HostConfig::hidden("ZenUI native delete")).expect("open a host");
    host.render().expect("render the initial frame");
    assert_eq!(checkbox_ids(&host).len(), 2);

    let (delete, scale) = button_at(&host, 8);
    post_click(&host, delete, scale);
    host.pump().expect("delete the first row");
    assert_eq!(
        checkbox_ids(&host).len(),
        1,
        "Delete removes the row from the native list"
    );

    post_close(&host);
    host.run().expect("process the close");
}
