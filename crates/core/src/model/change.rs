use super::id::{FieldId, FilterId, RowId};

/// 一次 Store 提交的精确净变化。
///
/// 非泛型：只使用行身份、字段与过滤标识。`inserted`/`removed` 是不同的行身份，
/// 同一业务键的删除后重建不会被合并。`fields` 只包含仍存活行上注册字段的变化
/// （按行身份、字段 id 去重排序）；`membership` 只列出成员真正发生迁移的过滤；
/// `order` 只表示存活行的相对顺序因显式重排而变化，插入/删除由
/// `inserted`/`removed`/`membership` 表达。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    pub inserted: Vec<RowId>,
    pub removed: Vec<RowId>,
    pub fields: Vec<(RowId, FieldId)>,
    pub membership: Vec<FilterId>,
    pub order: bool,
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.inserted.is_empty()
            && self.removed.is_empty()
            && self.fields.is_empty()
            && self.membership.is_empty()
            && !self.order
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_change_set_is_empty() {
        assert!(ChangeSet::default().is_empty());
    }

    #[test]
    fn any_recorded_change_is_not_empty() {
        let id = super::super::RowId::new(0, super::super::id::next_generation());
        assert!(
            !ChangeSet {
                inserted: vec![id],
                ..ChangeSet::default()
            }
            .is_empty()
        );
        assert!(
            !ChangeSet {
                order: true,
                ..ChangeSet::default()
            }
            .is_empty()
        );
    }
}
