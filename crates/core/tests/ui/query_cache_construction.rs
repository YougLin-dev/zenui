// `ADR-0022` decision 5: the cache is not constructible from outside the
// crate. Removing the re-export was not enough — the module itself is private.

fn main() {
    let _ = zenui_core::query::QueryCache::new();
}
