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
    ACCEL, ACCEL_VIRT_FLAGS, AppendMenuW, CreateAcceleratorTableW, CreateMenu, CreatePopupMenu,
    DestroyAcceleratorTable, DestroyMenu, FALT, FCONTROL, FSHIFT, FVIRTKEY, HACCEL, HMENU,
    MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING,
};
use windows::core::{HSTRING, PCWSTR};
use zenui::{MenuCommand, MenuItem, MenuShortcut, MenuSpec, ShortcutKey, Submenu};

/// 构建窗口菜单或快捷键表时保留的 Win32 失败。
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
    commands: Vec<NativeCommand>,
}

/// 一个命令与它在原生菜单里的数字 id。
struct NativeCommand {
    id: u16,
    command: MenuCommand,
}

impl NativeMenu {
    /// 按键声明构建顶级菜单；任何一步失败都销毁已创建的部分。
    pub(crate) fn build(spec: &MenuSpec) -> Result<Self, MenuError> {
        // 安全：句柄由本函数创建；失败路径不把未挂接的句柄留给调用方。
        let handle = unsafe { CreateMenu() }.map_err(menu_error)?;
        Self::populate(handle, spec)
    }

    /// 按声明构建一个由 `TrackPopupMenuEx` 展示的弹出菜单。
    ///
    /// `CreateMenu` 的句柄是菜单栏；`TrackPopupMenuEx` 收到它时系统只建立菜单栏
    /// 形态的窗口，不排版命令标签。弹出展示必须使用 `CreatePopupMenu`。
    pub(crate) fn build_popup(spec: &MenuSpec) -> Result<Self, MenuError> {
        // 安全：句柄由本函数创建；失败路径不把未挂接的句柄留给调用方。
        let handle = unsafe { CreatePopupMenu() }.map_err(menu_error)?;
        Self::populate(handle, spec)
    }

    /// 把声明项追加到刚创建的句柄；失败时注销尚未挂接的句柄。
    fn populate(handle: HMENU, spec: &MenuSpec) -> Result<Self, MenuError> {
        let mut commands = Vec::new();
        if let Err(error) = append_items(handle, spec.items(), &mut commands) {
            // 安全：句柄由调用方刚创建，尚未交给窗口或父菜单。
            unsafe {
                let _ = DestroyMenu(handle);
            }
            return Err(error);
        }
        Ok(Self {
            handle,
            owns_handle: true,
            commands,
        })
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
    pub(crate) fn command(&self, id: u16) -> Option<&MenuCommand> {
        let index = usize::from(id).checked_sub(1)?;
        self.commands
            .get(index)
            .filter(|entry| entry.id == id)
            .map(|entry| &entry.command)
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

/// 一个窗口的快捷键表；没有快捷键的菜单不创建 `HACCEL`。
///
/// 表项的命令 id 与 `NativeMenu` 的前序编号一致。表由窗口条目独占，窗口关闭
/// 或创建失败时在这里注销，不跨 activation 复用。
pub(crate) struct NativeAccelerator {
    handle: HACCEL,
}

impl NativeAccelerator {
    /// 从菜单命令的声明快捷键构建；全部命令都没有快捷键时返回 `None`。
    pub(crate) fn build(menu: &NativeMenu) -> Result<Option<Self>, MenuError> {
        let mut entries = Vec::new();
        for entry in &menu.commands {
            let Some(shortcut) = entry.command.shortcut() else {
                continue;
            };
            let key = accelerator_key(shortcut.key()).ok_or_else(|| {
                MenuError(String::from("a menu shortcut has no Windows virtual key"))
            })?;
            entries.push(ACCEL {
                fVirt: accelerator_flags(shortcut),
                key,
                cmd: entry.id,
            });
        }
        if entries.is_empty() {
            return Ok(None);
        }
        // 安全：表项在调用期间有效；成功返回的 HACCEL 由本结构注销。
        let handle = unsafe { CreateAcceleratorTableW(&entries) }.map_err(menu_error)?;
        Ok(Some(Self { handle }))
    }

    /// 交给 `TranslateAcceleratorW` 的句柄。
    pub(crate) const fn handle(&self) -> HACCEL {
        self.handle
    }
}

impl Drop for NativeAccelerator {
    fn drop(&mut self) {
        // 安全：句柄由本结构创建，只注销一次。
        unsafe {
            let _ = DestroyAcceleratorTable(self.handle);
        }
    }
}

fn accelerator_flags(shortcut: MenuShortcut) -> ACCEL_VIRT_FLAGS {
    let mut flags = FVIRTKEY;
    if shortcut.control() {
        flags |= FCONTROL;
    }
    if shortcut.alt() {
        flags |= FALT;
    }
    if shortcut.shift() {
        flags |= FSHIFT;
    }
    flags
}

fn accelerator_key(key: ShortcutKey) -> Option<u16> {
    /// `VK_F1`；F24 紧随其后。
    const VK_F1: u16 = 0x70;
    match key {
        ShortcutKey::Character(character) if character.is_ascii_alphanumeric() => {
            Some(u16::from(character as u8))
        }
        ShortcutKey::Function(number) if (1..=24).contains(&number) => {
            Some(VK_F1 + u16::from(number - 1))
        }
        _ => None,
    }
}

fn menu_error(error: windows::core::Error) -> MenuError {
    MenuError(error.to_string())
}

/// 追加声明项；命令 id 按前序递增，成功挂接的子菜单归父菜单所有。
fn append_items(
    menu: HMENU,
    items: &[MenuItem],
    commands: &mut Vec<NativeCommand>,
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
                let label = match command.shortcut() {
                    // 制表符让 Windows 把快捷键文本右对齐；它仍是同一个菜单项。
                    Some(shortcut) => HSTRING::from(format!("{}\t{shortcut}", command.label())),
                    None => HSTRING::from(command.label()),
                };
                // 安全：label 在调用期间有效，命令 id 落在 u16 范围。
                unsafe { AppendMenuW(menu, flags, usize::from(id), &label) }.map_err(menu_error)?;
                commands.push(NativeCommand {
                    id,
                    command: command.clone(),
                });
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
    commands: &mut Vec<NativeCommand>,
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
    use windows::Win32::UI::WindowsAndMessaging::{GetMenuItemCount, IsMenu};

    use super::*;

    #[test]
    fn an_accelerator_table_is_built_only_for_declared_shortcuts() {
        let menu =
            NativeMenu::build(&MenuSpec::new().command("file.new", "New")).expect("build the menu");
        assert!(
            NativeAccelerator::build(&menu)
                .expect("build the empty accelerator table")
                .is_none()
        );

        let menu = NativeMenu::build(
            &MenuSpec::new()
                .item(
                    MenuCommand::new("file.save", "Save")
                        .bind_shortcut("Ctrl+S")
                        .expect("bind save"),
                )
                .command("file.open", "Open")
                .item(
                    MenuCommand::new("view.refresh", "Refresh")
                        .bind_shortcut("Alt+F5")
                        .expect("bind refresh"),
                ),
        )
        .expect("build the menu with shortcuts");
        let accelerator = NativeAccelerator::build(&menu)
            .expect("build the accelerator table")
            .expect("two commands declare shortcuts");

        use windows::Win32::UI::WindowsAndMessaging::CopyAcceleratorTableW;
        let mut entries = [ACCEL::default(); 2];
        // 安全：表由本测试构建；缓冲长度与表项数一致。
        let copied = unsafe { CopyAcceleratorTableW(accelerator.handle(), Some(&mut entries)) };
        assert_eq!(copied, 2);
        assert_eq!(entries[0].key, u16::from(b'S'));
        assert_eq!(entries[0].cmd, 1);
        assert!(entries[0].fVirt.contains(FVIRTKEY | FCONTROL));
        assert_eq!(entries[1].key, 0x74, "F5");
        assert_eq!(entries[1].cmd, 3, "the unshortcuted command keeps its id");
        assert!(entries[1].fVirt.contains(FVIRTKEY | FALT));
    }

    #[test]
    fn a_dropped_accelerator_table_is_destroyed() {
        use windows::Win32::UI::WindowsAndMessaging::CopyAcceleratorTableW;

        let menu = NativeMenu::build(
            &MenuSpec::new().item(
                MenuCommand::new("file.save", "Save")
                    .bind_shortcut("Ctrl+S")
                    .expect("bind save"),
            ),
        )
        .expect("build the menu");
        let accelerator = NativeAccelerator::build(&menu)
            .expect("build the accelerator table")
            .expect("one declared shortcut");
        let handle = accelerator.handle();
        // 安全：句柄由本测试创建；只查询表项数。
        assert_eq!(unsafe { CopyAcceleratorTableW(handle, None) }, 1);
        drop(accelerator);
        // 安全：句柄已被本测试的结构注销；API 用返回值报告不可用，不解引用。
        let after = unsafe { CopyAcceleratorTableW(handle, None) };
        assert_ne!(after, 1, "the destroyed table exposes no entries");
    }

    #[test]
    fn a_popup_menu_keeps_declared_items_and_command_ids() {
        let menu = NativeMenu::build_popup(
            &MenuSpec::new()
                .command("edit.copy", "Copy")
                .separator()
                .submenu("More", MenuSpec::new().command("edit.paste", "Paste")),
        )
        .expect("build the popup menu");

        // 安全：句柄由本测试持有；只查询项数。
        assert_eq!(
            unsafe { GetMenuItemCount(Some(menu.handle())) },
            3,
            "commands, separators and submenus stay in the popup menu"
        );
        assert_eq!(menu.command(1).map(MenuCommand::id), Some("edit.copy"));
        assert_eq!(menu.command(2).map(MenuCommand::id), Some("edit.paste"));
        let handle = menu.handle();
        drop(menu);
        // 安全：句柄已被本测试的结构注销；API 只报告它不再是菜单。
        assert!(!unsafe { IsMenu(handle) }.as_bool());
    }

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
