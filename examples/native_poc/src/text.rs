//! Real Parley/Glifo adapter with the current NormalizedCoord conversion.
//! Font fallback is inspected on the current host, not a portable promise.
use glifo::Glyph;
use parley::{FontContext, Layout, LayoutContext, PositionedLayoutItem, StyleProperty};
use vello_gpu::{Image, ImageSource, TextureId, TextureRegion};

pub const WIDTH: u16 = 128;
pub const HEIGHT: u16 = 64;
pub const EXTERNAL_ID: u64 = 7;
pub const TEXT: &str = "Zenui 中文 wrap";

pub fn build_text(
    fonts: &mut FontContext,
    contexts: &mut LayoutContext<()>,
    content: &str,
    max_width: Option<f32>,
) -> Layout<()> {
    let mut builder = contexts.ranged_builder(fonts, content, 1., true);
    builder.push_default(StyleProperty::FontSize(24.));
    builder.push_default(parley::LineHeight::FontSizeRelative(1.25));
    let mut layout = builder.build(content);
    layout.break_all_lines(max_width);
    layout
}

pub fn build_layout(
    fonts: &mut FontContext,
    contexts: &mut LayoutContext<()>,
    max_width: Option<f32>,
) -> Layout<()> {
    build_text(fonts, contexts, TEXT, max_width)
}

pub fn draw_gpu(
    scene: &mut vello_gpu::Scene,
    resources: &mut vello_gpu::Resources,
    layout: &Layout<()>,
) {
    scene.set_paint(vello_gpu::color::palette::css::WHITE);
    for line in layout.lines() {
        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(run) = item {
                // Parley's value and glifo's value use the same documented bits.
                let coords: Vec<i16> = run
                    .run()
                    .normalized_coords()
                    .iter()
                    .map(|coord| coord.to_bits())
                    .collect();
                let glyphs = run.positioned_glyphs().map(|glyph| Glyph {
                    id: glyph.id,
                    x: glyph.x,
                    y: glyph.y,
                });
                scene
                    .glyph_run(resources, run.run().font())
                    .font_size(run.run().font_size())
                    .normalized_coords(&coords)
                    .atlas_cache(false)
                    .fill_glyphs(glyphs)
                    .expect("glyph run rasterization succeeded in probe");
            }
        }
    }
}

#[cfg(test)]
pub fn draw_cpu(
    context: &mut vello_cpu::RenderContext,
    resources: &mut vello_cpu::Resources,
    layout: &Layout<()>,
) {
    context.set_paint(vello_gpu::color::palette::css::WHITE);
    for line in layout.lines() {
        for item in line.items() {
            if let PositionedLayoutItem::GlyphRun(run) = item {
                let coords: Vec<i16> = run
                    .run()
                    .normalized_coords()
                    .iter()
                    .map(|coord| coord.to_bits())
                    .collect();
                let glyphs = run.positioned_glyphs().map(|glyph| Glyph {
                    id: glyph.id,
                    x: glyph.x,
                    y: glyph.y,
                });
                context
                    .glyph_run(resources, run.run().font())
                    .font_size(run.run().font_size())
                    .normalized_coords(&coords)
                    .atlas_cache(false)
                    .fill_glyphs(glyphs)
                    .expect("glyph run rasterization succeeded in probe");
            }
        }
    }
}

pub fn external_texture_paint() -> Image {
    Image {
        image: ImageSource::external_texture(TextureId(EXTERNAL_ID), TextureRegion::Full, false),
        sampler: vello_gpu::peniko::ImageSampler::default(),
    }
}
