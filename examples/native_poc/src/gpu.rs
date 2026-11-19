use crate::text;
use parley::{FontContext, LayoutContext};
use vello_gpu::color::AlphaColor;
use vello_gpu::kurbo::Rect;
use vello_gpu::{
    ClearSettings, RenderOptions, RenderSize, RenderTarget, RenderTargetConfig, Scene, TargetInit,
    TextureBindings, TextureId,
};

const WIDTH: u32 = 128;
const HEIGHT: u32 = 64;
const ROW: u32 = 512;

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    pollster::block_on(async {
        let adapter = wgpu::Instance::default()
            .request_adapter(&Default::default())
            .await?;
        println!(
            "Adapter: {:?} / {}",
            adapter.get_info().name,
            adapter.get_info().backend
        );
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await?;
        let external = texture(
            &device,
            "external opaque viewport placeholder",
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let view = external.create_view(&Default::default());
        record_clear_viewport(&device, &queue, &view);
        let target = texture(
            &device,
            "sparse strips readback target",
            wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
        );
        let target_view = target.create_view(&Default::default());
        let (mut renderer, mut resources) = vello_gpu::Renderer::new(
            &device,
            &queue,
            &RenderTargetConfig {
                format: wgpu::TextureFormat::Rgba8Unorm,
                width: text::WIDTH,
                height: text::HEIGHT,
            },
        );
        let depth_view = renderer.create_depth_texture_view(&RenderSize {
            width: text::WIDTH,
            height: text::HEIGHT,
        });
        let mut bindings = TextureBindings::new();
        bindings.insert(TextureId(text::EXTERNAL_ID), view);
        let mut fonts = FontContext::new();
        let mut contexts = LayoutContext::new();
        let layout = text::build_layout(&mut fonts, &mut contexts, Some(80.));
        let mut scene = Scene::new(text::WIDTH, text::HEIGHT);
        scene.set_paint(AlphaColor::from_rgba8(15, 23, 42, 255));
        scene.fill_rect(&Rect::new(0., 0., WIDTH.into(), HEIGHT.into()));
        scene.set_paint(text::external_texture_paint());
        scene.fill_rect(&Rect::new(4., 4., 36., 36.));
        scene.set_transform(vello_gpu::kurbo::Affine::translate((40., 0.)));
        text::draw_gpu(&mut scene, &mut resources, &layout);
        scene.reset_transform();
        scene.set_paint(AlphaColor::from_rgba8(255, 0, 0, 128));
        scene.fill_rect(&Rect::new(0., 50., 20., 60.));
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(
            &scene,
            &mut resources,
            &mut encoder,
            RenderTarget {
                view: &target_view,
                depth_view: Some(&depth_view),
            },
            &bindings,
            render_options(),
        )?;
        queue.submit([encoder.finish()]);
        let pixels = readback(&target, &device, &queue);
        let at = |x, y| pixel(&pixels, x, y);
        assert_eq!(
            at(16, 20),
            [0, 255, 0, 255],
            "ordinary external texture image"
        );
        assert_eq!(at(127, 63), [15, 23, 42, 255], "opaque 2d background");
        assert!(
            (46..128)
                .flat_map(|x| (0..44).map(move |y| (x, y)))
                .filter(|(x, y)| at(*x, *y)[0] > 100)
                .count()
                > 20,
            "Parley glyph runs produce visible white glyph pixels"
        );
        let mixed = at(5, 55);
        assert!(
            mixed[0] > 100 && mixed[0] < 200 && mixed[3] == 255,
            "premultiplied overlay over opaque background, got {mixed:?}"
        );

        let mut alpha = Scene::new(text::WIDTH, text::HEIGHT);
        alpha.set_paint(AlphaColor::from_rgba8(255, 0, 0, 128));
        alpha.fill_rect(&Rect::new(0., 0., 16., 16.));
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(
            &alpha,
            &mut resources,
            &mut encoder,
            RenderTarget {
                view: &target_view,
                depth_view: Some(&depth_view),
            },
            &bindings,
            render_options(),
        )?;
        queue.submit([encoder.finish()]);
        let gpu_alpha = pixel(&readback(&target, &device, &queue), 8, 8);
        let cpu_alpha = transparent_cpu();
        assert_eq!(
            gpu_alpha, cpu_alpha,
            "GPU transparent target vs software premultiplied reference"
        );
        println!(
            "PASS: sparse-strips GPU, real Parley/Glifo CJK pixel runs, ordinary external texture binding."
        );
        println!("Transparent semi-red GPU pixel: {gpu_alpha:?}; CPU reference matches.");
        println!(
            "SCOPE: headless Windows RGBA graph; no glTF/window/IME/a11y/platform-font/platform-GPU/mask/filter/performance claim."
        );
        Ok(())
    })
}

fn render_options() -> RenderOptions<'static> {
    let mut options = RenderOptions::default();
    options.target_init = TargetInit::Clear(ClearSettings::Viewport {
        color: AlphaColor::TRANSPARENT,
    });
    options
}

pub fn transparent_cpu() -> [u8; 4] {
    let mut resources = vello_cpu::Resources::new();
    let mut ctx = vello_cpu::RenderContext::new(16, 16);
    ctx.set_paint(AlphaColor::from_rgba8(255, 0, 0, 128));
    ctx.fill_rect(&Rect::new(0., 0., 16., 16.));
    ctx.flush();
    let mut pixels = vello_cpu::Pixmap::new(16, 16);
    ctx.render(&mut pixels, &mut resources);
    let p = pixels.data()[0];
    [p.r, p.g, p.b, p.a]
}

fn texture(device: &wgpu::Device, label: &str, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage,
        view_formats: &[],
    })
}

fn record_clear_viewport(device: &wgpu::Device, queue: &wgpu::Queue, view: &wgpu::TextureView) {
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.,
                        g: 1.,
                        b: 0.,
                        a: 1.,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    queue.submit([encoder.finish()]);
}

fn readback(target: &wgpu::Texture, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u8> {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("native pixel readback"),
        size: u64::from(ROW) * u64::from(HEIGHT),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ROW),
                rows_per_image: Some(HEIGHT),
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.expect("map readback"));
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("GPU wait");
    let data = buffer
        .slice(..)
        .get_mapped_range()
        .expect("mapped bytes")
        .to_vec();
    buffer.unmap();
    data
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = (y * ROW + x * 4) as usize;
    pixels[i..i + 4].try_into().unwrap()
}
