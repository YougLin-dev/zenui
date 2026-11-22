//! 最小 ZenUI 应用：一个计数器组件与一个原生窗口。

#![allow(
    unused_braces,
    reason = "{} is the intentionally supported event-expression syntax"
)]

use zenui::prelude::*;

/// 计数器窗口的目录键。
pub const STARTER_WINDOW: &str = "main";

/// 窗口标题；也是可见验收中要核对的标题。
pub const STARTER_TITLE: &str = "ZenUI Starter";

/// 计数器组件；不依赖 demo、平台句柄或任务。
#[component]
pub fn Starter() -> MountResult<MountPlan> {
    let count = signal(0u32);
    let label = memo(move || format!("Count: {}", count.get()));

    view! {
        <view role="main" class="p-6 gap-4">
            <text class="text-lg font-semibold">{label}</text>
            <button
                class="px-4 py-2 bg-accent text-inverse rounded-md font-semibold hover:bg-accent-hover active:bg-accent-pressed"
                aria:label="Increment counter"
                on:click={move || count.set(count.get() + 1)}
            >"Increment"</button>
        </view>
    }
}

/// 声明并显示计数器窗口；宿主随后把它呈现为真实窗口。
///
/// # Errors
///
/// 目录声明失败或 primary 窗口无法打开时返回 [`WindowError`]。
pub fn open_starter_window(runtime: &mut Runtime) -> Result<WindowCtx, WindowError> {
    runtime.declare_windows(|catalog| {
        catalog.window(
            WindowSpec::new(STARTER_WINDOW)
                .primary()
                .titled(STARTER_TITLE)
                .sized(420, 280),
            |_| view! { <Starter /> },
        )?;
        Ok(())
    })?;
    runtime.show(STARTER_WINDOW)
}
