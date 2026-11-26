//! UI 节点身份与 arena 扩展层。
//!
//! 本切片在现有 mounted-node 存储旁边建立统一的节点身份：每个元素、文本、
//! 组件、Show 与 For 节点都在 [`NodeArena`] 中有一个槽位和 generation。
//! 公开元素身份 [`crate::NodeId`] 内部由 [`NodeRef`] 组成，呈现层使用
//! [`PresentationKey`]；旧的 payload 存储保持不变，契约切片再把它们迁入。

use std::fmt;
use std::num::NonZeroU64;

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

    /// 只用于验证编码的往返测试；呈现层只持有 opaque 值。
    #[cfg(test)]
    pub(crate) const fn node(self) -> Option<NodeRef> {
        NodeRef::from_raw(self.0)
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
#[derive(Debug)]
pub(crate) struct NodeSlot {
    pub(crate) payload: u32,
    pub(crate) tree: TreeKey,
    pub(crate) kind: NodeKindTag,
    /// 脏路径在契约切片（#327）接入；本切片先固定字段与默认值。
    #[expect(
        dead_code,
        reason = "the dirty path lands with the arena contract (#327)"
    )]
    pub(crate) dirty: DirtyFlags,
    pub(crate) parent: Option<NodeRef>,
    pub(crate) first_child: Option<NodeRef>,
    pub(crate) last_child: Option<NodeRef>,
    pub(crate) prev_sibling: Option<NodeRef>,
    pub(crate) next_sibling: Option<NodeRef>,
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

struct Arena<T> {
    values: Vec<Option<T>>,
    generations: Vec<u32>,
    free: Vec<u32>,
    live: usize,
}

impl<T> Arena<T> {
    const fn new() -> Self {
        Self {
            values: Vec::new(),
            generations: Vec::new(),
            free: Vec::new(),
            live: 0,
        }
    }

    fn reserve(&mut self) -> (u32, u32) {
        match self.free.pop() {
            Some(slot) => (slot, self.generations[slot as usize]),
            None => {
                let slot = u32::try_from(self.values.len()).expect("node arena exhausted");
                self.values.push(None);
                self.generations.push(1);
                (slot, 1)
            }
        }
    }

    fn fill(&mut self, slot: u32, value: T) {
        self.values[slot as usize] = Some(value);
        self.live += 1;
    }

    fn get(&self, slot: u32, generation: u32) -> Result<&T, NodeError> {
        let Some(entry) = self.values.get(slot as usize) else {
            return Err(NodeError::UnknownSlot { slot });
        };
        let Some(value) = entry.as_ref() else {
            return Err(NodeError::Vacant { slot });
        };
        if self.generations[slot as usize] != generation {
            return Err(NodeError::StaleGeneration {
                slot,
                live: self.generations[slot as usize],
                found: generation,
            });
        }
        Ok(value)
    }

    fn get_mut(&mut self, slot: u32, generation: u32) -> Result<&mut T, NodeError> {
        let Some(entry) = self.values.get_mut(slot as usize) else {
            return Err(NodeError::UnknownSlot { slot });
        };
        let Some(value) = entry.as_mut() else {
            return Err(NodeError::Vacant { slot });
        };
        if self.generations[slot as usize] != generation {
            return Err(NodeError::StaleGeneration {
                slot,
                live: self.generations[slot as usize],
                found: generation,
            });
        }
        Ok(value)
    }

    fn release(&mut self, slot: u32) -> bool {
        let Some(entry) = self.values.get_mut(slot as usize) else {
            return false;
        };
        if entry.take().is_none() {
            return false;
        }
        self.live -= 1;
        let generation = self.generations[slot as usize];
        if generation == u32::MAX {
            // 代际耗尽：该槽退休，不再复用。
            return true;
        }
        self.generations[slot as usize] = generation + 1;
        self.free.push(slot);
        true
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    const fn live(&self) -> usize {
        self.live
    }
}

/// 统一 UI 节点 arena；payload 仍指向旧的 mounted-node 存储。
pub(crate) struct NodeArena {
    slots: Arena<NodeSlot>,
    next_tree: u32,
    aux: AuxArenas,
}

impl NodeArena {
    pub(crate) const fn new() -> Self {
        Self {
            slots: Arena::new(),
            next_tree: 0,
            aux: AuxArenas::new(),
        }
    }

    /// 分配一棵树的稳定键；耗尽时是 fatal 开发期错误。
    pub(crate) fn allocate_tree(&mut self) -> TreeKey {
        let key = TreeKey::from_raw(self.next_tree);
        self.next_tree = self
            .next_tree
            .checked_add(1)
            .expect("tree key space exhausted");
        key
    }

    /// 插入节点并把 child 链到 `parent` 的末尾。
    pub(crate) fn insert(
        &mut self,
        kind: NodeKindTag,
        payload: u32,
        tree: TreeKey,
        parent: Option<NodeRef>,
    ) -> Result<NodeRef, NodeError> {
        if let Some(parent) = parent {
            self.get(parent)?;
        }
        let (slot, generation) = self.slots.reserve();
        let node = NodeRef::new(slot, generation).expect("arena generations start at 1");
        let mut previous_sibling = None;
        if let Some(parent_ref) = parent {
            let parent_slot = self.get_mut(parent_ref)?;
            debug_assert_eq!(
                parent_slot.tree, tree,
                "a child node must belong to its parent's tree"
            );
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
        self.slots.fill(
            slot,
            NodeSlot {
                payload,
                tree,
                kind,
                dirty: DirtyFlags::NONE,
                parent,
                first_child: None,
                last_child: None,
                prev_sibling: previous_sibling,
                next_sibling: None,
            },
        );
        Ok(node)
    }

    pub(crate) fn get(&self, node: NodeRef) -> Result<&NodeSlot, NodeError> {
        self.slots.get(node.slot(), node.generation())
    }

    pub(crate) fn get_mut(&mut self, node: NodeRef) -> Result<&mut NodeSlot, NodeError> {
        self.slots.get_mut(node.slot(), node.generation())
    }

    pub(crate) fn require_kind(
        &self,
        node: NodeRef,
        kind: NodeKindTag,
    ) -> Result<&NodeSlot, NodeError> {
        let slot = self.get(node)?;
        if slot.kind == kind {
            Ok(slot)
        } else {
            Err(NodeError::WrongKind {
                expected: kind,
                found: slot.kind,
            })
        }
    }

    pub(crate) fn set_payload(&mut self, node: NodeRef, payload: u32) -> Result<(), NodeError> {
        self.get_mut(node)?.payload = payload;
        Ok(())
    }

    /// 释放节点并解除它与父/兄弟的链接；重复释放无操作。
    pub(crate) fn release(&mut self, node: NodeRef) {
        let Ok(slot) = self.slots.get(node.slot(), node.generation()) else {
            return;
        };
        let parent = slot.parent;
        let previous = slot.prev_sibling;
        let next = slot.next_sibling;
        let first = slot.first_child;
        if first.is_some() && parent.is_none() {
            // 子节点仍在；它们由各自的 owner 释放，链在释放时修复。
        }
        if let Some(previous) = previous
            && let Ok(previous_slot) = self.slots.get_mut(previous.slot(), previous.generation())
        {
            previous_slot.next_sibling = next;
        }
        if let Some(next) = next
            && let Ok(next_slot) = self.slots.get_mut(next.slot(), next.generation())
        {
            next_slot.prev_sibling = previous;
        }
        if let Some(parent) = parent
            && let Ok(parent_slot) = self.slots.get_mut(parent.slot(), parent.generation())
        {
            if parent_slot.first_child == Some(node) {
                parent_slot.first_child = next;
            }
            if parent_slot.last_child == Some(node) {
                parent_slot.last_child = previous;
            }
        }
        self.slots.release(node.slot());
    }

    /// 槽位编号是否曾被分配；用于把已释放身份判为 stale 而不是 unknown。
    pub(crate) fn slot_exists(&self, node: NodeRef) -> bool {
        (node.slot() as usize) < self.slots.len()
    }

    pub(crate) fn live(&self) -> usize {
        self.slots.live()
    }

    pub(crate) fn storage_len(&self) -> usize {
        self.slots.len()
    }

    pub(crate) fn reserve_element_aux(&mut self) -> AuxHandles {
        self.aux.reserve()
    }

    pub(crate) fn release_element_aux(&mut self, handles: AuxHandles) {
        self.aux.release(handles);
    }

    /// 当前存活的辅助句柄总数；测试用它验证元素卸载时的释放。
    #[cfg(test)]
    pub(crate) const fn aux_live(&self) -> usize {
        self.aux.live()
    }
}

macro_rules! list_id {
    ($($name:ident),+ $(,)?) => {
        $(
            #[derive(Clone, Copy, PartialEq, Eq, Debug)]
            pub(crate) struct $name(u32);

            impl $name {
                pub(crate) const NONE: Self = Self(u32::MAX);
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

/// 一个元素在辅助 arena 中的句柄集合；本切片的列表内容仍留在旧存储里。
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

impl AuxHandles {
    pub(crate) const NONE: Self = Self {
        attrs: AttrListId::NONE,
        class_styles: StyleListId::NONE,
        inline_styles: StyleListId::NONE,
        events: EventListId::NONE,
        edit: EditStateId::NONE,
        bindings: BindingListId::NONE,
        semantics: SemanticsId::NONE,
    };
}

struct ListArena {
    used: Vec<bool>,
    free: Vec<u32>,
    live: usize,
}

impl ListArena {
    const fn new() -> Self {
        Self {
            used: Vec::new(),
            free: Vec::new(),
            live: 0,
        }
    }

    fn reserve(&mut self) -> u32 {
        match self.free.pop() {
            Some(slot) => {
                self.used[slot as usize] = true;
                self.live += 1;
                slot
            }
            None => {
                let slot = u32::try_from(self.used.len()).expect("aux arena exhausted");
                self.used.push(true);
                self.live += 1;
                slot
            }
        }
    }

    fn release(&mut self, slot: u32) {
        let Some(used) = self.used.get_mut(slot as usize) else {
            return;
        };
        if *used {
            *used = false;
            self.live -= 1;
            self.free.push(slot);
        }
    }

    const fn live(&self) -> usize {
        self.live
    }
}

/// 按树拥有的辅助 arena；列表、编辑与语义数据在契约切片迁入。
pub(crate) struct AuxArenas {
    attrs: ListArena,
    class_styles: ListArena,
    inline_styles: ListArena,
    events: ListArena,
    edits: ListArena,
    binding_lists: ListArena,
    semantics: ListArena,
}

impl AuxArenas {
    const fn new() -> Self {
        Self {
            attrs: ListArena::new(),
            class_styles: ListArena::new(),
            inline_styles: ListArena::new(),
            events: ListArena::new(),
            edits: ListArena::new(),
            binding_lists: ListArena::new(),
            semantics: ListArena::new(),
        }
    }

    fn reserve(&mut self) -> AuxHandles {
        AuxHandles {
            attrs: AttrListId(self.attrs.reserve()),
            class_styles: StyleListId(self.class_styles.reserve()),
            inline_styles: StyleListId(self.inline_styles.reserve()),
            events: EventListId(self.events.reserve()),
            edit: EditStateId(self.edits.reserve()),
            bindings: BindingListId(self.binding_lists.reserve()),
            semantics: SemanticsId(self.semantics.reserve()),
        }
    }

    fn release(&mut self, handles: AuxHandles) {
        self.attrs.release(handles.attrs.0);
        self.class_styles.release(handles.class_styles.0);
        self.inline_styles.release(handles.inline_styles.0);
        self.events.release(handles.events.0);
        self.edits.release(handles.edit.0);
        self.binding_lists.release(handles.bindings.0);
        self.semantics.release(handles.semantics.0);
    }

    pub(crate) const fn live(&self) -> usize {
        self.attrs.live()
            + self.class_styles.live()
            + self.inline_styles.live()
            + self.events.live()
            + self.edits.live()
            + self.binding_lists.live()
            + self.semantics.live()
    }
}

impl fmt::Debug for NodeArena {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeArena")
            .field("live", &self.live())
            .field("storage", &self.storage_len())
            .field("aux_live", &self.aux.live())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ref_and_presentation_key_round_trip() {
        let node = NodeRef::new(7, 3).expect("nonzero generation");
        assert_eq!(node.slot(), 7);
        assert_eq!(node.generation(), 3);
        assert_eq!(NodeRef::from_raw(node.raw()), Some(node));
        assert_eq!(NodeRef::new(0, 0), None);

        let key = PresentationKey::from_node(node);
        assert_eq!(key.raw(), node.raw());
        assert_eq!(key.node(), Some(node));
    }

    #[test]
    fn a_recycled_slot_rejects_the_old_generation() {
        let mut arena = NodeArena::new();
        let tree = arena.allocate_tree();
        let first = arena
            .insert(NodeKindTag::Text, NO_PAYLOAD, tree, None)
            .expect("insert");
        arena.release(first);
        assert!(matches!(arena.get(first), Err(NodeError::Vacant { .. })));

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
        let mut arena = NodeArena::new();
        let tree = arena.allocate_tree();
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
    fn tree_keys_are_monotonic_and_not_reused() {
        let mut arena = NodeArena::new();
        let first = arena.allocate_tree();
        let second = arena.allocate_tree();
        let third = arena.allocate_tree();
        assert!(
            first < second && second < third,
            "tree keys increase monotonically"
        );
    }

    #[test]
    fn auxiliary_handles_are_allocated_and_released_per_element() {
        let mut arena = NodeArena::new();
        let first = arena.reserve_element_aux();
        assert_ne!(first.attrs, AttrListId::NONE);
        assert_eq!(arena.aux.live(), 7);
        arena.release_element_aux(first);
        assert_eq!(arena.aux.live(), 0);

        let second = arena.reserve_element_aux();
        assert_eq!(first.attrs, second.attrs, "the handle slot is reused");
        arena.release_element_aux(second);
    }
}
