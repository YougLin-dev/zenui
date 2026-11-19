use proc_macro::TokenStream as TokenStream1;
use proc_macro2::{Span, TokenStream};
use quote::{quote, quote_spanned};
use rstml::node::{
    KVAttributeValue, KeyedAttributeValue, Node, NodeAttribute, NodeBlock, NodeElement,
};
use syn::{Block, Expr, ExprClosure, ExprLit, Lit, spanned::Spanned};

#[proc_macro]
pub fn view(input: TokenStream1) -> TokenStream1 {
    expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

struct Lowered {
    initialization: TokenStream,
    content: Option<TokenStream>,
}

fn expand(input: TokenStream) -> syn::Result<TokenStream> {
    let nodes = rstml::parse2(input)?;
    if nodes.len() != 1 {
        return Err(syn::Error::new(
            Span::call_site(),
            "probe expects exactly one root element",
        ));
    }
    let lowered = lower_node(&nodes[0])?;
    if lowered.content.is_some() {
        return Err(syn::Error::new(
            Span::call_site(),
            "root must be a container element",
        ));
    }
    Ok(lowered.initialization)
}

fn lower_element(element: &NodeElement<rstml::Infallible>) -> syn::Result<Lowered> {
    let tag = element.name().to_string();
    if !matches!(tag.as_str(), "view" | "text" | "button") {
        return Err(syn::Error::new_spanned(
            element.name(),
            "probe accepts only native view/text/button",
        ));
    }
    let mut methods = Vec::new();
    for attribute in element.attributes() {
        let NodeAttribute::Attribute(attribute) = attribute else {
            return Err(syn::Error::new_spanned(
                attribute,
                "probe does not implement spread attributes",
            ));
        };
        let key = attribute.key.to_string();
        if key == "class" {
            let string = attribute.value_literal_string().ok_or_else(|| {
                syn::Error::new_spanned(&attribute.key, "class must be a literal string")
            })?;
            let span = expression(attribute)?.span();
            let declarations = compile_class_literal(span, &string)?;
            methods.push(quote_spanned! {span=> .styles(#declarations) });
        } else if let Some(event) = key.strip_prefix("on:") {
            if !matches!(event, "click" | "change" | "input") {
                return Err(syn::Error::new_spanned(
                    &attribute.key,
                    "unsupported probe event",
                ));
            }
            let expression = expression(attribute)?;
            methods.push(quote_spanned! {expression.span()=> .on(#event, #expression) });
        } else {
            return Err(syn::Error::new_spanned(
                &attribute.key,
                "probe implements only class and on:click/change/input",
            ));
        }
    }
    let children = element
        .children
        .iter()
        .map(lower_node)
        .collect::<syn::Result<Vec<_>>>()?;
    if children
        .iter()
        .filter(|child| child.content.is_some())
        .count()
        > 1
    {
        return Err(syn::Error::new_spanned(
            element.name(),
            "this probe supports at most one text content child per element",
        ));
    }
    let initialization = children
        .iter()
        .filter(|child| child.content.is_none())
        .map(|child| {
            let initialization = &child.initialization;
            quote! { __zenui_node = __zenui_node.child(#initialization); }
        });
    let content = children.iter().find_map(|child| child.content.as_ref());
    let content_assignment =
        content.map(|expression| quote! { __zenui_node.content = #expression; });
    Ok(Lowered {
        initialization: quote! {{
            let mut __zenui_node = ::zenui_macro_runtime::Node::view(#tag) #(#methods)*;
            #(#initialization)*
            #content_assignment
            __zenui_node
        }},
        content: None,
    })
}

fn lower_node(node: &Node) -> syn::Result<Lowered> {
    match node {
        Node::Element(element) => lower_element(element),
        Node::Text(text) => {
            let literal = text.value_string();
            Ok(contents(quote! {
                Some(::zenui_macro_runtime::Content::Static(::std::string::String::from(#literal)))
            }))
        }
        Node::Block(NodeBlock::ValidBlock(block)) => lower_block(block),
        other => Err(syn::Error::new_spanned(
            other,
            "probe does not implement HTML comments, doctypes, raw text or dynamic fragments",
        )),
    }
}

fn contents(expression: TokenStream) -> Lowered {
    Lowered {
        initialization: quote! { unreachable!("content is installed by its parent element") },
        content: Some(expression),
    }
}

fn expression(attribute: &rstml::node::KeyedAttribute) -> syn::Result<&Expr> {
    match &attribute.possible_value {
        KeyedAttributeValue::Value(value) => match &value.value {
            KVAttributeValue::Expr(Expr::Lit(ExprLit {
                lit: Lit::Str(_), ..
            })) if attribute.key.to_string().starts_with("on:") => Err(syn::Error::new_spanned(
                value,
                "event handlers must be Rust expressions, never script strings",
            )),
            KVAttributeValue::Expr(expression) => Ok(expression),
            _ => Err(syn::Error::new_spanned(value, "invalid expression")),
        },
        _ => Err(syn::Error::new_spanned(
            attribute,
            "expected a Rust expression",
        )),
    }
}

fn lower_block(block: &Block) -> syn::Result<Lowered> {
    if block.stmts.len() != 1 {
        return Err(syn::Error::new_spanned(
            block,
            "probe text content expects one string literal or move closure",
        ));
    }
    let syn::Stmt::Expr(expression, None) = &block.stmts[0] else {
        return Err(syn::Error::new_spanned(
            block,
            "probe text content expects one string literal or move closure",
        ));
    };
    if let Expr::Lit(literal) = expression
        && matches!(literal.lit, Lit::Str(_))
    {
        return Ok(contents(quote_spanned! {literal.span()=>
            Some(::zenui_macro_runtime::Content::Static(::std::string::String::from(#literal)))
        }));
    }

    if matches!(
        expression,
        Expr::Closure(ExprClosure {
            capture: Some(_),
            ..
        })
    ) {
        Ok(contents(quote_spanned! {expression.span()=>
            Some(::zenui_macro_runtime::Content::Binding(::std::boxed::Box::new(#expression)))
        }))
    } else {
        Err(syn::Error::new_spanned(
            expression,
            "probe text content expects a string literal or `move || String` expression",
        ))
    }
}

fn compile_class_literal(span: Span, input: &str) -> syn::Result<TokenStream> {
    let rules = zenui_macro_runtime::compile_classes(input).map_err(|error| {
        syn::Error::new(
            span,
            format!(
                "unsupported native class `{}` at byte {} in class literal",
                error.token, error.offset
            ),
        )
    })?;
    let mut declarations = Vec::new();
    for rule in rules {
        let property = match rule.property {
            zenui_macro_runtime::Property::DisplayState => quote! { DisplayState },
            zenui_macro_runtime::Property::Background => quote! { Background },
            zenui_macro_runtime::Property::Width => quote! { Width },
        };
        let value = match rule.value {
            zenui_macro_runtime::Value::Flex => quote! { Flex },
            zenui_macro_runtime::Value::Row => quote! { Row },
            zenui_macro_runtime::Value::Column => quote! { Column },
            zenui_macro_runtime::Value::Rgba([r, g, b, a]) => quote! { Rgba([#r, #g, #b, #a]) },
            zenui_macro_runtime::Value::LogicalPoints(value) => quote! { LogicalPoints(#value) },
        };
        let condition = match rule.condition {
            zenui_macro_runtime::Condition::Always => quote! { Always },
            zenui_macro_runtime::Condition::Hover => quote! { Hover },
            zenui_macro_runtime::Condition::Pressed => quote! { Pressed },
        };
        declarations.push(quote_spanned! {span=>
            ::zenui_macro_runtime::StyleRule {
                property: ::zenui_macro_runtime::Property::#property,
                value: ::zenui_macro_runtime::Value::#value,
                condition: ::zenui_macro_runtime::Condition::#condition,
            },
        });
    }
    Ok(quote! { ::std::vec![#(#declarations)*] })
}
