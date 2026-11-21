#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = zenui::Runtime::new();
    zenui_demo::open_todo_window(&mut runtime)?;
    zenui_demo::seed(&mut runtime)?;
    let mut host = zenui_windows::Host::open(runtime, zenui_windows::HostConfig::visible())?;
    host.run()?;
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
