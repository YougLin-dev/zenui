//! 窗口级上下文菜单的宿主 seam。
//!
//! 运行时只声明菜单；宿主在 `WM_CONTEXTMENU` 中把请求交给 presenter。presenter
//! 在任何 `&mut Host` 借用之外打开菜单，因此菜单的模态消息循环仍可交付 worker
//! 唤醒并重入宿主。返回的命令 id 由宿主重新验证窗口状态后再派发一次。

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_NULL,
};
use zenui::{MenuSpec, WindowCtx};

use crate::host::HostError;
use crate::menus::NativeMenu;

/// 一次上下文菜单打开请求。
pub struct ContextMenuRequest {
    window: WindowCtx,
    hwnd: HWND,
    point: (i32, i32),
    menu: MenuSpec,
}

impl ContextMenuRequest {
    pub(crate) const fn new(
        window: WindowCtx,
        hwnd: HWND,
        point: (i32, i32),
        menu: MenuSpec,
    ) -> Self {
        Self {
            window,
            hwnd,
            point,
            menu,
        }
    }

    /// 打开菜单的 activation。
    pub const fn window(&self) -> WindowCtx {
        self.window
    }

    /// 菜单的 owner HWND；fake presenter 不需要使用它。
    pub const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// 屏幕物理像素坐标；键盘打开时是客户区中心。
    pub const fn point(&self) -> (i32, i32) {
        self.point
    }

    /// 本次要展示的声明。
    pub const fn menu(&self) -> &MenuSpec {
        &self.menu
    }
}

/// 展示上下文菜单并从原生选择中返回一个稳定命令 id。
///
/// 返回 `None` 表示用户取消。实现不得在调用期间访问宿主；本 trait 在宿主借用
/// 之外运行，正是为了允许菜单自己的模态循环重入窗口过程。
pub trait ContextMenuPresenter {
    /// 打开菜单；错误作为 `HostError` 交给消息循环。
    fn show(&mut self, request: ContextMenuRequest) -> Result<Option<&'static str>, HostError>;
}

/// 可见宿主使用 `TrackPopupMenuEx(TPM_RETURNCMD)`；它不发送第二次 `WM_COMMAND`。
pub(crate) struct NativeContextMenuPresenter;

impl ContextMenuPresenter for NativeContextMenuPresenter {
    fn show(&mut self, request: ContextMenuRequest) -> Result<Option<&'static str>, HostError> {
        let menu = NativeMenu::build_popup(request.menu()).map_err(HostError::ContextMenu)?;
        let (x, y) = request.point();
        let owner = request.hwnd();
        // 安全：owner 由宿主持有；菜单句柄由局部 NativeMenu 持有到调用结束。
        unsafe {
            let _ = SetForegroundWindow(owner);
            let command = TrackPopupMenuEx(
                menu.handle(),
                (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
                x,
                y,
                owner,
                None,
            );
            // 菜单关闭后投递空消息，让系统完成前台与捕获清理。
            let _ = PostMessageW(Some(owner), WM_NULL, WPARAM(0), LPARAM(0));
            if command.0 == 0 {
                return Ok(None);
            }
            let id = (command.0 as u32 & 0xFFFF) as u16;
            Ok(menu.command(id).map(|command| command.id()))
        }
    }
}
