#![cfg(windows)]
#![allow(unused_braces)]

use std::collections::BTreeSet;
use std::ffi::c_void;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    IsWindowEnabled, VIRTUAL_KEY, VK_BACK, VK_DELETE, VK_END, VK_ESCAPE, VK_HOME, VK_NEXT,
    VK_PRIOR, VK_RETURN, VK_RIGHT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, SPI_GETWHEELSCROLLLINES, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SystemParametersInfoW, WM_CHAR, WM_CLOSE, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
};
use zenui::prelude::*;
use zenui::{Appearance, Color, ColorRole, NodeId};
use zenui_render::{Point, SceneElement, Visual};
use zenui_windows::accessibility::ControlType;
use zenui_windows::{Host, HostConfig, HostError};

/// Todo demo 的 primary 目录键。
const MAIN: &str = "main";

fn open_host() -> Host {
    open_host_with(HostConfig::hidden())
}

fn open_host_with(config: HostConfig) -> Host {
    let mut runtime = zenui::Runtime::new();
    zenui_demo::open_todo_window(&mut runtime).expect("open the todo window");
    zenui_demo::seed(&mut runtime).expect("seed the todo rows");
    Host::open(runtime, config).expect("open a host")
}

fn open_tall_host(rows: usize) -> Host {
    let mut runtime = zenui::Runtime::new();
    zenui_demo::open_todo_window(&mut runtime).expect("open the todo window");
    for index in 0..rows {
        runtime
            .input("What needs to be done?", &format!("Task {index}"))
            .expect("fill the draft");
        runtime.submit("Add task").expect("add a row");
    }
    Host::open(runtime, HostConfig::hidden()).expect("open a tall host")
}

#[component]
fn WindowTaskProbe(window: WindowCtx) -> MountResult<MountPlan> {
    let start = move || {
        let _ = window.task_scope().spawn(|_| String::from("late"));
    };
    view! {
        <view role="main">
            <button aria:label="Start task" on:click={move || start()}>
                "Start task"
            </button>
        </view>
    }
}

fn open_task_host() -> Host {
    let mut runtime = zenui::Runtime::new();
    runtime
        .declare_windows(|catalog| {
            catalog.window(
                zenui::WindowSpec::new(MAIN).primary().sized(320, 240),
                |window| view! { <WindowTaskProbe window={window} /> },
            )?;
            Ok(())
        })
        .expect("declare the task probe window");
    runtime.show(MAIN).expect("show the task probe window");
    Host::open(runtime, HostConfig::hidden()).expect("open the task probe host")
}

fn open_empty_host() -> Host {
    let mut runtime = zenui::Runtime::new();
    runtime
        .declare_windows(|catalog| {
            catalog.window(
                zenui::WindowSpec::new("main").primary().sized(0, 0),
                |_| zenui::view! { <text>"empty"</text> },
            )?;
            Ok(())
        })
        .expect("declare the empty window");
    Host::open(runtime, HostConfig::hidden()).expect("open a zero-sized host")
}

fn pointer_lparam(point: Point, scale: f32) -> LPARAM {
    let x = (point.x * scale).round() as i16;
    let y = (point.y * scale).round() as i16;
    LPARAM((x as isize & 0xffff) | ((y as isize & 0xffff) << 16))
}

fn post_pointer(host: &Host, message: u32, point: Point, scale: f32) {
    let hwnd = host.hwnd(MAIN).expect("a live window");
    // 安全：HWND 由 Host 持有，消息只排队，不跨线程解引用。
    unsafe {
        PostMessageW(Some(hwnd), message, WPARAM(0), pointer_lparam(point, scale))
            .expect("post the pointer message");
    }
}

fn post_down(host: &Host, point: Point, scale: f32) {
    post_pointer(host, WM_LBUTTONDOWN, point, scale);
}

fn post_up(host: &Host, point: Point, scale: f32) {
    post_pointer(host, WM_LBUTTONUP, point, scale);
}

fn post_move(host: &Host, point: Point, scale: f32) {
    post_pointer(host, WM_MOUSEMOVE, point, scale);
}

fn post_focus_lost(host: &Host) {
    let hwnd = host.hwnd(MAIN).expect("a live window");
    // 安全：HWND 由 Host 持有，消息只排队，不跨线程解引用。
    unsafe {
        PostMessageW(Some(hwnd), WM_KILLFOCUS, WPARAM(0), LPARAM(0)).expect("post focus loss");
    }
}

fn post_leave(host: &Host) {
    let hwnd = host.hwnd(MAIN).expect("a live window");
    // 安全：HWND 由 Host 持有，消息只排队，不跨线程解引用。
    unsafe {
        PostMessageW(Some(hwnd), WM_MOUSELEAVE, WPARAM(0), LPARAM(0)).expect("post the leave");
    }
}

fn post_click(host: &Host, point: Point, scale: f32) {
    post_down(host, point, scale);
    post_up(host, point, scale);
}

fn post_char(host: &Host, character: char) {
    let hwnd = host.hwnd(MAIN).expect("a live window");
    // 安全：HWND 由 Host 持有，消息只排队，不跨线程解引用。
    unsafe {
        PostMessageW(Some(hwnd), WM_CHAR, WPARAM(character as usize), LPARAM(0))
            .expect("post the character");
    }
}

fn post_key(host: &Host, key: VIRTUAL_KEY) {
    let hwnd = host.hwnd(MAIN).expect("a live window");
    // 安全：HWND 由 Host 持有，消息只排队，不跨线程解引用。
    unsafe {
        PostMessageW(Some(hwnd), WM_KEYDOWN, WPARAM(key.0 as usize), LPARAM(0))
            .expect("post the key");
    }
}

fn post_wheel(host: &Host, delta: i16) {
    let hwnd = host.hwnd(MAIN).expect("a live window");
    let wparam = WPARAM(((delta as u16 as u32) << 16) as usize);
    // 安全：HWND 由 Host 持有，消息只排队，不跨线程解引用。
    unsafe {
        PostMessageW(Some(hwnd), WM_MOUSEWHEEL, wparam, LPARAM(0)).expect("post the wheel");
    }
}

fn system_wheel_lines() -> u32 {
    let mut lines = 3u32;
    // 安全：lines 是本地输出缓冲；读取设置不改变系统状态。
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWHEELSCROLLLINES,
            0,
            Some((&raw mut lines).cast::<c_void>()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    lines
}

fn glyphs(host: &Host, id: NodeId) -> usize {
    host.scene(MAIN)
        .expect("a rendered scene")
        .node(id)
        .expect("element")
        .text()
        .iter()
        .map(|run| run.glyphs().len())
        .sum()
}

fn focused_id(host: &Host) -> Option<NodeId> {
    host.scene(MAIN)
        .expect("a rendered scene")
        .elements()
        .iter()
        .find(|element| element.focused())
        .map(SceneElement::id)
}

fn post_close(host: &Host) {
    post_close_in(host, MAIN);
}

fn post_close_in(host: &Host, key: &str) {
    let hwnd = host.hwnd(key).expect("a live window");
    // 安全：HWND 由 Host 持有，消息只排队，不跨线程解引用。
    unsafe {
        PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).expect("post the close");
    }
}

fn first_text_glyph(host: &Host) -> u32 {
    host.scene(MAIN)
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
    let scene = host.scene(MAIN).expect("a rendered scene");
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
    host.scene(MAIN)
        .expect("a rendered scene")
        .elements()
        .iter()
        .filter(|element| matches!(element.visual(), Visual::Checkbox { .. }))
        .map(SceneElement::id)
        .collect()
}

fn button_at(host: &Host, index: usize) -> (Point, f32) {
    let scene = host.scene(MAIN).expect("a rendered scene");
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
    let scene = host.scene(MAIN).expect("a rendered scene");
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
    let mut host = open_host();

    host.render().expect("render and present a surface frame");
    let frame = host.read_frame(MAIN).expect("render and read back a frame");

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
    let mut host = open_host();
    host.render().expect("render the initial frame");
    let before = host.scene(MAIN).expect("scene").viewport().width();

    host.resize(MAIN, 640, 480).expect("resize the host");
    let scene = host.scene(MAIN).expect("scene after resize");
    let scale = scene.viewport().scale();
    assert!(
        (scene.viewport().width() - 640.0).abs() < 2.0,
        "the logical viewport follows the requested size"
    );
    assert_ne!(scene.viewport().width(), before);

    let frame = host.read_frame(MAIN).expect("frame after resize");
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
    let mut host = open_host();
    host.request_close(MAIN).expect("post WM_CLOSE");
    host.run().expect("the message loop exits on close");
    drop(host);

    let mut reopened = open_host();
    reopened.render().expect("render a fresh activation");
}

#[test]
fn an_empty_client_area_renders_nothing_and_refuses_readback() {
    let mut host = open_empty_host();

    host.render()
        .expect("rendering an empty client area is a no-op");
    assert!(
        matches!(host.read_frame(MAIN), Err(HostError::Readback(_))),
        "readback of an empty client area reports a readback error"
    );
}

#[test]
fn clicking_a_text_input_focuses_it_and_changes_the_frame() {
    let mut host = open_host();
    host.render().expect("render the initial frame");
    let before = host.read_frame(MAIN).expect("baseline frame");

    let (id, _, scale) = center_of(&host, |element| {
        matches!(element.visual(), Visual::TextInput) && !element.text().is_empty()
    });
    // 先点击靠右处聚焦，让 caret 落在文本末尾。
    post_click(&host, point_in_text(&host, id, 0.8), scale);
    assert!(
        !host.pump().expect("focus the text input"),
        "no close was posted"
    );
    let initial_caret = host.scene(MAIN).expect("scene").caret(id).expect("caret").x;

    // 再点击靠左处应把它移回文本内部。
    post_click(&host, point_in_text(&host, id, 0.2), scale);
    assert!(
        !host.pump().expect("process the click"),
        "no close was posted"
    );

    let focused = host.read_frame(MAIN).expect("frame after the click");
    assert!(
        host.scene(MAIN)
            .expect("scene after the click")
            .node(id)
            .expect("text input")
            .focused(),
        "the click focuses the text input"
    );
    let placed_caret = host.scene(MAIN).expect("scene").caret(id).expect("caret").x;
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
    let mut host = open_host();
    host.render().expect("render the initial frame");

    let (id, _, scale) = center_of(&host, |element| {
        matches!(element.visual(), Visual::TextInput) && !element.text().is_empty()
    });
    post_click(&host, point_in_text(&host, id, 0.8), scale);
    host.pump().expect("process the focus click");
    assert!(
        host.scene(MAIN)
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
            .scene(MAIN)
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
    let mut host = open_host();
    host.render().expect("render the initial frame");

    let (id, point, scale) = center_of(&host, |element| {
        matches!(element.visual(), Visual::Checkbox { .. })
    });
    assert!(
        matches!(
            host.scene(MAIN)
                .expect("scene")
                .node(id)
                .expect("checkbox")
                .visual(),
            Visual::Checkbox { checked: true }
        ),
        "the first item starts completed"
    );

    let before = host.read_frame(MAIN).expect("baseline frame");
    let counter = first_text_glyph(&host);
    post_click(&host, point, scale);
    assert!(
        !host.pump().expect("process the click"),
        "no close was posted"
    );

    let after = host.read_frame(MAIN).expect("frame after the click");
    assert_ne!(
        before.pixels(),
        after.pixels(),
        "the toggled checkbox changes the frame"
    );
    assert!(
        matches!(
            host.scene(MAIN)
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
    let mut host = open_host();
    host.render().expect("render the initial frame");

    let (_, active, scale) = {
        let scene = host.scene(MAIN).expect("scene");
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
        host.scene(MAIN)
            .expect("scene")
            .elements()
            .iter()
            .filter(|element| matches!(element.visual(), Visual::Checkbox { .. }))
            .count(),
        2,
        "the unfiltered list shows both items"
    );

    post_click(&host, active, scale);
    assert!(
        !host.pump().expect("process the click"),
        "the window stays live"
    );
    assert_eq!(
        host.scene(MAIN)
            .expect("scene after click")
            .elements()
            .iter()
            .filter(|element| matches!(element.visual(), Visual::Checkbox { .. }))
            .count(),
        1,
        "the Active filter hides the completed item"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn typing_and_enter_add_a_todo() {
    let mut host = open_host();
    host.render().expect("render the initial frame");

    let (draft, point, scale) = center_of(&host, |element| {
        matches!(element.visual(), Visual::TextInput) && element.text().is_empty()
    });
    post_click(&host, point, scale);
    host.pump().expect("focus the draft input");
    let before = host.read_frame(MAIN).expect("focused frame");

    for character in "Read".chars() {
        post_char(&host, character);
    }
    host.pump().expect("process the typed characters");
    assert_eq!(glyphs(&host, draft), 4, "typing inserts the characters");

    let typed = host.read_frame(MAIN).expect("frame after typing");
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
    let mut host = open_host();
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
    let home = host
        .scene(MAIN)
        .expect("scene")
        .caret(editor)
        .expect("caret")
        .x;
    post_key(&host, VK_RIGHT);
    host.pump().expect("step right");
    let stepped = host
        .scene(MAIN)
        .expect("scene")
        .caret(editor)
        .expect("caret")
        .x;
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
    let mut host = open_host();
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
    let mut host = open_host();
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
    let mut host = open_host();
    host.render().expect("render the initial frame");
    let (editor, _, scale) = center_of(&host, |element| {
        matches!(element.visual(), Visual::TextInput) && !element.text().is_empty()
    });
    post_click(&host, point_in_text(&host, editor, 0.2), scale);
    host.pump().expect("focus the editor");
    let caret = host
        .scene(MAIN)
        .expect("scene")
        .caret(editor)
        .expect("caret")
        .x;

    let (reverse, scale) = button_at(&host, 4);
    post_click(&host, reverse, scale);
    host.pump().expect("reverse the list");

    assert_eq!(
        focused_id(&host),
        Some(editor),
        "the focused row keeps its Node ID"
    );
    assert_eq!(
        host.scene(MAIN)
            .expect("scene")
            .caret(editor)
            .expect("caret")
            .x,
        caret,
        "the caret stays in the same logical row"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn a_delivered_task_wakes_the_host_and_repaints() {
    let mut host = open_host();
    host.render().expect("render the initial frame");
    let before = host.read_frame(MAIN).expect("baseline frame");
    assert_eq!(checkbox_ids(&host).len(), 2);

    let (load, scale) = button_at(&host, 6);
    post_click(&host, load, scale);
    // 不轮询任务状态：只有 worker 投递的唤醒消息会触发交付与重绘。
    pump_until(&mut host, |host| checkbox_ids(host).len() == 4);

    let after = host.read_frame(MAIN).expect("frame after delivery");
    assert_ne!(
        before.pixels(),
        after.pixels(),
        "the delivered rows repaint the native window"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn closing_while_a_task_runs_discards_the_late_wake_and_reopens_fresh() {
    let mut host = open_task_host();
    host.render().expect("render the initial frame");
    let (_, start, scale) = center_label(&host, MAIN, "Start task");
    post_click(&host, start, scale);
    host.pump().expect("start the window-owned task");
    post_close(&host);
    host.run().expect("process the close");
    // 关闭意味着 activation 结束与 HWND 销毁；迟到结果只能被丢弃。
    assert!(
        host.hwnd(MAIN).is_none(),
        "closing destroys the native window"
    );
    host.pump()
        .expect("process any late wake without repainting");
    assert!(
        host.scene(MAIN).is_none(),
        "a closed window has no scene to repaint"
    );
    drop(host);

    let mut reopened = open_task_host();
    reopened.render().expect("render the reopened window");
    assert!(
        reopened
            .presentation(MAIN)
            .expect("a window snapshot")
            .find_label("Start task")
            .is_some(),
        "reopening mounts a fresh activation"
    );
}

#[test]
fn delete_removes_a_native_row() {
    let mut host = open_host();
    host.render().expect("render the initial frame");
    assert_eq!(checkbox_ids(&host).len(), 2);

    let (_, delete, scale) = center_label(&host, MAIN, "Delete Write tests");
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

#[test]
fn the_details_action_opens_the_shared_second_window() {
    let mut host = open_host();
    host.render().expect("render the primary window");
    let (_, point, scale) = center_label(&host, MAIN, "Open details");
    post_click(&host, point, scale);
    assert!(!host.pump().expect("open the details window"));
    assert_ne!(
        host.hwnd(zenui_demo::DETAILS_WINDOW)
            .expect("a details HWND"),
        host.hwnd(MAIN).expect("the primary HWND")
    );

    let details = host
        .presentation(zenui_demo::DETAILS_WINDOW)
        .expect("the details snapshot");
    assert!(
        details
            .elements()
            .any(|element| element.text() == "1 remaining"),
        "details follows the shared remaining count"
    );
    assert!(
        details
            .elements()
            .any(|element| element.text() == "Read docs"),
        "details shows the shared rows"
    );

    // 详情窗口有自己的焦点和 caret；输入不会落到主窗口的草稿。
    let (_, note_point, note_scale) =
        center_label(&host, zenui_demo::DETAILS_WINDOW, "Details note");
    post_click_in(&host, zenui_demo::DETAILS_WINDOW, note_point, note_scale);
    post_char_in(&host, zenui_demo::DETAILS_WINDOW, 'N');
    assert!(!host.pump().expect("edit the details note"));
    assert_eq!(
        host.presentation(zenui_demo::DETAILS_WINDOW)
            .expect("the details snapshot")
            .find_label("Details note")
            .expect("the note input")
            .text_value(),
        Some("N")
    );
    assert_eq!(
        host.presentation(MAIN)
            .expect("the primary snapshot")
            .find_label("What needs to be done?")
            .expect("the main draft")
            .text_value(),
        Some(""),
        "typing in details leaves the main draft alone"
    );

    // 关闭详情只销毁该窗口；主窗口与消息循环继续。
    let (_, close_point, close_scale) =
        center_label(&host, zenui_demo::DETAILS_WINDOW, "Close details");
    post_click_in(&host, zenui_demo::DETAILS_WINDOW, close_point, close_scale);
    assert!(!host.pump().expect("close the details window"));
    assert!(host.hwnd(zenui_demo::DETAILS_WINDOW).is_none());
    assert!(host.scene(zenui_demo::DETAILS_WINDOW).is_none());
    assert!(host.hwnd(MAIN).is_some());

    post_close(&host);
    host.run().expect("the last window closes the loop");
}

#[test]
fn closing_with_a_draft_prompts_and_deny_restores_the_parent() {
    let mut host = open_host();
    host.render().expect("render the primary window");
    let draft = type_main_draft(&mut host, "draft");
    let caret = host
        .scene(MAIN)
        .expect("scene")
        .caret(draft)
        .expect("caret");
    let main_hwnd = host.hwnd(MAIN).expect("the main HWND");

    post_close(&host);
    assert!(!host.pump().expect("show the close prompt"));
    let prompt = host
        .hwnd(zenui_demo::CLOSE_CONFIRMATION_WINDOW)
        .expect("the confirmation HWND");
    assert_ne!(prompt, main_hwnd);
    assert!(
        !unsafe { IsWindowEnabled(main_hwnd) }.as_bool(),
        "the pending parent HWND is disabled"
    );

    // 待决期间的再次关闭只聚焦既有确认窗，不创建第二个 HWND。
    post_close(&host);
    assert!(!host.pump().expect("focus the pending prompt"));
    assert_eq!(
        host.hwnd(zenui_demo::CLOSE_CONFIRMATION_WINDOW),
        Some(prompt)
    );
    assert!(!unsafe { IsWindowEnabled(main_hwnd) }.as_bool());

    // 禁用期间即使消息直接投递到父 HWND，草稿也不接受输入。
    post_char_in(&host, MAIN, 'x');
    assert!(!host.pump().expect("ignore the suspended parent input"));
    assert_eq!(
        value_in(&host, MAIN, "What needs to be done?").as_deref(),
        Some("draft")
    );

    // Continue editing 通过按钮回复 Deny，并恢复父窗编辑状态。
    let (_, continue_editing, scale) = center_label(
        &host,
        zenui_demo::CLOSE_CONFIRMATION_WINDOW,
        "Continue editing",
    );
    post_click_in(
        &host,
        zenui_demo::CLOSE_CONFIRMATION_WINDOW,
        continue_editing,
        scale,
    );
    assert!(!host.pump().expect("deny via Continue editing"));
    assert!(host.hwnd(zenui_demo::CLOSE_CONFIRMATION_WINDOW).is_none());
    assert!(
        unsafe { IsWindowEnabled(main_hwnd) }.as_bool(),
        "Deny re-enables the parent HWND"
    );
    assert_eq!(
        focused_in(&host, MAIN),
        Some(draft),
        "the draft keeps focus"
    );
    assert_eq!(
        host.scene(MAIN)
            .expect("scene")
            .caret(draft)
            .expect("caret"),
        caret,
        "the caret is restored"
    );
    assert_eq!(
        value_in(&host, MAIN, "What needs to be done?").as_deref(),
        Some("draft")
    );

    // X 关闭提示窗同样等于 Deny。
    post_close(&host);
    assert!(!host.pump().expect("reopen the close prompt"));
    post_close_in(&host, zenui_demo::CLOSE_CONFIRMATION_WINDOW);
    assert!(!host.pump().expect("X denies the close"));
    assert!(host.hwnd(zenui_demo::CLOSE_CONFIRMATION_WINDOW).is_none());
    assert!(unsafe { IsWindowEnabled(main_hwnd) }.as_bool());
    assert_eq!(
        value_in(&host, MAIN, "What needs to be done?").as_deref(),
        Some("draft")
    );

    // Escape 在提示窗上同样等于 Deny。
    post_close(&host);
    assert!(!host.pump().expect("reopen the close prompt"));
    post_key_in(&host, zenui_demo::CLOSE_CONFIRMATION_WINDOW, VK_ESCAPE);
    assert!(!host.pump().expect("escape denies the close"));
    assert!(host.hwnd(zenui_demo::CLOSE_CONFIRMATION_WINDOW).is_none());
    assert!(unsafe { IsWindowEnabled(main_hwnd) }.as_bool());
    assert_eq!(
        value_in(&host, MAIN, "What needs to be done?").as_deref(),
        Some("draft")
    );
}

#[test]
fn discarding_the_draft_closes_the_group_and_ends_the_loop() {
    let mut host = open_host();
    host.render().expect("render the primary window");
    let _ = type_main_draft(&mut host, "discard me");

    post_close(&host);
    assert!(!host.pump().expect("show the close prompt"));
    let (_, discard, scale) = center_label(
        &host,
        zenui_demo::CLOSE_CONFIRMATION_WINDOW,
        "Discard draft",
    );
    post_click_in(&host, zenui_demo::CLOSE_CONFIRMATION_WINDOW, discard, scale);
    assert!(host.pump().expect("the last window closes the loop"));
    assert!(host.hwnd(MAIN).is_none());
    assert!(host.hwnd(zenui_demo::CLOSE_CONFIRMATION_WINDOW).is_none());
    host.run().expect("the message loop exits");
}

#[test]
fn an_empty_draft_closes_without_a_prompt() {
    let mut host = open_host();
    host.render().expect("render the primary window");

    post_close(&host);
    assert!(host.pump().expect("the empty draft closes synchronously"));
    assert!(host.hwnd(MAIN).is_none());
    assert!(host.hwnd(zenui_demo::CLOSE_CONFIRMATION_WINDOW).is_none());
    host.run().expect("the message loop exits");
}

#[test]
fn hover_press_and_release_activate_a_filter_button_once() {
    let mut host = open_host();
    host.render().expect("render the initial frame");
    let (point, scale) = button_at(&host, 2);
    let id = host
        .scene(MAIN)
        .expect("scene")
        .hit_test(point)
        .expect("the Active button");
    let baseline = host.read_frame(MAIN).expect("baseline frame");

    post_move(&host, point, scale);
    host.pump().expect("process the move");
    let scene = host.scene(MAIN).expect("scene after the move");
    let element = scene.node(id).expect("Active button");
    assert!(element.hovered(), "moving over the button shows hover");
    assert!(
        matches!(element.visual(), Visual::Button { pressed: false }),
        "hover does not change aria-pressed"
    );
    assert_ne!(
        baseline.pixels(),
        host.read_frame(MAIN).expect("hovered frame").pixels(),
        "hover changes the pixels"
    );

    post_down(&host, point, scale);
    host.pump().expect("process the press");
    let scene = host.scene(MAIN).expect("scene after the press");
    let element = scene.node(id).expect("Active button");
    assert!(
        element.pointer_pressed(),
        "holding the pointer shows pressed"
    );
    assert!(
        matches!(element.visual(), Visual::Button { pressed: false }),
        "a transient press stays distinct from aria-pressed"
    );

    post_up(&host, point, scale);
    host.pump().expect("process the release");
    let scene = host.scene(MAIN).expect("scene after the release");
    let element = scene.node(id).expect("Active button");
    assert!(!element.pointer_pressed(), "release clears pressed");
    assert!(
        matches!(element.visual(), Visual::Button { pressed: true }),
        "release over the same node activates once"
    );
    assert!(element.hovered(), "the pointer is still over the button");

    post_leave(&host);
    host.pump().expect("process the leave");
    let scene = host.scene(MAIN).expect("scene after the leave");
    assert!(
        !scene.node(id).expect("Active button").hovered(),
        "leaving the client area clears hover"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn focus_loss_clears_the_pressed_visual() {
    let mut host = open_host();
    host.render().expect("render the initial frame");
    let (point, scale) = button_at(&host, 2);
    let id = host
        .scene(MAIN)
        .expect("scene")
        .hit_test(point)
        .expect("the Active button");

    post_down(&host, point, scale);
    host.pump().expect("process the press");
    assert!(
        host.scene(MAIN)
            .expect("scene after the press")
            .node(id)
            .expect("Active button")
            .pointer_pressed(),
        "the button is the press target"
    );
    let pressed = host.read_frame(MAIN).expect("pressed frame");

    post_focus_lost(&host);
    host.pump().expect("process focus loss");
    assert!(
        !host
            .scene(MAIN)
            .expect("scene after focus loss")
            .node(id)
            .expect("Active button")
            .pointer_pressed(),
        "focus loss clears the press target"
    );
    assert_ne!(
        pressed.pixels(),
        host.read_frame(MAIN)
            .expect("frame after focus loss")
            .pixels(),
        "focus loss repaints without the pressed state"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn dragging_off_cancels_activation() {
    let mut host = open_host();
    host.render().expect("render the initial frame");
    let (id, point, scale) = center_of(&host, |element| {
        matches!(element.visual(), Visual::Checkbox { .. })
    });
    assert!(matches!(
        host.scene(MAIN)
            .expect("scene")
            .node(id)
            .expect("checkbox")
            .visual(),
        Visual::Checkbox { checked: true }
    ));

    post_down(&host, point, scale);
    host.pump().expect("process the press");
    assert!(
        host.scene(MAIN)
            .expect("scene")
            .node(id)
            .expect("checkbox")
            .pointer_pressed(),
        "the checkbox is the press target"
    );

    let off = Point::new(2.0, 2.0);
    post_move(&host, off, scale);
    host.pump().expect("process the move off");
    post_up(&host, off, scale);
    host.pump().expect("process the release off the target");
    assert!(
        matches!(
            host.scene(MAIN)
                .expect("scene")
                .node(id)
                .expect("checkbox")
                .visual(),
            Visual::Checkbox { checked: true }
        ),
        "dragging off and releasing does not activate"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn hover_does_not_change_focus_or_caret() {
    let mut host = open_host();
    host.render().expect("render the initial frame");
    let scene = host.scene(MAIN).expect("scene");
    let input = scene
        .elements()
        .iter()
        .find(|element| matches!(element.visual(), Visual::TextInput))
        .expect("a text input");
    let id = input.id();
    let point = point_in_text(&host, id, 0.4);
    let scale = scene.viewport().scale();
    post_click(&host, point, scale);
    host.pump().expect("focus the input");
    let focused = focused_id(&host).expect("a focused input");
    let caret = host
        .scene(MAIN)
        .expect("scene")
        .caret(focused)
        .expect("caret");

    let (button, scale) = button_at(&host, 1);
    post_move(&host, button, scale);
    host.pump().expect("hover a button");
    let scene = host.scene(MAIN).expect("scene after hover");
    assert_eq!(focused_id(&host), Some(focused), "hover keeps focus");
    assert_eq!(
        scene.caret(focused).expect("caret after hover"),
        caret,
        "hover keeps caret geometry"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn a_relayout_keeps_hover_consistent_with_the_hit_test() {
    let mut host = open_host();
    host.render().expect("render the initial frame");
    let (point, scale) = button_at(&host, 7);
    post_move(&host, point, scale);
    host.pump().expect("hover the button");
    assert!(
        host.scene(MAIN)
            .expect("scene")
            .elements()
            .iter()
            .any(SceneElement::hovered)
    );

    host.resize(MAIN, 320, 100).expect("resize the window");
    let scene = host.scene(MAIN).expect("scene after resize");
    let under = scene
        .hit_test(point)
        .filter(|node| scene.node(*node).is_some_and(|element| !element.disabled()));
    let hovered = scene
        .elements()
        .iter()
        .find(|element| element.hovered())
        .map(SceneElement::id);
    assert_eq!(
        hovered, under,
        "after a relayout the hover agrees with the new hit test"
    );
    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn focus_visible_follows_keyboard_not_pointer_modality() {
    let mut host = open_host();
    host.render().expect("render the initial frame");
    let scene = host.scene(MAIN).expect("scene");
    let input = scene
        .elements()
        .iter()
        .find(|element| matches!(element.visual(), Visual::TextInput))
        .expect("a text input");
    let id = input.id();
    let point = point_in_text(&host, id, 0.5);
    let scale = scene.viewport().scale();
    post_click(&host, point, scale);
    host.pump().expect("focus the input with the pointer");
    assert_eq!(
        host.scene(MAIN)
            .expect("scene")
            .node(id)
            .expect("input")
            .resolved_style()
            .border_color(),
        None,
        "pointer focus is not focus-visible"
    );

    post_key(&host, VK_RIGHT);
    host.pump().expect("use the keyboard");
    assert_eq!(
        host.scene(MAIN)
            .expect("scene")
            .node(id)
            .expect("input")
            .resolved_style()
            .border_color(),
        Some(Color::Role(ColorRole::BorderAccent)),
        "keyboard focus selects focus-visible"
    );

    post_down(&host, point, scale);
    host.pump().expect("press with the pointer");
    assert_eq!(
        host.scene(MAIN)
            .expect("scene")
            .node(id)
            .expect("input")
            .resolved_style()
            .border_color(),
        None,
        "pointer input clears focus-visible"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn a_configured_dark_appearance_paints_and_the_toggle_preserves_edit_state() {
    let mut host = open_host_with(HostConfig::hidden().with_appearance(Appearance::Dark));
    host.render().expect("render the dark frame");
    let dark = host.read_frame(MAIN).expect("dark frame");
    assert!(
        dark.pixel(4, 4).expect("dark pixel")[..3]
            .iter()
            .all(|channel| *channel < 60),
        "the configured dark appearance paints a dark surface: {:?}",
        dark.pixel(4, 4).expect("dark pixel")
    );

    let scene = host.scene(MAIN).expect("scene");
    let input = scene
        .elements()
        .iter()
        .find(|element| matches!(element.visual(), Visual::TextInput))
        .expect("a text input");
    let id = input.id();
    let point = point_in_text(&host, id, 0.6);
    let scale = scene.viewport().scale();
    post_click(&host, point, scale);
    host.pump().expect("focus the input");
    let focused = focused_id(&host).expect("focused input");
    let caret = host
        .scene(MAIN)
        .expect("scene")
        .caret(focused)
        .expect("caret");

    let (toggle, scale) = button_at(&host, 8);
    post_click(&host, toggle, scale);
    host.pump().expect("toggle the theme");
    let light = host.read_frame(MAIN).expect("light frame");
    assert!(
        light.pixel(4, 4).expect("light pixel")[..3]
            .iter()
            .all(|channel| *channel > 200),
        "the toggle switches the palette back to light: {:?}",
        light.pixel(4, 4).expect("light pixel")
    );
    assert_eq!(
        focused_id(&host),
        Some(focused),
        "focus survives the switch"
    );
    assert_eq!(
        host.scene(MAIN)
            .expect("scene")
            .caret(focused)
            .expect("caret"),
        caret,
        "caret geometry survives the switch"
    );

    post_close(&host);
    host.run().expect("process the close");
}

fn open_shared_host() -> Host {
    let mut runtime = zenui::Runtime::new();
    let shared = runtime
        .setup(|| zenui::signal(String::from("start")))
        .expect("application setup");
    runtime
        .declare_windows(move |catalog| {
            catalog.window(
                zenui::WindowSpec::new("main")
                    .primary()
                    .titled("Main shared")
                    .sized(640, 480),
                move |window| {
                    let draft = zenui::signal(String::from("main-draft"));
                    zenui::view! {
                        <view>
                            <text-input
                                class="focus-visible:border-accent"
                                aria:label="main-input"
                                value={draft}
                                on:input={move |value: String| draft.set(value)}
                            />
                            <text aria:label="main-shared">{shared}</text>
                            <button
                                aria:label="main-bump"
                                on:click={move || shared.set(format!("{}!", shared.get()))}
                            >"Bump"</button>
                            <button
                                aria:label="main-theme"
                                on:click={move || {
                                    let next = if window.appearance().is_ok_and(|value| value == Appearance::Dark) {
                                        Appearance::Light
                                    } else {
                                        Appearance::Dark
                                    };
                                    let _ = window.set_appearance(next);
                                }}
                            >"Theme"</button>
                        </view>
                    }
                },
            )?;
            catalog.window(
                zenui::WindowSpec::new("second")
                    .titled("Second shared")
                    .sized(480, 360),
                move |_window| {
                    let draft = zenui::signal(String::from("second-draft"));
                    zenui::view! {
                        <view>
                            <text-input
                                class="focus-visible:border-accent"
                                aria:label="second-input"
                                value={draft}
                                on:input={move |value: String| draft.set(value)}
                            />
                            <text aria:label="second-shared">{shared}</text>
                        </view>
                    }
                },
            )?;
            Ok(())
        })
        .expect("declare two windows");
    let mut host = Host::open(runtime, HostConfig::hidden()).expect("open the host");
    host.show("second").expect("show the secondary window");
    host.render().expect("render both windows");
    host
}

fn post_pointer_in(host: &Host, key: &str, message: u32, point: Point, scale: f32) {
    let hwnd = host.hwnd(key).expect("a live window");
    // 安全：HWND 由 Host 持有，消息只排队，不跨线程解引用。
    unsafe {
        PostMessageW(Some(hwnd), message, WPARAM(0), pointer_lparam(point, scale))
            .expect("post the pointer message");
    }
}

fn post_click_in(host: &Host, key: &str, point: Point, scale: f32) {
    post_pointer_in(host, key, WM_LBUTTONDOWN, point, scale);
    post_pointer_in(host, key, WM_LBUTTONUP, point, scale);
}

fn post_char_in(host: &Host, key: &str, character: char) {
    let hwnd = host.hwnd(key).expect("a live window");
    // 安全：HWND 由 Host 持有，消息只排队，不跨线程解引用。
    unsafe {
        PostMessageW(Some(hwnd), WM_CHAR, WPARAM(character as usize), LPARAM(0))
            .expect("post the character");
    }
}

fn post_key_in(host: &Host, key: &str, key_code: VIRTUAL_KEY) {
    let hwnd = host.hwnd(key).expect("a live window");
    // 安全：HWND 由 Host 持有，消息只排队，不跨线程解引用。
    unsafe {
        PostMessageW(
            Some(hwnd),
            WM_KEYDOWN,
            WPARAM(key_code.0 as usize),
            LPARAM(0),
        )
        .expect("post the key");
    }
}

fn center_in(
    host: &Host,
    key: &str,
    predicate: impl Fn(&SceneElement) -> bool,
) -> (NodeId, Point, f32) {
    let scene = host.scene(key).expect("a rendered scene");
    let element = scene
        .elements()
        .iter()
        .find(|element| predicate(element))
        .expect("a matching scene element");
    let rect = element.rect();
    (
        element.id(),
        Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0),
        scene.viewport().scale(),
    )
}

/// 按 `aria:label` 定位窗口内元素并取盒中心；标签变化不会移动其他按钮的序号。
fn center_label(host: &Host, key: &str, label: &str) -> (NodeId, Point, f32) {
    let id = host
        .presentation(key)
        .expect("a window snapshot")
        .find_label(label)
        .expect("a labelled element")
        .id();
    center_in(host, key, |element| element.id() == id)
}

fn input_glyphs(host: &Host, key: &str) -> usize {
    let scene = host.scene(key).expect("a rendered scene");
    scene
        .elements()
        .iter()
        .find(|element| matches!(element.visual(), Visual::TextInput))
        .map(|element| element.text().iter().map(|run| run.glyphs().len()).sum())
        .expect("a text input")
}

fn shared_glyphs(host: &Host, key: &str) -> usize {
    let scene = host.scene(key).expect("a rendered scene");
    scene
        .elements()
        .iter()
        .filter(|element| matches!(element.visual(), Visual::Text))
        .map(|element| element.text().iter().map(|run| run.glyphs().len()).sum())
        .max()
        .expect("a text element")
}

fn focused_in(host: &Host, key: &str) -> Option<NodeId> {
    host.scene(key)
        .expect("a rendered scene")
        .elements()
        .iter()
        .find(|element| element.focused())
        .map(SceneElement::id)
}

fn value_in(host: &Host, key: &str, label: &str) -> Option<String> {
    host.presentation(key)
        .expect("a window snapshot")
        .find_label(label)
        .expect("a labelled element")
        .text_value()
        .map(str::to_owned)
}

fn type_main_draft(host: &mut Host, text: &str) -> NodeId {
    let (draft, point, scale) = center_label(host, MAIN, "What needs to be done?");
    post_click_in(host, MAIN, point, scale);
    assert!(!host.pump().expect("focus the draft"));
    for character in text.chars() {
        post_char_in(host, MAIN, character);
    }
    assert!(!host.pump().expect("type the draft"));
    draft
}

fn window_title(hwnd: windows::Win32::Foundation::HWND) -> String {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};
    // 安全：只读取本窗口的标题；缓冲长度来自系统。
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    let mut buffer = vec![0_u16; length.max(0) as usize + 1];
    // 安全：buffer 是本地的，长度足够容纳标题与终止符。
    let written = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..written.max(0) as usize])
}

#[test]
fn two_windows_have_catalog_titles_sizes_and_independent_frames() {
    let mut host = open_shared_host();
    let main = host.hwnd("main").expect("the primary HWND");
    let second = host.hwnd("second").expect("the secondary HWND");
    assert_ne!(main, second, "each live window owns one HWND");
    assert_eq!(window_title(main), "Main shared");
    assert_eq!(window_title(second), "Second shared");

    let main_scene = host.scene("main").expect("the primary scene");
    let second_scene = host.scene("second").expect("the secondary scene");
    assert!(
        (main_scene.viewport().width() - 640.0).abs() < 2.0,
        "the primary uses its catalog logical width"
    );
    assert!(
        (second_scene.viewport().width() - 480.0).abs() < 2.0,
        "the secondary uses its catalog logical width"
    );

    let main_frame = host.read_frame("main").expect("the primary frame");
    let second_frame = host.read_frame("second").expect("the secondary frame");
    assert!(
        main_frame.width() > second_frame.width(),
        "each HWND renders its own catalog size"
    );
    assert_ne!(
        main_frame.pixels().len(),
        second_frame.pixels().len(),
        "each HWND renders its own surface size"
    );
}

#[test]
fn a_shared_model_change_repaints_both_windows() {
    let mut host = open_shared_host();
    let before_main = host.read_frame("main").expect("primary frame");
    let before_second = host.read_frame("second").expect("secondary frame");
    let before_main_text = shared_glyphs(&host, "main");
    let before_second_text = shared_glyphs(&host, "second");

    let (_, point, scale) = center_in(&host, "main", |element| {
        matches!(element.visual(), Visual::Button { .. })
    });
    post_click_in(&host, "main", point, scale);
    assert!(!host.pump().expect("deliver the click"));

    assert_eq!(
        shared_glyphs(&host, "main"),
        before_main_text + 1,
        "the primary shows the shared value"
    );
    assert_eq!(
        shared_glyphs(&host, "second"),
        before_second_text + 1,
        "the secondary follows the same model change"
    );
    let after_main = host.read_frame("main").expect("primary frame after");
    let after_second = host.read_frame("second").expect("secondary frame after");
    assert_ne!(
        before_main.pixels(),
        after_main.pixels(),
        "the primary repaints its own surface"
    );
    assert_ne!(
        before_second.pixels(),
        after_second.pixels(),
        "the secondary repaints its own surface"
    );

    host.request_close("second").expect("close the secondary");
    host.request_close("main").expect("close the primary");
    host.run().expect("the loop exits after the last window");
}

#[test]
fn focus_and_drafts_stay_per_window_and_hide_stops_input() {
    let mut host = open_shared_host();
    let (main_input, main_point, main_scale) = center_in(&host, "main", |element| {
        matches!(element.visual(), Visual::TextInput)
    });
    let (second_input, second_point, second_scale) = center_in(&host, "second", |element| {
        matches!(element.visual(), Visual::TextInput)
    });
    assert_ne!(main_input, second_input, "each window mounts its own tree");
    let before_main_edit = host.read_frame("second").expect("secondary frame");

    post_click_in(&host, "main", main_point, main_scale);
    assert!(!host.pump().expect("focus the primary input"));
    post_char_in(&host, "main", 'M');
    assert!(!host.pump().expect("edit the primary draft"));
    let after_main_edit = host.read_frame("second").expect("secondary frame after");
    assert_eq!(
        before_main_edit.pixels(),
        after_main_edit.pixels(),
        "editing the primary does not change the secondary's own frame"
    );

    post_click_in(&host, "second", second_point, second_scale);
    assert!(!host.pump().expect("focus the secondary input"));
    post_char_in(&host, "second", 'S');
    post_char_in(&host, "second", 'S');
    assert!(!host.pump().expect("edit the secondary draft"));

    assert_eq!(focused_in(&host, "main"), Some(main_input));
    assert_eq!(focused_in(&host, "second"), Some(second_input));
    assert_eq!(
        input_glyphs(&host, "main"),
        "main-draft".len() + 1,
        "the primary draft keeps its own edit"
    );
    assert_eq!(
        input_glyphs(&host, "second"),
        "second-draft".len() + 2,
        "the secondary draft keeps its own edit"
    );
    assert!(
        host.scene("main")
            .expect("primary scene")
            .caret(main_input)
            .is_some(),
        "the primary caret stays in its own scene"
    );

    // Tab 和空白点击必须使用接收消息的 HWND，而不是运行时当前的焦点窗口。
    post_key_in(&host, "main", VK_TAB);
    assert!(!host.pump().expect("tab inside the primary"));
    assert_ne!(
        focused_in(&host, "main"),
        Some(main_input),
        "Tab moves the primary's own focus order"
    );
    assert_eq!(
        focused_in(&host, "second"),
        Some(second_input),
        "Tab in the primary leaves the secondary's focus untouched"
    );

    let (_, text_point, text_scale) = center_in(&host, "main", |element| {
        matches!(element.visual(), Visual::Text)
    });
    post_click_in(&host, "main", text_point, text_scale);
    assert!(
        !host
            .pump()
            .expect("click non-focusable text in the primary")
    );
    assert_eq!(
        focused_in(&host, "main"),
        None,
        "a blank click clears the primary's focus"
    );
    assert_eq!(
        focused_in(&host, "second"),
        Some(second_input),
        "a blank click in the primary leaves the secondary's focus untouched"
    );

    host.hide("second").expect("hide the secondary");
    assert!(!host.pump().expect("process the hidden state"));
    post_char_in(&host, "second", 'X');
    assert!(!host.pump().expect("hidden input is ignored"));
    assert_eq!(
        input_glyphs(&host, "second"),
        "second-draft".len() + 2,
        "a hidden window rejects input"
    );
    assert_eq!(
        input_glyphs(&host, "main"),
        "main-draft".len() + 1,
        "hiding the secondary leaves the primary untouched"
    );

    host.show("second").expect("show the secondary again");
    assert!(!host.pump().expect("process the restore"));
    assert_eq!(
        input_glyphs(&host, "second"),
        "second-draft".len() + 2,
        "show restores the preserved draft"
    );
    post_char_in(&host, "second", 'Y');
    assert!(!host.pump().expect("edit after restore"));
    assert_eq!(
        input_glyphs(&host, "second"),
        "second-draft".len() + 3,
        "input resumes after show"
    );

    host.request_close("second").expect("close the secondary");
    host.request_close("main").expect("close the primary");
    host.run().expect("the loop exits after the last window");
}

#[test]
fn closing_the_secondary_keeps_the_primary_live() {
    let mut host = open_shared_host();
    let main = host.hwnd("main").expect("the primary HWND");
    host.request_close("second").expect("close the secondary");
    assert!(
        !host.pump().expect("process the secondary close"),
        "the primary keeps the loop alive"
    );
    assert!(host.hwnd("second").is_none(), "the secondary HWND is gone");
    assert_eq!(
        host.hwnd("main"),
        Some(main),
        "the primary HWND keeps its identity"
    );
    assert!(host.scene("main").is_some());

    host.request_close("main").expect("close the primary");
    host.run().expect("the loop exits after the last window");
    assert!(host.hwnd("main").is_none());
}

fn nth_button_in(host: &Host, key: &str, index: usize) -> (NodeId, Point, f32) {
    let scene = host.scene(key).expect("a rendered scene");
    let button = scene
        .elements()
        .iter()
        .filter(|element| matches!(element.visual(), Visual::Button { .. }))
        .nth(index)
        .expect("a button");
    let rect = button.rect();
    (
        button.id(),
        Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0),
        scene.viewport().scale(),
    )
}

#[test]
fn appearance_changes_repaint_every_open_window() {
    let mut host = open_shared_host();
    let before_main = host.read_frame("main").expect("primary frame");
    let before_second = host.read_frame("second").expect("secondary frame");

    // 第二个按钮是 Theme；外观属于应用状态，不绑定某一个 HWND。
    let (_, point, scale) = nth_button_in(&host, "main", 1);
    post_click_in(&host, "main", point, scale);
    assert!(!host.pump().expect("toggle the appearance"));

    let after_main = host.read_frame("main").expect("primary dark frame");
    let after_second = host.read_frame("second").expect("secondary dark frame");
    for (name, pixel) in [
        ("primary", after_main.pixel(4, 4)),
        ("secondary", after_second.pixel(4, 4)),
    ] {
        let pixel = pixel.expect("a dark pixel");
        assert!(
            pixel[..3].iter().all(|channel| *channel < 60),
            "the {name} window follows the shared dark appearance: {pixel:?}"
        );
    }
    assert_ne!(before_main.pixels(), after_main.pixels());
    assert_ne!(before_second.pixels(), after_second.pixels());

    host.request_close("second").expect("close the secondary");
    host.request_close("main").expect("close the primary");
    host.run().expect("the loop exits after the last window");
}

#[test]
fn focus_visible_modality_stays_with_each_window() {
    let mut host = open_shared_host();
    let (main_input, main_point, main_scale) = center_in(&host, "main", |element| {
        matches!(element.visual(), Visual::TextInput)
    });
    let (second_input, second_point, second_scale) = center_in(&host, "second", |element| {
        matches!(element.visual(), Visual::TextInput)
    });

    post_click_in(&host, "main", main_point, main_scale);
    assert!(!host.pump().expect("focus the primary input"));
    post_char_in(&host, "main", 'K');
    assert!(!host.pump().expect("use the keyboard in the primary"));
    assert_eq!(
        host.scene("main")
            .expect("primary scene")
            .node(main_input)
            .expect("primary input")
            .resolved_style()
            .border_color(),
        Some(Color::Role(ColorRole::BorderAccent)),
        "keyboard focus selects focus-visible in the primary"
    );

    post_click_in(&host, "second", second_point, second_scale);
    assert!(!host.pump().expect("focus the secondary input"));
    post_char_in(&host, "second", 'K');
    assert!(!host.pump().expect("use the keyboard in the secondary"));
    assert_eq!(
        host.scene("second")
            .expect("secondary scene")
            .node(second_input)
            .expect("secondary input")
            .resolved_style()
            .border_color(),
        Some(Color::Role(ColorRole::BorderAccent)),
        "the secondary has its own keyboard modality"
    );
    assert_eq!(
        host.scene("main")
            .expect("primary scene")
            .node(main_input)
            .expect("primary input")
            .resolved_style()
            .border_color(),
        Some(Color::Role(ColorRole::BorderAccent)),
        "pointer input in another window does not clear the primary modality"
    );

    post_click_in(&host, "main", main_point, main_scale);
    assert!(!host.pump().expect("use the pointer in the primary"));
    assert_eq!(
        host.scene("main")
            .expect("primary scene")
            .node(main_input)
            .expect("primary input")
            .resolved_style()
            .border_color(),
        None,
        "pointer input clears only the primary's focus-visible"
    );
    assert_eq!(
        host.scene("second")
            .expect("secondary scene")
            .node(second_input)
            .expect("secondary input")
            .resolved_style()
            .border_color(),
        Some(Color::Role(ColorRole::BorderAccent)),
        "the secondary keeps its keyboard modality"
    );

    host.request_close("second").expect("close the secondary");
    host.request_close("main").expect("close the primary");
    host.run().expect("the loop exits after the last window");
}

#[test]
fn wheel_and_paging_scroll_the_native_scene() {
    let mut host = open_tall_host(500);
    host.render().expect("initial render");
    let (max, page) = {
        let scene = host.scene(MAIN).expect("a rendered scene");
        assert_eq!(scene.scroll_offset(), 0);
        let max = scene.max_scroll_offset();
        assert!(max > 0, "500 rows overflow the viewport");
        (
            max,
            ((scene.viewport().height() - 24.0).floor() as i32).max(1),
        )
    };

    if system_wheel_lines() != 0 {
        // 负 delta 表示滚轮朝用户方向，内容向下滚动。
        post_wheel(&host, -120);
        host.pump().expect("one wheel notch");
        let after = host.scene(MAIN).expect("scene").scroll_offset();
        assert!(after > 0, "a notch toward the user scrolls down");

        post_wheel(&host, 120);
        host.pump().expect("reverse wheel");
        assert_eq!(
            host.scene(MAIN).expect("scene").scroll_offset(),
            0,
            "the reverse notch returns to the top"
        );
    }

    let top_frame = host.read_frame(MAIN).expect("read the top frame");
    post_key(&host, VK_NEXT);
    host.pump().expect("PageDown");
    assert_eq!(
        host.scene(MAIN).expect("scene").scroll_offset(),
        page.min(max),
        "PageDown moves by the overlapping page step"
    );
    let scrolled_frame = host.read_frame(MAIN).expect("read the scrolled frame");
    assert_ne!(
        top_frame.pixels(),
        scrolled_frame.pixels(),
        "the painted frame moves with the offset"
    );

    post_key(&host, VK_PRIOR);
    host.pump().expect("PageUp");
    assert_eq!(host.scene(MAIN).expect("scene").scroll_offset(), 0);

    post_key(&host, VK_END);
    host.pump().expect("End");
    assert_eq!(
        host.scene(MAIN).expect("scene").scroll_offset(),
        max,
        "End goes to the maximum"
    );

    if system_wheel_lines() != 0 {
        post_wheel(&host, -120);
        host.pump().expect("further down at the bottom");
        assert_eq!(
            host.scene(MAIN).expect("scene").scroll_offset(),
            max,
            "the over-scroll is discarded at the bottom"
        );
        post_wheel(&host, 120);
        host.pump().expect("reverse at the bottom");
        assert!(
            host.scene(MAIN).expect("scene").scroll_offset() < max,
            "the first reverse wheel step moves immediately"
        );
    }

    post_key(&host, VK_HOME);
    host.pump().expect("Home");
    assert_eq!(host.scene(MAIN).expect("scene").scroll_offset(), 0);

    if system_wheel_lines() != 0 {
        post_wheel(&host, 120);
        host.pump().expect("further up at the top");
        assert_eq!(
            host.scene(MAIN).expect("scene").scroll_offset(),
            0,
            "the over-scroll is discarded at the top"
        );
        post_wheel(&host, -120);
        host.pump().expect("reverse at the top");
        assert!(
            host.scene(MAIN).expect("scene").scroll_offset() > 0,
            "the first reverse wheel step moves immediately"
        );
    }
}

#[test]
fn scroll_only_input_moves_the_row_window_without_touching_the_model() {
    let mut host = open_tall_host(500);
    host.render().expect("initial render");
    let before = host.presentation(MAIN).expect("a committed snapshot");
    let band = before.row_bands()[0];
    assert_eq!(band.total_rows(), 500, "the whole list is declared");
    assert_eq!(band.first_row(), 0);
    assert!(
        before.elements().count() < 500,
        "only the visible window is mounted"
    );
    let draft = before
        .find_label("What needs to be done?")
        .expect("the draft input")
        .id();
    let first_row = before
        .find_label("Edit Task 0")
        .expect("the first row")
        .id();

    post_key(&host, VK_END);
    host.pump().expect("scroll to the bottom");
    let scene = host.scene(MAIN).expect("scene");
    assert!(scene.scroll_offset() > 0);

    let after = host.presentation(MAIN).expect("the committed snapshot");
    let band = after.row_bands()[0];
    assert_eq!(band.total_rows(), 500, "the declared extent stays whole");
    assert!(
        band.first_row() > 0,
        "the mounted window followed the scroll"
    );
    assert!(
        after.elements().count() < 500,
        "the mounted window stays bounded"
    );
    assert!(
        after.node(draft).is_some(),
        "the chrome outside the list stays mounted"
    );
    assert!(
        after.node(first_row).is_none(),
        "a row outside the window is unmounted"
    );
    assert!(
        scene.node(first_row).is_none(),
        "the scene is scoped to the mounted window"
    );

    post_key(&host, VK_HOME);
    host.pump().expect("scroll to the top");
    let top = host.presentation(MAIN).expect("the committed snapshot");
    assert_eq!(top.row_bands()[0].first_row(), 0);
    let remounted = top
        .find_label("Edit Task 0")
        .expect("the first row is mounted again")
        .id();
    assert_ne!(
        remounted, first_row,
        "a re-entering row mounts with a fresh NodeId"
    );
}

#[test]
fn scrolling_revalidates_hover_against_the_translated_scene() {
    let mut host = open_tall_host(100);
    host.render().expect("initial render");
    let (first, point, scale) = center_label(&host, MAIN, "Edit Task 0");
    post_move(&host, point, scale);
    host.pump().expect("hover the first row");
    assert_eq!(
        host.scene(MAIN)
            .expect("scene")
            .elements()
            .iter()
            .find(|element| element.hovered())
            .map(SceneElement::id),
        Some(first)
    );

    post_key(&host, VK_END);
    host.pump().expect("scroll to the bottom");
    let scene = host.scene(MAIN).expect("scene");
    let expected = scene
        .hit_test(point)
        .filter(|node| scene.node(*node).is_some_and(|element| !element.disabled()));
    let hovered = scene
        .elements()
        .iter()
        .find(|element| element.hovered())
        .map(SceneElement::id);
    assert_eq!(
        hovered, expected,
        "hover follows the hit test at the unchanged pointer"
    );
    assert!(hovered.is_some(), "the translated scene still has a hover");
    assert_ne!(hovered, Some(first), "the old hover node moved away");
}

#[test]
fn hide_show_keeps_the_offset_and_reopen_starts_at_zero() {
    let mut host = open_tall_host(100);
    host.render().expect("initial render");
    post_key(&host, VK_END);
    host.pump().expect("scroll to the bottom");
    let bottom = host.scene(MAIN).expect("scene").scroll_offset();
    assert!(bottom > 0);

    host.hide(MAIN).expect("hide the window");
    let before_hidden_wheel = host.scene(MAIN).expect("scene").scroll_offset();
    post_wheel(&host, 120);
    host.pump().expect("wheel on a hidden window");
    assert_eq!(
        host.scene(MAIN).expect("scene").scroll_offset(),
        before_hidden_wheel,
        "hidden windows ignore wheel input"
    );

    host.show(MAIN).expect("show the window");
    host.pump().expect("show the window");
    assert_eq!(
        host.scene(MAIN).expect("scene").scroll_offset(),
        bottom,
        "show keeps the offset"
    );

    host.close(MAIN).expect("close the window");
    host.show(MAIN).expect("reopen the window");
    host.pump().expect("render the reopened window");
    assert_eq!(
        host.scene(MAIN).expect("scene").scroll_offset(),
        0,
        "a new activation starts at zero"
    );
}

#[test]
fn paging_keys_leave_a_focused_editor_in_place() {
    let mut host = open_tall_host(50);
    host.render().expect("initial render");
    let (_, point, scale) = center_label(&host, MAIN, "What needs to be done?");
    post_click(&host, point, scale);
    host.pump().expect("focus the draft input");
    let input = focused_id(&host).expect("the draft input has focus");
    post_char(&host, 'a');
    post_char(&host, 'b');
    host.pump().expect("type into the draft");
    assert_eq!(glyphs(&host, input), 2, "two characters were inserted");

    post_key(&host, VK_END);
    host.pump().expect("End while editing");
    assert_eq!(
        host.scene(MAIN).expect("scene").scroll_offset(),
        0,
        "End edits the focused input instead of scrolling"
    );
    let caret_end = host
        .scene(MAIN)
        .expect("scene")
        .caret(input)
        .expect("caret at the end")
        .x;
    assert_eq!(
        focused_id(&host),
        Some(input),
        "End keeps the editor focused"
    );

    post_key(&host, VK_HOME);
    host.pump().expect("Home while editing");
    assert_eq!(
        host.scene(MAIN).expect("scene").scroll_offset(),
        0,
        "Home edits the focused input instead of scrolling"
    );
    let caret_home = host
        .scene(MAIN)
        .expect("scene")
        .caret(input)
        .expect("caret at the start")
        .x;
    assert!(
        caret_home < caret_end,
        "Home moved the caret to the start of the text"
    );

    post_key(&host, VK_NEXT);
    host.pump().expect("PageDown while editing");
    let page = host.scene(MAIN).expect("scene").scroll_offset();
    assert!(page > 0, "PageDown still scrolls with a focused editor");
    assert_eq!(
        focused_id(&host),
        Some(input),
        "PageDown keeps the same editor focused"
    );

    post_key(&host, VK_HOME);
    host.pump().expect("Home while editing");
    let revealed = host.scene(MAIN).expect("scene").scroll_offset();
    assert!(
        revealed < page,
        "Home edits the caret and reveals the focused input instead of paging"
    );
    assert_eq!(
        focused_id(&host),
        Some(input),
        "Home keeps the editor focused"
    );

    post_key(&host, VK_PRIOR);
    host.pump().expect("PageUp while editing");
    assert_eq!(host.scene(MAIN).expect("scene").scroll_offset(), 0);
    assert_eq!(
        focused_id(&host),
        Some(input),
        "PageUp keeps the same editor focused"
    );
}

#[test]
fn tab_reveals_an_offscreen_focus_target_and_restores_the_top_window() {
    let mut host = open_tall_host(100);
    host.render().expect("initial render");
    let header = host
        .presentation(MAIN)
        .expect("a committed snapshot")
        .find_label("What needs to be done?")
        .expect("the draft input")
        .id();

    post_key(&host, VK_END);
    host.pump().expect("scroll to the bottom");
    let bottom = host.scene(MAIN).expect("scene").scroll_offset();
    assert!(bottom > 0);

    // 没有焦点时 Tab 聚焦第一个控件；它在视口顶部，当前已滚出视口。
    post_key(&host, VK_TAB);
    host.pump().expect("focus the first control");
    let focused = focused_id(&host).expect("a focused control");
    assert_eq!(
        focused, header,
        "Tab focuses the first control in paint order"
    );
    let scene = host.scene(MAIN).expect("scene");
    let rect = scene.node(focused).expect("focused element").rect();
    assert!(
        rect.y >= 0.0 && rect.bottom() <= scene.viewport().height(),
        "Tab reveals the focused control"
    );
    assert!(
        scene.scroll_offset() < bottom,
        "the reveal moves the offset back"
    );
    let snapshot = host.presentation(MAIN).expect("the committed snapshot");
    assert_eq!(
        snapshot.row_bands()[0].first_row(),
        0,
        "the reveal restores the top row window"
    );
    assert!(
        snapshot.node(header).is_some(),
        "the focused chrome element survives the window change"
    );
}

#[test]
fn an_edit_after_wheel_scrolling_reveals_the_focused_input() {
    let mut host = open_tall_host(50);
    host.render().expect("initial render");
    let (_, point, scale) = center_label(&host, MAIN, "What needs to be done?");
    post_click(&host, point, scale);
    host.pump().expect("focus the draft input");
    let input = focused_id(&host).expect("the draft has focus");
    post_char(&host, 'a');
    host.pump().expect("type into the draft");

    post_wheel(&host, -600);
    host.pump().expect("scroll the draft offscreen");
    let scrolled = host.scene(MAIN).expect("scene").scroll_offset();
    assert!(scrolled > 0);
    assert_eq!(focused_id(&host), Some(input), "the wheel keeps focus");
    let snapshot = host.presentation(MAIN).expect("snapshot");
    let edit = snapshot
        .node(input)
        .expect("input")
        .edit()
        .expect("edit state");
    assert_eq!(edit.caret(), 1, "the wheel keeps the caret");

    post_char(&host, 'b');
    host.pump().expect("type after scrolling");
    let scene = host.scene(MAIN).expect("scene");
    assert!(
        scene.scroll_offset() < scrolled,
        "the edit reveals the focused input"
    );
    let rect = scene.node(input).expect("input").rect();
    assert!(
        rect.y >= 0.0 && rect.bottom() <= scene.viewport().height(),
        "the input is fully visible after the edit"
    );
    assert_eq!(glyphs(&host, input), 2, "the second character was inserted");
    assert_eq!(focused_id(&host), Some(input), "the editor keeps focus");
}

#[test]
fn dragging_the_scrollbar_thumb_scrolls_the_shared_offset_and_clamps() {
    let mut host = open_tall_host(100);
    host.render().expect("initial render");
    let scene = host.scene(MAIN).expect("scene");
    let scale = scene.viewport().scale();
    let bar = scene.scrollbar().expect("overflow has a scrollbar");
    let max = scene.max_scroll_offset();
    assert!(max > 0);

    let track = bar.track();
    let thumb = bar.thumb();
    let grab = Point::new(thumb.x + thumb.width / 2.0, thumb.y + 4.0);
    post_down(&host, grab, scale);
    host.pump().expect("grab the thumb");
    let past_bottom = Point::new(grab.x, track.bottom() + 200.0);
    post_move(&host, past_bottom, scale);
    host.pump().expect("drag past the bottom");
    assert_eq!(
        host.scene(MAIN).expect("scene").scroll_offset(),
        max,
        "the drag clamps at the maximum"
    );
    post_up(&host, past_bottom, scale);
    host.pump().expect("release at the bottom");

    let scene = host.scene(MAIN).expect("scene at max");
    let bar = scene.scrollbar().expect("scrollbar at max");
    let thumb = bar.thumb();
    let grab = Point::new(thumb.x + thumb.width / 2.0, thumb.y + thumb.height / 2.0);
    post_down(&host, grab, scale);
    host.pump().expect("grab the bottom thumb");
    let past_top = Point::new(grab.x, bar.track().y - 200.0);
    post_move(&host, past_top, scale);
    host.pump().expect("drag past the top");
    assert_eq!(
        host.scene(MAIN).expect("scene").scroll_offset(),
        0,
        "the drag clamps at zero"
    );
    post_up(&host, past_top, scale);
    host.pump().expect("release at the top");
    assert_eq!(host.scene(MAIN).expect("scene").scroll_offset(), 0);

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn a_scrollbar_track_click_pages_towards_the_clicked_side() {
    let mut host = open_tall_host(100);
    host.render().expect("initial render");
    let scene = host.scene(MAIN).expect("scene");
    let scale = scene.viewport().scale();
    let page = ((scene.viewport().height() - 24.0).floor() as i32).max(1);
    let bar = scene.scrollbar().expect("scrollbar");
    let below = Point::new(
        bar.track().x + bar.track().width / 2.0,
        bar.thumb().bottom() + 40.0,
    );
    post_click(&host, below, scale);
    host.pump().expect("page down");
    assert_eq!(host.scene(MAIN).expect("scene").scroll_offset(), page);

    let scene = host.scene(MAIN).expect("scene after paging");
    let bar = scene.scrollbar().expect("scrollbar");
    let above = Point::new(
        bar.track().x + bar.track().width / 2.0,
        bar.thumb().y - 40.0,
    );
    post_click(&host, above, scale);
    host.pump().expect("page up");
    assert_eq!(host.scene(MAIN).expect("scene").scroll_offset(), 0);

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn the_scrollbar_wins_hit_priority_over_the_content_under_it() {
    let mut host = open_tall_host(100);
    host.render().expect("initial render");
    let (_, point, scale) = center_label(&host, MAIN, "What needs to be done?");
    post_click(&host, point, scale);
    host.pump().expect("focus the draft input");
    let focused = focused_id(&host).expect("a focused input");

    let scene = host.scene(MAIN).expect("scene");
    let bar = scene.scrollbar().expect("scrollbar");
    let click = Point::new(bar.track().x + 2.0, bar.thumb().bottom() + 20.0);
    assert!(
        scene.hit_test(click).is_some(),
        "content sits under the overlay"
    );
    post_click(&host, click, scale);
    host.pump().expect("click the track over content");
    assert_eq!(
        focused_id(&host),
        Some(focused),
        "the scrollbar does not blur the focused input"
    );
    assert!(
        host.scene(MAIN).expect("scene").scroll_offset() > 0,
        "the track click pages instead of activating content"
    );

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn wheel_drag_and_keys_share_one_clamped_offset_without_stale_transients() {
    let mut host = open_tall_host(100);
    host.render().expect("initial render");
    post_wheel(&host, -120);
    host.pump().expect("one wheel step");
    let scrolled = host.scene(MAIN).expect("scene").scroll_offset();
    assert!(scrolled > 0);

    let (scale, bar) = {
        let scene = host.scene(MAIN).expect("scene");
        (
            scene.viewport().scale(),
            scene.scrollbar().expect("scrollbar"),
        )
    };
    let grab = Point::new(
        bar.thumb().x + bar.thumb().width / 2.0,
        bar.thumb().y + bar.thumb().height / 2.0,
    );
    post_down(&host, grab, scale);
    host.pump().expect("grab the thumb");
    let middle = Point::new(grab.x, bar.track().y + bar.track().height / 2.0);
    post_move(&host, middle, scale);
    host.pump().expect("drag to the middle");
    let dragged = host.scene(MAIN).expect("scene").scroll_offset();
    assert!(
        dragged > scrolled,
        "the drag continues from the wheel offset"
    );
    post_up(&host, middle, scale);
    host.pump().expect("release the drag");

    let scene = host.scene(MAIN).expect("scene");
    let max = scene.max_scroll_offset();
    let page = ((scene.viewport().height() - 24.0).floor() as i32).max(1);
    post_key(&host, VK_END);
    host.pump().expect("End");
    assert_eq!(host.scene(MAIN).expect("scene").scroll_offset(), max);
    post_key(&host, VK_PRIOR);
    host.pump().expect("PageUp");
    assert_eq!(
        host.scene(MAIN).expect("scene").scroll_offset(),
        (max - page).max(0)
    );

    // 拖动结束后指针移动仍能正常 hover 内容，没有残留的按压状态。
    let scene = host.scene(MAIN).expect("scene");
    let button = scene
        .elements()
        .iter()
        .find(|element| {
            matches!(element.visual(), Visual::Button { .. })
                && element.rect().y >= 0.0
                && element.rect().bottom() <= scene.viewport().height()
        })
        .expect("a visible button");
    let id = button.id();
    let rect = button.rect();
    let point = Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
    post_move(&host, point, scale);
    host.pump().expect("hover a button after the drag");
    let node = host.scene(MAIN).expect("scene").node(id).expect("button");
    assert!(node.hovered(), "the drag leaves no stale hover block");
    assert!(!node.pointer_pressed(), "the drag leaves no pressed state");

    post_close(&host);
    host.run().expect("process the close");
}

#[test]
fn accessibility_tree_mirrors_the_committed_scene_and_scroll() {
    let mut host = open_tall_host(40);
    host.render().expect("render");

    let tree = host.accessibility_tree(MAIN).expect("published tree");
    assert_eq!(tree.root().control_type(), ControlType::Window);
    assert_eq!(tree.root().name(), "ZenUI Todo");
    let form = tree.find_name("Add task").expect("the add form");
    assert_eq!(form.control_type(), ControlType::Group);
    let input = tree
        .find_name("What needs to be done?")
        .expect("the draft input");
    assert_eq!(input.control_type(), ControlType::Edit);
    assert!(input.focusable());
    let delete = tree
        .find_name("Delete Task 0")
        .expect("the first delete button");
    assert_eq!(delete.control_type(), ControlType::Button);
    assert_eq!(delete.automation_id(), "");

    let scene = host.scene(MAIN).expect("scene");
    let bounds = tree.root_bounds((0, 0));
    assert!(
        (bounds.width - f64::from(scene.viewport().width()) * f64::from(scene.viewport().scale()))
            .abs()
            < 0.01,
        "the root bounds use the committed viewport and scale"
    );

    let node = form.node_id().expect("a node id");
    let runtime_id = form.runtime_id().expect("a runtime id");
    let top = tree.logical_rect(form).y;

    let input_id = input.node_id().expect("a node id");
    let input_rect = scene.node(input_id).expect("the input scene node").rect();
    let point = Point::new(
        input_rect.x + input_rect.width / 2.0,
        input_rect.y + input_rect.height / 2.0,
    );
    let scale = scene.viewport().scale();
    post_down(&host, point, scale);
    post_up(&host, point, scale);
    host.pump().expect("focus the draft input");
    let focused = host.accessibility_tree(MAIN).expect("published tree");
    assert!(
        focused
            .find_name("What needs to be done?")
            .is_some_and(|input| input.focused()),
        "the snapshot focus is exposed on the matching node"
    );

    if system_wheel_lines() != 0 {
        post_wheel(&host, -120);
        host.pump().expect("one wheel notch");
        let offset = host.scene(MAIN).expect("scene").scroll_offset();
        let after = host.accessibility_tree(MAIN).expect("republished tree");
        let moved = after.node(node).expect("the form stays mounted");
        assert_eq!(moved.runtime_id(), Some(runtime_id));
        assert!(
            (after.logical_rect(moved).y - (top - offset as f32)).abs() < 0.5,
            "accessibility bounds follow the applied offset"
        );
    }
}

#[test]
fn hiding_a_window_withdraws_its_accessibility_tree() {
    let mut host = open_host();
    host.render().expect("render");
    assert!(host.accessibility_tree(MAIN).is_some());

    host.hide(MAIN).expect("hide");
    assert!(host.accessibility_tree(MAIN).is_none());

    host.show(MAIN).expect("show");
    host.render().expect("render after showing");
    assert!(host.accessibility_tree(MAIN).is_some());
}
