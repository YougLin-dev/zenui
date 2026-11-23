//! 可见的原生菜单栏示例：File/View 菜单通过运行时命令路径修改窗口内容。

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use zenui::prelude::*;

    let mut runtime = Runtime::new();
    let status = runtime.setup(|| signal(String::from("Ready")))?;

    runtime.declare_windows(move |catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .titled("ZenUI Menu")
                .sized(560, 320),
            move |window| {
                window
                    .on_command("view.status", move |_| {
                        status.set(String::from("Command ran"));
                    })
                    .expect("the declared view.status registers");
                window
                    .on_command("file.close", move |_| {
                        window.request_close().expect("queue the close request");
                    })
                    .expect("the declared file.close registers");
                view! {
                    <view>
                        <text aria:label="status">{status}</text>
                    </view>
                }
            },
        )?;
        catalog.menu(
            "main",
            MenuSpec::new()
                .submenu("File", MenuSpec::new().command("file.close", "Close"))
                .submenu(
                    "View",
                    MenuSpec::new().command("view.status", "Show status"),
                ),
        )?;
        Ok(())
    })?;

    let mut host = zenui_windows::Host::open(runtime, zenui_windows::HostConfig::visible())?;
    host.run()?;
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
