// `SPEC-GATE-A0-02`: the authority has no owned `get`; a title read is
// `Tx::read_once`, which returns `Result<String, InvalidHandle>`.

use zenui_core::{RowId, Runtime};

fn owned_get(runtime: &Runtime, row: RowId) {
    let _ = runtime.get(row);
}

fn main() {
    let _ = owned_get;
}
