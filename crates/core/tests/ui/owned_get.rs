use zenui_core::{RowId, Runtime};

fn owned_get(runtime: &Runtime, row: RowId) {
    let _ = runtime.get(row);
}

fn main() {
    let _ = owned_get;
}
