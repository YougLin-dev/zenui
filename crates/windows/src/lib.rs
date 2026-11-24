//! Win32 宿主：一个 HWND 对应一个 activation，DX12 surface 呈现 `zenui-render`
//! 从 presentation snapshot 生成的场景。
//!
//! 该 crate 只在 Windows 上提供 API；其他平台编译为空的 workspace 成员，因此
//! 根 workspace 的 format、Clippy 与测试不需要 Windows SDK。
//!
//! unsafe 只出现在这个宿主 crate；根 crate 仍保持 unsafe-free。

#![deny(missing_docs)]

// 平台能力的模块归属一次固定：后续工单只增加实现，不再改动 crate 结构。
#[cfg(windows)]
pub mod accessibility;
#[cfg(windows)]
pub mod clipboard;
#[cfg(windows)]
mod context_menu;
#[cfg(windows)]
pub mod drag_drop;
#[cfg(windows)]
mod file_dialog;
#[cfg(windows)]
mod host;
#[cfg(windows)]
pub mod menus;
#[cfg(windows)]
mod ole;
#[cfg(windows)]
mod shortcuts;

#[cfg(windows)]
pub use context_menu::{ContextMenuPresenter, ContextMenuRequest};
#[cfg(windows)]
pub use file_dialog::FileDialogPresenter;
#[cfg(windows)]
pub use host::{Frame, Host, HostConfig, HostError};
