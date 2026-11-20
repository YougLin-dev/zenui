use std::fmt;

use zenui_core::RowSet;

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
    pub(crate) children: Vec<PlanNode>,
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

pub enum Event {
    Click(Box<dyn FnMut()>),
    Input(Box<dyn FnMut(String)>),
    Change(Box<dyn FnMut(bool)>),
    Submit(Box<dyn FnMut(SubmitEvent)>),
    KeyDown(Box<dyn FnMut(KeyEvent)>),
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
    })
}

pub fn on_click(handler: impl FnMut() + 'static) -> Attr {
    Attr::Event(Event::Click(Box::new(handler)))
}

pub fn on_input(handler: impl FnMut(String) + 'static) -> Attr {
    Attr::Event(Event::Input(Box::new(handler)))
}

pub fn on_change(handler: impl FnMut(bool) + 'static) -> Attr {
    Attr::Event(Event::Change(Box::new(handler)))
}

pub fn on_submit(handler: impl FnMut(SubmitEvent) + 'static) -> Attr {
    Attr::Event(Event::Submit(Box::new(handler)))
}

pub fn on_keydown(handler: impl FnMut(KeyEvent) + 'static) -> Attr {
    Attr::Event(Event::KeyDown(Box::new(handler)))
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Escape,
    Enter,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyEvent {
    key: Key,
}

impl KeyEvent {
    pub const fn new(key: Key) -> Self {
        Self { key }
    }

    pub const fn key(self) -> Key {
        self.key
    }
}
