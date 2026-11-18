//! Headless sparse-strips validation. This is not a working UI window.
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
    fn taffy_measures_parley_and_invalidates_changed_text() {
        layout::measurement_probe();
    }

    #[test]
    fn software_semi_transparency_is_premultiplied() {
        let cpu = gpu::transparent_cpu();
        assert_eq!(cpu, [128, 0, 0, 128]);
    }
}
