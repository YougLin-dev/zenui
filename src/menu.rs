//! 静态菜单声明与稳定的命令身份。
//!
//! 菜单在窗口目录声明期附加到一个窗口键，不创建原生菜单，也不拥有运行时
//! 状态。菜单栏、快捷键、上下文菜单、辅助技术与托盘共享同一棵命令树和
//! 同一个派发入口；每个命令项由调用方给出的 `&'static str` id 标识，id 在
//! 一个窗口的菜单树内唯一。

use std::collections::BTreeSet;
use std::fmt;
use std::rc::Rc;

use crate::presentation::NodeId;
use crate::view::MountError;

/// 命令来源。
///
/// 非穷尽：菜单栏、快捷键、上下文菜单、辅助技术与托盘已经共享一条路径，
/// 后续平台入口可以在不破坏下游匹配的情况下加入。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandSource {
    /// 原生或框架绘制的菜单栏。
    MenuBar,
    /// 键盘快捷键。
    Accelerator,
    /// 上下文菜单。
    ContextMenu,
    /// 辅助技术的动作调用。
    Accessibility,
    /// 托盘或状态区动作。
    Tray,
}

/// 一次命令派发的负载：来源与可选目标节点。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandEvent {
    source: CommandSource,
    target: Option<NodeId>,
}

impl CommandEvent {
    /// 构造一个事件；`target` 是派发时的可选上下文节点。
    pub const fn new(source: CommandSource, target: Option<NodeId>) -> Self {
        Self { source, target }
    }

    /// 触发这次命令的入口。
    pub const fn source(self) -> CommandSource {
        self.source
    }

    /// 命令作用的上下文节点；菜单栏和快捷键通常没有目标。
    pub const fn target(self) -> Option<NodeId> {
        self.target
    }
}

pub(crate) type CommandHandler = Rc<dyn Fn(CommandEvent)>;

/// 命令注册或派发失败的原因。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CommandError {
    /// 窗口属于另一个 Runtime。
    WrongRuntime,
    /// activation 已关闭或已被新一代替换。
    Stale,
    /// 窗口仍存活但被隐藏。
    Hidden,
    /// 窗口正在等待关闭确认，输入已挂起。
    Suspended,
    /// 该窗口的菜单树没有声明这个 command id。
    UnknownCommand(&'static str),
    /// 菜单声明了该命令，但 activation 没有注册处理器。
    MissingHandler(&'static str),
    /// 同一 activation 重复注册同一 command id。
    AlreadyRegistered(&'static str),
    /// 上下文目标不属于这个窗口或已经失效。
    InvalidTarget(NodeId),
    /// handler panic；错误被 contain，不向调用方展开。
    HandlerPanicked(&'static str),
    /// handler 之后的界面刷新失败。
    Mount(MountError),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongRuntime => {
                f.write_str("a WindowCtx from another ZenUI Runtime cannot dispatch here")
            }
            Self::Stale => f.write_str("the window activation is no longer live"),
            Self::Hidden => f.write_str("the window is hidden"),
            Self::Suspended => f.write_str("the window is awaiting close confirmation"),
            Self::UnknownCommand(id) => {
                write!(f, "command id {id} is not declared for this window")
            }
            Self::MissingHandler(id) => {
                write!(
                    f,
                    "command {id} has no handler registered during the window factory"
                )
            }
            Self::AlreadyRegistered(id) => {
                write!(f, "command {id} already has a handler for this activation")
            }
            Self::InvalidTarget(node) => {
                write!(
                    f,
                    "command target {node:?} is not a live node of this window"
                )
            }
            Self::HandlerPanicked(id) => {
                write!(f, "the command handler for {id} panicked")
            }
            Self::Mount(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Mount(error) => Some(error),
            _ => None,
        }
    }
}

/// 一个菜单项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuItem {
    /// 可派发的命令。
    Command(MenuCommand),
    /// 视觉分隔线；不携带 id，也不注册处理器。
    Separator,
    /// 带子菜单的节点。
    Submenu(Submenu),
}

impl MenuItem {
    /// 作为命令项的 id；分隔线与子菜单返回 `None`。
    pub const fn command_id(&self) -> Option<&'static str> {
        match self {
            Self::Command(command) => Some(command.id),
            Self::Separator | Self::Submenu(_) => None,
        }
    }
}

/// 一个命令项的静态描述：稳定 id、标签与打开时的默认 enabled/checked 状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuCommand {
    id: &'static str,
    label: &'static str,
    enabled: bool,
    checked: bool,
}

impl MenuCommand {
    /// 一个默认可用、未勾选的命令。
    pub const fn new(id: &'static str, label: &'static str) -> Self {
        Self {
            id,
            label,
            enabled: true,
            checked: false,
        }
    }

    /// 覆盖打开时的默认可用状态。
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// 覆盖打开时的默认勾选状态。
    pub const fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    /// 稳定命令 id；菜单栏、快捷键与其他来源都用它派发。
    pub const fn id(&self) -> &'static str {
        self.id
    }

    /// 展示给用户的标签。
    pub const fn label(&self) -> &'static str {
        self.label
    }

    /// 打开菜单时的默认可用状态；这不是运行时状态。
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 打开菜单时的默认勾选状态；这不是运行时状态。
    pub const fn is_checked(&self) -> bool {
        self.checked
    }
}

/// 菜单或子菜单中的一组声明项。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuSpec {
    items: Vec<MenuItem>,
}

impl MenuSpec {
    /// 空菜单；空菜单不要求窗口注册任何命令处理器。
    pub const fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// 追加一个使用默认状态的命令。
    pub fn command(mut self, id: &'static str, label: &'static str) -> Self {
        self.items
            .push(MenuItem::Command(MenuCommand::new(id, label)));
        self
    }

    /// 追加一个已配置的命令。
    pub fn item(mut self, command: MenuCommand) -> Self {
        self.items.push(MenuItem::Command(command));
        self
    }

    /// 追加一条分隔线。
    pub fn separator(mut self) -> Self {
        self.items.push(MenuItem::Separator);
        self
    }

    /// 追加一个子菜单。
    pub fn submenu(mut self, label: &'static str, items: MenuSpec) -> Self {
        self.items
            .push(MenuItem::Submenu(Submenu::with_items(label, items)));
        self
    }

    /// 声明项，保持声明顺序。
    pub fn items(&self) -> &[MenuItem] {
        &self.items
    }

    /// 整棵树里的项数；命令、分隔线与子菜单节点各计一项。
    pub(crate) fn item_count(&self) -> usize {
        self.items
            .iter()
            .map(|item| match item {
                MenuItem::Submenu(submenu) => 1 + submenu.spec.item_count(),
                MenuItem::Command(_) | MenuItem::Separator => 1,
            })
            .sum()
    }

    /// 按声明顺序遍历全部命令 id。
    pub(crate) fn command_ids(&self) -> Vec<&'static str> {
        let mut ids = Vec::new();
        self.collect_command_ids(&mut ids);
        ids
    }

    /// 第一个在树中重复出现的命令 id；没有重复时为 `None`。
    pub(crate) fn first_duplicate_command(&self) -> Option<&'static str> {
        fn visit(items: &[MenuItem], seen: &mut BTreeSet<&'static str>) -> Option<&'static str> {
            for item in items {
                match item {
                    MenuItem::Command(command) => {
                        if !seen.insert(command.id) {
                            return Some(command.id);
                        }
                    }
                    MenuItem::Separator => {}
                    MenuItem::Submenu(submenu) => {
                        if let Some(duplicate) = visit(&submenu.spec.items, seen) {
                            return Some(duplicate);
                        }
                    }
                }
            }
            None
        }

        visit(&self.items, &mut BTreeSet::new())
    }

    /// 这棵树是否声明了该命令 id。
    pub(crate) fn contains_command(&self, id: &str) -> bool {
        self.items.iter().any(|item| match item {
            MenuItem::Command(command) => command.id == id,
            MenuItem::Separator => false,
            MenuItem::Submenu(submenu) => submenu.spec.contains_command(id),
        })
    }

    fn collect_command_ids(&self, ids: &mut Vec<&'static str>) {
        for item in &self.items {
            match item {
                MenuItem::Command(command) => ids.push(command.id),
                MenuItem::Separator => {}
                MenuItem::Submenu(submenu) => submenu.spec.collect_command_ids(ids),
            }
        }
    }
}

/// 子菜单：一个标签与它自己的声明项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submenu {
    label: &'static str,
    spec: MenuSpec,
}

impl Submenu {
    /// 空子菜单。
    pub const fn new(label: &'static str) -> Self {
        Self {
            label,
            spec: MenuSpec::new(),
        }
    }

    pub(crate) fn with_items(label: &'static str, spec: MenuSpec) -> Self {
        Self { label, spec }
    }

    /// 追加一个使用默认状态的命令。
    pub fn command(mut self, id: &'static str, label: &'static str) -> Self {
        self.spec = self.spec.command(id, label);
        self
    }

    /// 追加一个已配置的命令。
    pub fn item(mut self, command: MenuCommand) -> Self {
        self.spec = self.spec.item(command);
        self
    }

    /// 追加一条分隔线。
    pub fn separator(mut self) -> Self {
        self.spec = self.spec.separator();
        self
    }

    /// 追加一个嵌套子菜单。
    pub fn submenu(mut self, label: &'static str, items: MenuSpec) -> Self {
        self.spec = self.spec.submenu(label, items);
        self
    }

    /// 子菜单标签。
    pub const fn label(&self) -> &'static str {
        self.label
    }

    /// 声明项，保持声明顺序。
    pub fn items(&self) -> &[MenuItem] {
        self.spec.items()
    }
}
