//! 最小 ZenUI 应用：一个计数器组件、随应用分发的固定字体与一个原生窗口。

#![allow(
    unused_braces,
    reason = "{} is the intentionally supported event-expression syntax"
)]

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use zenui::prelude::*;
use zenui_windows::{FontAsset, TextAssets};

/// 计数器窗口的目录键。
pub const STARTER_WINDOW: &str = "main";

/// 窗口标题；也是可见验收中要核对的标题。
pub const STARTER_TITLE: &str = "ZenUI Starter";

/// 随模板分发的字体家族名。
pub const FONT_FAMILY: &str = "Noto Sans SC";

/// 字体文件名；资产目录与发布目录使用同一名字。
pub const FONT_FILE: &str = "NotoSansSC-Regular.otf";

/// 资产目录名；与 exe 或工作目录相邻。
pub const ASSET_DIR: &str = "assets";

/// 启动时读取固定字体失败。
#[derive(Debug)]
pub enum TextAssetLoadError {
    /// 字体文件不存在。
    Missing {
        /// 按 exe 旁、工作目录顺序检查后得到的路径。
        path: PathBuf,
    },
    /// 文件存在但读取失败。
    Read {
        /// 读取的路径。
        path: PathBuf,
        /// 文件系统错误。
        error: io::Error,
    },
}

impl fmt::Display for TextAssetLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { path } => write!(
                f,
                "bundled font missing: {}. Copy the template's assets/ directory beside the executable.",
                path.display()
            ),
            Self::Read { path, error } => {
                write!(f, "bundled font unreadable: {} ({error})", path.display())
            }
        }
    }
}

impl std::error::Error for TextAssetLoadError {}

/// 读取随应用分发的固定字体。
///
/// 先找可执行文件旁的 `assets/`，再找工作目录的 `assets/`：发布目录把字体
/// 复制到 exe 旁，`cargo run` 与测试从模板目录运行。文件缺失时返回带路径的
/// [`TextAssetLoadError::Missing`]，不会静默回退到系统字体。
///
/// # Errors
///
/// 字体文件缺失或读取失败时返回 [`TextAssetLoadError`]。
pub fn load_text_assets() -> Result<TextAssets, TextAssetLoadError> {
    load_text_assets_from(asset_path())
}

/// 从指定字体文件读取文本资产；测试与嵌入方可以显式指定路径。
///
/// # Errors
///
/// 路径缺失或读取失败时返回 [`TextAssetLoadError`]。
pub fn load_text_assets_from(font: impl AsRef<Path>) -> Result<TextAssets, TextAssetLoadError> {
    let path = font.as_ref();
    let bytes = std::fs::read(path).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => TextAssetLoadError::Missing {
            path: path.to_path_buf(),
        },
        _ => TextAssetLoadError::Read {
            path: path.to_path_buf(),
            error,
        },
    })?;
    Ok(TextAssets::new(FONT_FAMILY).with_font(FontAsset::new(bytes)))
}

/// 字体文件路径：exe 旁的 `assets/` 优先，其次是工作目录的 `assets/`。
fn asset_path() -> PathBuf {
    let exe_beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .map(|dir| dir.join(ASSET_DIR).join(FONT_FILE));
    if let Some(path) = &exe_beside
        && path.is_file()
    {
        return path.clone();
    }
    if let Ok(cwd) = std::env::current_dir() {
        let local = cwd.join(ASSET_DIR).join(FONT_FILE);
        if local.is_file() {
            return local;
        }
    }
    exe_beside.unwrap_or_else(|| PathBuf::from(ASSET_DIR).join(FONT_FILE))
}

/// 计数器组件；不依赖 demo、平台句柄或任务。
#[component]
pub fn Starter() -> MountResult<MountPlan> {
    let count = signal(0u32);
    let label = memo(move || format!("Count: {}", count.get()));

    view! {
        <view role="main" class="p-6 gap-4">
            <text class="text-lg font-semibold">{label}</text>
            <text class="text-sm text-muted">"中文示例：你好，世界"</text>
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
