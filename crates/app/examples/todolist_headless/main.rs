// crates/app/examples/todolist_headless/main.rs
//!
//! `SPEC-GATE-A0-01` entry point:
//! `cargo run -p zenui-app --example todolist_headless --locked`
//!
//! The example prints the deterministic scenario trace to stdout and nothing
//! else, so two runs can be compared with `diff`.

fn main() {
    print!("{}", zenui_app::todolist_trace());
}
