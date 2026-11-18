//! A deliberately small Tailwind-like class -> typed declaration probe.
//! This verifies looking up complete utility keys, diagnostic spans and
//! variant masks. It is NOT an implementation of a CSS cascade or Tailwind 4.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub [u8; 4]);

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    FlexDirection(FlexDirection),
    Points(f32),
    Color(Color),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexDirection {
    Flex,
    Row,
    Column,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Property {
    Display,
    Background,
    TextColor,
    BorderColor,
    Padding,
    Gap,
    Width,
    Radius,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Modifier {
    Base,
    Hover,
    FocusVisible,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub property: Property,
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub modifier: Modifier,
    pub declaration: Declaration,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ClassError {
    pub token: String,
    pub byte_offset: usize,
    pub message: &'static str,
}

pub fn parse_classes(input: &str) -> Result<Vec<Rule>, ClassError> {
    let mut rules = Vec::new();
    for (offset, raw) in input
        .split_whitespace()
        .map(|token| (token.as_ptr() as usize - input.as_ptr() as usize, token))
    {
        let (modifier, base) = match raw.split_once(':') {
            Some(("hover", base)) => (Modifier::Hover, base),
            Some(("focus-visible", base)) => (Modifier::FocusVisible, base),
            Some(_) => {
                return Err(ClassError {
                    token: raw.into(),
                    byte_offset: offset,
                    message: "unsupported native modifier",
                });
            }
            None => (Modifier::Base, raw),
        };
        // Complete tokens only. Never prefix-match `w-[72px]` as spacing `w-`.
        if let Some(value) = base.strip_prefix("w-[").and_then(|v| v.strip_suffix("px]"))
            && let Some(points) = finite_positive(value)
        {
            rules.push(Rule {
                modifier,
                declaration: Declaration {
                    property: Property::Width,
                    value: Value::Points(points),
                },
            });
            continue;
        }
        let declaration = match base {
            "flex" => Declaration {
                property: Property::Display,
                value: Value::FlexDirection(FlexDirection::Flex),
            },
            "flex-row" => Declaration {
                property: Property::Display,
                value: Value::FlexDirection(FlexDirection::Row),
            },
            "flex-col" => Declaration {
                property: Property::Display,
                value: Value::FlexDirection(FlexDirection::Column),
            },
            "bg-blue-600" => Declaration {
                property: Property::Background,
                value: Value::Color(Color([28, 100, 242, 255])),
            },
            "bg-blue-800" => Declaration {
                property: Property::Background,
                value: Value::Color(Color([30, 64, 175, 255])),
            },
            "text-zinc-100" => Declaration {
                property: Property::TextColor,
                value: Value::Color(Color([244, 244, 245, 255])),
            },
            "border-amber-400" => Declaration {
                property: Property::BorderColor,
                value: Value::Color(Color([251, 191, 36, 255])),
            },
            "rounded-md" => Declaration {
                property: Property::Radius,
                value: Value::Points(6.0),
            },
            _ => {
                let (property, value) = base.split_once('-').ok_or_else(|| error(raw, offset))?;
                let property = match property {
                    "p" if !value.contains('[') => Property::Padding,
                    "gap" if !value.contains('[') => Property::Gap,
                    "rounded" if !value.contains('[') => Property::Radius,
                    "w" if !value.contains('[') => Property::Width,
                    _ => return Err(error(raw, offset)),
                };
                let points = finite_positive(value).ok_or_else(|| error(raw, offset))?;
                Declaration {
                    property,
                    value: Value::Points(points * 4.0),
                }
            }
        };
        rules.push(Rule {
            modifier,
            declaration,
        });
    }
    Ok(rules)
}

/// The decimal grammar accepted by this toy's fixed 4px spacing fixture.
/// Signs, exponents, NaN, infinity and overflowing scaled values are rejected.
fn finite_positive(input: &str) -> Option<f32> {
    let mut parts = input.split('.');
    let integer = parts.next()?;
    let fraction = parts.next();
    if parts.next().is_some()
        || integer.is_empty()
        || fraction.is_some_and(str::is_empty)
        || !integer.bytes().all(|b| b.is_ascii_digit())
        || !fraction
            .unwrap_or_default()
            .bytes()
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let value = input.parse::<f32>().ok()?;
    (value.is_finite() && (value * 4.0).is_finite()).then_some(value)
}
fn error(token: &str, offset: usize) -> ClassError {
    ClassError {
        token: token.into(),
        byte_offset: offset,
        message: "unknown native utility",
    }
}

/// Only *active eligible* competing declarations override the underlying write
/// mask. Real v4 variant/cascade ordering is a separate specification and test
/// corpus; this small proof never treats the whole class list as one dirty key.
pub fn resolve(rules: &[Rule], state: Modifier) -> BTreeMap<Property, &Value> {
    let mut fields = BTreeMap::new();
    for rule in rules {
        if rule.modifier == Modifier::Base || rule.modifier == state {
            fields.insert(rule.declaration.property, &rule.declaration.value);
        }
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_utility_keys_lowering_to_typed_fields() {
        let rules =
            parse_classes("flex flex-row p-4 gap-2 rounded-md bg-blue-600 text-zinc-100 w-[72px]")
                .unwrap();
        assert_eq!(rules.len(), 8);
        assert!(rules.iter().all(|rule| match rule.declaration.property {
            Property::Display => matches!(rule.declaration.value, Value::FlexDirection(_)),
            Property::Width => rule.declaration.value == Value::Points(72.0),
            _ => true,
        }));
        assert_eq!(
            rules
                .iter()
                .filter(|r| r.declaration.property == Property::Display)
                .count(),
            2
        );
    }

    #[test]
    fn hover_only_supersedes_the_matching_property() {
        let rules =
            parse_classes("p-4 bg-blue-600 hover:bg-blue-800 gap-2 focus-visible:text-zinc-100")
                .unwrap();
        let normal = resolve(&rules, Modifier::Base);
        let hover = resolve(&rules, Modifier::Hover);
        assert_eq!(
            normal[&Property::Background],
            &Value::Color(Color([28, 100, 242, 255]))
        );
        assert_eq!(
            hover[&Property::Background],
            &Value::Color(Color([30, 64, 175, 255]))
        );
        assert_eq!(normal[&Property::Padding], hover[&Property::Padding]);
        assert!(!normal.contains_key(&Property::TextColor));
    }

    #[test]
    fn invalid_values_and_unknown_classes_report_source_location() {
        let error = parse_classes("p-2 p--2 bg-syzygy-900").unwrap_err();
        assert_eq!(error.token, "p--2");
        assert_eq!(error.byte_offset, 4);
        for input in [
            "w-[100%-2rem]",
            "w-[72rem]",
            "hover:bg-???",
            "side:bg-blue-600",
        ] {
            assert!(
                parse_classes(input).is_err(),
                "expected rejection for {input:?}"
            );
        }
    }
}
