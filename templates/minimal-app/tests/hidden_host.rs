//! Windows 隐藏宿主 smoke：不显示 HWND，但在 WARP 上渲染并读回一帧。
//!
//! 这是 partial evidence；可见窗口、真实 GPU 与关闭退出仍由人工运行记录。

#![cfg(windows)]

use minimal_app::{STARTER_WINDOW, open_starter_window};
use zenui_windows::{Host, HostConfig};

#[test]
fn hidden_host_renders_the_starter_on_warp() {
    let mut runtime = zenui::Runtime::new();
    open_starter_window(&mut runtime).expect("open the starter window");
    let mut host = Host::open(runtime, HostConfig::hidden()).expect("open a hidden host");

    let frame = host.read_frame(STARTER_WINDOW).expect("read a frame");
    assert!(frame.width() > 0 && frame.height() > 0);
    let first = frame.pixel(0, 0).expect("a corner pixel");
    assert!(
        frame
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| *pixel != first),
        "the frame should contain text and a button, not one flat color"
    );

    let scene = host.scene(STARTER_WINDOW).expect("a rendered scene");
    assert!(
        scene
            .elements()
            .iter()
            .any(|element| !element.text().is_empty()),
        "the counter label should produce scene text runs"
    );
}
