// `SPEC-GATE-A0-03`: the separation is mutual.
use zenui_core::{LayoutNodeId, ViewId};

fn takes_a_view(_view: ViewId) {}

fn pass_a_layout_node(node: LayoutNodeId) {
    takes_a_view(node);
}

fn main() {
    let _ = pass_a_layout_node;
}
