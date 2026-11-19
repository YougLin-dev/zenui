use zenui_core::{RowKey, ViewId};

fn takes_row_key(_key: RowKey) {}

fn pass_a_view(view: ViewId) {
    takes_row_key(view);
}

fn main() {
    let _ = pass_a_view;
}
