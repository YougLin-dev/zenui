//! 可见的托盘示例：左键恢复隐藏窗口，右键菜单运行应用级命令。
//!
//! `Close` 结束唯一窗口后托盘图标随最后一个窗口移除；托盘独存与最小化到
//! 托盘属于后续会话工单。

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use zenui::prelude::*;

    let mut runtime = Runtime::new();
    let (status, detail) = runtime.setup(|| {
        (
            signal(String::from("Window visible")),
            signal(String::from("Hide the window, then click the tray icon")),
        )
    })?;

    runtime.on_app_command("tray.reset", move |_| {
        status.set(String::from("Reset from the tray menu"))
    })?;
    runtime.on_app_command("tray.hide", move |event| {
        let Some(window) = event.window() else {
            return;
        };
        status.set(String::from("Hidden to the tray"));
        window.request_hide().expect("queue the hide request");
    })?;
    runtime.on_app_command("tray.close", |event| {
        if let Some(window) = event.window() {
            window.request_close().expect("queue the close request");
        }
    })?;

    runtime.declare_tray(
        TraySpec::new("ZenUI Tray").with_menu(
            MenuSpec::new()
                .command("tray.reset", "Reset status")
                .separator()
                .command("tray.hide", "Hide window")
                .command("tray.close", "Close"),
        ),
    )?;

    runtime.declare_windows(move |catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .titled("ZenUI Tray")
                .sized(560, 320),
            move |window| {
                let hide = window;
                view! {
                    <view>
                        <text aria:label="status">{status}</text>
                        <text aria:label="detail">{detail}</text>
                        <button
                            aria:label="Hide to tray"
                            on:click={move || {
                                status.set(String::from("Hidden to the tray"));
                                hide.request_hide().expect("queue the hide request");
                            }}
                        >"Hide to tray"</button>
                    </view>
                }
            },
        )?;
        Ok(())
    })?;

    let mut host = zenui_windows::Host::open(runtime, zenui_windows::HostConfig::visible())?;
    host.run()?;
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
