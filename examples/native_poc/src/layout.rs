use crate::text;
use parley::{FontContext, LayoutContext};
use taffy::prelude::*;
use taffy::{AvailableSpace, LayoutInput, LayoutOutput, ResolveOrZero, compute_leaf_layout};

struct Measurement {
    layout: parley::Layout<()>,
    output: LayoutOutput,
}

struct Paragraph {
    fonts: FontContext,
    contexts: LayoutContext<()>,
    text: &'static str,
    calls: usize,
    accepted: Option<parley::Layout<()>>,
}

impl Paragraph {
    fn new(text: &'static str) -> Self {
        Self {
            fonts: FontContext::new(),
            contexts: LayoutContext::new(),
            text,
            calls: 0,
            accepted: None,
        }
    }

    fn measure(
        &mut self,
        dimensions: Size<Option<f32>>,
        available: Size<AvailableSpace>,
    ) -> Measurement {
        let mut layout = text::build_text(&mut self.fonts, &mut self.contexts, self.text);
        let content = layout.calculate_content_widths();
        let width = match available.width {
            AvailableSpace::MinContent => content.min,
            AvailableSpace::MaxContent => content.max,
            AvailableSpace::Definite(available_width) => {
                dimensions.width.unwrap_or(available_width)
            }
        };
        layout.break_all_lines(Some(width));
        let intrinsic_width = match available.width {
            AvailableSpace::MinContent => content.min,
            AvailableSpace::MaxContent => content.max,
            AvailableSpace::Definite(width) if dimensions.width.is_none() => {
                layout.width().max(content.min.min(width))
            }
            AvailableSpace::Definite(_) => layout.width(),
        };
        self.calls += 1;
        let height = layout.height();
        let baseline = layout.first_baseline().or(height.into());
        Measurement {
            layout,
            output: LayoutOutput {
                size: Size {
                    width: intrinsic_width,
                    height,
                },
                baselines: taffy::tree::Baselines::from_first(baseline),
                ..LayoutOutput::HIDDEN
            },
        }
    }
}

fn measure_leaf(
    inputs: LayoutInput,
    context: Option<&mut Paragraph>,
    style: &Style,
) -> LayoutOutput {
    let paragraph = context.expect("text leaf context");
    let measured = paragraph.measure(inputs.known_dimensions, inputs.available_space);
    paragraph.accepted = Some(measured.layout);
    let mut output = compute_leaf_layout(inputs, style, |_, _| 0., |_, _| measured.output.size);
    // Taffy only has a size measurement callback and clears this field internally.
    output.baselines = measured.output.baselines;
    let border = style
        .border
        .resolve_or_zero(inputs.parent_size.width, |_, _| 0.);
    let padding = style
        .padding
        .resolve_or_zero(inputs.parent_size.width, |_, _| 0.);
    if let Some(baseline) = output.baselines.first.as_mut() {
        *baseline += border.top + padding.top;
    }
    output
}

fn compute(
    tree: &mut TaffyTree<Paragraph>,
    leaf: NodeId,
    available: Size<AvailableSpace>,
) -> Option<LayoutOutput> {
    let mut output = None;
    tree.compute_layout_with_measure(leaf, available, |inputs, _, context, style| {
        let measured = measure_leaf(inputs, context, style);
        output = Some(measured.clone());
        measured
    })
    .unwrap();
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taffy_measures_parley_and_invalidates_changed_text() {
        let mut tree = TaffyTree::<Paragraph>::new();
        let leaf = tree
            .new_leaf_with_context(Style::default(), Paragraph::new("short text"))
            .expect("taffy text leaf");
        let size = Size {
            width: AvailableSpace::MaxContent,
            height: AvailableSpace::MaxContent,
        };
        let first = compute(&mut tree, leaf, size).expect("first measurement");
        assert_eq!(
            first.baselines.first,
            tree.get_node_context(leaf)
                .unwrap()
                .accepted
                .as_ref()
                .unwrap()
                .first_baseline()
        );
        let wide = tree.layout(leaf).unwrap().size;
        assert!(wide.height > 0.);
        let calls = tree.get_node_context(leaf).unwrap().calls;
        assert_eq!(calls, 1, "the actual measure function invokes Parley");

        let cached = compute(&mut tree, leaf, size);
        assert_eq!(tree.get_node_context(leaf).unwrap().calls, 1);
        let cached = cached.unwrap_or(LayoutOutput {
            size: tree.layout(leaf).unwrap().size,
            ..LayoutOutput::HIDDEN
        });
        assert_eq!(cached.size, wide, "cached output preserves measured size");
        assert!(!tree.dirty(leaf).unwrap());

        tree.set_node_context(
            leaf,
            Some(Paragraph::new(
                "changed content now spans multiple wrapped display lines",
            )),
        )
        .unwrap();
        assert!(tree.dirty(leaf).unwrap());
        compute(&mut tree, leaf, size);
        let updated = tree.layout(leaf).unwrap().size;
        assert!(updated.width > wide.width && updated.height > 0.);
    }

    #[test]
    fn known_size_retains_the_accepted_paragraphs_baseline() {
        let mut tree = TaffyTree::<Paragraph>::new();
        let leaf = tree
            .new_leaf_with_context(
                Style {
                    size: Size {
                        width: length(50.),
                        height: length(80.),
                    },
                    ..Style::default()
                },
                Paragraph::new("fixed-size text"),
            )
            .expect("text leaf");
        let output = compute(&mut tree, leaf, Size::MAX_CONTENT).expect("known-size measurement");
        let paragraph = tree.get_node_context(leaf).unwrap();
        let accepted = paragraph.accepted.as_ref().unwrap();
        assert_eq!(
            paragraph.calls, 1,
            "known size must shape a paragraph for paint and caret"
        );
        assert_eq!(output.baselines.first, accepted.first_baseline());
        let baseline = output.baselines.first.unwrap();
        assert!(baseline > 0. && baseline <= 80.);
        assert!(accepted.layout_max_advance() >= 50.);
    }

    #[test]
    fn a_parent_aligns_cached_paragraph_baselines() {
        let mut tree = TaffyTree::<Paragraph>::new();
        tree.disable_rounding();
        let style = |padding| Style {
            size: Size {
                width: length(70.),
                height: length(60.),
            },
            padding: Rect {
                top: length(padding),
                ..Rect {
                    left: length(0.),
                    right: length(0.),
                    top: length(0.),
                    bottom: length(0.),
                }
            },
            align_self: Some(AlignItems::BASELINE),
            ..Style::default()
        };
        let first = tree
            .new_leaf_with_context(style(0.), Paragraph::new("first"))
            .unwrap();
        let second = tree
            .new_leaf_with_context(style(18.), Paragraph::new("second"))
            .unwrap();
        let root = tree
            .new_with_children(
                Style {
                    display: Display::Flex,
                    ..Style::default()
                },
                &[first, second],
            )
            .unwrap();
        let compute_parent = |tree: &mut TaffyTree<Paragraph>| {
            tree.compute_layout_with_measure(
                root,
                Size {
                    width: AvailableSpace::Definite(200.),
                    height: AvailableSpace::MaxContent,
                },
                |inputs, _, context, style| measure_leaf(inputs, context, style),
            )
            .unwrap();
            assert!(
                ((tree.layout(first).unwrap().location.y
                    + tree
                        .get_node_context(first)
                        .unwrap()
                        .accepted
                        .as_ref()
                        .unwrap()
                        .first_baseline()
                        .unwrap())
                    - (tree.layout(second).unwrap().location.y
                        + tree
                            .get_node_context(second)
                            .unwrap()
                            .accepted
                            .as_ref()
                            .unwrap()
                            .first_baseline()
                            .unwrap()
                        + 18.))
                    .abs()
                    < f32::EPSILON
            );
        };
        compute_parent(&mut tree);
        let calls = tree.get_node_context(first).unwrap().calls;
        compute_parent(&mut tree);
        assert_eq!(
            tree.get_node_context(first).unwrap().calls,
            calls,
            "cached parent retains the full child output"
        );
    }

    #[test]
    fn intrinsic_modes_use_content_widths_not_zero_wrapping() {
        let mut paragraph = Paragraph::new("several different words");
        let min = paragraph.measure(
            Size::NONE,
            Size {
                width: AvailableSpace::MinContent,
                height: AvailableSpace::MaxContent,
            },
        );
        let max = paragraph.measure(Size::NONE, Size::MAX_CONTENT);
        let constrained = paragraph.measure(
            Size::NONE,
            Size {
                width: AvailableSpace::Definite(80.),
                height: AvailableSpace::MaxContent,
            },
        );
        assert_eq!(
            min.output.size.width,
            min.layout.calculate_content_widths().min
        );
        assert_eq!(
            max.output.size.width,
            max.layout.calculate_content_widths().max
        );
        assert!(min.output.size.width > 0. && min.output.size.width < max.output.size.width);
        assert!(
            constrained.output.size.width >= min.output.size.width,
            "an unbreakable word may overflow the requested width"
        );
        assert!(constrained.layout.len() > max.layout.len());

        let known = paragraph.measure(
            Size {
                width: Some(50.),
                height: None,
            },
            Size {
                width: AvailableSpace::Definite(86.),
                height: AvailableSpace::MaxContent,
            },
        );
        assert!(known.layout.width() >= min.output.size.width);
        assert!(known.layout.width() < max.output.size.width);
        assert_eq!(known.layout.layout_max_advance(), 50.);
        assert!(known.layout.len() <= constrained.layout.len());
    }
}
