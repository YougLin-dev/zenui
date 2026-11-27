//! 相邻两次可访问性发布之间的事件规划与投递。
//!
//! 规划器只比较两棵已提交的树，不读取呈现瞬态：指针 hover/press、caret 闪烁与
//! 滚动几何都不会单独产生事件。节点身份使用 UIA 已经看到的 provider 本地
//! runtime ID，它在发布之间保持稳定，卸载后不再复用。
//!
//! 事件在发布线程上、发布锁释放之后投递。UIA 可能在回调期间同步查询
//! provider，持锁投递会与属性读取互相等待。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    IRawElementProviderSimple, StructureChangeType_ChildAdded, StructureChangeType_ChildRemoved,
    StructureChangeType_ChildrenReordered, ToggleState_Off, ToggleState_On,
    UIA_AutomationFocusChangedEventId, UIA_IsEnabledPropertyId, UIA_ToggleToggleStatePropertyId,
    UIA_ValueValuePropertyId, UiaAppendRuntimeId, UiaRaiseAutomationEvent,
    UiaRaiseAutomationPropertyChangedEvent, UiaRaiseStructureChangedEvent,
};

use super::{
    AccessibilityTree, AccessibleNode, ProviderKey, WindowAccessibility, ZenProvider, bool_variant,
    bstr_variant, i32_variant,
};

/// 一次发布对某个节点属性造成的变化。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum PropertyChange {
    Enabled {
        old: bool,
        new: bool,
    },
    /// `checked` 与 `aria:pressed` 共用 UIA 的 ToggleState 属性。
    ToggleState {
        old: bool,
        new: bool,
    },
    Value {
        old: String,
        new: String,
    },
}

/// 一条待投递的 UIA 事件。
///
/// 节点目标使用 runtime ID；`None` 表示窗口根。根没有 provider 本地 runtime ID，
/// 由宿主窗口的默认运行时 ID 标识。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Event {
    Focus {
        node: i32,
    },
    Property {
        target: Option<i32>,
        change: PropertyChange,
    },
    ChildAdded {
        child: i32,
    },
    ChildRemoved {
        parent: Option<i32>,
        child: i32,
    },
    ChildrenReordered {
        parent: Option<i32>,
    },
}

/// 一次提交产生的事件序列；顺序固定为结构、属性、焦点。
pub(super) struct EventPlan(Vec<Event>);

/// 比较上一棵发布树与本次树，生成至多一条事件对应一处变化的计划。
///
/// `foreground` 是发布时的窗口前台状态。后台窗口的焦点变化不投递聚焦事件；
/// 之后窗口激活只改变 `HasKeyboardFocus` 查询结果，不会补发焦点事件。
pub(super) fn plan(
    previous: Option<&AccessibilityTree>,
    next: &AccessibilityTree,
    foreground: bool,
) -> EventPlan {
    let mut events = Vec::new();
    let Some(previous) = previous else {
        return EventPlan(events);
    };
    let previous_by_runtime: HashMap<i32, &AccessibleNode> = previous
        .iter()
        .filter_map(|node| node.runtime_id.map(|id| (id, node)))
        .collect();

    // 结构：按前序遍历本次树的父节点。新子树只在本层产生一条 ChildAdded；
    // 其内部子节点由客户端在收到事件后重新读取。已移除的子树同理。
    for parent in next.iter() {
        let old_parent = match parent.runtime_id {
            None => previous.root(),
            Some(id) => match previous_by_runtime.get(&id) {
                Some(node) => *node,
                None => continue,
            },
        };
        let old_children: Vec<Option<i32>> = previous
            .children(old_parent)
            .map(|node| node.runtime_id)
            .collect();
        let new_children: Vec<Option<i32>> =
            next.children(parent).map(|node| node.runtime_id).collect();
        let old_ids: HashSet<i32> = old_children.iter().flatten().copied().collect();
        let new_ids: HashSet<i32> = new_children.iter().flatten().copied().collect();
        let removed: Vec<i32> = old_children
            .iter()
            .flatten()
            .copied()
            .filter(|id| !new_ids.contains(id))
            .collect();
        let added: Vec<i32> = new_children
            .iter()
            .flatten()
            .copied()
            .filter(|id| !old_ids.contains(id))
            .collect();
        if removed.is_empty() && added.is_empty() {
            let same_order = old_children
                .iter()
                .copied()
                .eq(new_children.iter().copied());
            if !same_order {
                events.push(Event::ChildrenReordered {
                    parent: parent.runtime_id,
                });
            }
        } else {
            // 添加/移除会移动余下子节点的位置；只有幸存子序列的相对顺序才
            // 表示真正的重排，避免把位移误报成 ChildrenReordered。
            let survivors_old: Vec<i32> = old_children
                .iter()
                .flatten()
                .copied()
                .filter(|id| new_ids.contains(id))
                .collect();
            let survivors_new: Vec<i32> = new_children
                .iter()
                .flatten()
                .copied()
                .filter(|id| old_ids.contains(id))
                .collect();
            for child in removed {
                events.push(Event::ChildRemoved {
                    parent: parent.runtime_id,
                    child,
                });
            }
            for child in added {
                events.push(Event::ChildAdded { child });
            }
            if survivors_old != survivors_new {
                events.push(Event::ChildrenReordered {
                    parent: parent.runtime_id,
                });
            }
        }
    }

    // 属性：只比较两次发布都存在的节点；新节点的初始状态由结构事件表达。
    // 根节点对应 HWND，原生启用状态由宿主窗口 provider 自己通知；这里再发
    // 一条同目标事件会让客户端收到两次 IsEnabled。
    for node in next.iter() {
        let Some(runtime_id) = node.runtime_id else {
            continue;
        };
        let Some(old) = previous_by_runtime.get(&runtime_id).copied() else {
            continue;
        };
        if old.enabled != node.enabled {
            events.push(Event::Property {
                target: Some(runtime_id),
                change: PropertyChange::Enabled {
                    old: old.enabled,
                    new: node.enabled,
                },
            });
        }
        if let (Some(old_state), Some(new_state)) = (old.toggle_state, node.toggle_state)
            && old_state != new_state
        {
            events.push(Event::Property {
                target: Some(runtime_id),
                change: PropertyChange::ToggleState {
                    old: old_state,
                    new: new_state,
                },
            });
        }
        if let (Some(old_value), Some(new_value)) = (old.value.as_deref(), node.value.as_deref())
            && old_value != new_value
        {
            events.push(Event::Property {
                target: Some(runtime_id),
                change: PropertyChange::Value {
                    old: old_value.to_owned(),
                    new: new_value.to_owned(),
                },
            });
        }
    }

    // 焦点最后投递：客户端先看到结构变化，再按新节点定位焦点。
    if foreground {
        let old_focus = focused_runtime(previous);
        let new_focus = focused_runtime(next);
        if old_focus != new_focus
            && let Some(node) = new_focus
        {
            events.push(Event::Focus { node });
        }
    }

    EventPlan(events)
}

fn focused_runtime(tree: &AccessibilityTree) -> Option<i32> {
    tree.iter()
        .find(|node| node.focused)
        .and_then(|node| node.runtime_id)
}

/// 在发布线程上投递计划；单条事件失败不会阻止后续事件。
///
/// 目标是本次刚发布的树中的节点；`tree` 与计划和发布在同一线程上产生，
/// 中间不会插入另一次发布。
pub(super) fn raise(window: &Arc<WindowAccessibility>, tree: &AccessibilityTree, plan: &EventPlan) {
    for event in &plan.0 {
        match event {
            Event::Focus { node } => {
                let Some(provider) = provider_for(window, tree, Some(*node)) else {
                    continue;
                };
                // 安全：provider 在一次调用期间有效；UIA 只读取它并转发事件。
                let _ = unsafe {
                    UiaRaiseAutomationEvent(&provider, UIA_AutomationFocusChangedEventId)
                };
            }
            Event::Property { target, change } => {
                let Some(provider) = provider_for(window, tree, *target) else {
                    continue;
                };
                let (property, old, new) = match change {
                    PropertyChange::Enabled { old, new } => (
                        UIA_IsEnabledPropertyId,
                        bool_variant(*old),
                        bool_variant(*new),
                    ),
                    PropertyChange::ToggleState { old, new } => (
                        UIA_ToggleToggleStatePropertyId,
                        toggle_variant(*old),
                        toggle_variant(*new),
                    ),
                    PropertyChange::Value { old, new } => (
                        UIA_ValueValuePropertyId,
                        bstr_variant(old),
                        bstr_variant(new),
                    ),
                };
                // 安全：两个 VARIANT 在调用期间存活；UIA 复制事件参数。
                let _ = unsafe {
                    UiaRaiseAutomationPropertyChangedEvent(&provider, property, &old, &new)
                };
            }
            Event::ChildAdded { child } => {
                let Some(provider) = provider_for(window, tree, Some(*child)) else {
                    continue;
                };
                // 安全：新增子节点的 provider 在调用期间有效；其余结构参数为空。
                let _ = unsafe {
                    UiaRaiseStructureChangedEvent(
                        &provider,
                        StructureChangeType_ChildAdded,
                        std::ptr::null_mut(),
                        0,
                    )
                };
            }
            Event::ChildRemoved { parent, child } => {
                let Some(provider) = provider_for(window, tree, *parent) else {
                    continue;
                };
                // 安全：runtime ID 数组在调用期间存活；移除事件按 UIA 契约
                // 携带被移除子节点的 runtime ID。
                let mut runtime_id = [UiaAppendRuntimeId as i32, *child];
                let _ = unsafe {
                    UiaRaiseStructureChangedEvent(
                        &provider,
                        StructureChangeType_ChildRemoved,
                        runtime_id.as_mut_ptr(),
                        runtime_id.len() as i32,
                    )
                };
            }
            Event::ChildrenReordered { parent } => {
                let Some(provider) = provider_for(window, tree, *parent) else {
                    continue;
                };
                // 安全：重排事件的目标是父节点；runtime ID 参数为空。
                let _ = unsafe {
                    UiaRaiseStructureChangedEvent(
                        &provider,
                        StructureChangeType_ChildrenReordered,
                        std::ptr::null_mut(),
                        0,
                    )
                };
            }
        }
    }
}

fn provider_for(
    window: &Arc<WindowAccessibility>,
    tree: &AccessibilityTree,
    target: Option<i32>,
) -> Option<IRawElementProviderSimple> {
    let key = match target {
        None => ProviderKey::Window,
        Some(runtime_id) => {
            let node = tree
                .iter()
                .find(|node| node.runtime_id == Some(runtime_id))?;
            ProviderKey::Node(node.node_id()?)
        }
    };
    Some(
        ZenProvider {
            window: Arc::downgrade(window),
            key,
        }
        .into(),
    )
}

fn toggle_variant(value: bool) -> VARIANT {
    let state = if value {
        ToggleState_On
    } else {
        ToggleState_Off
    };
    i32_variant(state.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accessibility::{ControlType, ScrollGeometry};
    use zenui::SnapshotActions;
    use zenui_render::Rect;

    #[derive(Clone, Copy)]
    struct Spec {
        runtime: i32,
        parent: Option<usize>,
        enabled: bool,
        focused: bool,
        toggle: Option<bool>,
        value: Option<&'static str>,
    }

    impl Spec {
        fn child(runtime: i32) -> Self {
            Self {
                runtime,
                parent: None,
                enabled: true,
                focused: false,
                toggle: None,
                value: None,
            }
        }

        fn under(runtime: i32, parent: usize) -> Self {
            Self {
                parent: Some(parent),
                ..Self::child(runtime)
            }
        }

        fn focused(self) -> Self {
            Self {
                focused: true,
                ..self
            }
        }

        fn disabled(self) -> Self {
            Self {
                enabled: false,
                ..self
            }
        }

        fn toggled(self, state: bool) -> Self {
            Self {
                toggle: Some(state),
                ..self
            }
        }

        fn valued(self, value: &'static str) -> Self {
            Self {
                value: Some(value),
                ..self
            }
        }
    }

    /// 合成树只用于纯规划测试：节点身份是 runtime ID，不构造 NodeId。
    fn tree(root_enabled: bool, specs: &[Spec]) -> AccessibilityTree {
        let mut nodes = vec![AccessibleNode {
            node_id: None,
            runtime_id: None,
            control_type: ControlType::Window,
            tag: "test",
            name: String::new(),
            automation_id: String::new(),
            enabled: root_enabled,
            focusable: false,
            focused: false,
            offscreen: false,
            toggle_state: None,
            actions: SnapshotActions::default(),
            value: None,
            composing: false,
            text: None,
            rect: Rect::new(0.0, 0.0, 100.0, 100.0),
            parent: None,
            children: Vec::new(),
        }];
        for spec in specs {
            let index = nodes.len();
            let parent = spec.parent.map_or(0, |parent| parent + 1);
            nodes.push(AccessibleNode {
                node_id: None,
                runtime_id: Some(spec.runtime),
                control_type: ControlType::Custom,
                tag: "test",
                name: String::new(),
                automation_id: String::new(),
                enabled: spec.enabled,
                focusable: true,
                focused: spec.focused,
                offscreen: false,
                toggle_state: spec.toggle,
                actions: SnapshotActions::default(),
                value: spec.value.map(str::to_owned),
                composing: false,
                text: None,
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                parent: Some(parent),
                children: Vec::new(),
            });
            nodes[parent].children.push(index);
        }
        AccessibilityTree {
            nodes,
            by_node: HashMap::new(),
            scale: 1.0,
            scroll: ScrollGeometry::new(100.0, 100.0, 0, 0),
        }
    }

    fn events(
        previous: &AccessibilityTree,
        next: &AccessibilityTree,
        foreground: bool,
    ) -> Vec<Event> {
        plan(Some(previous), next, foreground).0
    }

    #[test]
    fn unchanged_and_first_trees_emit_nothing() {
        let old = tree(true, &[Spec::child(1).focused(), Spec::child(2)]);
        let next = tree(true, &[Spec::child(1).focused(), Spec::child(2)]);
        assert!(events(&old, &next, true).is_empty());
        assert!(plan(None, &next, true).0.is_empty());
    }

    #[test]
    fn focus_changes_are_announced_once_only_while_foreground() {
        let old = tree(true, &[Spec::child(1).focused(), Spec::child(2)]);
        let next = tree(true, &[Spec::child(1), Spec::child(2).focused()]);
        assert_eq!(events(&old, &next, true), [Event::Focus { node: 2 }]);
        assert!(events(&old, &next, false).is_empty());
    }

    #[test]
    fn a_background_focus_change_is_not_replayed_on_activation() {
        let old = tree(true, &[Spec::child(1).focused(), Spec::child(2)]);
        let next = tree(true, &[Spec::child(1), Spec::child(2).focused()]);
        assert!(events(&old, &next, false).is_empty());
        // 激活后 HasKeyboardFocus 由查询返回 true，但焦点没有再次变化。
        assert!(events(&next, &next, true).is_empty());
    }

    #[test]
    fn clearing_focus_is_not_a_focus_event() {
        // UIA 没有“焦点丢失”事件；客户端以 GetFocus/GetFocusedElement 为空
        // 观察这一状态，不能把根节点当作新焦点来播报。
        let old = tree(true, &[Spec::child(1).focused(), Spec::child(2)]);
        let next = tree(true, &[Spec::child(1), Spec::child(2)]);
        assert!(events(&old, &next, true).is_empty());
    }

    #[test]
    fn enabled_toggle_and_value_changes_emit_one_property_event_each() {
        // checked 与 aria:pressed 都归约为 toggle_state，共用 ToggleState 属性。
        let old = tree(
            true,
            &[Spec::child(1).toggled(false), Spec::child(2).valued("a")],
        );
        let next = tree(
            true,
            &[
                Spec::child(1).disabled().toggled(true),
                Spec::child(2).valued("ab"),
            ],
        );
        assert_eq!(
            events(&old, &next, true),
            [
                Event::Property {
                    target: Some(1),
                    change: PropertyChange::Enabled {
                        old: true,
                        new: false,
                    },
                },
                Event::Property {
                    target: Some(1),
                    change: PropertyChange::ToggleState {
                        old: false,
                        new: true,
                    },
                },
                Event::Property {
                    target: Some(2),
                    change: PropertyChange::Value {
                        old: "a".to_owned(),
                        new: "ab".to_owned(),
                    },
                },
            ]
        );
    }

    #[test]
    fn disabling_and_reenabling_a_window_reports_each_changed_property_once() {
        let enabled_tree = tree(true, &[Spec::child(1), Spec::child(2)]);
        let disabled_tree = tree(
            false,
            &[Spec::child(1).disabled(), Spec::child(2).disabled()],
        );
        let enabled = |target: Option<i32>, old: bool, new: bool| Event::Property {
            target,
            change: PropertyChange::Enabled { old, new },
        };
        // 根是 HWND 的宿主窗口 provider；它自己通知原生 IsEnabled。
        assert_eq!(
            events(&enabled_tree, &disabled_tree, true),
            [enabled(Some(1), true, false), enabled(Some(2), true, false),]
        );
        assert_eq!(
            events(&disabled_tree, &enabled_tree, true),
            [enabled(Some(1), false, true), enabled(Some(2), false, true),]
        );
    }

    #[test]
    fn added_and_removed_children_emit_one_structure_event_each() {
        let old = tree(true, &[Spec::child(1), Spec::child(2)]);
        let next = tree(true, &[Spec::child(2), Spec::child(3)]);
        assert_eq!(
            events(&old, &next, true),
            [
                Event::ChildRemoved {
                    parent: None,
                    child: 1,
                },
                Event::ChildAdded { child: 3 },
            ]
        );
    }

    #[test]
    fn nested_children_report_the_parent_runtime_id() {
        let old = tree(
            true,
            &[Spec::child(10), Spec::under(11, 0), Spec::under(12, 0)],
        );
        let next = tree(
            true,
            &[Spec::child(10), Spec::under(12, 0), Spec::under(13, 0)],
        );
        assert_eq!(
            events(&old, &next, true),
            [
                Event::ChildRemoved {
                    parent: Some(10),
                    child: 11,
                },
                Event::ChildAdded { child: 13 },
            ]
        );
    }

    #[test]
    fn keyed_reordering_emits_one_event_on_the_parent() {
        let old = tree(true, &[Spec::child(1), Spec::child(2), Spec::child(3)]);
        let next = tree(true, &[Spec::child(3), Spec::child(1), Spec::child(2)]);
        assert_eq!(
            events(&old, &next, true),
            [Event::ChildrenReordered { parent: None }]
        );
    }

    #[test]
    fn a_reorder_coinciding_with_add_or_remove_is_still_reported() {
        let old = tree(true, &[Spec::child(1), Spec::child(2), Spec::child(3)]);
        let next = tree(true, &[Spec::child(3), Spec::child(1)]);
        assert_eq!(
            events(&old, &next, true),
            [
                Event::ChildRemoved {
                    parent: None,
                    child: 2,
                },
                Event::ChildrenReordered { parent: None },
            ]
        );

        let old = tree(true, &[Spec::child(1), Spec::child(2)]);
        let next = tree(true, &[Spec::child(3), Spec::child(2), Spec::child(1)]);
        assert_eq!(
            events(&old, &next, true),
            [
                Event::ChildAdded { child: 3 },
                Event::ChildrenReordered { parent: None },
            ]
        );
    }

    #[test]
    fn a_new_or_removed_subtree_reports_only_its_top_level_child() {
        let old = tree(true, &[Spec::child(1)]);
        let next = tree(true, &[Spec::child(1), Spec::child(3), Spec::under(4, 1)]);
        assert_eq!(events(&old, &next, true), [Event::ChildAdded { child: 3 }]);
        assert_eq!(
            events(&next, &old, true),
            [Event::ChildRemoved {
                parent: None,
                child: 3,
            }]
        );
    }

    #[test]
    fn events_are_ordered_structure_then_property_then_focus() {
        let old = tree(
            true,
            &[
                Spec::child(1).focused().toggled(false),
                Spec::child(2).toggled(false),
            ],
        );
        let next = tree(
            true,
            &[Spec::child(2).toggled(true), Spec::child(3).focused()],
        );
        assert_eq!(
            events(&old, &next, true),
            [
                Event::ChildRemoved {
                    parent: None,
                    child: 1,
                },
                Event::ChildAdded { child: 3 },
                Event::Property {
                    target: Some(2),
                    change: PropertyChange::ToggleState {
                        old: false,
                        new: true,
                    },
                },
                Event::Focus { node: 3 },
            ]
        );
    }

    #[test]
    fn a_new_node_reports_no_property_events_for_its_initial_state() {
        let old = tree(true, &[Spec::child(1).valued("a")]);
        let next = tree(
            true,
            &[Spec::child(1).valued("a"), Spec::child(2).valued("b")],
        );
        assert_eq!(events(&old, &next, true), [Event::ChildAdded { child: 2 }]);
    }

    #[test]
    fn a_removed_node_produces_no_property_or_focus_event_for_itself() {
        let old = tree(true, &[Spec::child(1).focused().toggled(false)]);
        let next = tree(true, &[Spec::child(2).focused()]);
        assert_eq!(
            events(&old, &next, true),
            [
                Event::ChildRemoved {
                    parent: None,
                    child: 1,
                },
                Event::ChildAdded { child: 2 },
                Event::Focus { node: 2 },
            ]
        );
    }
}
