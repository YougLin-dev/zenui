//! Minimal inspection records used by the real proc-macro probe.
//!
//! `Binding` is explicitly evaluated by this test. It makes no claim to be a
//! dependency-tracking signal system; that is the independent reactive probe.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Property {
    DisplayState,
    Background,
    Width,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    Always,
    Hover,
    Pressed,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Flex,
    Row,
    Column,
    Rgba([u8; 4]),
    LogicalPoints(f32),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StyleRule {
    pub property: Property,
    pub value: Value,
    pub condition: Condition,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ClassError {
    pub token: String,
    pub offset: usize,
}
pub fn error(token: &str, offset: usize) -> ClassError {
    ClassError {
        token: token.into(),
        offset,
    }
}

/// A deliberately enumerated subset, shared by the macro and inspection tests.
/// No runtime parsing, CSS cascade, arbitrary Tailwind compatibility or hidden
/// fallback for a typo is implemented here.
pub fn compile_classes(input: &str) -> Result<Vec<StyleRule>, ClassError> {
    let mut out = Vec::new();
    for (offset, token) in input
        .split_whitespace()
        .map(|token| (token.as_ptr() as usize - input.as_ptr() as usize, token))
    {
        let (condition, base) = match token.split_once(':') {
            Some(("hover", base)) => (Condition::Hover, base),
            Some(("active", base)) => (Condition::Pressed, base),
            Some(_) => return Err(error(token, offset)),
            None => (Condition::Always, token),
        };
        let rule = match base {
            "flex" => StyleRule {
                property: Property::DisplayState,
                value: Value::Flex,
                condition,
            },
            "flex-row" => StyleRule {
                property: Property::DisplayState,
                value: Value::Row,
                condition,
            },
            "flex-col" => StyleRule {
                property: Property::DisplayState,
                value: Value::Column,
                condition,
            },
            "bg-blue-600" => StyleRule {
                property: Property::Background,
                value: Value::Rgba([28, 100, 242, 255]),
                condition,
            },
            "bg-blue-800" => StyleRule {
                property: Property::Background,
                value: Value::Rgba([30, 64, 175, 255]),
                condition,
            },
            "w-72" => StyleRule {
                property: Property::Width,
                value: Value::LogicalPoints(288.0),
                condition,
            },
            _ => return Err(error(token, offset)),
        };
        out.push(rule);
    }
    Ok(out)
}

pub enum Content {
    Static(String),
    Binding(Box<dyn Fn() -> String>),
}

type Event = (&'static str, Box<dyn FnMut()>);

pub struct Node {
    pub tag: &'static str,
    pub styles: Vec<StyleRule>,
    pub children: Vec<Node>,
    pub content: Option<Content>,
    events: Vec<Event>,
}
impl Node {
    pub fn view(tag: &'static str) -> Self {
        Self {
            tag,
            styles: Vec::new(),
            children: Vec::new(),
            content: None,
            events: Vec::new(),
        }
    }
    pub fn styles(mut self, styles: Vec<StyleRule>) -> Self {
        self.styles = styles;
        self
    }
    pub fn child(mut self, child: Self) -> Self {
        self.children.push(child);
        self
    }
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.content = Some(Content::Static(text.into()));
        self
    }
    pub fn bind_text(mut self, value: impl Fn() -> String + 'static) -> Self {
        self.content = Some(Content::Binding(Box::new(value)));
        self
    }
    pub fn on(mut self, event: &'static str, handler: impl FnMut() + 'static) -> Self {
        self.events.push((event, Box::new(handler)));
        self
    }
    pub fn fire(&mut self, event: &str) -> bool {
        let Some((_, handler)) = self.events.iter_mut().find(|(name, _)| *name == event) else {
            return false;
        };
        handler();
        true
    }
    pub fn content(&self) -> Option<&str> {
        match &self.content {
            Some(Content::Static(value)) => Some(value),
            // A real RenderBinding would be committed before layout, not lazily
            // within render access; this probe only verifies closure preservation.
            Some(Content::Binding(_)) => panic!("test must explicitly evaluate the binding"),
            None => None,
        }
    }
    pub fn evaluate_content(&self) -> Option<String> {
        match &self.content {
            Some(Content::Static(value)) => Some(value.clone()),
            Some(Content::Binding(value)) => Some(value()),
            None => None,
        }
    }
}
