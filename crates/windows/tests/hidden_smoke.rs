#![cfg(windows)]

use std::collections::BTreeSet;

use zenui_windows::{Host, HostConfig, HostError};

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
