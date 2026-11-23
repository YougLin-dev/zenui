//! 平台无关的文本编辑归约。
//!
//! 输入是一次编辑前的字符偏移、本地 selection 与当前文本；输出是替换后的文本和
//! 新的 caret。偏移始终以字符计并夹取到文本长度，因此不会切开 UTF-8 编码。

/// 一次空编辑之外的文本修改。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Edit<'a> {
    Insert(&'a str),
    /// 整体替换已提交值；caret 落在新值末尾。
    Replace(&'a str),
    Backspace,
    DeleteForward,
}

/// caret / selection 的移动；不改变文本。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Movement {
    Caret(usize),
    Selection(usize, usize),
    SelectAll,
}

/// 一次会改变文本的编辑结果。
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct EditOutcome {
    pub(crate) value: String,
    pub(crate) caret: usize,
}

/// 把两个字符偏移归一化并夹取到 `length`。
pub(crate) fn normalized_range(first: usize, second: usize, length: usize) -> (usize, usize) {
    let low = first.min(second).min(length);
    let high = first.max(second).min(length);
    (low, high)
}

/// 用 `insert` 替换 `[start, end)`；返回新文本与新 caret。
pub(crate) fn replace_range(
    value: &str,
    start: usize,
    end: usize,
    insert: &str,
) -> (String, usize) {
    let length = value.chars().count();
    let (start, end) = normalized_range(start, end, length);
    let mut result: String = value.chars().take(start).collect();
    result.push_str(insert);
    result.extend(value.chars().skip(end));
    (result, start + insert.chars().count())
}

/// 把一次移动归一化为 `(start, end)`；caret 落在 active end。
pub(crate) fn movement_range(length: usize, movement: Movement) -> (usize, usize) {
    match movement {
        Movement::Caret(offset) => {
            let offset = offset.min(length);
            (offset, offset)
        }
        Movement::Selection(first, second) => normalized_range(first, second, length),
        Movement::SelectAll => (0, length),
    }
}

/// 归一化并夹取当前选择；没有 selection 时候选区间折叠在 caret。
pub(crate) fn selected_range(
    caret: usize,
    selection: Option<(usize, usize)>,
    length: usize,
) -> (usize, usize) {
    match selection {
        Some((first, second)) => normalized_range(first, second, length),
        None => movement_range(length, Movement::Caret(caret)),
    }
}

/// 应用一次编辑；没有可删除/插入的内容时返回 `None`，不产生 `on:input`。
pub(crate) fn apply(
    value: &str,
    caret: usize,
    selection: Option<(usize, usize)>,
    edit: Edit<'_>,
) -> Option<EditOutcome> {
    let length = value.chars().count();
    let (start, end) = selected_range(caret, selection, length);
    match edit {
        Edit::Insert(insert) => {
            if start == end && insert.is_empty() {
                return None;
            }
            let (value, caret) = replace_range(value, start, end, insert);
            Some(EditOutcome { value, caret })
        }
        Edit::Replace(replacement) => {
            if value == replacement {
                return None;
            }
            let caret = replacement.chars().count();
            Some(EditOutcome {
                value: replacement.to_owned(),
                caret,
            })
        }
        Edit::Backspace => {
            let (start, end) = if start != end {
                (start, end)
            } else {
                (start.checked_sub(1)?, start)
            };
            let (value, caret) = replace_range(value, start, end, "");
            Some(EditOutcome { value, caret })
        }
        Edit::DeleteForward => {
            let (start, end) = if start != end {
                (start, end)
            } else if start < length {
                (start, start + 1)
            } else {
                return None;
            };
            let (value, caret) = replace_range(value, start, end, "");
            Some(EditOutcome { value, caret })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_replaces_the_selection_and_clamps_offsets() {
        assert_eq!(
            apply("abcd", 2, Some((1, 3)), Edit::Insert("XY")),
            Some(EditOutcome {
                value: String::from("aXYd"),
                caret: 3
            })
        );
        assert_eq!(
            apply("abcd", 99, None, Edit::Insert("!")),
            Some(EditOutcome {
                value: String::from("abcd!"),
                caret: 5
            })
        );
        assert_eq!(apply("abcd", 2, None, Edit::Insert("")), None);
    }

    #[test]
    fn replace_ignores_the_selection_and_moves_the_caret_to_the_end() {
        assert_eq!(
            apply("abcd", 1, Some((2, 3)), Edit::Replace("新值")),
            Some(EditOutcome {
                value: String::from("新值"),
                caret: 2
            })
        );
        assert_eq!(apply("same", 4, None, Edit::Replace("same")), None);
    }

    #[test]
    fn backspace_and_delete_forward_stop_at_the_edges() {
        assert_eq!(
            apply("abcd", 2, None, Edit::Backspace),
            Some(EditOutcome {
                value: String::from("acd"),
                caret: 1
            })
        );
        assert_eq!(apply("abcd", 0, None, Edit::Backspace), None);
        assert_eq!(
            apply("abcd", 1, None, Edit::DeleteForward),
            Some(EditOutcome {
                value: String::from("acd"),
                caret: 1
            })
        );
        assert_eq!(apply("abcd", 4, None, Edit::DeleteForward), None);
        assert_eq!(
            apply("abcd", 1, Some((1, 3)), Edit::DeleteForward),
            Some(EditOutcome {
                value: String::from("ad"),
                caret: 1
            })
        );
    }

    #[test]
    fn movement_normalizes_and_clamps_ranges() {
        assert_eq!(movement_range(4, Movement::Caret(9)), (4, 4));
        assert_eq!(movement_range(4, Movement::Selection(3, 1)), (1, 3));
        assert_eq!(movement_range(4, Movement::Selection(1, 9)), (1, 4));
        assert_eq!(movement_range(4, Movement::SelectAll), (0, 4));
    }

    #[test]
    fn cjk_values_are_edited_at_character_boundaries() {
        assert_eq!(
            apply("你好世界", 2, None, Edit::Backspace),
            Some(EditOutcome {
                value: String::from("你世界"),
                caret: 1
            })
        );
        assert_eq!(
            apply("你世界", 1, None, Edit::Insert("🙂")),
            Some(EditOutcome {
                value: String::from("你🙂世界"),
                caret: 2
            })
        );
    }
}
