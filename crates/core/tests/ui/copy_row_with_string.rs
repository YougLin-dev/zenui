#[derive(Clone, Copy)]
struct CopyRow {
    title: String,
}

fn main() {
    let _ = std::mem::size_of::<CopyRow>();
}
