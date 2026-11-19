mod gpu;
#[cfg(test)]
mod layout;
mod text;

pub use gpu::run;

#[cfg(test)]
fn shaping(max_width: Option<f32>) -> parley::Layout<()> {
    let mut fonts = parley::FontContext::new();
    let mut contexts = parley::LayoutContext::new();
    text::build_layout(&mut fonts, &mut contexts, max_width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cjk_parley_produces_glyph_runs_and_software_pixels() {
        let layout = shaping(Some(80.));
        assert!(layout.len() > 1);
        let glyphs: Vec<_> = layout
            .lines()
            .flat_map(|line| line.items())
            .filter_map(|item| match item {
                parley::PositionedLayoutItem::GlyphRun(run) => Some(run),
                _ => None,
            })
            .flat_map(|run| run.positioned_glyphs().collect::<Vec<_>>())
            .collect();
        assert!(!glyphs.is_empty());
        assert!(glyphs.iter().any(|glyph| glyph.y > 0.));

        let mut resources = vello_cpu::Resources::new();
        let mut context = vello_cpu::RenderContext::new(text::WIDTH, text::HEIGHT);
        text::draw_cpu(&mut context, &mut resources, &layout);
        context.flush();
        let mut pixels = vello_cpu::Pixmap::new(text::WIDTH, text::HEIGHT);
        context.render(&mut pixels, &mut resources);
        assert!(pixels.data().iter().any(|pixel| pixel.a > 0));
    }

    #[test]
    fn complex_script_dictionary_actually_changes_word_boundaries() {
        let mut analysis = parley_engine::Analysis::default();
        parley_engine::Analyzer::new().analyze(
            "中文文本测试",
            &parley_engine::AnalysisOptions::default(),
            &mut analysis,
            &parley_engine::AnalysisDataSources::default(),
        );
        let boundaries: Vec<_> = analysis
            .char_info()
            .iter()
            .map(|c| c.is_word_boundary())
            .collect();
        assert_eq!(boundaries, [true, false, true, false, true, false]);
    }

    #[test]
    fn synthetic_bold_and_italic_use_fixed_font_outlines() {
        use parley::fontique::{Blob, Collection, CollectionOptions};
        use parley::{FontFamily, FontStyle, FontWeight, StyleProperty};
        use std::sync::Arc;

        fn render(style: FontStyle, weight: FontWeight, synthesize: bool) -> Vec<u32> {
            let mut collection = Collection::new(CollectionOptions {
                shared: false,
                system_fonts: false,
            });
            let registered = collection.register_fonts(
                Blob::new(Arc::new(include_bytes!("../assets/Tuffy.ttf").to_vec())),
                None,
            );
            assert_eq!(collection.family_names().collect::<Vec<_>>(), ["Tuffy"]);
            assert_eq!(registered.len(), 1);
            assert_eq!(registered[0].1.len(), 1);
            assert_eq!(registered[0].1[0].style(), FontStyle::Normal);
            let mut fonts = parley::FontContext {
                collection,
                source_cache: Default::default(),
            };
            let mut contexts = parley::LayoutContext::new();
            let mut builder = contexts.ranged_builder(&mut fonts, "I", 1., true);
            builder.push_default(FontFamily::named("Tuffy"));
            builder.push_default(StyleProperty::FontSize(64.));
            builder.push_default(StyleProperty::FontStyle(style));
            builder.push_default(StyleProperty::FontWeight(weight));
            let mut layout = builder.build("I");
            layout.break_all_lines(None);
            for run in layout
                .lines()
                .flat_map(|line| line.items())
                .filter_map(|item| match item {
                    parley::PositionedLayoutItem::GlyphRun(run) => Some(run),
                    _ => None,
                })
            {
                let synthesis = run.run().synthesis();
                assert_eq!(
                    synthesis.embolden(),
                    synthesize,
                    "style={style:?} weight={weight:?} synthesis={synthesis:?} font={:?}",
                    run.run().font()
                );
                assert_eq!(synthesis.skew().is_some(), style == FontStyle::Italic);
            }
            let mut resources = vello_cpu::Resources::new();
            let mut context = vello_cpu::RenderContext::new(128, 128);
            context.set_transform(vello_gpu::kurbo::Affine::translate((64., 0.)));
            text::draw_cpu(&mut context, &mut resources, &layout);
            context.flush();
            let mut pixels = vello_cpu::Pixmap::new(128, 128);
            context.render(&mut pixels, &mut resources);
            pixels
                .data()
                .iter()
                .map(|pixel| u32::from(pixel.a))
                .collect()
        }

        let regular = render(FontStyle::Normal, FontWeight::NORMAL, false);
        let bold = render(FontStyle::Normal, FontWeight::BLACK, true);
        let italic = render(FontStyle::Italic, FontWeight::NORMAL, false);
        assert!(!regular.iter().all(|alpha| *alpha == 0));
        assert!(
            regular.iter().sum::<u32>() < bold.iter().sum(),
            "faux bold adds outline coverage"
        );
        assert_ne!(
            regular, bold,
            "the raster exposes actual emboldened outlines"
        );
        assert_ne!(regular, italic, "the raster exposes skewed outlines");

        let center = |pixels: &[u32], top: usize, bottom: usize| {
            let (weight, sum) = (top..bottom)
                .flat_map(|y| (0..128).map(move |x| (y, x)))
                .fold((0_u32, 0_u64), |(weight, sum), (y, x)| {
                    let alpha = pixels[y * 128 + x];
                    (weight + alpha, sum + u64::from(alpha) * x as u64)
                });
            assert!(weight > 0);
            sum as f64 / f64::from(weight)
        };
        let rows = |pixels: &[u32]| {
            (0..128)
                .filter(|y| (0..128).any(|x| pixels[y * 128 + x] > 0))
                .collect::<Vec<_>>()
        };
        let min_y = *rows(&regular).first().unwrap();
        let max_y = *rows(&regular).last().unwrap();
        let quarter = (max_y - min_y + 1) / 4;
        let top_delta =
            center(&italic, min_y, min_y + quarter) - center(&regular, min_y, min_y + quarter);
        let center_y = (min_y + max_y) / 2;
        let mid_delta = center(&italic, center_y - 1, center_y + 2)
            - center(&regular, center_y - 1, center_y + 2);
        assert!(
            mid_delta >= 0. && mid_delta < top_delta,
            "top={top_delta} middle={mid_delta}"
        );
    }

    #[test]
    fn software_semi_transparency_is_premultiplied() {
        let cpu = gpu::transparent_cpu();
        assert_eq!(cpu, [128, 0, 0, 128]);
    }
}
