//! 托盘图标的宿主 seam。
//!
//! 宿主拥有托盘 HWND、共享系统图标与声明；presenter 执行 `Shell_NotifyIconW`
//! 和弹出菜单。隐藏模式不安装原生 presenter，测试用 fake 记录 add/update/
//! remove 与菜单选择，CI 不会触碰真实通知区域。

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFY_ICON_MESSAGE, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HICON, PostMessageW, SetForegroundWindow, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenuEx,
    WM_NULL,
};
use zenui::MenuSpec;

use crate::host::HostError;
use crate::menus::NativeMenu;

/// 托盘图标在通知区域的稳定 id。
const TRAY_ID: u32 = 1;

/// 添加图标时宿主提供的 HWND、共享图标与回调消息。
pub struct TrayIconRequest {
    hwnd: HWND,
    icon: HICON,
    callback_message: u32,
}

impl TrayIconRequest {
    pub(crate) const fn new(hwnd: HWND, icon: HICON, callback_message: u32) -> Self {
        Self {
            hwnd,
            icon,
            callback_message,
        }
    }

    /// 接收托盘回调的宿主窗口。
    pub const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// 共享系统图标；presenter 不得销毁它。
    pub const fn icon(&self) -> HICON {
        self.icon
    }

    /// 与 worker 唤醒不同的托盘回调消息。
    pub const fn callback_message(&self) -> u32 {
        self.callback_message
    }
}

/// tooltip 更新请求。
pub struct TrayTooltipRequest<'a> {
    hwnd: HWND,
    tooltip: &'a str,
}

impl<'a> TrayTooltipRequest<'a> {
    pub(crate) const fn new(hwnd: HWND, tooltip: &'a str) -> Self {
        Self { hwnd, tooltip }
    }

    /// 接收托盘回调的宿主窗口。
    pub const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// 已通过声明校验的 tooltip。
    pub const fn tooltip(&self) -> &'a str {
        self.tooltip
    }
}

/// 右键菜单请求。
pub struct TrayMenuRequest<'a> {
    hwnd: HWND,
    point: (i32, i32),
    menu: &'a MenuSpec,
}

impl<'a> TrayMenuRequest<'a> {
    pub(crate) const fn new(hwnd: HWND, point: (i32, i32), menu: &'a MenuSpec) -> Self {
        Self { hwnd, point, menu }
    }

    /// 菜单的 owner 托盘窗口。
    pub const fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// 打开菜单的屏幕物理坐标。
    pub const fn point(&self) -> (i32, i32) {
        self.point
    }

    /// 本次展示的声明。
    pub const fn menu(&self) -> &'a MenuSpec {
        self.menu
    }
}

/// 原生通知区域的展示 seam。
///
/// `show_menu` 在自己的模态循环里运行；返回的 id 必须是请求菜单声明过的稳定
/// 命令，返回 `None` 表示用户取消。宿主在展示期间不持有 presenter 槽，实现
/// 不得依赖宿主再次进入；panic 被 contain 为宿主错误。
pub trait TrayPresenter {
    /// 添加图标并设置回调版本。
    fn add(&mut self, request: TrayIconRequest) -> Result<(), HostError>;
    /// 更新 tooltip。
    fn update(&mut self, request: TrayTooltipRequest<'_>) -> Result<(), HostError>;
    /// 移除图标；重复移除必须安全。
    fn remove(&mut self, hwnd: HWND) -> Result<(), HostError>;
    /// 展示右键菜单并返回选中的稳定命令 id。
    fn show_menu(
        &mut self,
        request: TrayMenuRequest<'_>,
    ) -> Result<Option<&'static str>, HostError>;
}

/// 可见宿主使用 `Shell_NotifyIconW` 与 `TrackPopupMenuEx(TPM_RETURNCMD)`。
pub(crate) struct NativeTrayPresenter;

impl NativeTrayPresenter {
    fn data(hwnd: HWND) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ID,
            ..Default::default()
        }
    }

    fn notify(message: NOTIFY_ICON_MESSAGE, data: &NOTIFYICONDATAW) -> Result<(), HostError> {
        // 安全：data 在调用期间有效。
        let result = unsafe { Shell_NotifyIconW(message, data) };
        result
            .as_bool()
            .then_some(())
            .ok_or_else(|| HostError::Tray(String::from("Shell_NotifyIconW failed")))
    }
}

impl TrayPresenter for NativeTrayPresenter {
    fn add(&mut self, request: TrayIconRequest) -> Result<(), HostError> {
        let mut data = Self::data(request.hwnd());
        data.uFlags = NIF_MESSAGE | NIF_ICON;
        data.uCallbackMessage = request.callback_message();
        data.hIcon = request.icon();
        Self::notify(NIM_ADD, &data)?;
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        if let Err(error) = Self::notify(NIM_SETVERSION, &data) {
            // 版本协商失败时撤回已经添加的图标；宿主不会把这次添加记为成功。
            let _ = Self::notify(NIM_DELETE, &Self::data(request.hwnd()));
            return Err(error);
        }
        Ok(())
    }

    fn update(&mut self, request: TrayTooltipRequest<'_>) -> Result<(), HostError> {
        let mut data = Self::data(request.hwnd());
        // 版本 4 下工具提示由系统绘制，需要显式声明标准 tooltip。
        data.uFlags = NIF_TIP | NIF_SHOWTIP;
        let units = request
            .tooltip()
            .encode_utf16()
            .take(data.szTip.len() - 1)
            .collect::<Vec<_>>();
        data.szTip[..units.len()].copy_from_slice(&units);
        Self::notify(NIM_MODIFY, &data)
    }

    fn remove(&mut self, hwnd: HWND) -> Result<(), HostError> {
        let data = Self::data(hwnd);
        Self::notify(NIM_DELETE, &data)
    }

    fn show_menu(
        &mut self,
        request: TrayMenuRequest<'_>,
    ) -> Result<Option<&'static str>, HostError> {
        let menu = NativeMenu::build_popup(request.menu()).map_err(HostError::TrayMenu)?;
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
