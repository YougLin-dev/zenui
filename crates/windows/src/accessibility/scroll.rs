//! 窗口根的 UIA `IScrollProvider`。
//!
//! 滚动只改变宿主呈现偏移，与滚轮、分页键和滚动条共用同一个整数偏移；这里
//! 不新增 scrollbar Node ID，也不实现水平、嵌套或 `ScrollItemPattern`。
//! 读取侧使用发布树的场景几何；写侧只把 [`ScrollCommand`] 入队，由 UI 线程
//! 按当前场景重新夹取后调用宿主的显式滚动路径。

use std::sync::{Arc, Weak};

use windows::Win32::UI::Accessibility::{
    IScrollProvider, IScrollProvider_Impl, ScrollAmount, ScrollAmount_LargeDecrement,
    ScrollAmount_LargeIncrement, ScrollAmount_NoAmount, ScrollAmount_SmallDecrement,
    ScrollAmount_SmallIncrement, UIA_ScrollPatternNoScroll,
};
use windows::core::{BOOL, Result as WinResult, implement};

use super::{
    AccessibilityAction, AccessibilityTree, ScrollCommand, WindowAccessibility,
    element_not_available, invalid_operation, window_enabled,
};

pub(super) fn provider(window: Weak<WindowAccessibility>) -> IScrollProvider {
    ScrollPatternProvider { window }.into()
}

#[implement(IScrollProvider)]
struct ScrollPatternProvider {
    window: Weak<WindowAccessibility>,
}

impl ScrollPatternProvider {
    fn resolve(&self) -> WinResult<(Arc<WindowAccessibility>, Arc<AccessibilityTree>)> {
        let window = self.window.upgrade().ok_or_else(element_not_available)?;
        let tree = window.tree().ok_or_else(element_not_available)?;
        Ok((window, tree))
    }

    /// 写动作的共同前置条件：窗口仍启用、内容确实可滚动。
    fn actionable(&self, window: &WindowAccessibility, tree: &AccessibilityTree) -> WinResult<()> {
        if !window_enabled(window.hwnd()) || !tree.root().enabled() {
            return Err(element_not_available());
        }
        if !tree.scroll_geometry().vertically_scrollable() {
            return Err(invalid_operation());
        }
        Ok(())
    }

    fn enqueue(&self, window: &WindowAccessibility, command: ScrollCommand) -> WinResult<()> {
        if !window.enqueue(AccessibilityAction::Scroll(command)) {
            return Err(element_not_available());
        }
        Ok(())
    }
}

/// 只接受垂直滚动；UIA 的 `NoAmount` 是合法空操作，其它未知值无效。
#[allow(non_upper_case_globals)]
fn vertical_command(amount: ScrollAmount) -> WinResult<Option<ScrollCommand>> {
    match amount {
        ScrollAmount_NoAmount => Ok(None),
        ScrollAmount_LargeIncrement => Ok(Some(ScrollCommand::PageDown)),
        ScrollAmount_LargeDecrement => Ok(Some(ScrollCommand::PageUp)),
        ScrollAmount_SmallIncrement => Ok(Some(ScrollCommand::LineDown)),
        ScrollAmount_SmallDecrement => Ok(Some(ScrollCommand::LineUp)),
        _ => Err(invalid_operation()),
    }
}

impl IScrollProvider_Impl for ScrollPatternProvider_Impl {
    fn Scroll(&self, horizontal: ScrollAmount, vertical: ScrollAmount) -> WinResult<()> {
        let (window, tree) = self.resolve()?;
        if horizontal != ScrollAmount_NoAmount {
            return Err(invalid_operation());
        }
        let command = vertical_command(vertical)?;
        self.actionable(&window, &tree)?;
        match command {
            Some(command) => self.enqueue(&window, command),
            None => Ok(()),
        }
    }

    fn SetScrollPercent(&self, horizontal: f64, vertical: f64) -> WinResult<()> {
        let (window, tree) = self.resolve()?;
        // 本切片没有水平滚动；只有 UIA 的 NoScroll 哨兵合法。
        if horizontal != UIA_ScrollPatternNoScroll {
            return Err(invalid_operation());
        }
        if vertical != UIA_ScrollPatternNoScroll
            && (!vertical.is_finite() || !(0.0..=100.0).contains(&vertical))
        {
            return Err(invalid_operation());
        }
        self.actionable(&window, &tree)?;
        if vertical == UIA_ScrollPatternNoScroll {
            return Ok(());
        }
        self.enqueue(&window, ScrollCommand::Percent(vertical))
    }

    fn HorizontalScrollPercent(&self) -> WinResult<f64> {
        self.resolve()?;
        Ok(UIA_ScrollPatternNoScroll)
    }

    fn VerticalScrollPercent(&self) -> WinResult<f64> {
        let (_, tree) = self.resolve()?;
        Ok(tree
            .scroll_geometry()
            .vertical_scroll_percent()
            .unwrap_or(UIA_ScrollPatternNoScroll))
    }

    fn HorizontalViewSize(&self) -> WinResult<f64> {
        self.resolve()?;
        Ok(UIA_ScrollPatternNoScroll)
    }

    fn VerticalViewSize(&self) -> WinResult<f64> {
        let (_, tree) = self.resolve()?;
        Ok(tree.scroll_geometry().vertical_view_size())
    }

    fn HorizontallyScrollable(&self) -> WinResult<BOOL> {
        self.resolve()?;
        Ok(false.into())
    }

    fn VerticallyScrollable(&self) -> WinResult<BOOL> {
        let (_, tree) = self.resolve()?;
        Ok(tree.scroll_geometry().vertically_scrollable().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uia_amounts_map_to_vertical_page_and_line_commands() {
        assert_eq!(
            vertical_command(ScrollAmount_LargeIncrement).expect("large increment"),
            Some(ScrollCommand::PageDown)
        );
        assert_eq!(
            vertical_command(ScrollAmount_LargeDecrement).expect("large decrement"),
            Some(ScrollCommand::PageUp)
        );
        assert_eq!(
            vertical_command(ScrollAmount_SmallIncrement).expect("small increment"),
            Some(ScrollCommand::LineDown)
        );
        assert_eq!(
            vertical_command(ScrollAmount_SmallDecrement).expect("small decrement"),
            Some(ScrollCommand::LineUp)
        );
        assert_eq!(
            vertical_command(ScrollAmount_NoAmount).expect("no amount"),
            None
        );
    }

    #[test]
    fn unknown_scroll_amounts_are_invalid() {
        let error = vertical_command(ScrollAmount(99)).expect_err("unknown amount");
        assert_eq!(error.code(), invalid_operation().code());
    }
}
