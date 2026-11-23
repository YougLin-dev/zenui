//! 原生菜单与托盘。
//!
//! `NativeMenu` 在 HWND 创建前构建一个窗口菜单栏并保存命令 id 映射；创建成功后
//! 菜单归窗口所有，由 `DestroyWindow` 注销，创建失败或窗口未真正接收菜单时由
//! `NativeMenu` 自己注销。菜单命令只携带稳定声明 id，宿主把它交回运行时命令
//! 路径。`MenuError` 是宿主错误链里的构建失败。托盘消息使用 `host_message`
//! 注册表预留的 `TRAY_CALLBACK`，并由托盘自己的 HWND 路由，不新增顶层
//! `WM_APP + N`。

use std::error::Error;
use std::fmt;

use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateMenu, CreatePopupMenu, DestroyMenu, HMENU, MF_CHECKED, MF_GRAYED, MF_POPUP,
    MF_SEPARATOR, MF_STRING,
};
use windows::core::{HSTRING, PCWSTR};
use zenui::{MenuItem, MenuSpec, Submenu};

/// 构建窗口菜单时保留的 Win32 失败。
#[derive(Debug)]
pub struct MenuError(String);

impl fmt::Display for MenuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for MenuError {}

/// 一个窗口的原生菜单栏。
///
/// 命令 id 按声明前序从 1 分配，与 `MenuSpec` 的命令遍历顺序一致；分隔线与
/// 子菜单节点不占 id。窗口接收菜单后 `owns_handle` 变为 false：Win32 把已挂接
/// 菜单的注销交给 `DestroyWindow`，重复 `DestroyMenu` 只会命中可能被复用的旧句柄。
pub(crate) struct NativeMenu {
    handle: HMENU,
    owns_handle: bool,
    commands: Vec<&'static str>,
}

impl NativeMenu {
    /// 按键声明构建顶级菜单；任何一步失败都销毁已创建的部分。
    pub(crate) fn build(spec: &MenuSpec) -> Result<Self, MenuError> {
        // 安全：句柄由本函数创建；失败路径不把未挂接的句柄留给调用方。
        unsafe {
            let handle = CreateMenu().map_err(menu_error)?;
            let mut commands = Vec::new();
            if let Err(error) = append_items(handle, spec.items(), &mut commands) {
                let _ = DestroyMenu(handle);
                return Err(error);
            }
            Ok(Self {
                handle,
                owns_handle: true,
                commands,
            })
        }
    }

    /// 交给 `CreateWindowExW` 的菜单句柄。
    pub(crate) const fn handle(&self) -> HMENU {
        self.handle
    }

    /// 记录窗口已经接收菜单；之后的注销由 `DestroyWindow` 负责。
    pub(crate) fn assigned_to_window(&mut self) {
        self.owns_handle = false;
    }

    /// 命令 id 对应的稳定声明 id；0、分隔线位置与悬空 id 返回 `None`。
    pub(crate) fn command(&self, id: u16) -> Option<&'static str> {
        self.commands.get(usize::from(id).checked_sub(1)?).copied()
    }
}

impl Drop for NativeMenu {
    fn drop(&mut self) {
        // 安全：只有未交给窗口的句柄仍由本结构拥有。
        if self.owns_handle {
            unsafe {
                let _ = DestroyMenu(self.handle);
            }
        }
    }
}

fn menu_error(error: windows::core::Error) -> MenuError {
    MenuError(error.to_string())
}

/// 追加声明项；命令 id 按前序递增，成功挂接的子菜单归父菜单所有。
fn append_items(
    menu: HMENU,
    items: &[MenuItem],
    commands: &mut Vec<&'static str>,
) -> Result<(), MenuError> {
    for item in items {
        match item {
            MenuItem::Command(command) => {
                let Some(id) = commands
                    .len()
                    .checked_add(1)
                    .and_then(|id| u16::try_from(id).ok())
                else {
                    return Err(MenuError(String::from(
                        "the menu declares more than 65535 commands",
                    )));
                };
                let mut flags = MF_STRING;
                if !command.is_enabled() {
                    flags |= MF_GRAYED;
                }
                if command.is_checked() {
                    flags |= MF_CHECKED;
                }
                let label = HSTRING::from(command.label());
                // 安全：label 在调用期间有效，命令 id 落在 u16 范围。
                unsafe { AppendMenuW(menu, flags, usize::from(id), &label) }.map_err(menu_error)?;
                commands.push(command.id());
            }
            MenuItem::Separator => {
                // 安全：分隔线不读取 `lpNewItem`。
                unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) }
                    .map_err(menu_error)?;
            }
            MenuItem::Submenu(submenu) => append_submenu(menu, submenu, commands)?,
        }
    }
    Ok(())
}

fn append_submenu(
    menu: HMENU,
    submenu: &Submenu,
    commands: &mut Vec<&'static str>,
) -> Result<(), MenuError> {
    // 安全：弹出菜单由本函数创建；挂接前失败时显式销毁，挂接后归父菜单所有。
    unsafe {
        let popup = CreatePopupMenu().map_err(menu_error)?;
        if let Err(error) = append_items(popup, submenu.items(), commands) {
            let _ = DestroyMenu(popup);
            return Err(error);
        }
        let label = HSTRING::from(submenu.label());
        if let Err(error) = AppendMenuW(menu, MF_POPUP, popup.0 as usize, &label) {
            let _ = DestroyMenu(popup);
            return Err(menu_error(error));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::WindowsAndMessaging::IsMenu;

    use super::*;

    #[test]
    fn an_unassigned_menu_is_destroyed_on_drop() {
        let menu =
            NativeMenu::build(&MenuSpec::new().command("file.new", "New")).expect("build the menu");
        let handle = menu.handle();
        drop(menu);
        assert!(
            // 安全：只查询这个刚刚由测试创建的句柄。
            !unsafe { IsMenu(handle) }.as_bool(),
            "a menu the window never received is destroyed by its owner"
        );
    }
}
