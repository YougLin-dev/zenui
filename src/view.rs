use std::fmt;

use zenui_core::{PanicInfo, RowSet};

use zenui_style::{Color, Length, Opacity, Property, StyleDeclaration, StyleValue};

use crate::context::Phase;
use crate::signal::{Memo, Once, Signal};
use crate::store::{Field, FieldError, TodoId, WriteError};

pub type MountResult<T> = Result<T, MountError>;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MountError {
    MissingProp(&'static str),
    Context {
        operation: &'static str,
        phase: Phase,
    },
    Field(FieldError),
    DuplicateKey,
    Write(WriteError),
    Delivery(&'static str),
    Component(&'static str),
    Setup(&'static str),
    /// 用户闭包 panic 已被 contain；运行时状态已恢复，调用方可继续。
    ///
    /// containment 依赖 `panic = "unwind"` 构建。
    Panic(PanicInfo),
}

impl MountError {
    pub const fn missing_prop(name: &'static str) -> Self {
        Self::MissingProp(name)
    }
}

impl fmt::Display for MountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingProp(name) => write!(f, "missing component prop `{name}`"),
            Self::Context { operation, phase } => {
                write!(f, "{operation} is not allowed during {phase:?}")
            }
            Self::Field(error) => write!(f, "{error}"),
            Self::DuplicateKey => f.write_str("duplicate For key before interface publication"),
            Self::Write(error) => write!(f, "{error}"),
            Self::Delivery(message) => f.write_str(message),
            Self::Component(label) => write!(f, "component {label} rejected its mount"),
            Self::Setup(message) => f.write_str(message),
            Self::Panic(panic) => write!(f, "{panic}"),
        }
    }
}

impl std::error::Error for MountError {}

impl From<FieldError> for MountError {
    fn from(error: FieldError) -> Self {
        Self::Field(error)
    }
}

impl From<WriteError> for MountError {
    fn from(error: WriteError) -> Self {
        Self::Write(error)
    }
}

pub struct MountPlan {
    pub(crate) nodes: Vec<PlanNode>,
}

impl MountPlan {
    pub fn new(nodes: Vec<PlanNode>) -> Self {
        Self { nodes }
    }
}

pub enum PlanNode {
    Text(TextSlot),
    Children(Children),
    Factory(ChildrenFactory),
    Element(ElementPlan),
    Component(ComponentPlan),
    Show(ShowPlan),
    For(ForPlan),
}

pub struct ElementPlan {
    pub(crate) tag: &'static str,
    pub(crate) attrs: Vec<Attr>,
    pub(crate) class_styles: &'static [StyleDeclaration],
    pub(crate) inline_styles: Vec<InlineStyle>,
    pub(crate) children: Vec<PlanNode>,
}

/// 一个值槽：静态值或可求值句柄；求值发生在提交帧，不在快照里执行闭包。
pub enum ValueSlot<T> {
    Static(T),
    Signal(Signal<T>),
    Memo(Memo<T>),
}

impl<T> From<T> for ValueSlot<T> {
    fn from(value: T) -> Self {
        Self::Static(value)
    }
}

impl<T> From<Once<T>> for ValueSlot<T> {
    fn from(value: Once<T>) -> Self {
        Self::Static(value.into_inner())
    }
}

impl<T> From<Signal<T>> for ValueSlot<T> {
    fn from(value: Signal<T>) -> Self {
        Self::Signal(value)
    }
}

impl<T> From<Memo<T>> for ValueSlot<T> {
    fn from(value: Memo<T>) -> Self {
        Self::Memo(value)
    }
}

impl<T: Clone + PartialEq + 'static> ValueSlot<T> {
    pub(crate) fn evaluate(&self) -> T {
        match self {
            Self::Static(value) => value.clone(),
            Self::Signal(signal) => signal.get(),
            Self::Memo(memo) => memo.get(),
        }
    }
}

/// 一条 inline `style:` 声明的类型化值槽。
pub enum StyleSlot {
    Color(ValueSlot<Color>),
    Length(ValueSlot<Length>),
    Opacity(ValueSlot<Opacity>),
}

impl StyleSlot {
    pub(crate) fn evaluate(&self) -> StyleValue {
        match self {
            Self::Color(slot) => StyleValue::Color(slot.evaluate()),
            Self::Length(slot) => StyleValue::Length(slot.evaluate()),
            Self::Opacity(slot) => StyleValue::Opacity(slot.evaluate().value()),
        }
    }
}

/// `ElementPlan` 里的一条 inline 声明。
pub struct InlineStyle {
    pub(crate) property: Property,
    pub(crate) slot: StyleSlot,
}

/// `style:background` 的类型化值。
pub fn style_background(value: impl Into<ValueSlot<Color>>) -> InlineStyle {
    InlineStyle {
        property: Property::Background,
        slot: StyleSlot::Color(value.into()),
    }
}

/// `style:color` 的类型化值。
pub fn style_color(value: impl Into<ValueSlot<Color>>) -> InlineStyle {
    InlineStyle {
        property: Property::TextColor,
        slot: StyleSlot::Color(value.into()),
    }
}

/// `style:border-color` 的类型化值。
pub fn style_border_color(value: impl Into<ValueSlot<Color>>) -> InlineStyle {
    InlineStyle {
        property: Property::BorderColor,
        slot: StyleSlot::Color(value.into()),
    }
}

/// `style:opacity` 的类型化值。
pub fn style_opacity(value: impl Into<ValueSlot<Opacity>>) -> InlineStyle {
    InlineStyle {
        property: Property::Opacity,
        slot: StyleSlot::Opacity(value.into()),
    }
}

/// `style:width` 的类型化值。
pub fn style_width(value: impl Into<ValueSlot<Length>>) -> InlineStyle {
    InlineStyle {
        property: Property::Width,
        slot: StyleSlot::Length(value.into()),
    }
}

/// `style:height` 的类型化值。
pub fn style_height(value: impl Into<ValueSlot<Length>>) -> InlineStyle {
    InlineStyle {
        property: Property::Height,
        slot: StyleSlot::Length(value.into()),
    }
}

pub struct ComponentPlan {
    pub(crate) label: &'static str,
    pub(crate) mount: Box<dyn FnOnce() -> MountResult<MountPlan>>,
}

pub struct ShowPlan {
    pub(crate) when: Memo<bool>,
    pub(crate) then: ChildrenFactory,
    pub(crate) fallback: Option<ChildrenFactory>,
}

pub struct ForPlan {
    pub(crate) each: Memo<RowSet>,
    pub(crate) key: Box<dyn Fn(TodoId) -> TodoId>,
    pub(crate) item: Box<dyn Fn(TodoId) -> MountResult<MountPlan>>,
    /// 窗口化 For 的统一行高；`None` 保持全量挂载（ADR-0010）。
    pub(crate) row_height: Option<Length>,
}

pub struct Children {
    mount: Box<dyn FnOnce() -> MountResult<MountPlan>>,
}

impl Children {
    pub(crate) fn mount(self) -> MountResult<MountPlan> {
        (self.mount)()
    }
}

impl<F> From<F> for Children
where
    F: FnOnce() -> MountResult<MountPlan> + 'static,
{
    fn from(mount: F) -> Self {
        Self {
            mount: Box::new(mount),
        }
    }
}

pub struct ChildrenFactory {
    mount: Box<dyn Fn() -> MountResult<MountPlan>>,
}

impl ChildrenFactory {
    pub(crate) fn mount(&self) -> MountResult<MountPlan> {
        (self.mount)()
    }
}

impl<F> From<F> for ChildrenFactory
where
    F: Fn() -> MountResult<MountPlan> + 'static,
{
    fn from(mount: F) -> Self {
        Self {
            mount: Box::new(mount),
        }
    }
}

#[derive(Clone, Debug)]
pub enum TextSlot {
    Static(&'static str),
    Once(String),
    Signal(Signal<String>),
    Memo(Memo<String>),
}

impl TextSlot {
    pub(crate) fn evaluate(&self) -> String {
        match self {
            Self::Static(value) => (*value).to_owned(),
            Self::Once(value) => value.clone(),
            Self::Signal(signal) => signal.get(),
            Self::Memo(memo) => memo.get(),
        }
    }

    /// 值绑定的写入代数；静态值没有写入路径，返回 `None`。
    ///
    /// 编辑器用它区分“处理器写过值绑定”和“模型尚未追上本地编辑”。
    /// Signal 绑定对相等写入也推进；Memo 绑定只观察计算值变化，写回原值的
    /// 归一化因此不会被误当成推迟写入（编辑器保留本地文本）。
    pub(crate) fn epoch(&self, state: &crate::runtime::State) -> Option<u64> {
        match self {
            Self::Static(_) | Self::Once(_) => None,
            Self::Signal(signal) => Some(signal.write_epoch(state)),
            Self::Memo(memo) => Some(memo.write_epoch(state)),
        }
    }
}

pub enum AttrValue {
    Text(TextSlot),
    Bool(BoolValue),
    ResultBool(ResultBoolValue),
}

#[derive(Clone, Debug)]
pub enum BoolValue {
    Static(bool),
    Once(bool),
    Signal(Signal<bool>),
    Memo(Memo<bool>),
}

impl BoolValue {
    pub(crate) fn evaluate(&self) -> bool {
        match self {
            Self::Static(value) | Self::Once(value) => *value,
            Self::Signal(signal) => signal.get(),
            Self::Memo(memo) => memo.get(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum ResultBoolValue {
    Field(Field<bool>),
    Memo(Memo<Result<bool, FieldError>>),
}

pub enum Attr {
    Value {
        name: &'static str,
        value: AttrValue,
    },
    Event(Event),
}

/// 用户闭包返回的错误；在 turn 边界进入 [`crate::TurnReport::errors`]。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HandlerError {
    message: String,
}

impl HandlerError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for HandlerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HandlerError {}

impl From<WriteError> for HandlerError {
    fn from(error: WriteError) -> Self {
        Self::new(error.to_string())
    }
}

impl From<&str> for HandlerError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

impl From<String> for HandlerError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

/// 事件槽的返回值；`Err` 停止本事件剩余 handler 与默认动作。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HandlerOutcome {
    Ok,
    Err(HandlerError),
}

impl HandlerOutcome {
    pub const fn is_err(&self) -> bool {
        matches!(self, Self::Err(_))
    }

    pub const fn error(&self) -> Option<&HandlerError> {
        match self {
            Self::Ok => None,
            Self::Err(error) => Some(error),
        }
    }
}

/// 把 `()`、`Result<(), E>` 或 [`HandlerOutcome`] 归一为事件槽结果。
pub trait IntoHandlerOutcome {
    fn into_handler_outcome(self) -> HandlerOutcome;
}

impl IntoHandlerOutcome for HandlerOutcome {
    fn into_handler_outcome(self) -> HandlerOutcome {
        self
    }
}

impl IntoHandlerOutcome for () {
    fn into_handler_outcome(self) -> HandlerOutcome {
        HandlerOutcome::Ok
    }
}

impl<E: Into<HandlerError>> IntoHandlerOutcome for Result<(), E> {
    fn into_handler_outcome(self) -> HandlerOutcome {
        match self {
            Ok(()) => HandlerOutcome::Ok,
            Err(error) => HandlerOutcome::Err(error.into()),
        }
    }
}

pub enum Event {
    Click(Box<dyn FnMut() -> HandlerOutcome>),
    Input(Box<dyn FnMut(String) -> HandlerOutcome>),
    Change(Box<dyn FnMut(bool)>),
    Submit(Box<dyn FnMut(SubmitEvent)>),
    KeyDown(Box<dyn FnMut(&mut KeyEvent) -> HandlerOutcome>),
    KeyDownCapture(Box<dyn FnMut(&mut KeyEvent) -> HandlerOutcome>),
    KeyDownBubble(Box<dyn FnMut(&mut KeyEvent) -> HandlerOutcome>),
    Focus(Box<dyn FnMut(FocusEvent)>),
    Blur(Box<dyn FnMut(BlurEvent)>),
    CompositionStart(Box<dyn FnMut(CompositionEvent)>),
    CompositionUpdate(Box<dyn FnMut(CompositionEvent)>),
    CompositionEnd(Box<dyn FnMut(CompositionEvent)>),
    CompositionCancel(Box<dyn FnMut(CompositionEvent)>),
}

pub trait IntoText {
    fn into_text(self) -> TextSlot;
}

impl IntoText for &'static str {
    fn into_text(self) -> TextSlot {
        TextSlot::Static(self)
    }
}

impl IntoText for String {
    fn into_text(self) -> TextSlot {
        TextSlot::Once(self)
    }
}

impl IntoText for Once<String> {
    fn into_text(self) -> TextSlot {
        TextSlot::Once(self.into_inner())
    }
}

impl IntoText for Signal<String> {
    fn into_text(self) -> TextSlot {
        TextSlot::Signal(self)
    }
}

impl IntoText for Memo<String> {
    fn into_text(self) -> TextSlot {
        TextSlot::Memo(self)
    }
}

pub trait IntoAttr {
    fn into_attr(self) -> AttrValue;
}

impl IntoAttr for &'static str {
    fn into_attr(self) -> AttrValue {
        AttrValue::Text(TextSlot::Static(self))
    }
}

impl IntoAttr for String {
    fn into_attr(self) -> AttrValue {
        AttrValue::Text(TextSlot::Once(self))
    }
}

impl IntoAttr for bool {
    fn into_attr(self) -> AttrValue {
        AttrValue::Bool(BoolValue::Static(self))
    }
}

impl IntoAttr for Once<String> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Text(TextSlot::Once(self.into_inner()))
    }
}

impl IntoAttr for Once<bool> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Bool(BoolValue::Once(self.into_inner()))
    }
}

impl IntoAttr for Signal<String> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Text(TextSlot::Signal(self))
    }
}

impl IntoAttr for Memo<String> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Text(TextSlot::Memo(self))
    }
}

impl IntoAttr for Signal<bool> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Bool(BoolValue::Signal(self))
    }
}

impl IntoAttr for Memo<bool> {
    fn into_attr(self) -> AttrValue {
        AttrValue::Bool(BoolValue::Memo(self))
    }
}

impl IntoAttr for Field<bool> {
    fn into_attr(self) -> AttrValue {
        AttrValue::ResultBool(ResultBoolValue::Field(self))
    }
}

impl IntoAttr for Memo<Result<bool, FieldError>> {
    fn into_attr(self) -> AttrValue {
        AttrValue::ResultBool(ResultBoolValue::Memo(self))
    }
}

pub fn text_static(value: &'static str) -> PlanNode {
    PlanNode::Text(TextSlot::Static(value))
}

pub fn child(value: impl IntoNode) -> PlanNode {
    value.into_node()
}

pub trait IntoNode {
    fn into_node(self) -> PlanNode;
}

impl IntoNode for &'static str {
    fn into_node(self) -> PlanNode {
        PlanNode::Text(self.into_text())
    }
}

impl IntoNode for String {
    fn into_node(self) -> PlanNode {
        PlanNode::Text(self.into_text())
    }
}

impl IntoNode for Once<String> {
    fn into_node(self) -> PlanNode {
        PlanNode::Text(self.into_text())
    }
}

impl IntoNode for Signal<String> {
    fn into_node(self) -> PlanNode {
        PlanNode::Text(self.into_text())
    }
}

impl IntoNode for Memo<String> {
    fn into_node(self) -> PlanNode {
        PlanNode::Text(self.into_text())
    }
}

impl IntoNode for Children {
    fn into_node(self) -> PlanNode {
        PlanNode::Children(self)
    }
}

impl IntoNode for ChildrenFactory {
    fn into_node(self) -> PlanNode {
        PlanNode::Factory(self)
    }
}

pub fn attr(name: &'static str, value: impl IntoAttr) -> Attr {
    Attr::Value {
        name,
        value: value.into_attr(),
    }
}

pub fn element(tag: &'static str, attrs: Vec<Attr>, children: Vec<PlanNode>) -> PlanNode {
    PlanNode::Element(ElementPlan {
        tag,
        attrs,
        class_styles: &[],
        inline_styles: Vec::new(),
        children,
    })
}

/// 带 type-safe 样式的元素；class 声明来自展开期，inline 声明来自 `style:` 属性。
pub fn styled_element(
    tag: &'static str,
    attrs: Vec<Attr>,
    class_styles: &'static [StyleDeclaration],
    inline_styles: Vec<InlineStyle>,
    children: Vec<PlanNode>,
) -> PlanNode {
    PlanNode::Element(ElementPlan {
        tag,
        attrs,
        class_styles,
        inline_styles,
        children,
    })
}

pub fn component(
    label: &'static str,
    mount: impl FnOnce() -> MountResult<MountPlan> + 'static,
) -> PlanNode {
    PlanNode::Component(ComponentPlan {
        label,
        mount: Box::new(mount),
    })
}

pub fn show(
    when: Memo<bool>,
    then: impl Into<ChildrenFactory>,
    fallback: Option<ChildrenFactory>,
) -> PlanNode {
    PlanNode::Show(ShowPlan {
        when,
        then: then.into(),
        fallback,
    })
}

pub fn for_each(
    each: Memo<RowSet>,
    key: impl Fn(TodoId) -> TodoId + 'static,
    item: impl Fn(TodoId) -> MountResult<MountPlan> + 'static,
) -> PlanNode {
    PlanNode::For(ForPlan {
        each,
        key: Box::new(key),
        item: Box::new(item),
        row_height: None,
    })
}

/// 窗口化 `For`：只挂载宿主声明的可见行窗口，行槽高由 `row_height` 声明。
///
/// 行高是每行的外层高度，父容器必须是零间距的纵向布局；For 用上下两个
/// spacer 保持 `总行数 × row_height` 的声明范围（ADR-0011）。
pub fn for_each_windowed(
    each: Memo<RowSet>,
    key: impl Fn(TodoId) -> TodoId + 'static,
    row_height: Length,
    item: impl Fn(TodoId) -> MountResult<MountPlan> + 'static,
) -> PlanNode {
    PlanNode::For(ForPlan {
        each,
        key: Box::new(key),
        item: Box::new(item),
        row_height: Some(row_height),
    })
}

pub fn on_click<R>(mut handler: impl FnMut() -> R + 'static) -> Attr
where
    R: IntoHandlerOutcome,
{
    Attr::Event(Event::Click(Box::new(move || {
        handler().into_handler_outcome()
    })))
}

pub fn on_input<R>(mut handler: impl FnMut(String) -> R + 'static) -> Attr
where
    R: IntoHandlerOutcome,
{
    Attr::Event(Event::Input(Box::new(move |value| {
        handler(value).into_handler_outcome()
    })))
}

pub fn on_change(handler: impl FnMut(bool) + 'static) -> Attr {
    Attr::Event(Event::Change(Box::new(handler)))
}

pub fn on_submit(handler: impl FnMut(SubmitEvent) + 'static) -> Attr {
    Attr::Event(Event::Submit(Box::new(handler)))
}

pub fn on_keydown<R>(mut handler: impl FnMut(&mut KeyEvent) -> R + 'static) -> Attr
where
    R: IntoHandlerOutcome,
{
    Attr::Event(Event::KeyDown(Box::new(move |event| {
        handler(event).into_handler_outcome()
    })))
}

pub fn on_keydown_capture<R>(mut handler: impl FnMut(&mut KeyEvent) -> R + 'static) -> Attr
where
    R: IntoHandlerOutcome,
{
    Attr::Event(Event::KeyDownCapture(Box::new(move |event| {
        handler(event).into_handler_outcome()
    })))
}

pub fn on_keydown_bubble<R>(mut handler: impl FnMut(&mut KeyEvent) -> R + 'static) -> Attr
where
    R: IntoHandlerOutcome,
{
    Attr::Event(Event::KeyDownBubble(Box::new(move |event| {
        handler(event).into_handler_outcome()
    })))
}

pub fn on_focus(handler: impl FnMut(FocusEvent) + 'static) -> Attr {
    Attr::Event(Event::Focus(Box::new(handler)))
}

pub fn on_blur(handler: impl FnMut(BlurEvent) + 'static) -> Attr {
    Attr::Event(Event::Blur(Box::new(handler)))
}

pub fn on_composition_start(handler: impl FnMut(CompositionEvent) + 'static) -> Attr {
    Attr::Event(Event::CompositionStart(Box::new(handler)))
}

pub fn on_composition_update(handler: impl FnMut(CompositionEvent) + 'static) -> Attr {
    Attr::Event(Event::CompositionUpdate(Box::new(handler)))
}

pub fn on_composition_end(handler: impl FnMut(CompositionEvent) + 'static) -> Attr {
    Attr::Event(Event::CompositionEnd(Box::new(handler)))
}

pub fn on_composition_cancel(handler: impl FnMut(CompositionEvent) + 'static) -> Attr {
    Attr::Event(Event::CompositionCancel(Box::new(handler)))
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct SubmitEvent {
    prevented: bool,
}

impl SubmitEvent {
    pub fn prevent_default(&mut self) {
        self.prevented = true;
    }

    pub const fn is_prevented(self) -> bool {
        self.prevented
    }
}

/// 焦点移动的来源：显式命令，或旧目标离开可见集合后的自动结算。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FocusCause {
    Command,
    Settlement,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FocusEvent {
    cause: FocusCause,
}

impl FocusEvent {
    pub(crate) const fn new(cause: FocusCause) -> Self {
        Self { cause }
    }

    pub const fn cause(self) -> FocusCause {
        self.cause
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BlurEvent {
    cause: FocusCause,
}

impl BlurEvent {
    pub(crate) const fn new(cause: FocusCause) -> Self {
        Self { cause }
    }

    pub const fn cause(self) -> FocusCause {
        self.cause
    }
}

/// 一次本地 composition 的阶段；候选确认是 `End`，取消恢复是 `Cancel`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompositionPhase {
    Start,
    Update,
    End,
    Cancel,
}

/// 编辑器本地 composition 的 typed 载荷；`text` 在 Start/Update 是 preedit，
/// 在 End 是确认的候选文本，在 Cancel 是被丢弃的 preedit。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CompositionEvent {
    phase: CompositionPhase,
    text: String,
    caret: usize,
    selection: Option<(usize, usize)>,
}

impl CompositionEvent {
    pub(crate) fn new(
        phase: CompositionPhase,
        text: String,
        caret: usize,
        selection: Option<(usize, usize)>,
    ) -> Self {
        Self {
            phase,
            text,
            caret,
            selection,
        }
    }

    pub const fn phase(&self) -> CompositionPhase {
        self.phase
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub const fn caret(&self) -> usize {
        self.caret
    }

    pub const fn selection(&self) -> Option<(usize, usize)> {
        self.selection
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Escape,
    Enter,
    Tab,
    Space,
    Backspace,
    Delete,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Home,
    End,
    PageUp,
    PageDown,
    Other,
}

/// 一次 keydown 的载荷；处理器可消费事件或只跳过默认动作。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyEvent {
    key: Key,
    consumed: bool,
    prevent_default: bool,
}

impl KeyEvent {
    pub const fn new(key: Key) -> Self {
        Self {
            key,
            consumed: false,
            prevent_default: false,
        }
    }

    pub const fn key(self) -> Key {
        self.key
    }

    /// 停止后续捕获/冒泡阶段与全部默认动作。
    pub fn consume(&mut self) {
        self.consumed = true;
    }

    /// 只跳过默认动作，传播继续。
    pub fn prevent_default(&mut self) {
        self.prevent_default = true;
    }

    pub const fn is_consumed(self) -> bool {
        self.consumed
    }

    pub const fn prevents_default(self) -> bool {
        self.prevent_default
    }
}
