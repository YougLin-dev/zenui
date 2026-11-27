use std::collections::BTreeMap;
use std::fmt;

use super::id::{FieldId, FilterId};

/// 应用行数据必须提供的业务键。
///
/// 键只用于查找、排序与 keyed 重排；行身份是 [`super::RowId`]。
pub trait RowKey: Copy + Eq + std::hash::Hash + Ord + 'static {}

impl<T: Copy + Eq + std::hash::Hash + Ord + 'static> RowKey for T {}

/// 通用行模型的行数据。
pub trait Row: Clone + 'static {
    type Key: RowKey;

    fn key(&self) -> Self::Key;
}

type FieldDiff<R> = Box<dyn Fn(&R, &R) -> bool>;

/// 模型声明的可观察字段与它的相等判定。
///
/// `Store` 在提交时用这些字段比较提交前后的行，产出精确的 `ChangeSet.fields`。
/// 未注册字段的修改仍会写入行数据，但不参与字段变化、过滤成员迁移或绑定通知。
pub struct FieldDef<R: Row> {
    id: FieldId,
    differs: FieldDiff<R>,
}

impl<R: Row> FieldDef<R> {
    /// 用字段访问器比较前后值。
    pub fn comparing<T: PartialEq + 'static>(id: FieldId, get: fn(&R) -> &T) -> Self {
        Self {
            id,
            differs: Box::new(move |before, after| get(before) != get(after)),
        }
    }

    pub const fn id(&self) -> FieldId {
        self.id
    }

    pub(crate) fn differs(&self, before: &R, after: &R) -> bool {
        (self.differs)(before, after)
    }
}

impl<R: Row> fmt::Debug for FieldDef<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FieldDef").field("id", &self.id).finish()
    }
}

/// 一个过滤规格：predicate 之外，还声明它观察哪些字段。
///
/// Store 用 `watched_fields` 把字段变化投影到候选行；一个字段变化会重新求值
/// 所有观察该字段的 filter，即使行当前不在结果内，因此“新匹配”的行不会漏掉。
pub trait FilterSpec<R: Row>: 'static {
    fn id(&self) -> FilterId;

    /// 该 filter 依赖的字段；空列表表示成员只在插入/删除时变化。
    fn watched_fields(&self) -> &'static [FieldId];

    fn matches(&self, row: &R) -> bool;
}

pub(crate) struct FilterDef<R: Row> {
    pub(crate) spec: Box<dyn FilterSpec<R>>,
    pub(crate) watched: Vec<FieldId>,
}

/// 校验过的模型 schema：字段注册表与过滤注册表。
pub struct Schema<R: Row> {
    fields: BTreeMap<FieldId, FieldDef<R>>,
    filters: BTreeMap<FilterId, FilterDef<R>>,
}

impl<R: Row> Schema<R> {
    pub(crate) fn into_parts(
        self,
    ) -> (
        BTreeMap<FieldId, FieldDef<R>>,
        BTreeMap<FilterId, FilterDef<R>>,
    ) {
        (self.fields, self.filters)
    }

    pub fn fields(&self) -> impl Iterator<Item = FieldId> + '_ {
        self.fields.keys().copied()
    }

    pub fn filters(&self) -> impl Iterator<Item = FilterId> + '_ {
        self.filters.keys().copied()
    }
}

impl<R: Row> fmt::Debug for Schema<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Schema")
            .field("fields", &self.fields.len())
            .field("filters", &self.filters.len())
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SchemaError {
    DuplicateField(FieldId),
    DuplicateFilter(FilterId),
    UnknownWatchedField { filter: FilterId, field: FieldId },
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateField(field) => write!(f, "duplicate field id {}", field.index()),
            Self::DuplicateFilter(filter) => write!(f, "duplicate filter id {}", filter.index()),
            Self::UnknownWatchedField { filter, field } => write!(
                f,
                "filter {} watches unregistered field {}",
                filter.index(),
                field.index()
            ),
        }
    }
}

impl std::error::Error for SchemaError {}

/// 构建 schema；重复 id 与未注册的观察字段在 [`SchemaBuilder::build`] 拒绝。
pub struct SchemaBuilder<R: Row> {
    fields: Vec<FieldDef<R>>,
    filters: Vec<Box<dyn FilterSpec<R>>>,
}

impl<R: Row> SchemaBuilder<R> {
    pub fn new() -> Self {
        Self {
            fields: Vec::new(),
            filters: Vec::new(),
        }
    }

    pub fn field(mut self, def: FieldDef<R>) -> Self {
        self.fields.push(def);
        self
    }

    pub fn filter(mut self, spec: impl FilterSpec<R>) -> Self {
        self.filters.push(Box::new(spec));
        self
    }

    pub fn build(self) -> Result<Schema<R>, SchemaError> {
        let mut fields = BTreeMap::new();
        for def in self.fields {
            let id = def.id();
            if fields.insert(id, def).is_some() {
                return Err(SchemaError::DuplicateField(id));
            }
        }
        let mut filters = BTreeMap::new();
        for spec in self.filters {
            let id = spec.id();
            let watched = spec.watched_fields().to_vec();
            for field in &watched {
                if !fields.contains_key(field) {
                    return Err(SchemaError::UnknownWatchedField {
                        filter: id,
                        field: *field,
                    });
                }
            }
            if filters.insert(id, FilterDef { spec, watched }).is_some() {
                return Err(SchemaError::DuplicateFilter(id));
            }
        }
        Ok(Schema { fields, filters })
    }
}

impl<R: Row> Default for SchemaBuilder<R> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Note {
        key: &'static str,
        text: String,
        pinned: bool,
    }

    impl Row for Note {
        type Key = &'static str;

        fn key(&self) -> Self::Key {
            self.key
        }
    }

    const TEXT: FieldId = FieldId::new(0);
    const PINNED: FieldId = FieldId::new(1);

    struct Pinned;

    impl FilterSpec<Note> for Pinned {
        fn id(&self) -> FilterId {
            FilterId::new(0)
        }

        fn watched_fields(&self) -> &'static [FieldId] {
            &[PINNED]
        }

        fn matches(&self, row: &Note) -> bool {
            row.pinned
        }
    }

    #[test]
    fn a_schema_keeps_the_registered_fields_and_filters() {
        let schema = SchemaBuilder::new()
            .field(FieldDef::comparing(TEXT, |row: &Note| &row.text))
            .field(FieldDef::comparing(PINNED, |row: &Note| &row.pinned))
            .filter(Pinned)
            .build()
            .expect("valid schema");
        assert_eq!(schema.fields().collect::<Vec<_>>(), vec![TEXT, PINNED]);
        assert_eq!(schema.filters().collect::<Vec<_>>(), vec![FilterId::new(0)]);
    }

    #[test]
    fn duplicate_field_ids_are_rejected() {
        let error = SchemaBuilder::new()
            .field(FieldDef::comparing(TEXT, |row: &Note| &row.text))
            .field(FieldDef::comparing(TEXT, |row: &Note| &row.text))
            .build()
            .expect_err("duplicate field");
        assert_eq!(error, SchemaError::DuplicateField(TEXT));
    }

    #[test]
    fn duplicate_filter_ids_are_rejected() {
        let error = SchemaBuilder::new()
            .field(FieldDef::comparing(PINNED, |row: &Note| &row.pinned))
            .filter(Pinned)
            .filter(Pinned)
            .build()
            .expect_err("duplicate filter");
        assert_eq!(error, SchemaError::DuplicateFilter(FilterId::new(0)));
    }

    #[test]
    fn watching_an_unregistered_field_is_rejected() {
        let error = SchemaBuilder::new()
            .filter(Pinned)
            .build()
            .expect_err("unregistered field");
        assert_eq!(
            error,
            SchemaError::UnknownWatchedField {
                filter: FilterId::new(0),
                field: PINNED,
            }
        );
    }
}
