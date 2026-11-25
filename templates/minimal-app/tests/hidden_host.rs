//! Windows 隐藏宿主 smoke：不显示 HWND，渲染 starter 并读回一帧。
//!
//! 宿主先请求硬件 DX12 适配器，没有时回退 WARP；读回证明隐藏窗口在某个
//! DX12 适配器上产生了非平坦帧，不区分硬件与 WARP，也不证明可见窗口、
//! 输入交付或真实 GPU 呈现——这些仍由人工可见运行记录。
//!
//! `Host` 的公开隐藏面没有输入注入；`Increment` 的点击语义由 `starter.rs`
//! 的 headless 测试覆盖，这里不重复，也不为测试新增生产 API。

#![cfg(windows)]

use minimal_app::{STARTER_WINDOW, open_starter_window};
use zenui_windows::{Host, HostConfig};

#[test]
fn hidden_host_renders_the_starter_counter() {
    let mut runtime = zenui::Runtime::new();
    open_starter_window(&mut runtime).expect("open the starter window");
    let mut host = Host::open(runtime, HostConfig::hidden()).expect("open a hidden host");
    let adapter = host.adapter_info();

    let frame = host.read_frame(STARTER_WINDOW).expect("read a frame");
    assert!(
        frame.width() >= 420 && frame.height() >= 280,
        "the hidden frame should cover the declared 420x280 logical client at 100% DPI, got {}x{} (adapter: {}, fallback: {})",
        frame.width(),
        frame.height(),
        adapter.name,
        adapter.fallback
    );
    let first = frame.pixel(0, 0).expect("a corner pixel");
    assert!(
        frame
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| *pixel != first),
        "the frame should contain text and a button, not one flat color (adapter: {}, fallback: {})",
        adapter.name,
        adapter.fallback
    );

    let presentation = host
        .presentation(STARTER_WINDOW)
        .expect("a presentation snapshot");
    assert!(
        presentation
            .elements()
            .any(|element| element.text() == "Count: 0"),
        "the starter counter label should be committed as `Count: 0` (adapter: {}, fallback: {})",
        adapter.name,
        adapter.fallback
    );
}
