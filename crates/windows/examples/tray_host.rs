//! 可见的常驻托盘示例：隐藏或关闭最后一个窗口后会话继续，托盘恢复窗口，
//! 菜单里的 Quit 显式结束会话。
//!
//! 关闭按钮只释放 activation；tray 的 Show window 或左键点击重新运行 factory。
//! 托盘图标在会话结束后移除。

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use zenui::prelude::*;

    let mut runtime = Runtime::new();
    runtime.declare_session(SessionPolicy::StayResident)?;
    let session = runtime.session();
    let (status, detail) = runtime.setup(|| {
        (
            signal(String::from("Window visible")),
            signal(String::from("Hide or close the window, then use the tray")),
        )
    })?;

    runtime.on_app_command("tray.show", move |_| {
        status.set(String::from("Restored from the tray"));
        session.request_show("main").expect("queue the show");
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
    runtime.on_app_command("tray.close", move |event| {
        let Some(window) = event.window() else {
            return;
        };
        status.set(String::from("Closed to the tray"));
        window.request_close().expect("queue the close request");
    })?;
    runtime.on_app_command("tray.quit", move |_| {
        session.request_quit().expect("queue the quit");
    })?;

    runtime.declare_tray(
        TraySpec::new("ZenUI Tray").with_menu(
            MenuSpec::new()
                .command("tray.show", "Show window")
                .command("tray.reset", "Reset status")
                .separator()
                .command("tray.hide", "Hide window")
                .command("tray.close", "Close window")
                .separator()
                .command("tray.quit", "Quit"),
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
                        <button
                            aria:label="Close to tray"
                            on:click={move || {
                                status.set(String::from("Closed to the tray"));
                                window.request_close().expect("queue the close request");
                            }}
                        >"Close to tray"</button>
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
