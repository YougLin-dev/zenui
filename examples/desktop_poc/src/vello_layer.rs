use crate::{Target, TargetSize};
use vello::peniko::Color;

pub fn solid_rect(size: TargetSize, rgba: [u8; 4]) -> vello::Scene {
    scene(
        size,
        [0.0, 0.0, size.width as f64, size.height as f64],
        rgba,
    )
}

pub fn semi_transparent_rect(size: TargetSize, rgba: [u8; 4]) -> vello::Scene {
    scene(
        size,
        [8.0, 8.0, (size.width - 8) as f64, (size.height - 8) as f64],
        rgba,
    )
}

fn scene(_size: TargetSize, rect: [f64; 4], rgba: [u8; 4]) -> vello::Scene {
    let mut scene = vello::Scene::new();
    scene.fill(
        vello::peniko::Fill::NonZero,
        vello::kurbo::Affine::IDENTITY,
        Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]),
        None,
        &vello::kurbo::Rect::new(rect[0], rect[1], rect[2], rect[3]),
    );
    scene
}

pub fn paint(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut vello::Renderer,
    target: &Target,
    scene: &vello::Scene,
    size: TargetSize,
) {
    renderer
        .render_to_texture(
            device,
            queue,
            scene,
            &target.view,
            &vello::RenderParams {

                base_color: Color::TRANSPARENT,
                width: size.width,
                height: size.height,
                antialiasing_method: vello::AaConfig::Area,
            },
        )
        .expect("Vello writes intermediate target");
}
