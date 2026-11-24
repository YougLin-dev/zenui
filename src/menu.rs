//! 静态菜单声明与稳定的命令身份。
//!
//! 菜单在窗口目录声明期附加到一个窗口键，不创建原生菜单，也不拥有运行时
//! 状态。菜单栏、快捷键、上下文菜单与辅助技术共享同一棵窗口命令树和窗口
//! 派发入口；托盘与应用动作使用应用级命令表。每个命令项由调用方给出的
//! `&'static str` id 标识，id 在一个窗口的菜单树内唯一。

use std::collections::BTreeSet;
use std::fmt;
use std::rc::Rc;

use crate::context::Phase;
use crate::presentation::NodeId;
use crate::store::WriteError;
use crate::view::MountError;
use crate::window::WindowCtx;

/// 命令来源。
///
/// 非穷尽：这些入口已经共享同一条运行时命令路径；托盘来源只选择应用级
/// 处理器。后续平台入口可以在不破坏下游匹配的情况下加入。
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

/// 一次命令派发的负载：来源、可选目标节点与可选目标窗口。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandEvent {
    source: CommandSource,
    target: Option<NodeId>,
    window: Option<WindowCtx>,
}

impl CommandEvent {
    /// 构造一个窗口内派发的事件；`target` 是可选上下文节点。
    pub const fn new(source: CommandSource, target: Option<NodeId>) -> Self {
        Self {
            source,
            target,
            window: None,
        }
    }

    /// 构造一个应用级事件；`window` 是可选的目标 activation。
    pub(crate) const fn for_app(source: CommandSource, window: Option<WindowCtx>) -> Self {
        Self {
            source,
            target: None,
            window,
        }
    }

    /// 给事件附加目标窗口；应用级回退使用。
    pub(crate) const fn with_window(self, window: WindowCtx) -> Self {
        Self {
            source: self.source,
            target: self.target,
            window: Some(window),
        }
    }

    /// 触发这次命令的入口。
    pub const fn source(self) -> CommandSource {
        self.source
    }

    /// 命令作用的上下文节点；菜单栏和快捷键通常没有目标。
    pub const fn target(self) -> Option<NodeId> {
        self.target
    }

    /// 这次派发暴露的目标 activation；由窗口自己的处理器运行时为 `None`，
    /// 应用级处理器收到调用方提供的目标窗口。
    pub const fn window(self) -> Option<WindowCtx> {
        self.window
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
    /// 命令在窗口 factory 或应用级表中没有注册处理器。
    MissingHandler(&'static str),
    /// 同一 activation 或应用级表重复注册同一 command id。
    AlreadyRegistered(&'static str),
    /// 应用级命令必须在运行时打开窗口或挂载根树之前注册。
    RegistrationClosed,
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
            Self::MissingHandler(id) => write!(f, "command {id} has no registered handler"),
            Self::AlreadyRegistered(id) => write!(f, "command {id} already has a handler"),
            Self::RegistrationClosed => f.write_str(
                "app command handlers must be registered before the first window or root mount",
            ),
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

/// 菜单状态回调的失败原因。
///
/// 回调在只读作用域内运行；Signal 写与 Store 事务被拒绝并在这里报告，模型与
/// 菜单项保持原来的已提交状态。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MenuStateError {
    /// 窗口属于另一个 Runtime。
    WrongRuntime,
    /// activation 已关闭或已被新一代替换。
    Stale,
    /// 回调尝试了只读作用域禁止的操作。
    Context {
        operation: &'static str,
        phase: Phase,
    },
    /// 回调尝试提交 Store 事务。
    Write(WriteError),
    /// handler panic；错误被 contain，不向调用方展开。
    HandlerPanicked,
}

impl fmt::Display for MenuStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongRuntime => {
                f.write_str("a WindowCtx from another ZenUI Runtime cannot query menu state")
            }
            Self::Stale => f.write_str("the window activation is no longer live"),
            Self::Context { operation, phase } => {
                write!(
                    f,
                    "{operation} is not allowed during menu state ({phase:?})"
                )
            }
            Self::Write(error) => write!(f, "{error}"),
            Self::HandlerPanicked => f.write_str("the menu state handler panicked"),
        }
    }
}

impl std::error::Error for MenuStateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Write(error) => Some(error),
            _ => None,
        }
    }
}

/// 一个菜单命令在打开时的启用与勾选状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuCommandState {
    enabled: bool,
    checked: bool,
}

impl MenuCommandState {
    /// 由启用与勾选位组成的状态；参数顺序为 `enabled`、`checked`。
    pub const fn new(enabled: bool, checked: bool) -> Self {
        Self { enabled, checked }
    }

    /// 命令是否可被选择。
    pub const fn enabled(self) -> bool {
        self.enabled
    }

    /// 命令是否显示勾选标记。
    pub const fn checked(self) -> bool {
        self.checked
    }
}

pub(crate) type MenuStateHandler = Rc<dyn Fn(&MenuCommand) -> MenuCommandState>;

/// 菜单快捷键里的非修饰键。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShortcutKey {
    /// ASCII 字母或数字；解析时统一为大写。
    Character(char),
    /// F1 到 F24。
    Function(u8),
}

/// 一个菜单命令的快捷键。
///
/// 至少包含 Ctrl 或 Alt，避免把普通输入或只带 Shift 的编辑键声明成快捷键。
/// 解析接受大小写不敏感的 `Ctrl`/`Control`、`Alt`、`Shift` 修饰键，以及
/// ASCII 字母、数字或 `F1`..`F24`；展示文本按 Ctrl、Alt、Shift、键位归一化。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MenuShortcut {
    key: ShortcutKey,
    control: bool,
    alt: bool,
    shift: bool,
}

impl MenuShortcut {
    /// 解析声明文本；失败时保留原文与原因。
    pub fn parse(text: &'static str) -> Result<Self, ShortcutError> {
        if text.trim().is_empty() {
            return Err(ShortcutError::new(text, "the shortcut is empty"));
        }
        let mut control = false;
        let mut alt = false;
        let mut shift = false;
        let mut parts = text.split('+');
        let Some(key_text) = parts.next_back() else {
            return Err(ShortcutError::new(text, "the shortcut is empty"));
        };
        for modifier in parts {
            let modifier = modifier.trim();
            if modifier.eq_ignore_ascii_case("ctrl") || modifier.eq_ignore_ascii_case("control") {
                if control {
                    return Err(ShortcutError::new(text, "Ctrl is repeated"));
                }
                control = true;
            } else if modifier.eq_ignore_ascii_case("alt") {
                if alt {
                    return Err(ShortcutError::new(text, "Alt is repeated"));
                }
                alt = true;
            } else if modifier.eq_ignore_ascii_case("shift") {
                if shift {
                    return Err(ShortcutError::new(text, "Shift is repeated"));
                }
                shift = true;
            } else {
                return Err(ShortcutError::new(text, "unknown modifier"));
            }
        }
        let key = parse_shortcut_key(key_text.trim()).ok_or_else(|| {
            ShortcutError::new(text, "the key must be an ASCII letter, digit, or F1-F24")
        })?;
        if !control && !alt {
            return Err(ShortcutError::new(text, "the shortcut needs Ctrl or Alt"));
        }
        Ok(Self {
            key,
            control,
            alt,
            shift,
        })
    }

    /// 非修饰键。
    pub const fn key(self) -> ShortcutKey {
        self.key
    }

    /// 是否要求按住 Ctrl。
    pub const fn control(self) -> bool {
        self.control
    }

    /// 是否要求按住 Alt。
    pub const fn alt(self) -> bool {
        self.alt
    }

    /// 是否要求按住 Shift。
    pub const fn shift(self) -> bool {
        self.shift
    }
}

impl fmt::Display for MenuShortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.control {
            f.write_str("Ctrl+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        match self.key {
            ShortcutKey::Character(character) => write!(f, "{character}"),
            ShortcutKey::Function(number) => write!(f, "F{number}"),
        }
    }
}

/// 菜单快捷键声明无效的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutError {
    text: &'static str,
    reason: &'static str,
}

impl ShortcutError {
    const fn new(text: &'static str, reason: &'static str) -> Self {
        Self { text, reason }
    }

    /// 原始声明文本。
    pub const fn text(self) -> &'static str {
        self.text
    }

    /// 可读的失败原因。
    pub const fn reason(self) -> &'static str {
        self.reason
    }
}

impl fmt::Display for ShortcutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid menu shortcut `{}`: {}", self.text, self.reason)
    }
}

impl std::error::Error for ShortcutError {}

fn parse_shortcut_key(text: &str) -> Option<ShortcutKey> {
    let mut characters = text.chars();
    let first = characters.next()?;
    let rest = characters.as_str();
    if rest.is_empty() {
        return first
            .is_ascii_alphanumeric()
            .then(|| ShortcutKey::Character(first.to_ascii_uppercase()));
    }
    if !first.eq_ignore_ascii_case(&'f') {
        return None;
    }
    let number = rest
        .parse::<u8>()
        .ok()
        .filter(|number| (1..=24).contains(number))?;
    Some(ShortcutKey::Function(number))
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

/// 一个命令项的静态描述：稳定 id、标签、快捷键与打开时的默认 enabled/checked 状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuCommand {
    id: &'static str,
    label: &'static str,
    shortcut: Option<MenuShortcut>,
    enabled: bool,
    checked: bool,
}

impl MenuCommand {
    /// 一个默认可用、未勾选且没有快捷键的命令。
    pub const fn new(id: &'static str, label: &'static str) -> Self {
        Self {
            id,
            label,
            shortcut: None,
            enabled: true,
            checked: false,
        }
    }

    /// 声明快捷键；解析失败时在窗口目录声明期返回错误。
    ///
    /// 同一个窗口的菜单树里两个命令不能使用同一个归一化快捷键。
    pub fn bind_shortcut(mut self, shortcut: &'static str) -> Result<Self, ShortcutError> {
        self.shortcut = Some(MenuShortcut::parse(shortcut)?);
        Ok(self)
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

    /// 声明的快捷键；没有时返回 `None`。
    pub const fn shortcut(&self) -> Option<MenuShortcut> {
        self.shortcut
    }

    /// 打开菜单时的默认可用状态；这不是运行时状态。
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 打开菜单时的默认勾选状态；这不是运行时状态。
    pub const fn is_checked(&self) -> bool {
        self.checked
    }

    /// 没有状态回调或回调返回默认值时使用的状态。
    pub const fn default_state(&self) -> MenuCommandState {
        MenuCommandState::new(self.enabled, self.checked)
    }
}

/// 菜单树的可表示上限；子菜单、分隔线与命令各计一项。
pub(crate) const MENU_ITEM_LIMIT: usize = u16::MAX as usize;

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

    /// 菜单树超过可表示上限时的项数；没有超过时为 `None`。
    pub(crate) fn items_over_limit(&self) -> Option<usize> {
        let items = self.item_count();
        (items > MENU_ITEM_LIMIT).then_some(items)
    }

    /// 按声明顺序遍历全部命令 id。
    pub(crate) fn command_ids(&self) -> Vec<&'static str> {
        let mut ids = Vec::new();
        self.collect_command_ids(&mut ids);
        ids
    }

    /// 第一个在树中重复出现的命令 id；没有重复时为 `None`。
    pub(crate) fn first_duplicate_command(&self) -> Option<&'static str> {
        let mut seen = BTreeSet::new();
        find_command(&self.items, &mut |command| {
            (!seen.insert(command.id)).then_some(command.id)
        })
    }

    /// 第一个声明了快捷键的命令 id；上下文菜单不接受快捷键。
    pub(crate) fn first_shortcut_command(&self) -> Option<&'static str> {
        find_command(&self.items, &mut |command| {
            command.shortcut.map(|_| command.id)
        })
    }

    /// 第一个与其他命令重复的归一化快捷键；没有重复时为 `None`。
    pub(crate) fn first_duplicate_shortcut(&self) -> Option<(&'static str, MenuShortcut)> {
        let mut seen = BTreeSet::new();
        find_command(&self.items, &mut |command| {
            let shortcut = command.shortcut?;
            (!seen.insert(shortcut)).then_some((command.id, shortcut))
        })
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

/// 按声明顺序访问命令，直到 `visit` 返回第一个结果。
fn find_command<R>(
    items: &[MenuItem],
    visit: &mut impl FnMut(&MenuCommand) -> Option<R>,
) -> Option<R> {
    for item in items {
        match item {
            MenuItem::Command(command) => {
                if let Some(result) = visit(command) {
                    return Some(result);
                }
            }
            MenuItem::Separator => {}
            MenuItem::Submenu(submenu) => {
                if let Some(result) = find_command(&submenu.spec.items, visit) {
                    return Some(result);
                }
            }
        }
    }
    None
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
