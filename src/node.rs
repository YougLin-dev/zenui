//! UI 节点身份与固定布局原语。
//!
//! [`NodeRef`] 是内部节点身份，公开元素身份 [`crate::NodeId`] 与呈现身份
//! [`PresentationKey`] 都由它派生。`SlotArena` 维护节点槽位、generation 与
//! 父子兄弟链；payload 与辅助列表由 runtime 的 `NodeArena`/`AuxArenas`
//! 按 [`NodeSlot::payload`] 和 [`AuxHandles`] 存放。

use std::any::Any;
use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU64;
use std::sync::Arc;

use zenui_core::RowSet;
use zenui_style::{Color, Length, Opacity, Property, StyleValue};

use crate::context::FieldKind;
use crate::presentation::SnapshotNode;
use crate::signal::ReactiveValue;
use crate::store::TodoId;
use crate::view::ReadFault;

/// 一棵已提交树的稳定键；单调分配、不复用。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) struct TreeKey(u32);

impl TreeKey {
    pub(crate) const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

/// 内部节点身份：32-bit slot + 32-bit generation，0 保留给 `Option`。
///
/// generation 从 1 开始；槽位复用会推进 generation，旧引用解析失败。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct NodeRef(NonZeroU64);

impl NodeRef {
    pub(crate) fn new(slot: u32, generation: u32) -> Option<Self> {
        if generation == 0 {
            return None;
        }
        NonZeroU64::new((u64::from(slot) << 32) | u64::from(generation)).map(Self)
    }

    pub(crate) const fn slot(self) -> u32 {
        (self.0.get() >> 32) as u32
    }

    pub(crate) const fn generation(self) -> u32 {
        self.0.get() as u32
    }

    /// 只用于验证编码的往返测试；公开的 `PresentationKey` 保持不透明。
    #[cfg(test)]
    pub(crate) const fn from_raw(raw: u64) -> Option<Self> {
        match NonZeroU64::new(raw) {
            Some(value) if value.get() as u32 != 0 => Some(Self(value)),
            _ => None,
        }
    }

    pub(crate) const fn raw(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Debug for NodeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeRef({}:{})", self.slot(), self.generation())
    }
}

/// 公开、opaque 的呈现节点身份；所有快照节点（包括文本）都将携带它。
///
/// 由 [`NodeRef`] 编码；槽位复用会换 generation，因此 renderer 的缓存键
/// 不会命中旧节点。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PresentationKey(u64);

impl PresentationKey {
    pub(crate) const fn from_node(node: NodeRef) -> Self {
        Self(node.raw())
    }

    /// 用于诊断的稳定数值；不承诺对应内部槽位布局。
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for PresentationKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PresentationKey").field(&self.0).finish()
    }
}

const _: () = assert!(std::mem::size_of::<Option<NodeRef>>() == 8);
const _: () = assert!(std::mem::size_of::<PresentationKey>() == 8);

/// 节点种类；结构节点在快照中透明，但保留自己的身份与子链。
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum NodeKindTag {
    Element = 0,
    Text,
    Component,
    Show,
    For,
}

/// 位标志留给后续的脏路径；本切片只建立稳定布局。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct DirtyFlags(u8);

impl DirtyFlags {
    pub(crate) const NONE: Self = Self(0);
}

/// 一个已分配 UI 节点；payload 指向该 kind 自己的密集 arena。
///
/// 字段顺序固定为 64 B（§4.1）：3×u32 + kind/dirty + 2 B padding +
/// 6×8 B 指针。`snapshot` 留给持久化快照切片（#342），本切片保持 `None`。
#[repr(C)]
#[derive(Debug)]
pub(crate) struct NodeSlot {
    pub(crate) payload: u32,
    pub(crate) generation: u32,
    pub(crate) tree: TreeKey,
    /// `None` 表示槽位空闲；generation 仍保留以拒绝旧引用。
    pub(crate) kind: Option<NodeKindTag>,
    /// 脏路径在后续切片接入；本切片先固定字段与默认值。
    pub(crate) dirty: DirtyFlags,
    pub(crate) parent: Option<NodeRef>,
    pub(crate) first_child: Option<NodeRef>,
    pub(crate) last_child: Option<NodeRef>,
    pub(crate) prev_sibling: Option<NodeRef>,
    pub(crate) next_sibling: Option<NodeRef>,
    pub(crate) snapshot: Option<Arc<SnapshotNode>>,
}

const _: () = assert!(std::mem::size_of::<NodeSlot>() == 64);

impl NodeSlot {
    const fn vacant() -> Self {
        Self {
            payload: NO_PAYLOAD,
            generation: 1,
            tree: TreeKey::from_raw(0),
            kind: None,
            dirty: DirtyFlags::NONE,
            parent: None,
            first_child: None,
            last_child: None,
            prev_sibling: None,
            next_sibling: None,
            snapshot: None,
        }
    }
}

pub(crate) const NO_PAYLOAD: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum NodeError {
    UnknownSlot {
        slot: u32,
    },
    Vacant {
        slot: u32,
    },
    StaleGeneration {
        slot: u32,
        live: u32,
        found: u32,
    },
    WrongKind {
        expected: NodeKindTag,
        found: NodeKindTag,
    },
}

impl fmt::Display for NodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSlot { slot } => write!(f, "node slot {slot} was never allocated"),
            Self::Vacant { slot } => write!(f, "node slot {slot} is vacant"),
            Self::StaleGeneration { slot, live, found } => write!(
                f,
                "node slot {slot} has generation {live}, reference has {found}"
            ),
            Self::WrongKind { expected, found } => {
                write!(f, "expected {expected:?} node, found {found:?}")
            }
        }
    }
}

impl std::error::Error for NodeError {}

/// 节点槽位、generation 与父子兄弟链的唯一权威。
pub(crate) struct SlotArena {
    slots: Vec<NodeSlot>,
    free: Vec<u32>,
    live: usize,
}

impl SlotArena {
    pub(crate) const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            live: 0,
        }
    }

    /// 插入节点并把 child 链到 `parent` 的末尾。
    pub(crate) fn insert(
        &mut self,
        kind: NodeKindTag,
        payload: u32,
        tree: TreeKey,
        parent: Option<NodeRef>,
    ) -> Result<NodeRef, NodeError> {
        if let Some(parent_ref) = parent {
            let parent_slot = self.get(parent_ref)?;
            debug_assert_eq!(
                parent_slot.tree, tree,
                "a child node must belong to its parent's tree"
            );
        }
        let slot = match self.free.pop() {
            Some(slot) => slot,
            None => {
                let slot = u32::try_from(self.slots.len()).expect("node arena exhausted");
                self.slots.push(NodeSlot::vacant());
                slot
            }
        };
        let generation = self.slots[slot as usize].generation;
        let node = NodeRef::new(slot, generation).expect("arena generations start at 1");
        let mut previous_sibling = None;
        if let Some(parent_ref) = parent {
            let parent_slot = self.get_mut(parent_ref)?;
            previous_sibling = parent_slot.last_child;
            parent_slot.last_child = Some(node);
            if parent_slot.first_child.is_none() {
                parent_slot.first_child = Some(node);
            }
        }
        if let Some(previous) = previous_sibling
            && let Ok(previous_slot) = self.get_mut(previous)
        {
            previous_slot.next_sibling = Some(node);
        }
        self.slots[slot as usize] = NodeSlot {
            payload,
            generation,
            tree,
            kind: Some(kind),
            dirty: DirtyFlags::NONE,
            parent,
            first_child: None,
            last_child: None,
            prev_sibling: previous_sibling,
            next_sibling: None,
            snapshot: None,
        };
        self.live += 1;
        Ok(node)
    }

    pub(crate) fn get(&self, node: NodeRef) -> Result<&NodeSlot, NodeError> {
        let slot = self.slot(node)?;
        if slot.kind.is_none() {
            return Err(NodeError::Vacant { slot: node.slot() });
        }
        Ok(slot)
    }

    pub(crate) fn get_mut(&mut self, node: NodeRef) -> Result<&mut NodeSlot, NodeError> {
        let generation = node.generation();
        let slot = self.slot_mut(node)?;
        if slot.kind.is_none() {
            return Err(NodeError::Vacant { slot: node.slot() });
        }
        debug_assert_eq!(slot.generation, generation);
        Ok(slot)
    }

    fn slot(&self, node: NodeRef) -> Result<&NodeSlot, NodeError> {
        let Some(slot) = self.slots.get(node.slot() as usize) else {
            return Err(NodeError::UnknownSlot { slot: node.slot() });
        };
        if slot.generation != node.generation() {
            return Err(NodeError::StaleGeneration {
                slot: node.slot(),
                live: slot.generation,
                found: node.generation(),
            });
        }
        Ok(slot)
    }

    fn slot_mut(&mut self, node: NodeRef) -> Result<&mut NodeSlot, NodeError> {
        let slot = node.slot();
        let Some(entry) = self.slots.get_mut(slot as usize) else {
            return Err(NodeError::UnknownSlot { slot });
        };
        if entry.generation != node.generation() {
            return Err(NodeError::StaleGeneration {
                slot,
                live: entry.generation,
                found: node.generation(),
            });
        }
        Ok(entry)
    }

    pub(crate) fn require_kind(
        &self,
        node: NodeRef,
        kind: NodeKindTag,
    ) -> Result<&NodeSlot, NodeError> {
        let slot = self.get(node)?;
        match slot.kind {
            Some(found) if found == kind => Ok(slot),
            Some(found) => Err(NodeError::WrongKind {
                expected: kind,
                found,
            }),
            None => Err(NodeError::Vacant { slot: node.slot() }),
        }
    }

    pub(crate) fn set_payload(&mut self, node: NodeRef, payload: u32) -> Result<(), NodeError> {
        self.get_mut(node)?.payload = payload;
        Ok(())
    }

    /// 释放节点并解除它与父/兄弟的链接；重复释放无操作。
    pub(crate) fn release(&mut self, node: NodeRef) {
        let Ok(slot) = self.get(node) else {
            return;
        };
        let parent = slot.parent;
        let previous = slot.prev_sibling;
        let next = slot.next_sibling;
        if let Some(previous) = previous
            && let Ok(previous_slot) = self.get_mut(previous)
        {
            previous_slot.next_sibling = next;
        }
        if let Some(next) = next
            && let Ok(next_slot) = self.get_mut(next)
        {
            next_slot.prev_sibling = previous;
        }
        if let Some(parent) = parent
            && let Ok(parent_slot) = self.get_mut(parent)
        {
            if parent_slot.first_child == Some(node) {
                parent_slot.first_child = next;
            }
            if parent_slot.last_child == Some(node) {
                parent_slot.last_child = previous;
            }
        }
        let index = node.slot() as usize;
        let generation = self.slots[index].generation;
        self.slots[index].kind = None;
        self.slots[index].payload = NO_PAYLOAD;
        self.slots[index].dirty = DirtyFlags::NONE;
        self.slots[index].parent = None;
        self.slots[index].first_child = None;
        self.slots[index].last_child = None;
        self.slots[index].prev_sibling = None;
        self.slots[index].next_sibling = None;
        self.slots[index].snapshot = None;
        self.live -= 1;
        if generation == u32::MAX {
            // 代际耗尽：该槽退休，不再复用。
            return;
        }
        self.slots[index].generation = generation + 1;
        self.free.push(node.slot());
    }

    /// 槽位编号是否曾被分配；用于把已释放身份判为 stale 而不是 unknown。
    pub(crate) fn slot_exists(&self, node: NodeRef) -> bool {
        (node.slot() as usize) < self.slots.len()
    }

    pub(crate) const fn live(&self) -> usize {
        self.live
    }

    pub(crate) fn storage_len(&self) -> usize {
        self.slots.len()
    }
}

/// 按句柄索引的密集 payload arena；句柄由调用方按 kind 保存。
pub(crate) struct Pool<T> {
    values: Vec<Option<T>>,
    free: Vec<u32>,
    live: usize,
}

impl<T> Pool<T> {
    pub(crate) const fn new() -> Self {
        Self {
            values: Vec::new(),
            free: Vec::new(),
            live: 0,
        }
    }

    pub(crate) fn reserve(&mut self, value: T) -> u32 {
        match self.free.pop() {
            Some(slot) => {
                self.values[slot as usize] = Some(value);
                self.live += 1;
                slot
            }
            None => {
                let slot = u32::try_from(self.values.len()).expect("payload arena exhausted");
                self.values.push(Some(value));
                self.live += 1;
                slot
            }
        }
    }

    pub(crate) fn get(&self, slot: u32) -> Option<&T> {
        self.values.get(slot as usize).and_then(Option::as_ref)
    }

    pub(crate) fn get_mut(&mut self, slot: u32) -> Option<&mut T> {
        self.values.get_mut(slot as usize).and_then(Option::as_mut)
    }

    pub(crate) fn release(&mut self, slot: u32) -> Option<T> {
        let value = self.values.get_mut(slot as usize)?.take()?;
        self.live -= 1;
        self.free.push(slot);
        Some(value)
    }

    /// 临时借出 payload 而不归还槽位；刷新期间使用，guard 负责放回。
    pub(crate) fn take(&mut self, slot: u32) -> Option<T> {
        self.values.get_mut(slot as usize)?.take()
    }

    pub(crate) fn put(&mut self, slot: u32, value: T) {
        debug_assert!(
            self.values.get(slot as usize).is_some_and(Option::is_none),
            "a taken payload slot is vacant"
        );
        self.values[slot as usize] = Some(value);
    }

    pub(crate) const fn live(&self) -> usize {
        self.live
    }

    #[cfg(test)]
    pub(crate) fn storage_len(&self) -> usize {
        self.values.len()
    }
}

macro_rules! list_id {
    ($($name:ident),+ $(,)?) => {
        $(
            #[derive(Clone, Copy, PartialEq, Eq, Debug)]
            pub(crate) struct $name(u32);

            impl $name {
                pub(crate) const fn from_raw(raw: u32) -> Self {
                    Self(raw)
                }

                pub(crate) const fn raw(self) -> u32 {
                    self.0
                }
            }
        )+
    };
}

list_id!(
    AttrListId,
    StyleListId,
    EventListId,
    EditStateId,
    BindingListId,
    SemanticsId,
);

/// 一个元素在辅助 arena 中的句柄集合。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct AuxHandles {
    pub(crate) attrs: AttrListId,
    pub(crate) class_styles: StyleListId,
    pub(crate) inline_styles: StyleListId,
    pub(crate) events: EventListId,
    pub(crate) edit: EditStateId,
    pub(crate) bindings: BindingListId,
    pub(crate) semantics: SemanticsId,
}

/// 注册表内的 tag 下标；字符串本身由 runtime 的注册表持有。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct TagId(u16);

impl TagId {
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u16::try_from(index).expect("tag registry exhausted"))
    }

    pub(crate) const fn raw(self) -> u16 {
        self.0
    }
}

/// 元素标志位；本切片只有默认值，后续切片按位扩展。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ElementFlags(u8);

impl ElementFlags {
    pub(crate) const NONE: Self = Self(0);
}

/// 元素 payload：只存 tag、flags、owner 与外部 arena 句柄（≤96 B）。
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ElementData {
    pub(crate) tag: TagId,
    pub(crate) flags: ElementFlags,
    pub(crate) owner: u32,
    pub(crate) aux: AuxHandles,
}

const _: () = assert!(std::mem::size_of::<ElementData>() <= 96);

/// 绑定槽的稳定 part 判别式。
///
/// 结构触发排在最前，保证补丁应用时结构先于值。属性名与样式属性都来自
/// 宏的静态 token，因此可以直接作为稳定键。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum BindingPart {
    ShowWhen,
    ForEach,
    Text,
    Attr(&'static str),
    Style(Property),
}

impl BindingPart {
    const fn rank(self) -> u8 {
        match self {
            Self::ShowWhen => 0,
            Self::ForEach => 1,
            Self::Text => 2,
            Self::Attr(_) => 3,
            Self::Style(_) => 4,
        }
    }
}

impl Ord for BindingPart {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank()
            .cmp(&other.rank())
            .then_with(|| match (self, other) {
                (Self::Attr(left), Self::Attr(right)) => left.cmp(right),
                (Self::Style(left), Self::Style(right)) => (*left as u8).cmp(&(*right as u8)),
                _ => std::cmp::Ordering::Equal,
            })
    }
}

impl PartialOrd for BindingPart {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// `BindingPart` 在绑定列表中的 16-bit 注册值。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) struct PartId(u16);

impl PartId {
    pub(crate) const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    #[cfg(test)]
    pub(crate) const fn raw(self) -> u16 {
        self.0
    }
}

/// 源句柄；runtime 身份 + arena 槽位。generation 由槽位自身校验。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct SourceRef {
    pub(crate) runtime: u32,
    pub(crate) id: u32,
}

/// 字段源；值类型由持有它的 `BindingSlot<T>` 决定。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FieldKey {
    pub(crate) runtime: u32,
    pub(crate) id: TodoId,
    pub(crate) kind: FieldKind,
}

/// once 来源的诊断 id；不携带值。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct OnceOrigin(u32);

impl OnceOrigin {
    pub(crate) const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

/// 来源描述；最后已提交值只在 `BindingSlot::current`。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ReadSlot {
    Static,
    Once(OnceOrigin),
    Signal(SourceRef),
    Memo(SourceRef),
    Field(FieldKey),
}

impl ReadSlot {
    /// 静态/once 值在挂载时写入 current，永不重算。
    pub(crate) const fn is_static(self) -> bool {
        matches!(self, Self::Static | Self::Once(_))
    }
}

/// 一个绑定槽；`current` 是最后已提交值的唯一权威。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BindingSlot<T> {
    pub(crate) source: ReadSlot,
    pub(crate) current: Option<T>,
    /// 本次求值失败；可与 `current` 同时存在。
    pub(crate) fault: Option<ReadFault>,
    pub(crate) generation: u32,
}

impl<T> BindingSlot<T> {
    fn new(
        source: ReadSlot,
        current: Option<T>,
        fault: Option<ReadFault>,
        generation: u32,
    ) -> Self {
        Self {
            source,
            current,
            fault,
            generation,
        }
    }
}

/// 8 B 打包引用：table u8 + slot u24 + generation u32。
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) struct BindingRef(u64);

impl BindingRef {
    const SLOT_BITS: u32 = 24;
    const SLOT_MASK: u64 = (1 << Self::SLOT_BITS) - 1;

    pub(crate) fn new(table: u8, slot: u32, generation: u32) -> Option<Self> {
        if generation == 0 || u64::from(slot) > Self::SLOT_MASK {
            return None;
        }
        Some(Self(
            (u64::from(table) << 56) | (u64::from(slot) << 32) | u64::from(generation),
        ))
    }

    #[cfg(test)]
    pub(crate) const fn raw(self) -> u64 {
        self.0
    }

    #[cfg(test)]
    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub(crate) const fn table(self) -> u8 {
        (self.0 >> 56) as u8
    }

    pub(crate) const fn slot(self) -> u32 {
        ((self.0 >> 32) & Self::SLOT_MASK) as u32
    }

    pub(crate) const fn generation(self) -> u32 {
        self.0 as u32
    }
}

const _: () = assert!(std::mem::size_of::<BindingRef>() == 8);
const _: () = assert!(std::mem::size_of::<(PartId, BindingRef)>() == 16);

const TEXT_TABLE: u8 = 0;
const BOOL_TABLE: u8 = 1;
const COLOR_TABLE: u8 = 2;
const LENGTH_TABLE: u8 = 3;
const OPACITY_TABLE: u8 = 4;
const ROW_SET_TABLE: u8 = 5;

enum BindingCell<T> {
    Live(BindingSlot<T>),
    Vacant { generation: u32 },
}

/// 单一值类型的绑定槽表；释放推进 generation，旧 `BindingRef` 失效。
pub(crate) struct BindingArena<T> {
    cells: Vec<BindingCell<T>>,
    free: Vec<u32>,
    live: usize,
}

impl<T> BindingArena<T> {
    const fn new() -> Self {
        Self {
            cells: Vec::new(),
            free: Vec::new(),
            live: 0,
        }
    }

    fn insert(
        &mut self,
        table: u8,
        source: ReadSlot,
        current: Option<T>,
        fault: Option<ReadFault>,
    ) -> BindingRef {
        match self.free.pop() {
            Some(slot) => {
                let generation = match &self.cells[slot as usize] {
                    BindingCell::Vacant { generation } => *generation,
                    BindingCell::Live(_) => unreachable!("a free binding slot is vacant"),
                };
                let reference = BindingRef::new(table, slot, generation)
                    .expect("a reusable binding slot is encodable");
                self.cells[slot as usize] =
                    BindingCell::Live(BindingSlot::new(source, current, fault, generation));
                self.live += 1;
                reference
            }
            None => {
                let slot = u32::try_from(self.cells.len()).expect("binding arena exhausted");
                let reference =
                    BindingRef::new(table, slot, 1).expect("the binding arena fits 24-bit slots");
                self.cells.push(BindingCell::Live(BindingSlot::new(
                    source, current, fault, 1,
                )));
                self.live += 1;
                reference
            }
        }
    }

    fn get(&self, reference: BindingRef) -> Option<&BindingSlot<T>> {
        match self.cells.get(reference.slot() as usize)? {
            BindingCell::Live(slot) if slot.generation == reference.generation() => Some(slot),
            _ => None,
        }
    }

    fn get_mut(&mut self, reference: BindingRef) -> Option<&mut BindingSlot<T>> {
        match self.cells.get_mut(reference.slot() as usize)? {
            BindingCell::Live(slot) if slot.generation == reference.generation() => Some(slot),
            _ => None,
        }
    }

    fn release(&mut self, reference: BindingRef) -> Option<BindingSlot<T>> {
        let cell = self.cells.get_mut(reference.slot() as usize)?;
        let BindingCell::Live(slot) = cell else {
            return None;
        };
        if slot.generation != reference.generation() {
            return None;
        }
        let generation = slot.generation;
        let BindingCell::Live(value) = std::mem::replace(
            cell,
            BindingCell::Vacant {
                generation: generation.wrapping_add(1),
            },
        ) else {
            unreachable!("the cell was just observed live");
        };
        self.live -= 1;
        if generation != u32::MAX {
            self.free.push(reference.slot());
        }
        Some(value)
    }

    pub(crate) const fn live(&self) -> usize {
        self.live
    }
}

/// 让 `BindingTable` 按值类型分表；`PartId` 决定类型，`BindingRef.table` 必须匹配。
///
/// `ReactiveValue` 是超 trait：绑定源与 memo 使用同一套值语义相等。
pub(crate) trait BindingValue: ReactiveValue + PartialEq + 'static {
    const TABLE: u8;

    fn arena(table: &BindingTable) -> &BindingArena<Self>;
    fn arena_mut(table: &mut BindingTable) -> &mut BindingArena<Self>;
}

macro_rules! binding_value {
    ($type:ty, $table:expr, $field:ident) => {
        impl BindingValue for $type {
            const TABLE: u8 = $table;

            fn arena(table: &BindingTable) -> &BindingArena<Self> {
                &table.$field
            }

            fn arena_mut(table: &mut BindingTable) -> &mut BindingArena<Self> {
                &mut table.$field
            }
        }
    };
}

binding_value!(String, TEXT_TABLE, text);
binding_value!(bool, BOOL_TABLE, bools);
binding_value!(Color, COLOR_TABLE, colors);
binding_value!(Length, LENGTH_TABLE, lengths);
binding_value!(Opacity, OPACITY_TABLE, opacities);
binding_value!(RowSet, ROW_SET_TABLE, row_sets);

/// 绑定槽的值类型；由 `BindingRef.table` 决定，读取方据此还原 typed 槽。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BindingType {
    Text,
    Bool,
    Color,
    Length,
    Opacity,
    RowSet,
}

/// 每棵树一份的绑定表；元素只存 `BindingListId`，Show/For 存直接 ref。
pub(crate) struct BindingTable {
    text: BindingArena<String>,
    bools: BindingArena<bool>,
    colors: BindingArena<Color>,
    lengths: BindingArena<Length>,
    opacities: BindingArena<Opacity>,
    row_sets: BindingArena<RowSet>,
    parts: Vec<BindingPart>,
    part_ids: BTreeMap<BindingPart, PartId>,
    next_once: u32,
}

impl BindingTable {
    pub(crate) const fn new() -> Self {
        Self {
            text: BindingArena::new(),
            bools: BindingArena::new(),
            colors: BindingArena::new(),
            lengths: BindingArena::new(),
            opacities: BindingArena::new(),
            row_sets: BindingArena::new(),
            parts: Vec::new(),
            part_ids: BTreeMap::new(),
            next_once: 0,
        }
    }

    pub(crate) fn insert<T: BindingValue>(
        &mut self,
        source: ReadSlot,
        current: Option<T>,
        fault: Option<ReadFault>,
    ) -> BindingRef {
        T::arena_mut(self).insert(T::TABLE, source, current, fault)
    }

    pub(crate) fn once_origin(&mut self) -> OnceOrigin {
        let origin = OnceOrigin::from_raw(self.next_once);
        self.next_once = self.next_once.wrapping_add(1);
        origin
    }

    /// 注册 part 并返回稳定 id；同一 part 复用同一 id。
    pub(crate) fn part_id(&mut self, part: BindingPart) -> PartId {
        if let Some(id) = self.part_ids.get(&part) {
            return *id;
        }
        let id = PartId::from_raw(
            u16::try_from(self.parts.len()).expect("binding part registry exhausted"),
        );
        self.parts.push(part);
        self.part_ids.insert(part, id);
        id
    }

    #[cfg(test)]
    pub(crate) fn part(&self, id: PartId) -> Option<BindingPart> {
        self.parts.get(id.raw() as usize).copied()
    }

    pub(crate) fn slot<T: BindingValue>(&self, reference: BindingRef) -> Option<&BindingSlot<T>> {
        if reference.table() != T::TABLE {
            return None;
        }
        T::arena(self).get(reference)
    }

    pub(crate) fn slot_mut<T: BindingValue>(
        &mut self,
        reference: BindingRef,
    ) -> Option<&mut BindingSlot<T>> {
        if reference.table() != T::TABLE {
            return None;
        }
        T::arena_mut(self).get_mut(reference)
    }

    /// 释放任意类型的槽；`false` 表示 ref 已过期或类型不可识别。
    pub(crate) fn release(&mut self, reference: BindingRef) -> bool {
        match reference.table() {
            TEXT_TABLE => self.text.release(reference).is_some(),
            BOOL_TABLE => self.bools.release(reference).is_some(),
            COLOR_TABLE => self.colors.release(reference).is_some(),
            LENGTH_TABLE => self.lengths.release(reference).is_some(),
            OPACITY_TABLE => self.opacities.release(reference).is_some(),
            ROW_SET_TABLE => self.row_sets.release(reference).is_some(),
            _ => false,
        }
    }

    /// 引用是否落在文本表；属性快照据此选择值类型。
    pub(crate) fn is_text(&self, reference: BindingRef) -> bool {
        reference.table() == TEXT_TABLE
    }

    /// 引用是否落在布尔表。
    pub(crate) fn is_bool(&self, reference: BindingRef) -> bool {
        reference.table() == BOOL_TABLE
    }

    /// 引用的值类型；未知表或过期引用返回 `None`。
    pub(crate) fn kind(&self, reference: BindingRef) -> Option<BindingType> {
        Some(match reference.table() {
            TEXT_TABLE => BindingType::Text,
            BOOL_TABLE => BindingType::Bool,
            COLOR_TABLE => BindingType::Color,
            LENGTH_TABLE => BindingType::Length,
            OPACITY_TABLE => BindingType::Opacity,
            ROW_SET_TABLE => BindingType::RowSet,
            _ => return None,
        })
    }

    /// 来源描述；类型擦除后按表分派，求值方按 [`BindingTable::kind`] 还原类型。
    pub(crate) fn read_slot(&self, reference: BindingRef) -> Option<ReadSlot> {
        match reference.table() {
            TEXT_TABLE => self.slot::<String>(reference).map(|slot| slot.source),
            BOOL_TABLE => self.slot::<bool>(reference).map(|slot| slot.source),
            COLOR_TABLE => self.slot::<Color>(reference).map(|slot| slot.source),
            LENGTH_TABLE => self.slot::<Length>(reference).map(|slot| slot.source),
            OPACITY_TABLE => self.slot::<Opacity>(reference).map(|slot| slot.source),
            ROW_SET_TABLE => self.slot::<RowSet>(reference).map(|slot| slot.source),
            _ => None,
        }
    }

    /// 已提交文本；`None` 表示首次求值尚未成功。
    pub(crate) fn text_current(&self, reference: BindingRef) -> Option<&str> {
        self.slot::<String>(reference)
            .and_then(|slot| slot.current.as_deref())
    }

    /// 已提交布尔值；`None` 表示首次求值尚未成功。
    pub(crate) fn bool_current(&self, reference: BindingRef) -> Option<bool> {
        self.slot::<bool>(reference).and_then(|slot| slot.current)
    }

    /// 已提交的 inline 样式值；颜色/长度/不透明度映射为统一的样式值。
    pub(crate) fn inline_style(&self, reference: BindingRef) -> Option<StyleValue> {
        match reference.table() {
            COLOR_TABLE => self
                .slot::<Color>(reference)
                .and_then(|slot| slot.current)
                .map(StyleValue::Color),
            LENGTH_TABLE => self
                .slot::<Length>(reference)
                .and_then(|slot| slot.current)
                .map(StyleValue::Length),
            OPACITY_TABLE => self
                .slot::<Opacity>(reference)
                .and_then(|slot| slot.current)
                .map(|value| StyleValue::Opacity(value.value())),
            _ => None,
        }
    }

    pub(crate) fn live(&self) -> usize {
        self.text.live()
            + self.bools.live()
            + self.colors.live()
            + self.lengths.live()
            + self.opacities.live()
            + self.row_sets.live()
    }
}

/// 元素的稳定绑定索引；按 `PartId` 排序，同一 part 最多一项。
#[derive(Clone, Debug, Default)]
pub(crate) struct BindingList {
    entries: Vec<(PartId, BindingRef)>,
}

impl BindingList {
    /// 写入一个 part；返回被替换的旧引用，调用方必须把它归还绑定表。
    ///
    /// 正常挂载按 part 去重，替换只用于防御未来新增的写入路径。
    pub(crate) fn insert(&mut self, part: PartId, binding: BindingRef) -> Option<BindingRef> {
        match self.entries.binary_search_by_key(&part, |(part, _)| *part) {
            Ok(index) => Some(std::mem::replace(&mut self.entries[index].1, binding)),
            Err(index) => {
                self.entries.insert(index, (part, binding));
                None
            }
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (PartId, BindingRef)> + '_ {
        self.entries.iter().copied()
    }
}

/// 一个 part 在本 turn 的已提交变化。
///
/// `old` 是补丁在该 turn 第一次应用前的最后值，`new` 是最终值；同一
/// `(tree, node, part)` 在一个 turn 内只保留一条记录。结构 part（`ShowWhen`/
/// `ForEach`）排在值 part 之前，供快照与 delta 复用，不在构建快照时重新读取源。
pub(crate) struct BindingPatch {
    pub(crate) tree: TreeKey,
    pub(crate) node: NodeRef,
    pub(crate) part: BindingPart,
    pub(crate) old: Option<Box<dyn Any>>,
    pub(crate) new: Option<Box<dyn Any>>,
    /// 值不变但求值失败时的 typed fault；本 turn 的即时诊断走
    /// `RefreshReport.faults`，fault-only 变化不单独生成补丁。
    pub(crate) fault: Option<ReadFault>,
}

/// 补丁日志在某个时刻的标记；拟议结构失败时回滚到它。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BindingPatchMark(usize);

/// 一个 turn 内按 `(tree, node, part)` 去重的补丁记录。
///
/// 值已经是 `BindingSlot.current` 的权威结果；这里只保留“本 turn 变过什么”。
/// 同一 part 再次变化时保留首次 `old`、更新 `new`；净变化为零时撤销记录。
#[derive(Default)]
pub(crate) struct BindingPatchLog {
    patches: Vec<BindingPatch>,
    index: BTreeMap<(TreeKey, NodeRef, BindingPart), usize>,
}

impl BindingPatchLog {
    /// 记录当前日志位置；只有拟议结构预刷新会产生需要回滚的补丁。
    pub(crate) fn mark(&self) -> BindingPatchMark {
        BindingPatchMark(self.patches.len())
    }

    /// 丢弃标记之后记录的所有补丁（拟议分支被拒绝或释放），并重建去重索引。
    pub(crate) fn rollback(&mut self, mark: BindingPatchMark) {
        if mark.0 >= self.patches.len() {
            return;
        }
        self.patches.truncate(mark.0);
        self.index = self
            .patches
            .iter()
            .enumerate()
            .map(|(index, patch)| ((patch.tree, patch.node, patch.part), index))
            .collect();
    }

    /// 节点释放时丢弃它在本 turn 的补丁；turn 日志只保留仍挂载的节点。
    pub(crate) fn remove_node(&mut self, tree: TreeKey, node: NodeRef) {
        if self.patches.is_empty() {
            return;
        }
        let before = self.patches.len();
        self.patches
            .retain(|patch| patch.tree != tree || patch.node != node);
        if self.patches.len() != before {
            self.index = self
                .patches
                .iter()
                .enumerate()
                .map(|(index, patch)| ((patch.tree, patch.node, patch.part), index))
                .collect();
        }
    }

    /// 记录一次已提交变化；同一 part 一个 turn 内最多保留一条净记录。
    pub(crate) fn record<T: BindingValue>(
        &mut self,
        tree: TreeKey,
        node: NodeRef,
        part: BindingPart,
        old: Option<T>,
        new: Option<T>,
        fault: Option<ReadFault>,
    ) {
        let key = (tree, node, part);
        if let Some(&index) = self.index.get(&key) {
            let net_zero = match (self.patches[index].old.as_ref(), new.as_ref()) {
                (Some(stored), Some(value)) => stored
                    .downcast_ref::<T>()
                    .is_some_and(|stored| stored == value),
                _ => false,
            };
            if net_zero {
                self.remove(index);
                return;
            }
            let patch = &mut self.patches[index];
            patch.new = new.map(|value| Box::new(value) as Box<dyn Any>);
            patch.fault = fault;
            return;
        }
        let index = self.patches.len();
        self.patches.push(BindingPatch {
            tree,
            node,
            part,
            old: old.map(|value| Box::new(value) as Box<dyn Any>),
            new: new.map(|value| Box::new(value) as Box<dyn Any>),
            fault,
        });
        self.index.insert(key, index);
    }

    fn remove(&mut self, index: usize) {
        let removed = self.patches.swap_remove(index);
        self.index
            .remove(&(removed.tree, removed.node, removed.part));
        if let Some(moved) = self.patches.get(index) {
            // swap_remove 把末项移到 index；索引表必须跟随。
            self.index
                .insert((moved.tree, moved.node, moved.part), index);
        }
    }

    /// 取走本 turn 的补丁；结构 part 先于值 part，同一节点按 part 稳定序。
    pub(crate) fn take(&mut self) -> Vec<BindingPatch> {
        self.index.clear();
        let mut patches = std::mem::take(&mut self.patches);
        patches.sort_by(|left, right| {
            (left.part.rank(), left.tree, left.node, left.part).cmp(&(
                right.part.rank(),
                right.tree,
                right.node,
                right.part,
            ))
        });
        patches
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.patches.len()
    }

    #[cfg(test)]
    pub(crate) fn parts(&self) -> Vec<(TreeKey, NodeRef, BindingPart)> {
        self.patches
            .iter()
            .map(|patch| (patch.tree, patch.node, patch.part))
            .collect()
    }
}

/// 绑定消费者的打包引用；反向订阅边在增量 flush 切片接入。
///
/// 固定 16 B：NodeRef 8 B + PartId 2 B + kind/padding。
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct ConsumerRef {
    node: NodeRef,
    part: u16,
    kind: ConsumerKind,
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[expect(
    dead_code,
    reason = "reverse subscription edges are wired by the incremental flush slice"
)]
enum ConsumerKind {
    Binding = 0,
    Structural = 1,
}

const _: () = assert!(std::mem::size_of::<ConsumerRef>() == 16);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_layouts_match_the_arena_contract() {
        assert_eq!(std::mem::size_of::<NodeSlot>(), 64);
        assert!(std::mem::size_of::<ElementData>() <= 96);
        assert_eq!(std::mem::size_of::<ConsumerRef>(), 16);
        assert_eq!(std::mem::size_of::<Option<NodeRef>>(), 8);
        assert_eq!(std::mem::size_of::<PresentationKey>(), 8);
    }

    #[test]
    fn binding_refs_round_trip_and_reject_stale_generations() {
        let reference = BindingRef::new(TEXT_TABLE, 7, 3).expect("encodable");
        assert_eq!(reference.table(), TEXT_TABLE);
        assert_eq!(reference.slot(), 7);
        assert_eq!(reference.generation(), 3);
        assert_eq!(BindingRef::from_raw(reference.raw()), reference);
        assert!(BindingRef::new(TEXT_TABLE, 7, 0).is_none());
        assert!(BindingRef::new(TEXT_TABLE, 1 << 24, 1).is_none());
        assert_eq!(std::mem::size_of::<(PartId, BindingRef)>(), 16);
    }

    #[test]
    fn a_released_binding_slot_rejects_the_old_reference() {
        let mut table = BindingTable::new();
        let reference = table.insert::<String>(ReadSlot::Static, Some(String::from("first")), None);
        assert_eq!(
            table
                .slot::<String>(reference)
                .and_then(|slot| slot.current.clone()),
            Some(String::from("first"))
        );
        assert!(table.release(reference));
        assert!(table.slot::<String>(reference).is_none());
        assert!(!table.release(reference));

        let reused = table.insert::<String>(ReadSlot::Static, Some(String::from("second")), None);
        assert_eq!(reused.slot(), reference.slot());
        assert_ne!(reused.generation(), reference.generation());
        assert!(table.slot::<String>(reference).is_none());
        assert_eq!(
            table
                .slot::<String>(reused)
                .and_then(|slot| slot.current.clone()),
            Some(String::from("second"))
        );
        assert_eq!(table.live(), 1);
        assert!(table.release(reused));
        assert_eq!(table.live(), 0);
    }

    #[test]
    fn binding_parts_register_once_and_sort_structure_first() {
        let mut table = BindingTable::new();
        let when = table.part_id(BindingPart::ShowWhen);
        let text = table.part_id(BindingPart::Text);
        let attr = table.part_id(BindingPart::Attr("aria:label"));
        assert_eq!(table.part_id(BindingPart::ShowWhen), when);
        assert_eq!(table.part_id(BindingPart::Text), text);
        assert_eq!(table.part(when), Some(BindingPart::ShowWhen));
        assert!(when < text);
        assert!(text < attr);

        let mut list = BindingList::default();
        assert!(
            list.insert(text, BindingRef::new(TEXT_TABLE, 0, 1).expect("ref"))
                .is_none()
        );
        assert!(
            list.insert(when, BindingRef::new(BOOL_TABLE, 0, 1).expect("ref"))
                .is_none()
        );
        let parts: Vec<PartId> = list.iter().map(|(part, _)| part).collect();
        assert_eq!(parts, vec![when, text]);
        assert_eq!(std::mem::size_of::<(PartId, BindingRef)>(), 16);
    }

    #[test]
    fn binding_types_stay_in_their_typed_tables() {
        let mut table = BindingTable::new();
        let text = table.insert::<String>(ReadSlot::Static, None, None);
        let flag = table.insert::<bool>(ReadSlot::Static, Some(true), None);
        assert!(table.slot::<bool>(text).is_none());
        assert!(table.slot::<String>(flag).is_none());
        assert!(table.slot::<bool>(flag).is_some());
        assert!(table.release(text));
        assert!(table.release(flag));
    }

    #[test]
    fn binding_patches_dedupe_by_node_part_and_apply_structure_first() {
        let tree = TreeKey::from_raw(3);
        let node = NodeRef::new(1, 1).expect("node ref");
        let mut log = BindingPatchLog::default();
        log.record(
            tree,
            node,
            BindingPart::Text,
            Some(String::from("a")),
            Some(String::from("b")),
            None,
        );
        log.record(
            tree,
            node,
            BindingPart::ShowWhen,
            Some(false),
            Some(true),
            None,
        );
        assert_eq!(log.len(), 2);
        // 同一 part 的第二次变化只更新最终值，保留首次 old。
        log.record(
            tree,
            node,
            BindingPart::Text,
            Some(String::from("b")),
            Some(String::from("c")),
            None,
        );
        assert_eq!(log.len(), 2);

        let patches = log.take();
        assert!(log.parts().is_empty(), "taking the log empties it");
        assert_eq!(patches.len(), 2);
        assert_eq!(patches[0].part, BindingPart::ShowWhen);
        assert_eq!(patches[1].part, BindingPart::Text);
        let old = patches[1]
            .old
            .as_ref()
            .and_then(|value| value.downcast_ref::<String>())
            .map(String::as_str);
        let new = patches[1]
            .new
            .as_ref()
            .and_then(|value| value.downcast_ref::<String>())
            .map(String::as_str);
        assert_eq!(old, Some("a"));
        assert_eq!(new, Some("c"));
    }

    #[test]
    fn a_patch_that_returns_to_its_first_value_is_dropped() {
        let tree = TreeKey::from_raw(4);
        let node = NodeRef::new(2, 1).expect("node ref");
        let mut log = BindingPatchLog::default();
        log.record(
            tree,
            node,
            BindingPart::Text,
            Some(String::from("a")),
            Some(String::from("b")),
            None,
        );
        log.record(
            tree,
            node,
            BindingPart::Text,
            Some(String::from("b")),
            Some(String::from("a")),
            None,
        );
        assert_eq!(log.len(), 0, "the net change over the turn is zero");
        assert!(log.take().is_empty());
    }

    #[test]
    fn removing_a_released_node_drops_its_patches_and_rebuilds_the_index() {
        let tree = TreeKey::from_raw(5);
        let released = NodeRef::new(2, 1).expect("node ref");
        let live = NodeRef::new(3, 1).expect("node ref");
        let mut log = BindingPatchLog::default();
        log.record(
            tree,
            released,
            BindingPart::Text,
            Some(String::from("a")),
            Some(String::from("b")),
            None,
        );
        log.record(
            tree,
            released,
            BindingPart::ShowWhen,
            Some(false),
            Some(true),
            None,
        );
        log.record(
            tree,
            live,
            BindingPart::Text,
            Some(String::from("x")),
            Some(String::from("y")),
            None,
        );

        log.remove_node(tree, released);
        assert_eq!(
            log.parts(),
            vec![(tree, live, BindingPart::Text)],
            "only the live node's patch remains"
        );
        // 索引已重建：再次记录同一节点会新增一条记录，而不是命中旧位置。
        log.record(
            tree,
            released,
            BindingPart::Text,
            None,
            Some(String::from("c")),
            None,
        );
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn node_ref_and_presentation_key_round_trip() {
        let node = NodeRef::new(7, 3).expect("nonzero generation");
        assert_eq!(node.slot(), 7);
        assert_eq!(node.generation(), 3);
        #[cfg(test)]
        {
            assert_eq!(NodeRef::from_raw(node.raw()), Some(node));
        }
        assert_eq!(NodeRef::new(0, 0), None);

        let key = PresentationKey::from_node(node);
        assert_eq!(key.raw(), node.raw());
    }

    #[test]
    fn a_recycled_slot_rejects_the_old_generation() {
        let mut arena = SlotArena::new();
        let tree = TreeKey::from_raw(0);
        let first = arena
            .insert(NodeKindTag::Text, NO_PAYLOAD, tree, None)
            .expect("insert");
        arena.release(first);
        assert!(matches!(
            arena.get(first),
            Err(NodeError::StaleGeneration { .. })
        ));

        let second = arena
            .insert(NodeKindTag::Text, NO_PAYLOAD, tree, None)
            .expect("insert");
        assert_eq!(first.slot(), second.slot());
        assert_ne!(first.generation(), second.generation());
        assert!(matches!(
            arena.get(first),
            Err(NodeError::StaleGeneration { .. })
        ));
        assert!(arena.get(second).is_ok());
        assert!(arena.require_kind(second, NodeKindTag::Element).is_err());
        assert!(arena.require_kind(second, NodeKindTag::Text).is_ok());
    }

    #[test]
    fn parent_and_sibling_links_are_maintained() {
        let mut arena = SlotArena::new();
        let tree = TreeKey::from_raw(0);
        let parent = arena
            .insert(NodeKindTag::Element, 0, tree, None)
            .expect("parent");
        let first = arena
            .insert(NodeKindTag::Text, NO_PAYLOAD, tree, Some(parent))
            .expect("first");
        let second = arena
            .insert(NodeKindTag::Text, NO_PAYLOAD, tree, Some(parent))
            .expect("second");

        let parent_slot = arena.get(parent).expect("parent");
        assert_eq!(parent_slot.first_child, Some(first));
        assert_eq!(parent_slot.last_child, Some(second));
        assert_eq!(arena.get(first).expect("first").next_sibling, Some(second));
        assert_eq!(arena.get(second).expect("second").prev_sibling, Some(first));

        arena.release(first);
        let parent_slot = arena.get(parent).expect("parent");
        assert_eq!(parent_slot.first_child, Some(second));
        assert_eq!(arena.get(second).expect("second").prev_sibling, None);
    }

    #[test]
    fn payload_pool_recycles_a_slot_without_reusing_a_live_handle() {
        let mut pool = Pool::new();
        let first = pool.reserve(String::from("first"));
        assert_eq!(pool.get(first).map(String::as_str), Some("first"));
        assert_eq!(pool.release(first).as_deref(), Some("first"));
        assert!(pool.get(first).is_none());
        assert_eq!(pool.live(), 0);

        let second = pool.reserve(String::from("second"));
        assert_eq!(first, second, "the payload slot is reused");
        assert_eq!(pool.get(second).map(String::as_str), Some("second"));
        assert_eq!(pool.storage_len(), 1);
    }
}
