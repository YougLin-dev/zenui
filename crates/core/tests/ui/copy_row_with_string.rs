// `SPEC-GATE-A0-02`: a handle stays `Copy`, so it cannot carry the title.
// E0204: `Copy` cannot be implemented for a type that owns a `String`.

#[derive(Clone, Copy)]
struct CopyRow {
    title: String,
}

fn main() {
    let _ = std::mem::size_of::<CopyRow>();
}
