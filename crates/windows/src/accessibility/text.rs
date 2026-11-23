//! 文本输入的 UIA TextPattern。
//!
//! 文档是已提交值，preedit 不进入文档。UIA 偏移是 UTF-16 code unit；运行时与
//! 呈现快照使用字符偏移，因此这里在两种偏移之间转换并夹取到字符边界。
//!
//! 只实现文档、选区、文本、矩形与 Select。查找、移动、扩展、比较、多选与
//! 滚动等 range surface 明确返回 `UIA_E_NOTSUPPORTED`；Select 通过动作队列
//! 交给 UI 线程，重验后调用 `Runtime::set_selection_node`，不派发 `on:input`。

use std::sync::{Arc, Weak};

use windows::Win32::Foundation::E_OUTOFMEMORY;
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayDestroy, SafeArrayPutElement};
use windows::Win32::System::Variant::{VARIANT, VT_R8, VT_UNKNOWN};
use windows::Win32::UI::Accessibility::{
    IRawElementProviderSimple, ITextProvider, ITextProvider_Impl, ITextRangeProvider,
    ITextRangeProvider_Impl, SupportedTextSelection, SupportedTextSelection_Single,
    TextPatternRangeEndpoint, TextUnit, UIA_TEXTATTRIBUTE_ID, UiaPoint,
};
use windows::core::{BOOL, BSTR, Error, Interface, Ref, Result as WinResult, implement};
use zenui::{NodeId, SnapshotAction};

use super::{
    AccessibilityAction, AccessibilityTree, AccessibleNode, ControlType, ProviderKey,
    WindowAccessibility, ZenProvider, available_node, client_origin, element_not_available,
    not_supported, physical_rect, resolve_node,
};

pub(super) fn provider(window: Weak<WindowAccessibility>, node: NodeId) -> ITextProvider {
    TextPatternProvider { window, node }.into()
}

#[implement(ITextProvider)]
struct TextPatternProvider {
    window: Weak<WindowAccessibility>,
    node: NodeId,
}

impl TextPatternProvider {
    fn resolve(&self) -> WinResult<(Arc<WindowAccessibility>, Arc<AccessibilityTree>, usize)> {
        text_node(&self.window, self.node)
    }

    fn range(&self, start: usize, end: usize, kind: TextRangeKind) -> ITextRangeProvider {
        TextRangeProvider {
            window: self.window.clone(),
            node: self.node,
            start,
            end,
            kind,
        }
        .into()
    }

    fn document_range(&self) -> WinResult<ITextRangeProvider> {
        let (_, tree, index) = self.resolve()?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        let text = node.value().ok_or_else(not_supported)?;
        Ok(self.range(0, utf16_len(text), TextRangeKind::Document))
    }

    fn selection_range(&self) -> WinResult<ITextRangeProvider> {
        let (_, tree, index) = self.resolve()?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        let text = node.value().ok_or_else(not_supported)?;
        let (start, end) = selection_utf16(node, text);
        Ok(self.range(start, end, TextRangeKind::Selection))
    }

    fn visible_range(&self) -> WinResult<ITextRangeProvider> {
        let (_, tree, index) = self.resolve()?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        let text = node.value().ok_or_else(not_supported)?;
        if node.offscreen() {
            // UIA requires a degenerate range when no text is visible.
            return Ok(self.range(0, 0, TextRangeKind::Document));
        }
        Ok(self.range(0, utf16_len(text), TextRangeKind::Document))
    }
}

impl ITextProvider_Impl for TextPatternProvider_Impl {
    fn GetSelection(&self) -> WinResult<*mut SAFEARRAY> {
        com_array(&[self.selection_range()?])
    }

    fn GetVisibleRanges(&self) -> WinResult<*mut SAFEARRAY> {
        // 文本输入的可见范围按整份文档返回；完全离开视口时退化为空范围。
        // 本切片不计算部分可见文本的精确裁剪。
        com_array(&[self.visible_range()?])
    }

    fn RangeFromChild(
        &self,
        _childelement: Ref<IRawElementProviderSimple>,
    ) -> WinResult<ITextRangeProvider> {
        Err(not_supported())
    }

    fn RangeFromPoint(&self, _point: &UiaPoint) -> WinResult<ITextRangeProvider> {
        Err(not_supported())
    }

    fn DocumentRange(&self) -> WinResult<ITextRangeProvider> {
        self.document_range()
    }

    fn SupportedTextSelection(&self) -> WinResult<SupportedTextSelection> {
        Ok(SupportedTextSelection_Single)
    }
}

/// 范围来自文档还是 selection；两者即使覆盖同一段文本，矩形语义也不同。
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextRangeKind {
    Document,
    Selection,
}

#[implement(ITextRangeProvider)]
struct TextRangeProvider {
    window: Weak<WindowAccessibility>,
    node: NodeId,
    start: usize,
    end: usize,
    kind: TextRangeKind,
}

impl TextRangeProvider {
    fn resolve(&self) -> WinResult<(Arc<WindowAccessibility>, Arc<AccessibilityTree>, usize)> {
        resolve_node(&self.window, self.node)
    }

    fn enclosing(&self) -> WinResult<IRawElementProviderSimple> {
        self.resolve()?;
        Ok(ZenProvider {
            window: self.window.clone(),
            key: ProviderKey::Node(self.node),
        }
        .into())
    }
}

impl ITextRangeProvider_Impl for TextRangeProvider_Impl {
    fn Clone(&self) -> WinResult<ITextRangeProvider> {
        Ok(TextRangeProvider {
            window: self.window.clone(),
            node: self.node,
            start: self.start,
            end: self.end,
            kind: self.kind,
        }
        .into())
    }

    fn Compare(&self, _range: Ref<ITextRangeProvider>) -> WinResult<BOOL> {
        Err(not_supported())
    }

    fn CompareEndpoints(
        &self,
        _endpoint: TextPatternRangeEndpoint,
        _targetrange: Ref<ITextRangeProvider>,
        _targetendpoint: TextPatternRangeEndpoint,
    ) -> WinResult<i32> {
        Err(not_supported())
    }

    fn ExpandToEnclosingUnit(&self, _unit: TextUnit) -> WinResult<()> {
        Err(not_supported())
    }

    fn FindAttribute(
        &self,
        _attributeid: UIA_TEXTATTRIBUTE_ID,
        _val: &VARIANT,
        _backward: BOOL,
    ) -> WinResult<ITextRangeProvider> {
        Err(not_supported())
    }

    fn FindText(
        &self,
        _text: &BSTR,
        _backward: BOOL,
        _ignorecase: BOOL,
    ) -> WinResult<ITextRangeProvider> {
        Err(not_supported())
    }

    fn GetAttributeValue(&self, _attributeid: UIA_TEXTATTRIBUTE_ID) -> WinResult<VARIANT> {
        Err(not_supported())
    }

    fn GetBoundingRectangles(&self) -> WinResult<*mut SAFEARRAY> {
        let (window, tree, index) = self.resolve()?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        let text = node.value().ok_or_else(not_supported)?;
        if node.offscreen() {
            return f64_array(&[]);
        }
        let (start, end) = clamp_utf16(text, self.start, self.end);
        let origin = client_origin(window.hwnd());
        let scale = tree.scale();
        let geometry = node.text_geometry().ok_or_else(not_supported)?;
        let caret = char_to_utf16(text, geometry.caret());
        let selection = geometry
            .selection()
            .map(|(selection_start, selection_end)| {
                (
                    char_to_utf16(text, selection_start),
                    char_to_utf16(text, selection_end),
                )
            })
            .unwrap_or((caret, caret));
        let rects: Vec<_> = if self.kind == TextRangeKind::Document {
            // 文档范围始终描述整段文本区域，即使当前 selection 覆盖全文。
            vec![tree.logical_rect(node)]
        } else if start == end {
            // 折叠范围只有当前位置的 caret 有几何；其它折叠位置没有矩形。
            if start == caret {
                geometry.caret_rect().into_iter().collect()
            } else {
                Vec::new()
            }
        } else if (start, end) == selection {
            geometry.selection_rects().to_vec()
        } else {
            // 客户端持有的过期 selection：退回元素文本区域。
            vec![tree.logical_rect(node)]
        };
        if rects.is_empty() {
            return f64_array(&[]);
        }
        let mut values = Vec::with_capacity(rects.len() * 4);
        for rect in rects {
            let bounds = physical_rect(rect, origin, scale);
            values.extend([bounds.left, bounds.top, bounds.width, bounds.height]);
        }
        f64_array(&values)
    }

    fn GetEnclosingElement(&self) -> WinResult<IRawElementProviderSimple> {
        self.enclosing()
    }

    fn GetText(&self, maxlength: i32) -> WinResult<BSTR> {
        let (_, tree, index) = self.resolve()?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        let text = node.value().ok_or_else(not_supported)?;
        let (start, end) = clamp_utf16(text, self.start, self.end);
        let range = slice_utf16(text, start, end);
        if maxlength < 0 {
            return Ok(BSTR::from(range));
        }
        Ok(BSTR::from(truncate_utf16(range, maxlength as usize)))
    }

    fn Move(&self, _unit: TextUnit, _count: i32) -> WinResult<i32> {
        Err(not_supported())
    }

    fn MoveEndpointByUnit(
        &self,
        _endpoint: TextPatternRangeEndpoint,
        _unit: TextUnit,
        _count: i32,
    ) -> WinResult<i32> {
        Err(not_supported())
    }

    fn MoveEndpointByRange(
        &self,
        _endpoint: TextPatternRangeEndpoint,
        _targetrange: Ref<ITextRangeProvider>,
        _targetendpoint: TextPatternRangeEndpoint,
    ) -> WinResult<()> {
        Err(not_supported())
    }

    fn Select(&self) -> WinResult<()> {
        let (window, tree, index) = available_node(&self.window, self.node)?;
        let node = tree.node_at(index).ok_or_else(element_not_available)?;
        // 只读状态与 ValuePattern 一致按元素不可用报告；pattern 本身仍可读取。
        if node.control_type() != ControlType::Edit
            || !node.has_action(SnapshotAction::Input)
            || node.composing()
        {
            return Err(element_not_available());
        }
        let text = node.value().ok_or_else(not_supported)?;
        let (start, end) = clamp_utf16(text, self.start, self.end);
        let start = utf16_to_char(text, start);
        let end = utf16_to_char(text, end);
        if !window.enqueue(AccessibilityAction::SetSelection(self.node, start, end)) {
            return Err(element_not_available());
        }
        Ok(())
    }

    fn AddToSelection(&self) -> WinResult<()> {
        Err(not_supported())
    }

    fn RemoveFromSelection(&self) -> WinResult<()> {
        Err(not_supported())
    }

    fn ScrollIntoView(&self, _aligntotop: BOOL) -> WinResult<()> {
        Err(not_supported())
    }

    fn GetChildren(&self) -> WinResult<*mut SAFEARRAY> {
        Err(not_supported())
    }
}

fn text_node(
    window: &Weak<WindowAccessibility>,
    node: NodeId,
) -> WinResult<(Arc<WindowAccessibility>, Arc<AccessibilityTree>, usize)> {
    let (window, tree, index) = resolve_node(window, node)?;
    let published = tree.node_at(index).ok_or_else(element_not_available)?;
    if published.control_type() != ControlType::Edit || published.value().is_none() {
        return Err(not_supported());
    }
    Ok((window, tree, index))
}

/// 当前选区换算到 UTF-16；没有 selection 时折叠在 caret。
fn selection_utf16(node: &AccessibleNode, text: &str) -> (usize, usize) {
    let geometry = node.text_geometry();
    let caret = geometry.map_or(0, |geometry| geometry.caret());
    geometry
        .and_then(|geometry| geometry.selection())
        .map(|(start, end)| (char_to_utf16(text, start), char_to_utf16(text, end)))
        .unwrap_or_else(|| {
            let caret = char_to_utf16(text, caret);
            (caret, caret)
        })
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn char_to_utf16(text: &str, character_offset: usize) -> usize {
    text.chars()
        .take(character_offset)
        .map(char::len_utf16)
        .sum()
}

/// 把 UTF-16 偏移换算为字符偏移；落在代理对内部时夹到该字符起点。
fn utf16_to_char(text: &str, unit_offset: usize) -> usize {
    let mut used = 0;
    for (index, character) in text.chars().enumerate() {
        let next = used + character.len_utf16();
        if unit_offset < next {
            return index;
        }
        used = next;
    }
    text.chars().count()
}

fn clamp_utf16(text: &str, start: usize, end: usize) -> (usize, usize) {
    let length = utf16_len(text);
    let start = start.min(length);
    let end = end.min(length);
    if start <= end {
        (start, end)
    } else {
        (end, start)
    }
}

fn utf16_byte(text: &str, unit_offset: usize) -> usize {
    let mut units = 0;
    for (byte, character) in text.char_indices() {
        let next = units + character.len_utf16();
        if unit_offset < next {
            return byte;
        }
        units = next;
    }
    text.len()
}

fn slice_utf16(text: &str, start: usize, end: usize) -> &str {
    let (start, end) = clamp_utf16(text, start, end);
    &text[utf16_byte(text, start)..utf16_byte(text, end)]
}

/// 截取前 `max_units` 个 UTF-16 code unit；不拆开代理对。
fn truncate_utf16(text: &str, max_units: usize) -> &str {
    let mut units = 0;
    for (byte, character) in text.char_indices() {
        let next = units + character.len_utf16();
        if next > max_units {
            return &text[..byte];
        }
        units = next;
    }
    text
}

fn com_array(ranges: &[ITextRangeProvider]) -> WinResult<*mut SAFEARRAY> {
    // 安全：数组长度来自本地 slice；失败时销毁已创建的数组。
    unsafe {
        let array = SafeArrayCreateVector(VT_UNKNOWN, 0, ranges.len() as u32);
        if array.is_null() {
            return Err(Error::from_hresult(E_OUTOFMEMORY));
        }
        for (position, range) in ranges.iter().enumerate() {
            let index = position as i32;
            // VT_UNKNOWN 元素存放接口指针本身，不再多一层间接；UIA 客户端会按
            // ITextRangeProvider 解释它。OleAut 负责 AddRef，数组持有自己的引用。
            if let Err(error) = SafeArrayPutElement(array, &index, range.as_raw() as *const _) {
                let _ = SafeArrayDestroy(array);
                return Err(error);
            }
        }
        Ok(array)
    }
}

fn f64_array(values: &[f64]) -> WinResult<*mut SAFEARRAY> {
    // 安全：数组长度来自本地 slice；失败时销毁已创建的数组。
    unsafe {
        let array = SafeArrayCreateVector(VT_R8, 0, values.len() as u32);
        if array.is_null() {
            return Err(Error::from_hresult(E_OUTOFMEMORY));
        }
        for (position, value) in values.iter().enumerate() {
            let index = position as i32;
            if let Err(error) = SafeArrayPutElement(array, &index, (value as *const f64).cast()) {
                let _ = SafeArrayDestroy(array);
                return Err(error);
            }
        }
        Ok(array)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_offsets_round_trip_ascii_cjk_non_bmp_and_combining() {
        // 字符：a, 中, 🙂, e, ◌́, z；UTF-16：1, 1, 2, 1, 1, 1。
        let text = "a中🙂e\u{301}z";
        let offsets = [0, 1, 2, 4, 5, 6, 7];
        assert_eq!(utf16_len(text), 7);
        for (character, units) in offsets.into_iter().enumerate() {
            assert_eq!(char_to_utf16(text, character), units);
            assert_eq!(utf16_to_char(text, units), character);
        }
        assert_eq!(
            utf16_to_char(text, 3),
            2,
            "a UTF-16 offset inside a surrogate pair clamps to the character start"
        );
        assert_eq!(
            char_to_utf16(text, 99),
            7,
            "character offsets beyond the value clamp to its end"
        );
    }

    #[test]
    fn slices_stay_on_character_boundaries_and_clamp() {
        let text = "a中🙂e\u{301}z";
        assert_eq!(slice_utf16(text, 1, 4), "中🙂");
        assert_eq!(slice_utf16(text, 2, 4), "🙂");
        assert_eq!(
            slice_utf16(text, 2, 3),
            "",
            "an end inside a surrogate pair clamps back to the character start"
        );
        assert_eq!(
            slice_utf16(text, 3, 2),
            "",
            "reversed interior offsets normalize to the same clamped range"
        );
        assert_eq!(slice_utf16(text, 0, 99), text, "end clamps to the value");
        assert_eq!(slice_utf16(text, 6, 7), "z");
        assert_eq!(
            slice_utf16("🙂x", 1, 2),
            "🙂",
            "a start inside a surrogate pair rounds back to the character start"
        );
        assert_eq!(slice_utf16("🙂x", 1, 3), "🙂x");
        assert_eq!(slice_utf16("🙂x", 2, 3), "x");
    }

    #[test]
    fn truncation_counts_utf16_units_without_splitting_surrogates() {
        assert_eq!(truncate_utf16("a中🙂", 0), "");
        assert_eq!(truncate_utf16("a中🙂", 2), "a中");
        assert_eq!(
            truncate_utf16("🙂x", 1),
            "",
            "a single unit cannot hold half a surrogate pair"
        );
        assert_eq!(truncate_utf16("🙂x", 2), "🙂");
        assert_eq!(truncate_utf16("🙂x", 3), "🙂x");
    }
}
