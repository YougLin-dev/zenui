#![allow(dead_code, unused_braces)]

use zenui::prelude::*;
use zenui_demo::*;
use zenui_render::{
    GpuContext, GpuRenderer, RenderError, RenderPalette, RenderTargetConfig, Scene, SceneElement,
    Viewport, wgpu,
};

/// 纯色调色板：0/255 分量在 Unorm 与 sRGB 目标下写出相同字节。
const TEST_PALETTE: RenderPalette = RenderPalette {
    background: [0, 0, 255, 255],
    surface: [0, 255, 0, 255],
    control: [255, 0, 0, 255],
    text: [0, 0, 0, 255],
    border: [0, 0, 0, 255],
    accent: [255, 255, 0, 255],
    selection: [255, 0, 255, 255],
    caret: [255, 255, 255, 255],
};

// BGRA byte order.
const BLUE_BGRA: [u8; 4] = [255, 0, 0, 255];
const GREEN_BGRA: [u8; 4] = [0, 255, 0, 255];
const RED_BGRA: [u8; 4] = [0, 0, 255, 255];

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

fn context() -> Option<GpuContext> {
    match GpuContext::new() {
        Ok(context) => Some(context),
        Err(error @ (RenderError::NoAdapter | RenderError::NoDevice { .. })) => {
            eprintln!("skipping GPU test: {error}");
            None
        }
        Err(error) => panic!("unexpected GPU context error: {error}"),
    }
}

fn scene(snapshot: &PresentationSnapshot, width: f32, height: f32, scale: f32) -> Scene {
    let viewport = Viewport::new(width, height, scale).expect("viewport");
    Scene::build(snapshot, viewport).expect("scene")
}

fn by_text(snapshot: &PresentationSnapshot, tag: &str, text: &str) -> NodeId {
    snapshot
        .elements()
        .find(|element| element.tag() == tag && element.text() == text)
        .map(SnapshotElement::id)
        .unwrap_or_else(|| panic!("no {tag} with text {text:?}"))
}

fn render(
    context: &GpuContext,
    scene: &Scene,
    format: wgpu::TextureFormat,
    width: u16,
    height: u16,
) -> Pixels {
    render_with(context, scene, format, width, height, TEST_PALETTE)
}

fn render_with(
    context: &GpuContext,
    scene: &Scene,
    format: wgpu::TextureFormat,
    width: u16,
    height: u16,
    palette: RenderPalette,
) -> Pixels {
    let config = RenderTargetConfig::new(format, width, height).expect("target config");
    let texture = context.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("zenui render test target"),
        size: wgpu::Extent3d {
            width: u32::from(width),
            height: u32::from(height),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let mut renderer = GpuRenderer::new(context, config, palette);
    let mut encoder = context.device().create_command_encoder(&Default::default());
    renderer
        .render(scene, &view, &mut encoder)
        .expect("render scene");
    context.queue().submit([encoder.finish()]);
    Pixels::read(context, &texture, width, height)
}

struct Pixels {
    data: Vec<u8>,
    row: u32,
    width: u32,
}

impl Pixels {
    fn read(context: &GpuContext, texture: &wgpu::Texture, width: u16, height: u16) -> Self {
        let unpadded = u32::from(width) * 4;
        let row = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = context.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("zenui render readback"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = context.device().create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(u32::from(height)),
                },
            },
            wgpu::Extent3d {
                width: u32::from(width),
                height: u32::from(height),
                depth_or_array_layers: 1,
            },
        );
        context.queue().submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |result| result.expect("map readback"));
        context
            .device()
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("GPU wait");
        let data = buffer
            .slice(..)
            .get_mapped_range()
            .expect("mapped bytes")
            .to_vec();
        buffer.unmap();
        Self {
            data,
            row,
            width: u32::from(width),
        }
    }

    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        assert!(x < self.width);
        let i = (y * self.row + x * 4) as usize;
        self.data[i..i + 4].try_into().expect("four channels")
    }

    fn any_in(&self, rect: RectLike, predicate: impl Fn([u8; 4]) -> bool) -> bool {
        let left = rect.x.floor().max(0.0) as u32;
        let top = rect.y.floor().max(0.0) as u32;
        let right = rect.right().ceil() as u32;
        let bottom = rect.bottom().ceil() as u32;
        (top..bottom.min(self.data.len() as u32 / self.row))
            .any(|y| (left..right.min(self.width)).any(|x| predicate(self.at(x, y))))
    }
}

struct RectLike {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl RectLike {
    fn right(&self) -> f32 {
        self.x + self.width
    }

    fn bottom(&self) -> f32 {
        self.y + self.height
    }
}

impl From<zenui_render::Rect> for RectLike {
    fn from(rect: zenui_render::Rect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        }
    }
}

#[test]
fn a_known_color_scene_renders_expected_pixels() {
    let Some(context) = context() else {
        return;
    };
    let mut app = runtime();
    add(&mut app, "Alpha");
    app.input("Edit Alpha", "Alpha").expect("seed the editor");
    app.focus("Edit Alpha").expect("focus");
    assert!(app.set_caret("Edit Alpha", 2), "place the caret");
    let snapshot = app.presentation().expect("snapshot");
    let scene = scene(&snapshot, 256.0, 256.0, 1.0);
    let pixels = render(&context, &scene, wgpu::TextureFormat::Bgra8Unorm, 256, 256);

    assert_eq!(
        pixels.at(4, 4),
        GREEN_BGRA,
        "root surface fills the viewport"
    );

    let all = scene
        .node(by_text(&snapshot, "button", "All"))
        .expect("scene button")
        .rect();
    assert_eq!(
        pixels.at((all.x + 4.0) as u32, (all.y + all.height / 2.0) as u32),
        [0, 255, 255, 255],
        "the pressed filter uses the accent color"
    );
    let active = scene
        .node(by_text(&snapshot, "button", "Active"))
        .expect("scene button")
        .rect();
    assert_eq!(
        pixels.at(
            (active.x + 4.0) as u32,
            (active.y + active.height / 2.0) as u32
        ),
        RED_BGRA,
        "button control color"
    );

    let text = scene
        .node(by_text(&snapshot, "text", "1 remaining"))
        .expect("scene text");
    let glyph = text
        .text()
        .iter()
        .flat_map(|run| run.glyphs())
        .next()
        .expect("a shaped glyph");
    let probe = RectLike {
        x: glyph.x,
        y: glyph.y - 10.0,
        width: glyph.advance.max(2.0),
        height: 12.0,
    };
    assert!(
        pixels.any_in(probe, |pixel| pixel != GREEN_BGRA),
        "glyph ink appears at the expected position"
    );

    let editor = snapshot.find_label("Edit Alpha").expect("editor").id();
    let caret = scene.caret(editor).expect("caret");
    assert!(
        pixels.any_in(caret.into(), |pixel| pixel[1] > 100),
        "the white caret bar appears at its geometry: {caret:?}"
    );
}

#[test]
fn both_bgra_formats_render_the_same_primary_colors() {
    let Some(context) = context() else {
        return;
    };
    let mut app = runtime();
    add(&mut app, "Alpha");
    let snapshot = app.presentation().expect("snapshot");
    let scene = scene(&snapshot, 256.0, 256.0, 1.0);
    let unorm = render(&context, &scene, wgpu::TextureFormat::Bgra8Unorm, 256, 256);
    let srgb = render(
        &context,
        &scene,
        wgpu::TextureFormat::Bgra8UnormSrgb,
        256,
        256,
    );

    let all = scene
        .node(by_text(&snapshot, "button", "All"))
        .expect("scene button")
        .rect();
    for (x, y) in [
        (4, 4),
        ((all.x + 4.0) as u32, (all.y + all.height / 2.0) as u32),
    ] {
        assert_eq!(unorm.at(x, y), srgb.at(x, y), "pixel {x},{y}");
    }
    assert_eq!(unorm.at(4, 4), GREEN_BGRA);
    assert_eq!(srgb.at(4, 4), GREEN_BGRA);
}

#[test]
fn window_scale_changes_physical_pixels_not_layout() {
    let Some(context) = context() else {
        return;
    };
    let mut app = runtime();
    add(&mut app, "Alpha");
    let snapshot = app.presentation().expect("snapshot");
    let logical = scene(&snapshot, 128.0, 128.0, 1.0);
    let scaled = scene(&snapshot, 128.0, 128.0, 2.0);

    let logical_ids: Vec<NodeId> = logical.elements().iter().map(SceneElement::id).collect();
    let scaled_ids: Vec<NodeId> = scaled.elements().iter().map(SceneElement::id).collect();
    assert_eq!(logical_ids, scaled_ids);
    for (a, b) in logical.elements().iter().zip(scaled.elements()) {
        assert_eq!(a.rect(), b.rect(), "layout stays logical across scale");
    }

    let one = render(
        &context,
        &logical,
        wgpu::TextureFormat::Bgra8Unorm,
        128,
        128,
    );
    let two = render(&context, &scaled, wgpu::TextureFormat::Bgra8Unorm, 256, 256);
    assert_eq!(one.at(4, 4), GREEN_BGRA);
    assert_eq!(two.at(8, 8), GREEN_BGRA);
    assert_eq!(
        two.at(8, 240),
        GREEN_BGRA,
        "logical (4, 120) lands on physical (8, 240)"
    );
    assert_eq!(one.at(4, 120), GREEN_BGRA);
    assert_eq!(one.at(4, 120), two.at(8, 240));
    assert_eq!(scaled.viewport().scale(), 2.0);
    assert_eq!(logical.viewport().scale(), 1.0);
}

#[test]
fn unsupported_formats_and_sizes_are_explicit_errors() {
    assert!(matches!(
        RenderTargetConfig::new(wgpu::TextureFormat::Rgba8Unorm, 16, 16),
        Err(RenderError::UnsupportedFormat {
            format: wgpu::TextureFormat::Rgba8Unorm
        })
    ));
    assert!(matches!(
        RenderTargetConfig::new(wgpu::TextureFormat::Bgra8Unorm, 0, 16),
        Err(RenderError::InvalidTargetSize {
            width: 0,
            height: 16
        })
    ));
}

#[test]
fn non_srgb_targets_keep_palette_bytes_and_srgb_targets_encode_them() {
    let Some(context) = context() else {
        return;
    };
    let midtone = RenderPalette {
        surface: [128, 128, 128, 255],
        ..TEST_PALETTE
    };
    let mut app = runtime();
    add(&mut app, "Alpha");
    let snapshot = app.presentation().expect("snapshot");
    let scene = scene(&snapshot, 128.0, 128.0, 1.0);

    let unorm = render_with(
        &context,
        &scene,
        wgpu::TextureFormat::Bgra8Unorm,
        128,
        128,
        midtone,
    );
    assert_eq!(
        unorm.at(4, 4),
        [128, 128, 128, 255],
        "a non-sRGB view keeps sRGB palette bytes unchanged"
    );

    let srgb = render_with(
        &context,
        &scene,
        wgpu::TextureFormat::Bgra8UnormSrgb,
        128,
        128,
        midtone,
    );
    let red = srgb.at(4, 4)[2];
    assert!(
        (180..=195).contains(&red),
        "an sRGB view applies another transfer encode to the paint value: {red}"
    );
}

#[test]
fn an_unusable_adapter_is_an_explicit_error() {
    match GpuContext::new() {
        Ok(_) => {}
        Err(RenderError::NoAdapter | RenderError::NoDevice { .. }) => {}
        Err(error) => panic!("unexpected GPU context error: {error}"),
    }
}
