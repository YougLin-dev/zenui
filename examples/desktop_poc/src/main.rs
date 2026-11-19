mod compositor;
mod vello_layer;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone, Copy)]
struct TargetSize {
    width: u32,
    height: u32,
}

impl TargetSize {
    const ROW_SIZE: u32 = 64 * 4;
    fn extent(self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        }
    }
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Target {
    // 此原型需要外部渲染、自身合成和 readback，不对应实际 Swapchain。
    fn intermediate(device: &wgpu::Device, label: &str, size: TargetSize, readback: bool) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: size.extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,

            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | if readback {
                    wgpu::TextureUsages::COPY_SRC
                } else {
                    wgpu::TextureUsages::empty()
                },
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view }
    }

    fn readback(&self, device: &wgpu::Device, queue: &wgpu::Queue, size: TargetSize) -> Vec<u8> {
        let row_size = TargetSize::ROW_SIZE as wgpu::BufferAddress;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: row_size * self.texture.height() as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(TargetSize::ROW_SIZE),
                    rows_per_image: Some(self.texture.height()),
                },
            },
            size.extent(),
        );
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| ());
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("GPU readback completed");
        readback
            .slice(..)
            .get_mapped_range()
            .expect("GPU readback mapped")
            .to_vec()
    }
}

fn pixel(image: &[u8], x: u32, y: u32) -> [u8; 4] {
    let start = (y * WIDTH + x) as usize * 4;
    image[start..start + 4].try_into().unwrap()
}

fn assert_pixel(
    image: &[u8],
    x: u32,
    y: u32,
    expected: [u8; 4],
    small_rounding_tolerance: bool,
    context: &str,
) {
    let actual = pixel(image, x, y);
    if small_rounding_tolerance {
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(&a, b)| a == b || a + 1 == b || a == b + 1),
            "{context}: {actual:?}, expected {expected:?} +/- 1 from RGBA8 quantization"
        );
    } else {
        assert_eq!(actual, expected, "{context}");
    }
}

fn triangle(device: &wgpu::Device, queue: &wgpu::Queue, target: &Target) {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("external renderer probe"),
        source: wgpu::ShaderSource::Wgsl(
            r#"
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = array<vec2f, 3>(vec2f(-0.5, -0.5), vec2f(0.5, -0.5), vec2f(0., 0.75));
    return vec4f(p[i], 0., 1.);
}
@fragment fn fs() -> @location(0) vec4f { return vec4f(1., 0., 0., 1.); }
"#
            .into(),
        ),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("foreign renderer"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("external pass overlays while retaining 2D"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.draw(0..3, 0..1);
    }

    queue.submit([encoder.finish()]);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    pollster::block_on(async {
        let size = TargetSize {
            width: WIDTH,
            height: HEIGHT,
        };
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
        let mut vello = vello::Renderer::new(&device, vello::RendererOptions::default())?;
        let base = Target::intermediate(&device, "2d underlay and 3d", size, true);
        let overlay = Target::intermediate(&device, "2d overlay", size, true);
        let output = Target::intermediate(&device, "presentation output", size, true);

        let underlay = vello_layer::solid_rect(size, [0, 0, 255, 255]);
        vello_layer::paint(&device, &queue, &mut vello, &base, &underlay, size);
        assert_pixel(
            &base.readback(&device, &queue, size),
            2,
            2,
            [0, 0, 255, 255],
            false,
            "2d underlay",
        );

        triangle(&device, &queue, &base);
        let foreign = base.readback(&device, &queue, size);
        assert_pixel(
            &foreign,
            32,
            32,
            [255, 0, 0, 255],
            false,
            "external triangle after LoadOp::Load",
        );
        assert_pixel(
            &foreign,
            2,
            2,
            [0, 0, 255, 255],
            false,
            "external pass preserves neighbors",
        );

        let above = vello_layer::semi_transparent_rect(size, [255, 0, 0, 128]);
        vello_layer::paint(&device, &queue, &mut vello, &overlay, &above, size);
        let raw = overlay.readback(&device, &queue, size);
        assert_pixel(&raw, 2, 2, [0, 0, 0, 0], false, "transparent region");
        assert_pixel(
            &raw,
            24,
            24,
            [255, 0, 0, 128],
            false,
            "straight-alpha overlay primitive",
        );

        let compositor = compositor::Compositor::new(&device, FORMAT);
        compositor.paint(&device, &queue, &base, &overlay, &output);
        let result = output.readback(&device, &queue, size);
        assert_pixel(
            &result,
            2,
            2,
            [0, 0, 255, 255],
            false,
            "composite does not paint transparent areas",
        );
        assert_pixel(
            &result,
            32,
            32,
            [255, 0, 0, 255],
            false,
            "composite over 3d",
        );
        assert_pixel(
            &result,
            24,
            24,
            [128, 0, 127, 255],
            true,
            "straight-alpha overlay over 2d",
        );
        println!(
            "PASS: Vello underlay, foreign wgpu LoadOp::Load, and straight-alpha overlay compositing."
        );
        println!(
            "SCOPE: Windows headless RGBA texture graph only; no glTF, window, IME, platform GPU matrix, or benchmark claim."
        );
        Ok(())
    })
}
