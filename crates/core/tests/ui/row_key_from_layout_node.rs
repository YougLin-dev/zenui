// `SPEC-GATE-A0-03`: identity domains do not convert into each other.
use zenui_core::{LayoutNodeId, RowKey};

fn takes_row_key(_key: RowKey) {}

fn pass_a_layout_node(node: LayoutNodeId) {
    takes_row_key(node);
}

fn main() {
    let _ = pass_a_layout_node;
}
