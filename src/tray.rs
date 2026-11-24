//! 应用级托盘声明。
//!
//! 托盘不绑定窗口 activation：图标、tooltip 与右键菜单在运行时启动前声明，
//! 菜单命令由应用级处理器承接。宿主负责原生图标与菜单的生存期；运行时只
//! 保存声明并校验结构。

use std::fmt;

use crate::menu::MenuSpec;

/// `NOTIFYICONDATAW::szTip` 能容纳的 UTF-16 代码单元数（含结尾 NUL 之前）。
const TOOLTIP_LIMIT: usize = 127;

/// 一个应用的托盘图标声明。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraySpec {
    tooltip: String,
    menu: MenuSpec,
}

impl TraySpec {
    /// 以 tooltip 创建一个空菜单的托盘声明。
    pub fn new(tooltip: impl Into<String>) -> Self {
        Self {
            tooltip: tooltip.into(),
            menu: MenuSpec::new(),
        }
    }

    /// 设置右键菜单；菜单命令由应用级处理器承接，宿主启动时校验每个 id。
    pub fn with_menu(mut self, menu: MenuSpec) -> Self {
        self.menu = menu;
        self
    }

    /// 图标的 tooltip。
    pub fn tooltip(&self) -> &str {
        &self.tooltip
    }

    /// 右键菜单声明。
    pub const fn menu(&self) -> &MenuSpec {
        &self.menu
    }

    pub(crate) fn validate(&self) -> Result<(), TrayError> {
        let units = self.tooltip.encode_utf16().count();
        if units > TOOLTIP_LIMIT {
            return Err(TrayError::TooltipTooLong { units });
        }
        if let Some(items) = self.menu.items_over_limit() {
            return Err(TrayError::MenuTooLarge { items });
        }
        if let Some(id) = self.menu.first_duplicate_command() {
            return Err(TrayError::DuplicateCommand(id));
        }
        if let Some(id) = self.menu.first_shortcut_command() {
            return Err(TrayError::UnsupportedShortcut { id });
        }
        Ok(())
    }

    /// 菜单声明但还没有应用级处理器的第一个命令 id。
    pub(crate) fn first_missing_handler(
        &self,
        has_handler: impl Fn(&str) -> bool,
    ) -> Option<&'static str> {
        self.menu
            .command_ids()
            .into_iter()
            .find(|id| !has_handler(id))
    }
}

/// 托盘声明失败的原因。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TrayError {
    /// 一个 Runtime 只能声明一个托盘。
    AlreadyDeclared,
    /// 托盘必须在首个窗口打开或根树挂载之前声明。
    RegistrationClosed,
    /// tooltip 超过 `NOTIFYICONDATAW` 的 127 个 UTF-16 单元。
    TooltipTooLong { units: usize },
    /// 菜单树超过 65535 项。
    MenuTooLarge { items: usize },
    /// 菜单树里同一个命令 id 出现多次。
    DuplicateCommand(&'static str),
    /// 托盘菜单不参与 `HACCEL`，因此不接受快捷键声明。
    UnsupportedShortcut { id: &'static str },
}

impl fmt::Display for TrayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyDeclared => f.write_str("the application already declares a tray icon"),
            Self::RegistrationClosed => {
                f.write_str("the tray must be declared before the first window or root mount")
            }
            Self::TooltipTooLong { units } => write!(
                f,
                "the tray tooltip needs {units} UTF-16 units; the limit is {TOOLTIP_LIMIT}"
            ),
            Self::MenuTooLarge { items } => write!(
                f,
                "the tray menu declares {items} items; the limit is {}",
                crate::menu::MENU_ITEM_LIMIT
            ),
            Self::DuplicateCommand(id) => {
                write!(f, "tray command id {id} appears more than once")
            }
            Self::UnsupportedShortcut { id } => {
                write!(
                    f,
                    "tray command {id} declares a shortcut outside an accelerator table"
                )
            }
        }
    }
}

impl std::error::Error for TrayError {}
