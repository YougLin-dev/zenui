use zenui_core::{LayoutNodeId, RowKey};

fn takes_a_layout_node(_node: LayoutNodeId) {}

fn pass_a_key(key: RowKey) {
    takes_a_layout_node(key);
}

fn main() {
    let _ = pass_a_key;
}
