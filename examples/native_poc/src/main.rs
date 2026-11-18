fn main() {
    if let Err(error) = zenui_native_poc::run() {
        eprintln!("GPU probe failed: {error}");
        std::process::exit(1);
    }
}
