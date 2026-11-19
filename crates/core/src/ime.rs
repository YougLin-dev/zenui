//! 平台无关的 IME 组合状态机。
//!
//! 平台适配层把原生消息归一化为 [`ImeEvent`] 后交给 [`CompositionStateMachine`]。
//! 文本缓冲区是唯一事实源，preedit 以带标记的区间直接存在于缓冲区中，由布局
//! 引擎按普通文本排版；提交时标记区间被提交文本原地替换，取消时恢复组合开始前
//! 被替换的已提交状态。
//!
//! 边界操作（移动选区、插入文本、退格）先结算组合：当前 preedit 视为已确认
//! 文本原地落定，再执行编辑。光标、选区与布局因此始终从缓冲区全量推导，不需要
//! 与组合态做增量对齐。
//!
//! 乱序事件按平台噪音处理：会话未开启时到达的 preedit 被忽略，丢弃它不会丢失
//! 已确认的文本；commit 一旦到达总是交付一次，避免吞掉用户已经确认的输入。

use std::ops::Range;

/// preedit 内部的组合光标，按 UTF-8 字节区间计。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreeditCursor {
    start: usize,
    end: usize,
}

impl PreeditCursor {
    /// `start` 与 `end` 应落在 preedit 文本的字符边界上。
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn start(self) -> usize {
        self.start
    }

    pub fn end(self) -> usize {
        self.end
    }
}

/// 平台适配层归一化后的 IME 事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImeEvent {
    /// 平台 IME 会话开始。
    Enabled,
    /// 组合串更新；`cursor` 是组合串内部的选区。
    Preedit {
        text: String,
        cursor: Option<PreeditCursor>,
    },
    /// 组合结果提交。
    Commit(String),
    /// 平台 IME 会话结束；未提交的组合取消。
    Disabled,
}

/// 组合开始前被替换的选区内容；取消组合时恢复。
#[derive(Clone, Debug, PartialEq, Eq)]
struct ReplacedSelection {
    selection: Range<usize>,
    text: String,
}

/// 一个编辑器的 IME 组合状态机。
///
/// 缓冲区文本包含组合中的 preedit；`selection` 始终落在字符边界上，组合期间
/// 折叠在 preedit 末尾，preedit 内部的光标单独记录。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositionStateMachine {
    text: String,
    active: bool,
    marked: Option<Range<usize>>,
    preedit_cursor: Option<PreeditCursor>,
    selection: Range<usize>,
    replaced: Option<ReplacedSelection>,
}

impl Default for CompositionStateMachine {
    fn default() -> Self {
        Self {
            text: String::new(),
            active: false,
            marked: None,
            preedit_cursor: None,
            selection: 0..0,
            replaced: None,
        }
    }
}

impl CompositionStateMachine {
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前缓冲区文本，包括组合中的 preedit。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 当前选区；折叠区间表示光标。
    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    /// 平台 IME 会话是否开启。
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// 当前 preedit 在缓冲区中的字节区间。
    pub fn preedit_range(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// preedit 内部的组合光标。
    pub fn preedit_cursor(&self) -> Option<PreeditCursor> {
        self.preedit_cursor
    }

    /// 应用一个平台事件，返回是否需要重排与重绘。
    pub fn handle_event(&mut self, event: ImeEvent) -> bool {
        match event {
            ImeEvent::Enabled => {
                self.active = true;
                false
            }
            ImeEvent::Preedit { text, cursor } => {
                if !self.active {
                    return false;
                }
                self.apply_preedit(&text, cursor);
                true
            }
            ImeEvent::Commit(text) => {
                self.apply_commit(&text);
                true
            }
            ImeEvent::Disabled => {
                self.active = false;
                self.cancel_composition()
            }
        }
    }

    /// 把当前 preedit 原地结算为已提交文本；没有组合时无操作。
    ///
    /// 结算后组合标记消失，文本内容不变；后续的 `Disabled` 不再回收这段文本。
    pub fn settle_composition(&mut self) {
        if let Some(marked) = self.marked.take() {
            self.selection = marked.end..marked.end;
            self.preedit_cursor = None;
            self.replaced = None;
        }
    }

    /// 先结算组合，再设置选区；越界或非字符边界的索引夹取到字符边界。
    pub fn set_selection(&mut self, range: Range<usize>) {
        self.settle_composition();
        let start = clamp_to_char_boundary(&self.text, range.start.min(range.end));
        let end = clamp_to_char_boundary(&self.text, range.start.max(range.end));
        self.selection = start..end;
    }

    /// 先结算组合，再用文本替换当前选区。
    pub fn insert_text(&mut self, text: &str) {
        self.settle_composition();
        let start = self.selection.start;
        self.text.replace_range(self.selection.clone(), text);
        let end = start + text.len();
        self.selection = end..end;
    }

    /// 先结算组合，再删除当前选区或光标前的一个字符。
    pub fn delete_backward(&mut self) {
        self.settle_composition();
        let selection = self.selection.clone();
        if selection.start != selection.end {
            self.text.replace_range(selection.clone(), "");
            self.selection = selection.start..selection.start;
            return;
        }
        let caret = selection.start;
        let previous = self.text[..caret]
            .char_indices()
            .next_back()
            .map(|(index, _)| index);
        if let Some(previous) = previous {
            self.text.replace_range(previous..caret, "");
            self.selection = previous..previous;
        }
    }

    /// 用新的组合串替换 marked 区间；首次组合从当前选区开始。
    fn apply_preedit(&mut self, preedit: &str, cursor: Option<PreeditCursor>) {
        let start = if let Some(marked) = self.marked.take() {
            self.text.replace_range(marked.clone(), preedit);
            marked.start
        } else {
            let selection = self.selection.clone();
            let replaced = self.text[selection.clone()].to_string();
            self.replaced = Some(ReplacedSelection {
                selection,
                text: replaced,
            });
            self.text.replace_range(self.selection.clone(), preedit);
            self.selection.start
        };
        let end = start + preedit.len();
        self.marked = Some(start..end);
        self.selection = end..end;
        self.preedit_cursor = cursor;
    }

    /// 用提交文本替换 marked 区间；没有组合时在选区处插入。
    fn apply_commit(&mut self, committed: &str) {
        let range = self.marked.take().unwrap_or_else(|| self.selection.clone());
        let start = range.start;
        self.text.replace_range(range, committed);
        let end = start + committed.len();
        self.selection = end..end;
        self.preedit_cursor = None;
        self.replaced = None;
    }

    /// 丢弃 preedit 并恢复组合开始前被替换的选区内容。
    fn cancel_composition(&mut self) -> bool {
        let Some(marked) = self.marked.take() else {
            self.replaced = None;
            return false;
        };
        let caret = marked.start;
        self.text.replace_range(marked, "");
        self.preedit_cursor = None;
        let restored = self.replaced.take();
        if let Some(replaced) = restored {
            self.text.insert_str(caret, &replaced.text);
            let end = caret + replaced.text.len();
            self.selection = caret..end;
        } else {
            self.selection = caret..caret;
        }
        true
    }
}

fn clamp_to_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine_with_text(text: &str) -> CompositionStateMachine {
        let mut machine = CompositionStateMachine::new();
        machine.insert_text(text);
        machine
    }

    #[test]
    fn commit_inserts_text_at_the_caret() {
        let mut machine = machine_with_text("hello");
        assert!(!machine.handle_event(ImeEvent::Enabled));
        assert!(machine.handle_event(ImeEvent::Commit(String::from("，"))));
        assert_eq!(machine.text(), "hello，");
        assert_eq!(machine.selection(), 8..8);
    }

    #[test]
    fn preedit_is_inline_and_updates_replace_the_marked_range() {
        let mut machine = machine_with_text("ab");
        machine.handle_event(ImeEvent::Enabled);
        machine.handle_event(ImeEvent::Preedit {
            text: String::from("n"),
            cursor: None,
        });
        assert_eq!(machine.text(), "abn");
        assert_eq!(machine.preedit_range(), Some(2..3));
        assert_eq!(machine.selection(), 3..3);

        let cursor = PreeditCursor::new(0, 2);
        machine.handle_event(ImeEvent::Preedit {
            text: String::from("ni"),
            cursor: Some(cursor),
        });
        assert_eq!(machine.text(), "abni");
        assert_eq!(machine.preedit_range(), Some(2..4));
        assert_eq!(machine.preedit_cursor(), Some(cursor));
        assert_eq!(machine.selection(), 4..4);
    }

    #[test]
    fn commit_replaces_the_marked_range_exactly_once() {
        let mut machine = machine_with_text("ab");
        machine.handle_event(ImeEvent::Enabled);
        machine.handle_event(ImeEvent::Preedit {
            text: String::from("ni"),
            cursor: None,
        });
        assert!(machine.handle_event(ImeEvent::Commit(String::from("你"))));
        assert_eq!(machine.text(), "ab你");
        assert_eq!(machine.text().matches('你').count(), 1);
        assert_eq!(machine.preedit_range(), None);
        assert_eq!(machine.selection(), 5..5);
    }

    #[test]
    fn cancel_restores_the_replaced_selection() {
        let mut machine = machine_with_text("abc");
        machine.set_selection(1..2);
        machine.handle_event(ImeEvent::Enabled);
        machine.handle_event(ImeEvent::Preedit {
            text: String::from("ni"),
            cursor: None,
        });
        assert_eq!(machine.text(), "anic");
        assert!(machine.handle_event(ImeEvent::Disabled));
        assert_eq!(machine.text(), "abc");
        assert_eq!(machine.selection(), 1..2);
        assert!(!machine.is_active());
    }

    #[test]
    fn cancel_discards_preedit_inserted_at_a_caret() {
        let mut machine = machine_with_text("ab");
        machine.handle_event(ImeEvent::Enabled);
        machine.handle_event(ImeEvent::Preedit {
            text: String::from("ni"),
            cursor: None,
        });
        assert!(machine.handle_event(ImeEvent::Disabled));
        assert_eq!(machine.text(), "ab");
        assert_eq!(machine.selection(), 2..2);
    }

    #[test]
    fn preedit_outside_a_session_is_ignored() {
        let mut machine = machine_with_text("ab");
        assert!(!machine.handle_event(ImeEvent::Preedit {
            text: String::from("ni"),
            cursor: None,
        }));
        assert_eq!(machine.text(), "ab");
        assert_eq!(machine.preedit_range(), None);
    }

    #[test]
    fn commit_outside_a_session_is_still_delivered() {
        let mut machine = machine_with_text("ab");
        assert!(machine.handle_event(ImeEvent::Commit(String::from("你"))));
        assert_eq!(machine.text(), "ab你");
    }

    #[test]
    fn boundary_selection_settles_the_active_preedit() {
        let mut machine = machine_with_text("ab");
        machine.handle_event(ImeEvent::Enabled);
        machine.handle_event(ImeEvent::Preedit {
            text: String::from("ni"),
            cursor: None,
        });
        machine.set_selection(0..0);
        assert_eq!(machine.text(), "abni");
        assert_eq!(machine.preedit_range(), None);
        assert_eq!(machine.selection(), 0..0);
        assert!(!machine.handle_event(ImeEvent::Disabled));
        assert_eq!(machine.text(), "abni");
    }

    #[test]
    fn backspace_settles_the_preedit_then_deletes() {
        let mut machine = machine_with_text("你好");
        machine.handle_event(ImeEvent::Enabled);
        machine.handle_event(ImeEvent::Preedit {
            text: String::from("ni"),
            cursor: None,
        });
        machine.delete_backward();
        assert_eq!(machine.text(), "你好n");
        assert_eq!(machine.preedit_range(), None);
        machine.delete_backward();
        assert_eq!(machine.text(), "你好");
    }

    #[test]
    fn backspace_deletes_a_whole_multibyte_character() {
        let mut machine = machine_with_text("a你");
        machine.delete_backward();
        assert_eq!(machine.text(), "a");
        assert_eq!(machine.selection(), 1..1);
    }

    #[test]
    fn set_selection_clamps_to_char_boundaries() {
        let mut machine = machine_with_text("a你");
        let (start, end) = (2, 1);
        machine.set_selection(start..end);
        assert_eq!(machine.selection(), 1..1);
    }

    #[test]
    fn insert_text_replaces_the_selection() {
        let mut machine = machine_with_text("abc");
        machine.set_selection(1..2);
        machine.insert_text("X");
        assert_eq!(machine.text(), "aXc");
        assert_eq!(machine.selection(), 2..2);
    }

    #[test]
    fn disabled_is_idempotent() {
        let mut machine = machine_with_text("ab");
        machine.handle_event(ImeEvent::Enabled);
        assert!(!machine.handle_event(ImeEvent::Disabled));
        assert!(!machine.handle_event(ImeEvent::Disabled));
        assert!(!machine.is_active());
    }
}
