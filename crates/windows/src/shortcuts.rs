//! 聚焦编辑器的快捷键策略。
//!
//! 这是宿主与文本编辑之间的唯一策略 seam：只有聚焦的可编辑输入、按下 Ctrl、
//! 且没有进行中的 IME 组合时才返回动作。Ctrl+Z/Y/C/X/V 由后续工单接到
//! runtime 的 undo/redo 与剪贴板；本工单先固定保留集合并让宿主消费按键，
//! 避免它们落入普通字符编辑。Ctrl+A 已经接到全选。

use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_A, VK_C, VK_V, VK_X, VK_Y, VK_Z,
};

/// 聚焦编辑器上的保留快捷键动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditorAction {
    /// 撤销聚焦输入的最近一次编辑。
    Undo,
    /// 重做聚焦输入最近撤销的编辑。
    Redo,
    /// 剪切选区。
    Cut,
    /// 复制选区。
    Copy,
    /// 用剪贴板内容替换选区或插入 caret。
    Paste,
    /// 全选聚焦输入。
    SelectAll,
}

/// 决定一个按键是否属于聚焦编辑器的保留快捷键。
///
/// 没有焦点、未按 Ctrl 或正在组合时返回 `None`，按键继续走原有路径：组合
/// 优先让 IME 消费，没有焦点时不吞掉任何编辑快捷键。
pub(crate) fn editor_shortcut(
    key: VIRTUAL_KEY,
    control: bool,
    focused: bool,
    composing: bool,
) -> Option<EditorAction> {
    if !control || !focused || composing {
        return None;
    }
    if key == VK_Z {
        Some(EditorAction::Undo)
    } else if key == VK_Y {
        Some(EditorAction::Redo)
    } else if key == VK_X {
        Some(EditorAction::Cut)
    } else if key == VK_C {
        Some(EditorAction::Copy)
    } else if key == VK_V {
        Some(EditorAction::Paste)
    } else if key == VK_A {
        Some(EditorAction::SelectAll)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_LEFT;

    use super::*;

    #[test]
    fn each_reserved_letter_maps_to_its_action() {
        for (key, action) in [
            (VK_Z, EditorAction::Undo),
            (VK_Y, EditorAction::Redo),
            (VK_X, EditorAction::Cut),
            (VK_C, EditorAction::Copy),
            (VK_V, EditorAction::Paste),
            (VK_A, EditorAction::SelectAll),
        ] {
            assert_eq!(editor_shortcut(key, true, true, false), Some(action));
        }
    }

    #[test]
    fn control_and_focus_are_both_required() {
        assert_eq!(editor_shortcut(VK_C, false, true, false), None);
        assert_eq!(editor_shortcut(VK_C, true, false, false), None);
        assert_eq!(editor_shortcut(VK_C, false, false, false), None);
    }

    #[test]
    fn a_composition_wins_over_every_reserved_key() {
        for key in [VK_Z, VK_Y, VK_X, VK_C, VK_V, VK_A] {
            assert_eq!(
                editor_shortcut(key, true, true, true),
                None,
                "the IME consumes the key during composition"
            );
        }
    }

    #[test]
    fn unrelated_keys_and_modified_navigation_are_not_shortcuts() {
        assert_eq!(editor_shortcut(VK_LEFT, true, true, false), None);
    }
}
