//! Parley 文本整形、测量与 caret/selection 几何。

use std::collections::HashMap;

use parley::layout::ContentWidths;
use parley::layout::PositionedLayoutItem;
use parley::{Affinity, Cursor, FontContext, Layout, LayoutContext, Selection, StyleProperty};
use taffy::prelude::{AvailableSpace, Size as TaffySize};

use crate::geometry::{Point, Rect};
use crate::scene::{PositionedGlyph, TextRun};

/// 正文与控件的默认字号；所有几何使用逻辑像素。
pub(crate) const FONT_SIZE: f32 = 14.0;
/// 未声明字重时的默认值。
pub(crate) const FONT_WEIGHT: u16 = 400;
const LINE_HEIGHT: f32 = 1.35;

/// 一次整形使用的字号与字重。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TextStyle {
    pub(crate) size: f32,
    pub(crate) weight: u16,
}

impl TextStyle {
    pub(crate) const DEFAULT: Self = Self {
        size: FONT_SIZE,
        weight: FONT_WEIGHT,
    };
}

/// 显示文本中的字符范围；构造时规范化顺序。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CharRange {
    start: usize,
    end: usize,
}

impl CharRange {
    pub(crate) fn new(start: usize, end: usize) -> Self {
        if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        }
    }

    const fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub(crate) const fn start(self) -> usize {
        self.start
    }

    pub(crate) const fn end(self) -> usize {
        self.end
    }
}

pub(crate) struct TextShaper {
    fonts: FontContext,
    layouts: LayoutContext<()>,
    /// 未断行 layout 与内容宽度；键包含文本、字号与字重。
    metrics: HashMap<TextKey, TextMetrics>,
    /// 已按宽度断行的 layout；重复场景构建直接克隆，不重新整形。
    shaped: HashMap<ShapedKey, Layout<()>>,
    /// Taffy 会对同一 text leaf 以相同输入重复测量；直接复用尺寸结果。
    measured: HashMap<MeasureKey, MeasureResult>,
}

struct TextMetrics {
    layout: Layout<()>,
    content: ContentWidths,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct TextKey {
    text: String,
    size_bits: u32,
    weight: u16,
}

impl TextKey {
    fn new(text: &str, style: TextStyle) -> Self {
        Self {
            text: text.to_owned(),
            size_bits: style.size.to_bits(),
            weight: style.weight,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ShapedKey {
    text: TextKey,
    width_bits: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum AvailableKey {
    MinContent,
    MaxContent,
    Definite(u32),
}

impl AvailableKey {
    const fn new(available: AvailableSpace) -> Self {
        match available {
            AvailableSpace::MinContent => Self::MinContent,
            AvailableSpace::MaxContent => Self::MaxContent,
            AvailableSpace::Definite(width) => Self::Definite(width.to_bits()),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct MeasureKey {
    text: TextKey,
    width_bits: Option<u32>,
    available: AvailableKey,
}

#[derive(Clone, Copy)]
struct MeasureResult {
    size: TaffySize<f32>,
    baseline: f32,
}

/// 每个缓存的条目都包含完整 glyph 数据；达到上限时整体清空，避免无界增长。
const METRICS_LIMIT: usize = 8192;
const SHAPED_LIMIT: usize = 16384;
const MEASURE_LIMIT: usize = 65536;

impl TextShaper {
    pub(crate) fn new() -> Self {
        Self {
            fonts: FontContext::new(),
            layouts: LayoutContext::new(),
            metrics: HashMap::new(),
            shaped: HashMap::new(),
            measured: HashMap::new(),
        }
    }

    fn build(&mut self, text: &str, style: TextStyle) -> Layout<()> {
        let Self { fonts, layouts, .. } = self;
        let mut builder = layouts.ranged_builder(fonts, text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(style.size));
        builder.push_default(StyleProperty::FontWeight(parley::FontWeight::new(
            f32::from(style.weight),
        )));
        builder.push_default(parley::LineHeight::FontSizeRelative(LINE_HEIGHT));
        builder.build(text)
    }

    fn metrics(&mut self, text: &str, style: TextStyle) -> (Layout<()>, ContentWidths) {
        let key = TextKey::new(text, style);
        if !self.metrics.contains_key(&key) {
            let layout = self.build(text, style);
            let content = layout.calculate_content_widths();
            if self.metrics.len() >= METRICS_LIMIT {
                self.metrics.clear();
            }
            self.metrics
                .insert(key.clone(), TextMetrics { layout, content });
        }
        let metrics = self.metrics.get(&key).expect("metrics were just inserted");
        (metrics.layout.clone(), metrics.content)
    }

    fn remember_shaped(&mut self, key: ShapedKey, layout: &Layout<()>) {
        if self.shaped.len() >= SHAPED_LIMIT {
            self.shaped.clear();
        }
        self.shaped.insert(key, layout.clone());
    }

    pub(crate) fn shape(&mut self, text: &str, max_width: f32, style: TextStyle) -> Layout<()> {
        let width = max_width.max(0.0);
        let key = ShapedKey {
            text: TextKey::new(text, style),
            width_bits: width.to_bits(),
        };
        if let Some(layout) = self.shaped.get(&key) {
            return layout.clone();
        }
        let (mut layout, _) = self.metrics(text, style);
        layout.break_all_lines(Some(width));
        self.remember_shaped(key, &layout);
        layout
    }

    /// 供 Taffy leaf 测量使用；返回边框盒尺寸与首行 baseline。
    pub(crate) fn measure(
        &mut self,
        text: &str,
        dimensions: TaffySize<Option<f32>>,
        available: TaffySize<AvailableSpace>,
        style: TextStyle,
    ) -> (TaffySize<f32>, f32) {
        let key = MeasureKey {
            text: TextKey::new(text, style),
            width_bits: dimensions.width.map(f32::to_bits),
            available: AvailableKey::new(available.width),
        };
        if let Some(result) = self.measured.get(&key) {
            return (result.size, result.baseline);
        }
        let (mut layout, content) = self.metrics(text, style);
        let width = match available.width {
            AvailableSpace::MinContent => content.min,
            AvailableSpace::MaxContent => content.max,
            AvailableSpace::Definite(available_width) => {
                dimensions.width.unwrap_or(available_width)
            }
        };
        layout.break_all_lines(Some(width));
        // emit 通常以最终布局宽度再次 shape；把这次断行结果留下。
        let shaped_key = ShapedKey {
            text: TextKey::new(text, style),
            width_bits: width.to_bits(),
        };
        self.remember_shaped(shaped_key, &layout);
        let intrinsic_width = match available.width {
            AvailableSpace::MinContent => content.min,
            AvailableSpace::MaxContent => content.max,
            AvailableSpace::Definite(width) if dimensions.width.is_none() => {
                layout.width().max(content.min.min(width))
            }
            AvailableSpace::Definite(_) => layout.width(),
        };
        // Taffy 把最终布局尺寸取整到整像素；向上取整测量结果，避免下一轮
        // 以少一个像素的宽度重新断行。
        let intrinsic_width = intrinsic_width.ceil();
        let height = layout.height();
        let baseline = layout.first_baseline().unwrap_or(height);
        let result = MeasureResult {
            size: TaffySize {
                width: intrinsic_width,
                height,
            },
            baseline,
        };
        if self.measured.len() >= MEASURE_LIMIT {
            self.measured.clear();
        }
        self.measured.insert(key, result);
        (result.size, result.baseline)
    }
}

/// 把已断行的段落转换成绝对坐标的 glyph run。
pub(crate) fn runs(layout: &Layout<()>, origin: Point) -> Vec<TextRun> {
    let mut runs = Vec::new();
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let run = glyph_run.run();
            runs.push(TextRun::new(
                run.font().clone(),
                run.font_size(),
                run.normalized_coords()
                    .iter()
                    .map(|coord| coord.to_bits())
                    .collect(),
                origin.y + glyph_run.baseline(),
                glyph_run
                    .positioned_glyphs()
                    .map(|glyph| PositionedGlyph {
                        id: glyph.id,
                        x: origin.x + glyph.x,
                        y: origin.y + glyph.y,
                        advance: glyph.advance,
                    })
                    .collect(),
            ));
        }
    }
    runs
}

/// caret 的字符偏移按显示文本计；Parley 的索引是 UTF-8 字节。
pub(crate) fn caret(layout: &Layout<()>, origin: Point, text: &str, char_offset: usize) -> Rect {
    caret_at_byte(layout, origin, byte_of_char(text, char_offset))
}

/// 显示文本每个字符边界（含两端）的 caret 矩形，索引是字符偏移。
pub(crate) fn caret_stops(layout: &Layout<()>, origin: Point, text: &str) -> Vec<(usize, Rect)> {
    let mut boundaries: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
    boundaries.push(text.len());
    boundaries
        .into_iter()
        .enumerate()
        .map(|(offset, byte)| (offset, caret_at_byte(layout, origin, byte)))
        .collect()
}

fn caret_at_byte(layout: &Layout<()>, origin: Point, byte: usize) -> Rect {
    let geometry =
        Cursor::from_byte_index(layout, byte, Affinity::Downstream).geometry(layout, 1.0);
    let rect = to_rect(geometry, origin);
    if rect.width > 0.0 && rect.height > 0.0 {
        rect
    } else {
        fallback_caret(layout, origin)
    }
}

/// 一行或跨行的 selection 矩形；折叠的 selection 没有几何。
pub(crate) fn selection(
    layout: &Layout<()>,
    origin: Point,
    text: &str,
    range: CharRange,
) -> Vec<Rect> {
    if range.is_empty() {
        return Vec::new();
    }
    let anchor = Cursor::from_byte_index(
        layout,
        byte_of_char(text, range.start),
        Affinity::Downstream,
    );
    let focus =
        Cursor::from_byte_index(layout, byte_of_char(text, range.end), Affinity::Downstream);
    Selection::new(anchor, focus)
        .geometry(layout)
        .into_iter()
        .map(|(geometry, _line)| to_rect(geometry, origin))
        .filter(|rect| rect.width > 0.0 && rect.height > 0.0)
        .collect()
}

fn byte_of_char(text: &str, char_offset: usize) -> usize {
    text.char_indices()
        .nth(char_offset)
        .map_or(text.len(), |(byte, _)| byte)
}

fn to_rect(geometry: parley::BoundingBox, origin: Point) -> Rect {
    Rect::new(
        origin.x + geometry.x0 as f32,
        origin.y + geometry.y0 as f32,
        (geometry.x1 - geometry.x0) as f32,
        (geometry.y1 - geometry.y0) as f32,
    )
}

/// 空段落可能没有可用于几何的 cluster；用首行行高给出一个可见 caret。
fn fallback_caret(layout: &Layout<()>, origin: Point) -> Rect {
    let height = layout
        .lines()
        .next()
        .map_or(FONT_SIZE * LINE_HEIGHT, |line| line.metrics().line_height);
    Rect::new(origin.x, origin.y, 1.0, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_widths_land_on_the_pixel_grid() {
        let mut shaper = TextShaper::new();
        let layout = shaper.build("Load sample", TextStyle::DEFAULT);
        let max_content = layout.calculate_content_widths().max;
        assert!(
            max_content.fract() != 0.0,
            "the fixture has a fractional width"
        );

        let (size, _) = shaper.measure(
            "Load sample",
            TaffySize {
                width: None,
                height: None,
            },
            TaffySize {
                width: AvailableSpace::MaxContent,
                height: AvailableSpace::MaxContent,
            },
            TextStyle::DEFAULT,
        );
        assert_eq!(size.width, max_content.ceil());
        assert!(
            size.width >= max_content,
            "the measured width never rounds below the line"
        );
    }

    #[test]
    fn shaped_layouts_are_reused_for_the_same_text_width_and_style() {
        let mut shaper = TextShaper::new();
        let first = shaper.shape("hello", 100.0, TextStyle::DEFAULT);
        assert_eq!(shaper.shaped.len(), 1);
        let cached = shaper.shape("hello", 100.0, TextStyle::DEFAULT);
        assert_eq!(shaper.shaped.len(), 1, "an identical shape is a cache hit");
        assert_eq!(
            (first.width(), first.height()),
            (cached.width(), cached.height())
        );

        shaper.shape("hello", 90.0, TextStyle::DEFAULT);
        assert_eq!(shaper.shaped.len(), 2, "a different width is a new entry");
        shaper.shape(
            "hello",
            100.0,
            TextStyle {
                size: 16.0,
                weight: 400,
            },
        );
        assert_eq!(shaper.shaped.len(), 3, "a different style is a new entry");
    }

    #[test]
    fn repeated_measurements_reuse_the_cached_result() {
        let mut shaper = TextShaper::new();
        let dimensions = TaffySize {
            width: Some(180.0),
            height: None,
        };
        let available = TaffySize {
            width: AvailableSpace::Definite(180.0),
            height: AvailableSpace::MaxContent,
        };
        let first = shaper.measure("hello world", dimensions, available, TextStyle::DEFAULT);
        assert_eq!(shaper.measured.len(), 1);
        let second = shaper.measure("hello world", dimensions, available, TextStyle::DEFAULT);
        assert_eq!(
            shaper.measured.len(),
            1,
            "an identical measure is a cache hit"
        );
        assert_eq!(first, second);

        let wider = TaffySize {
            width: AvailableSpace::Definite(240.0),
            height: AvailableSpace::MaxContent,
        };
        shaper.measure("hello world", dimensions, wider, TextStyle::DEFAULT);
        assert_eq!(shaper.measured.len(), 2, "a different width is a new entry");
    }

    #[test]
    fn a_non_collapsed_selection_produces_ordered_rects() {
        let mut shaper = TextShaper::new();
        let layout = shaper.shape("hello world", 200.0, TextStyle::DEFAULT);
        let origin = Point::new(10.0, 20.0);
        let rects = selection(&layout, origin, "hello world", CharRange::new(1, 4));
        assert!(!rects.is_empty(), "a selection has geometry");
        assert!(
            rects
                .iter()
                .all(|rect| rect.width > 0.0 && rect.height > 0.0)
        );
        let caret = caret(&layout, origin, "hello world", 1);
        assert!(rects[0].x >= caret.x, "selection starts at its first char");

        let reversed = selection(&layout, origin, "hello world", CharRange::new(4, 1));
        assert_eq!(rects, reversed, "the range is normalized");
        assert!(
            selection(&layout, origin, "hello world", CharRange::new(2, 2)).is_empty(),
            "a collapsed selection has no highlight"
        );
    }
}
