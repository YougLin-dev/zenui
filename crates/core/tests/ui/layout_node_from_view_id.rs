// `SPEC-GATE-A0-03`: the separation is mutual.
use zenui_core::{LayoutNodeId, ViewId};

fn takes_a_layout_node(_node: LayoutNodeId) {}

fn pass_a_view(view: ViewId) {
    takes_a_layout_node(view);
}

fn main() {
    let _ = pass_a_view;
}
