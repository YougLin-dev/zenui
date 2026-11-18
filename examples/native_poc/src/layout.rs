//! Taffy/Parley adapter feasibility for the CURRENT taffy 0.15 signature.
//! This measures a paragraph in an available width and explicitly invalidates
//! it on content change. It does not claim full CSS bidi intrinsic sizing.

use crate::text;
use parley::{FontContext, LayoutContext};
use taffy::prelude::*;
use taffy::{AvailableSpace, LayoutOutput, compute_leaf_layout};

struct Paragraph {
    fonts: FontContext,
    contexts: LayoutContext<()>,
    text: &'static str,
    calls: usize,
}

impl Paragraph {
    fn measure(&mut self, width: Option<f32>) -> LayoutOutput {
        let mut layout = text::build_text(&mut self.fonts, &mut self.contexts, self.text, width);
        layout.break_all_lines(width);
        self.calls += 1;
        LayoutOutput {
            size: Size {
                width: layout.width(),
                height: layout.height(),
            },
            baselines: taffy::tree::Baselines::from_first(layout.first_baseline()),
            ..LayoutOutput::HIDDEN
        }
    }
}

pub fn measurement_probe() {
    let mut tree = TaffyTree::<Paragraph>::new();
    let leaf = tree
        .new_leaf_with_context(
            Style::default(),
            Paragraph {
                fonts: FontContext::new(),
                contexts: LayoutContext::new(),
                text: "short text",
                calls: 0,
            },
        )
        .expect("taffy text leaf");
    let size = Size {
        width: AvailableSpace::MaxContent,
        height: AvailableSpace::MaxContent,
    };
    compute(&mut tree, leaf, size);
    let wide = tree.layout(leaf).unwrap().size;
    assert!(wide.height > 0.);
    let initial_width = wide.width;
    let calls = tree.get_node_context(leaf).unwrap().calls;
    assert!(
        calls > 0,
        "the actual compute_leaf_layout measure invoked Parley"
    );

    compute(&mut tree, leaf, Size::MAX_CONTENT);
    assert!(
        !tree.dirty(leaf).unwrap(),
        "computing clears only this leaf's dirty state"
    );
    // set_node_context invalidates: no assumption that changing Parley text,
    // the font database or scale informs Taffy automatically.
    tree.set_node_context(
        leaf,
        Some(Paragraph {
            fonts: FontContext::new(),
            contexts: LayoutContext::new(),
            text: "changed content now spans multiple wrapped display lines",
            calls: 0,
        }),
    )
    .unwrap();
    assert!(
        tree.dirty(leaf).unwrap(),
        "context replacement must dirty layout"
    );
    compute(&mut tree, leaf, size);
    assert!(!tree.dirty(leaf).unwrap());
    let updated = tree.layout(leaf).unwrap().size;
    assert!(
        updated.width > initial_width && updated.height > 0.,
        "width must change with the new text: before={wide:?} after={updated:?}"
    );
    assert_eq!(
        tree.get_node_context(leaf).unwrap().calls,
        1,
        "the new layout was computed by a fresh paragraph build"
    );
}

fn compute(tree: &mut TaffyTree<Paragraph>, leaf: NodeId, available: Size<AvailableSpace>) {
    tree.compute_layout_with_measure(leaf, available, |inputs, _, context, style| {
        let paragraph = context.expect("leaf supplied its own text measurement context");
        // This callback must NOT mutate any retained UI ViewId; it produces a
        // temporary layout and explicit dimensions for Taffy's leaf algorithm.
        compute_leaf_layout(
            inputs,
            style,
            |_, _| 0.,
            |known, _| paragraph.measure(known.width).size,
        )
    })
    .expect("valid text layout");
}
