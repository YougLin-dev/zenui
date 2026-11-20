#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut host = zenui_windows::Host::open(zenui_windows::HostConfig::visible("ZenUI Todo"))?;
    host.run()?;
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
