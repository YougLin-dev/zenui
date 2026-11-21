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

impl Property {
    /// 绘制属性不参与布局；状态变体只允许设置这些属性。
    const fn is_paint(self) -> bool {
        matches!(
            self,
            Self::Background
                | Self::TextColor
                | Self::BorderColor
                | Self::BorderWidth
                | Self::Radius
                | Self::Opacity
        )
    }
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

/// 一条声明生效的状态条件。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Condition {
    /// 无条件，按源码顺序参与。
    Base,
    /// 元素持有焦点。
    Focus,
    /// 元素处于禁用状态。
    Disabled,
    /// checkbox 处于勾选状态。
    Checked,
    /// 按钮的语义选中状态来自 `aria:pressed`。
    AriaPressed,
}

impl Condition {
    /// 同一属性被多条活动声明设置时的优先级；数值大者胜出，相同数值用更晚的源码顺序。
    pub const fn rank(self) -> u8 {
        match self {
            Self::Base => 0,
            Self::Focus => 1,
            Self::Disabled => 2,
            Self::Checked => 3,
            Self::AriaPressed => 4,
        }
    }
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
    condition: Condition,
}

impl StyleDeclaration {
    /// 组合一个属性、它的类型化取值与生效条件。
    pub const fn new(property: Property, value: StyleValue, condition: Condition) -> Self {
        Self {
            property,
            value,
            condition,
        }
    }

    /// 这条声明影响的属性。
    pub const fn property(self) -> Property {
        self.property
    }

    /// 这条声明的类型化取值。
    pub const fn value(self) -> StyleValue {
        self.value
    }

    /// 这条声明生效所需的控件状态。
    pub const fn condition(self) -> Condition {
        self.condition
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
/// 未知或复合条件、尚未支持的条件，以及状态变体里的非绘制属性都是错误。
pub fn parse_class_literal(text: &str) -> Result<Vec<StyleDeclaration>, Vec<ClassError>> {
    let mut declarations = Vec::new();
    let mut errors = Vec::new();
    for (token, offset) in tokens(text) {
        let mut error = |message| {
            errors.push(ClassError {
                token: token.to_owned(),
                offset,
                message,
            });
        };
        match parse_token(token) {
            Ok(parsed)
                if parsed.iter().any(|declaration| {
                    declaration.condition() != Condition::Base && !declaration.property().is_paint()
                }) =>
            {
                error("state variants may only set paint properties");
            }
            Ok(mut parsed) => declarations.append(&mut parsed),
            Err(message) => error(message),
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
    let (condition, token) = split_condition(token)?;
    let mut declarations = Vec::new();
    let mut push = |property, value| {
        declarations.push(StyleDeclaration::new(property, value, condition));
    };
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

/// 把 `[condition:]utility` 拆成条件与 utility。
fn split_condition(token: &str) -> Result<(Condition, &str), &'static str> {
    let Some((condition, utility)) = token.split_once(':') else {
        return Ok((Condition::Base, token));
    };
    if utility.contains(':') {
        return Err("compound class conditions are not supported");
    }
    let condition = match condition {
        "focus" => Condition::Focus,
        "disabled" => Condition::Disabled,
        "checked" => Condition::Checked,
        "aria-pressed" => Condition::AriaPressed,
        "hover" | "focus-visible" | "active" | "dark" => {
            return Err("class condition is not supported yet");
        }
        _ => return Err("unknown class condition"),
    };
    Ok((condition, utility))
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
        StyleDeclaration::new(property, value, Condition::Base)
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
    fn semantic_conditions_select_their_declarations() {
        let parsed =
            parse_class_literal("disabled:bg-control-hover checked:bg-accent focus:opacity-75 aria-pressed:text-inverse")
                .expect("parse");
        assert_eq!(
            parsed,
            vec![
                StyleDeclaration::new(
                    Property::Background,
                    StyleValue::Color(Color::Role(ColorRole::ControlHover)),
                    Condition::Disabled
                ),
                StyleDeclaration::new(
                    Property::Background,
                    StyleValue::Color(Color::Role(ColorRole::Accent)),
                    Condition::Checked
                ),
                StyleDeclaration::new(Property::Opacity, StyleValue::Opacity(75), Condition::Focus),
                StyleDeclaration::new(
                    Property::TextColor,
                    StyleValue::Color(Color::Role(ColorRole::TextInverse)),
                    Condition::AriaPressed
                ),
            ]
        );
        assert!(Condition::Focus.rank() < Condition::Disabled.rank());
        assert!(Condition::Disabled.rank() < Condition::Checked.rank());
        assert!(Condition::Checked.rank() < Condition::AriaPressed.rank());
    }

    #[test]
    fn state_variants_may_only_set_paint_properties() {
        let errors = parse_class_literal("disabled:px-2 disabled:w-full").expect_err("layout");
        assert_eq!(errors.len(), 2);
        assert_eq!(
            errors[0].message(),
            "state variants may only set paint properties"
        );
        let paint = parse_class_literal("disabled:border disabled:opacity-50 disabled:rounded-md")
            .expect("border width, opacity and radius are paint");
        assert_eq!(paint.len(), 3);
    }

    #[test]
    fn unsupported_and_compound_conditions_are_errors() {
        let unsupported = parse_class_literal("hover:px-2").expect_err("unsupported");
        assert_eq!(
            unsupported[0].message(),
            "class condition is not supported yet"
        );
        let compound = parse_class_literal("dark:hover:bg-accent").expect_err("compound");
        assert_eq!(
            compound[0].message(),
            "compound class conditions are not supported"
        );
        let unknown = parse_class_literal("pressed:bg-accent").expect_err("unknown");
        assert_eq!(unknown[0].message(), "unknown class condition");
    }

    #[test]
    fn an_empty_literal_has_no_declarations() {
        assert!(parse_class_literal("   ").expect("parse").is_empty());
    }
}
