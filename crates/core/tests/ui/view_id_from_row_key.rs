use zenui_core::{RowKey, ViewId};

fn takes_a_view(_view: ViewId) {}

fn pass_a_key(key: RowKey) {
    takes_a_view(key);
}

fn main() {
    let _ = pass_a_key;
}
