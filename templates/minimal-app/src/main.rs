//! 原生入口；debug 构建保留控制台便于诊断，release 构建不附带控制台窗口。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    if let Err(error) = run() {
        zenui_windows::show_startup_error(minimal_app::STARTER_TITLE, &error.to_string());
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = zenui::Runtime::new();
    minimal_app::open_starter_window(&mut runtime)?;
    let config =
        zenui_windows::HostConfig::visible().with_text_assets(minimal_app::load_text_assets()?);
    let mut host = zenui_windows::Host::open(runtime, config)?;
    host.run()?;
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("ZenUI Starter needs the Windows host; run `cargo test` for the headless check.");
}
