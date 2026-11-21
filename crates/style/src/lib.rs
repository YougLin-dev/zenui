//! 编译期 class 词表与类型化样式声明。
//!
//! `view!` 在展开期把 `class="..."` 字面量解析成本模块的声明；运行时与渲染器
//! 只消费已解析的字段，不解析 CSS 字符串。词表是封闭的：未登记的 token 是编译错误。

/// class 声明影响的属性。简写在解析时展开到这些长手属性。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Property {
    PaddingX,
    PaddingY,
    Gap,
    FlexDirection,
    AlignItems,
    JustifyContent,
    Width,
    FlexGrow,
    Background,
    TextColor,
    BorderColor,
    BorderWidth,
    Radius,
    Opacity,
}

/// 语义颜色角色；具体字节由当前调色板决定。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorRole {
    Surface,
    Control,
    ControlHover,
    ControlPressed,
    Accent,
    AccentHover,
    AccentPressed,
    Danger,
    DangerHover,
    TextDefault,
    TextMuted,
    TextInverse,
    TextDisabled,
    BorderDefault,
    BorderAccent,
}

/// 一个颜色：语义角色或显式 RGBA 字节。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    /// 按当前调色板解析的角色。
    Role(ColorRole),
    /// 绕过调色板的固定 RGBA 字节。
    Literal([u8; 4]),
}

/// 属性的类型化取值。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StyleValue {
    /// 逻辑点；整数避免快照出现 `f32`。
    Points(i32),
    Row,
    Column,
    CenterItems,
    SpaceBetween,
    FullWidth,
    Grow,
    Color(Color),
    /// 不透明度百分比（0–100）。
    Opacity(u8),
}

/// 一条已解析的 class 声明。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyleDeclaration {
    property: Property,
    value: StyleValue,
}

impl StyleDeclaration {
    /// 组合一个属性与它的类型化取值。
    pub const fn new(property: Property, value: StyleValue) -> Self {
        Self { property, value }
    }

    /// 这条声明影响的属性。
    pub const fn property(self) -> Property {
        self.property
    }

    /// 这条声明的类型化取值。
    pub const fn value(self) -> StyleValue {
        self.value
    }
}

/// 一个无法解析的 class token。`offset` 是 token 在字面量中的字符偏移。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassError {
    token: String,
    offset: usize,
    message: &'static str,
}

impl ClassError {
    /// 无法解析的 token 文本。
    pub fn token(&self) -> &str {
        &self.token
    }

    /// token 在字面量中的字符偏移。
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// 稳定的诊断文本，不带 token 与偏移。
    pub const fn message(&self) -> &'static str {
        self.message
    }
}

/// 把 class 字面量解析为有顺序的类型化声明。
///
/// # Errors
///
/// 返回字面量中的全部坏 token；顺序与它们在字面量中出现的顺序一致。
/// 条件前缀尚未支持，`hover:` 之类的 token 同样是错误。
pub fn parse_class_literal(text: &str) -> Result<Vec<StyleDeclaration>, Vec<ClassError>> {
    let mut declarations = Vec::new();
    let mut errors = Vec::new();
    for (token, offset) in tokens(text) {
        match parse_token(token) {
            Ok(mut parsed) => declarations.append(&mut parsed),
            Err(message) => errors.push(ClassError {
                token: token.to_owned(),
                offset,
                message,
            }),
        }
    }
    if errors.is_empty() {
        Ok(declarations)
    } else {
        Err(errors)
    }
}

/// 按字符遍历，保留每个 token 的文本与字符偏移。class 字面量很小，
/// 不需要更复杂的词法分析。
fn tokens(text: &str) -> Vec<(&str, usize)> {
    let mut tokens = Vec::new();
    let mut start: Option<(usize, usize)> = None;
    for (char_offset, (byte, character)) in text.char_indices().enumerate() {
        if character.is_whitespace() {
            if let Some((start_byte, start_char)) = start.take() {
                tokens.push((&text[start_byte..byte], start_char));
            }
        } else if start.is_none() {
            start = Some((byte, char_offset));
        }
    }
    if let Some((start_byte, start_char)) = start {
        tokens.push((&text[start_byte..], start_char));
    }
    tokens
}

fn parse_token(token: &str) -> Result<Vec<StyleDeclaration>, &'static str> {
    if token.contains(':') {
        return Err("class conditions are not supported yet");
    }
    let mut declarations = Vec::new();
    let mut push = |property, value| declarations.push(StyleDeclaration::new(property, value));
    match token {
        "flex-row" => push(Property::FlexDirection, StyleValue::Row),
        "flex-col" => push(Property::FlexDirection, StyleValue::Column),
        "items-center" => push(Property::AlignItems, StyleValue::CenterItems),
        "justify-between" => push(Property::JustifyContent, StyleValue::SpaceBetween),
        "w-full" => push(Property::Width, StyleValue::FullWidth),
        "flex-1" => push(Property::FlexGrow, StyleValue::Grow),
        "bg-surface" => push(Property::Background, role(ColorRole::Surface)),
        "bg-control" => push(Property::Background, role(ColorRole::Control)),
        "bg-control-hover" => push(Property::Background, role(ColorRole::ControlHover)),
        "bg-control-pressed" => push(Property::Background, role(ColorRole::ControlPressed)),
        "bg-accent" => push(Property::Background, role(ColorRole::Accent)),
        "bg-accent-hover" => push(Property::Background, role(ColorRole::AccentHover)),
        "bg-accent-pressed" => push(Property::Background, role(ColorRole::AccentPressed)),
        "bg-danger" => push(Property::Background, role(ColorRole::Danger)),
        "bg-danger-hover" => push(Property::Background, role(ColorRole::DangerHover)),
        "text-default" => push(Property::TextColor, role(ColorRole::TextDefault)),
        "text-muted" => push(Property::TextColor, role(ColorRole::TextMuted)),
        "text-inverse" => push(Property::TextColor, role(ColorRole::TextInverse)),
        "text-disabled" => push(Property::TextColor, role(ColorRole::TextDisabled)),
        "border" => push(Property::BorderWidth, StyleValue::Points(1)),
        "border-default" => push(Property::BorderColor, role(ColorRole::BorderDefault)),
        "border-accent" => push(Property::BorderColor, role(ColorRole::BorderAccent)),
        "rounded-sm" => push(Property::Radius, StyleValue::Points(4)),
        "rounded-md" => push(Property::Radius, StyleValue::Points(6)),
        "rounded-lg" => push(Property::Radius, StyleValue::Points(10)),
        "opacity-25" => push(Property::Opacity, StyleValue::Opacity(25)),
        "opacity-50" => push(Property::Opacity, StyleValue::Opacity(50)),
        "opacity-75" => push(Property::Opacity, StyleValue::Opacity(75)),
        "opacity-100" => push(Property::Opacity, StyleValue::Opacity(100)),
        _ => {
            if let Some(points) = spacing(token, "px-")? {
                push(Property::PaddingX, StyleValue::Points(points));
            } else if let Some(points) = spacing(token, "py-")? {
                push(Property::PaddingY, StyleValue::Points(points));
            } else if let Some(points) = spacing(token, "p-")? {
                push(Property::PaddingX, StyleValue::Points(points));
                push(Property::PaddingY, StyleValue::Points(points));
            } else if let Some(points) = spacing(token, "gap-")? {
                push(Property::Gap, StyleValue::Points(points));
            } else {
                return Err("unknown class token");
            }
        }
    }
    Ok(declarations)
}

fn role(role: ColorRole) -> StyleValue {
    StyleValue::Color(Color::Role(role))
}

/// 解析 `<前缀><步长>`；不是该前缀时返回 `Ok(None)`。
fn spacing(token: &str, prefix: &str) -> Result<Option<i32>, &'static str> {
    let Some(step) = token.strip_prefix(prefix) else {
        return Ok(None);
    };
    let points = match step {
        "1" => 4,
        "2" => 8,
        "3" => 12,
        "4" => 16,
        "6" => 24,
        _ => return Err("unknown spacing step"),
    };
    Ok(Some(points))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration(property: Property, value: StyleValue) -> StyleDeclaration {
        StyleDeclaration::new(property, value)
    }

    #[test]
    fn spacing_shorthands_expand_to_longhands() {
        let parsed = parse_class_literal("p-2 px-3 py-1 gap-4").expect("parse");
        assert_eq!(
            parsed,
            vec![
                declaration(Property::PaddingX, StyleValue::Points(8)),
                declaration(Property::PaddingY, StyleValue::Points(8)),
                declaration(Property::PaddingX, StyleValue::Points(12)),
                declaration(Property::PaddingY, StyleValue::Points(4)),
                declaration(Property::Gap, StyleValue::Points(16)),
            ]
        );
    }

    #[test]
    fn layout_utilities_map_to_their_properties() {
        let parsed =
            parse_class_literal("flex-row flex-col items-center justify-between w-full flex-1")
                .expect("parse");
        assert_eq!(
            parsed,
            vec![
                declaration(Property::FlexDirection, StyleValue::Row),
                declaration(Property::FlexDirection, StyleValue::Column),
                declaration(Property::AlignItems, StyleValue::CenterItems),
                declaration(Property::JustifyContent, StyleValue::SpaceBetween),
                declaration(Property::Width, StyleValue::FullWidth),
                declaration(Property::FlexGrow, StyleValue::Grow),
            ]
        );
    }

    #[test]
    fn paint_utilities_map_to_their_properties() {
        let parsed = parse_class_literal(
            "bg-accent text-inverse border border-accent rounded-md opacity-50",
        )
        .expect("parse");
        assert_eq!(
            parsed,
            vec![
                declaration(
                    Property::Background,
                    StyleValue::Color(Color::Role(ColorRole::Accent))
                ),
                declaration(
                    Property::TextColor,
                    StyleValue::Color(Color::Role(ColorRole::TextInverse))
                ),
                declaration(Property::BorderWidth, StyleValue::Points(1)),
                declaration(
                    Property::BorderColor,
                    StyleValue::Color(Color::Role(ColorRole::BorderAccent))
                ),
                declaration(Property::Radius, StyleValue::Points(6)),
                declaration(Property::Opacity, StyleValue::Opacity(50)),
            ]
        );
    }

    #[test]
    fn unknown_tokens_report_their_character_offset() {
        let errors = parse_class_literal("p-3 nope").expect_err("bad token");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].token(), "nope");
        assert_eq!(errors[0].offset(), 4);
        assert_eq!(errors[0].message(), "unknown class token");
    }

    #[test]
    fn every_bad_token_is_reported() {
        let errors = parse_class_literal("nope also-bad p-9").expect_err("bad tokens");
        assert_eq!(
            errors.iter().map(ClassError::token).collect::<Vec<_>>(),
            vec!["nope", "also-bad", "p-9"]
        );
        assert_eq!(errors[2].message(), "unknown spacing step");
    }

    #[test]
    fn conditions_are_rejected_until_they_are_implemented() {
        let errors = parse_class_literal("hover:px-2").expect_err("condition");
        assert_eq!(
            errors[0].message(),
            "class conditions are not supported yet"
        );
    }

    #[test]
    fn an_empty_literal_has_no_declarations() {
        assert!(parse_class_literal("   ").expect("parse").is_empty());
    }
}
