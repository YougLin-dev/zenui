//! 可见的原生菜单栏、快捷键与上下文菜单示例：命令修改共享状态，菜单再按状态
//! 显示勾选与禁用；窗口级上下文菜单与菜单栏共享命令处理器。

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use zenui::prelude::*;

    let mut runtime = Runtime::new();
    let (status, shown, has_content, draft) = runtime.setup(|| {
        (
            signal(String::from("Ready")),
            signal(true),
            signal(true),
            signal(String::new()),
        )
    })?;

    runtime.declare_windows(move |catalog| {
        catalog.window(
            WindowSpec::new("main")
                .primary()
                .titled("ZenUI Menu")
                .sized(560, 320),
            move |window| {
                window
                    .on_command("file.save", move |_| {
                        status.set(String::from("Saved"));
                    })
                    .expect("the declared file.save registers");
                window
                    .on_command("view.status", move |_| {
                        let next = !shown.get();
                        shown.set(next);
                        status.set(String::from(if next {
                            "Status shown"
                        } else {
                            "Status hidden"
                        }));
                    })
                    .expect("the declared view.status registers");
                window
                    .on_command("file.clear", move |_| {
                        has_content.set(false);
                        status.set(String::from("Cleared"));
                    })
                    .expect("the declared file.clear registers");
                window
                    .on_command("file.close", move |_| {
                        window.request_close().expect("queue the close request");
                    })
                    .expect("the declared file.close registers");
                window
                    .on_command("edit.copy", move |_| {
                        status.set(String::from("Copied"));
                    })
                    .expect("the declared edit.copy registers");
                window
                    .on_command("edit.paste", move |_| {
                        status.set(String::from("Pasted"));
                    })
                    .expect("the declared edit.paste registers");
                window
                    .on_command("edit.clear", move |_| {
                        draft.set(String::new());
                        status.set(String::from("Note cleared"));
                    })
                    .expect("the declared edit.clear registers");
                window
                    .on_menu_state(move |command| match command.id() {
                        "view.status" => MenuCommandState::new(true, shown.get()),
                        "file.clear" => MenuCommandState::new(has_content.get(), false),
                        _ => command.default_state(),
                    })
                    .expect("the menu state callback registers");
                view! {
                    <view>
                        <text aria:label="status">{status}</text>
                        <text-input
                            aria:label="note"
                            value=draft
                            on:input=move |value: String| draft.set(value)
                        />
                    </view>
                }
            },
        )?;
        catalog.menu(
            "main",
            MenuSpec::new()
                .submenu(
                    "File",
                    MenuSpec::new()
                        .item(MenuCommand::new("file.save", "Save").bind_shortcut("Ctrl+S")?)
                        .command("file.clear", "Clear")
                        .separator()
                        .command("file.close", "Close"),
                )
                .submenu(
                    "View",
                    MenuSpec::new().command("view.status", "Show status"),
                ),
        )?;
        catalog.context_menu(
            "main",
            MenuSpec::new()
                .command("edit.copy", "Copy")
                .command("edit.paste", "Paste")
                .separator()
                .command("edit.clear", "Clear note"),
        )?;
        Ok(())
    })?;

    let mut host = zenui_windows::Host::open(runtime, zenui_windows::HostConfig::visible())?;
    host.run()?;
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
