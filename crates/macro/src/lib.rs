use proc_macro::TokenStream as TokenStream1;
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use rstml::node::{
    KVAttributeValue, KeyedAttributeValue, Node, NodeAttribute, NodeBlock, NodeElement,
};
use syn::{Block, Expr, FnArg, ItemFn, Pat, ReturnType, Type, spanned::Spanned};
use zenui_style::{Condition, Property, StyleDeclaration, StyleValue};

#[proc_macro]
pub fn view(input: TokenStream1) -> TokenStream1 {
    expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_attribute]
pub fn component(_attribute: TokenStream1, item: TokenStream1) -> TokenStream1 {
    expand_component(item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand(input: TokenStream) -> syn::Result<TokenStream> {
    let nodes = rstml::parse2(input)?;
    let nodes = nodes
        .iter()
        .map(lower_node)
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(quote! {
        ::core::result::Result::Ok(::zenui::view::MountPlan::new(::std::vec![#(#nodes),*]))
    })
}

fn lower_node(node: &Node) -> syn::Result<TokenStream> {
    match node {
        Node::Element(element) => lower_element(element),
        Node::Text(text) => {
            let literal = text.value_string();
            Ok(quote! { ::zenui::view::text_static(#literal) })
        }
        Node::Block(NodeBlock::ValidBlock(block)) => lower_block(block),
        other => Err(syn::Error::new_spanned(
            other,
            "view! does not implement comments, doctypes, raw text or dynamic fragments",
        )),
    }
}

fn lower_block(block: &Block) -> syn::Result<TokenStream> {
    let Some(expression) = single_expression(block)? else {
        return Err(syn::Error::new_spanned(
            block,
            "view! text content expects one expression",
        ));
    };
    match expression {
        Expr::Lit(literal) if matches!(literal.lit, syn::Lit::Str(_)) => {
            Ok(quote_spanned! {literal.span()=>
                ::zenui::view::text_static(#literal)
            })
        }
        expression => Ok(quote_spanned! {expression.span()=>
            ::zenui::view::child(#expression)
        }),
    }
}

fn single_expression(block: &Block) -> syn::Result<Option<&Expr>> {
    if block.stmts.len() != 1 {
        return Ok(None);
    }
    let syn::Stmt::Expr(expression, None) = &block.stmts[0] else {
        return Ok(None);
    };
    Ok(Some(expression))
}

fn lower_element(element: &NodeElement<rstml::Infallible>) -> syn::Result<TokenStream> {
    let tag = element.name().to_string();
    match tag.as_str() {
        "Show" => lower_show(element),
        "For" => lower_for(element),
        _ if tag.chars().next().is_some_and(char::is_uppercase) => lower_component(element, &tag),
        _ => lower_intrinsic(element, &tag),
    }
}

fn lower_intrinsic(
    element: &NodeElement<rstml::Infallible>,
    tag: &str,
) -> syn::Result<TokenStream> {
    let mut attributes = Vec::new();
    let mut styles = Vec::new();
    let mut inline_styles = Vec::new();
    let mut class_seen = false;
    let mut style_names = Vec::new();
    for attribute in element.attributes() {
        if let NodeAttribute::Attribute(attribute) = attribute {
            let key = attribute.key.to_string();
            if key == "class" {
                if class_seen {
                    return Err(syn::Error::new_spanned(
                        &attribute.key,
                        "duplicate class attribute",
                    ));
                }
                class_seen = true;
                styles = lower_class(attribute)?;
                continue;
            }
            if key.starts_with("style:") {
                if style_names.contains(&key) {
                    return Err(syn::Error::new_spanned(
                        &attribute.key,
                        format!("duplicate style attribute `{key}`"),
                    ));
                }
                style_names.push(key.clone());
                inline_styles.push(lower_style(&key, attribute)?);
                continue;
            }
        }
        attributes.push(lower_attribute(attribute)?);
    }
    let children = element
        .children
        .iter()
        .map(lower_node)
        .collect::<syn::Result<Vec<_>>>()?;
    if styles.is_empty() && inline_styles.is_empty() {
        return Ok(quote! {
            ::zenui::view::element(
                #tag,
                ::std::vec![#(#attributes),*],
                ::std::vec![#(#children),*],
            )
        });
    }
    let declarations = styles.iter().map(declaration_tokens);
    Ok(quote! {
        ::zenui::view::styled_element(
            #tag,
            ::std::vec![#(#attributes),*],
            const { &[#(#declarations),*] },
            ::std::vec![#(#inline_styles),*],
            ::std::vec![#(#children),*],
        )
    })
}

/// 把 `style:<property>` 降到对应的类型化构造器；属性名在展开期校验。
fn lower_style(key: &str, attribute: &rstml::node::KeyedAttribute) -> syn::Result<TokenStream> {
    let constructor = match key {
        "style:background" => quote! { style_background },
        "style:color" => quote! { style_color },
        "style:border-color" => quote! { style_border_color },
        "style:opacity" => quote! { style_opacity },
        "style:width" => quote! { style_width },
        other => {
            return Err(syn::Error::new_spanned(
                &attribute.key,
                format!("unsupported style property `{other}`"),
            ));
        }
    };
    let expression = unbraced(expression(attribute)?);
    Ok(quote_spanned! {expression.span()=>
        ::zenui::view::#constructor(#expression)
    })
}

/// `style:width={value}` 的块表达式只包一层；直接传递内层表达式，
/// 避免生成代码触发 `unused_braces`。
fn unbraced(expression: &Expr) -> &Expr {
    if let Expr::Block(block) = expression
        && let [syn::Stmt::Expr(inner, None)] = block.block.stmts.as_slice()
    {
        inner
    } else {
        expression
    }
}

/// 把 `class="..."` 字面量解析成类型化声明；不是字面量时给出定位到表达式的错误。
fn lower_class(attribute: &rstml::node::KeyedAttribute) -> syn::Result<Vec<StyleDeclaration>> {
    let span = expression(attribute)?.span();
    let Some(text) = literal_string(attribute) else {
        return Err(syn::Error::new(span, "class must be a string literal"));
    };
    zenui_style::parse_class_literal(&text).map_err(|errors| class_errors(span, &errors))
}

/// 字面量里的全部坏 token 合成一条诊断，保持它们在字面量中的顺序。
/// 所有消息共享同一个字面量 span；表达式位置的 `view!` 不能展开成多条
/// 并列的 `compile_error!`，否则后续 token 会被当成宏展开的一部分解析。
fn class_errors(span: Span, errors: &[zenui_style::ClassError]) -> syn::Error {
    let message = errors
        .iter()
        .map(class_error_message)
        .collect::<Vec<_>>()
        .join("\n");
    syn::Error::new(span, message)
}

fn class_error_message(error: &zenui_style::ClassError) -> String {
    format!(
        "{} `{}` at character {}",
        error.message(),
        error.token(),
        error.offset()
    )
}

fn declaration_tokens(declaration: &StyleDeclaration) -> TokenStream {
    let property = property_tokens(declaration.property());
    let value = style_value_tokens(declaration.value());
    let condition = condition_tokens(declaration.condition());
    quote! {
        ::zenui::style::StyleDeclaration::new(
            ::zenui::style::Property::#property,
            #value,
            ::zenui::style::Condition::#condition,
        )
    }
}

fn condition_tokens(condition: Condition) -> TokenStream {
    match condition {
        Condition::Base => quote! { Base },
        Condition::Focus => quote! { Focus },
        Condition::Disabled => quote! { Disabled },
        Condition::Checked => quote! { Checked },
        Condition::AriaPressed => quote! { AriaPressed },
    }
}

fn property_tokens(property: Property) -> TokenStream {
    match property {
        Property::PaddingX => quote! { PaddingX },
        Property::PaddingY => quote! { PaddingY },
        Property::Gap => quote! { Gap },
        Property::FlexDirection => quote! { FlexDirection },
        Property::AlignItems => quote! { AlignItems },
        Property::JustifyContent => quote! { JustifyContent },
        Property::Width => quote! { Width },
        Property::FlexGrow => quote! { FlexGrow },
        Property::Background => quote! { Background },
        Property::TextColor => quote! { TextColor },
        Property::BorderColor => quote! { BorderColor },
        Property::BorderWidth => quote! { BorderWidth },
        Property::Radius => quote! { Radius },
        Property::Opacity => quote! { Opacity },
    }
}

fn style_value_tokens(value: StyleValue) -> TokenStream {
    match value {
        StyleValue::Points(points) => quote! { ::zenui::style::StyleValue::Points(#points) },
        StyleValue::Row => quote! { ::zenui::style::StyleValue::Row },
        StyleValue::Column => quote! { ::zenui::style::StyleValue::Column },
        StyleValue::CenterItems => quote! { ::zenui::style::StyleValue::CenterItems },
        StyleValue::SpaceBetween => quote! { ::zenui::style::StyleValue::SpaceBetween },
        StyleValue::FullWidth => quote! { ::zenui::style::StyleValue::FullWidth },
        StyleValue::Grow => quote! { ::zenui::style::StyleValue::Grow },
        StyleValue::Color(color) => {
            let color = color_tokens(color);
            quote! { ::zenui::style::StyleValue::Color(#color) }
        }
        StyleValue::Length(length) => {
            let fixed = length.fixed();
            let unit = length_unit_tokens(length.unit());
            quote! {
                ::zenui::style::StyleValue::Length(
                    ::zenui::style::Length::from_fixed(
                        #fixed,
                        ::zenui::style::LengthUnit::#unit,
                    )
                )
            }
        }
        StyleValue::Opacity(percent) => {
            quote! { ::zenui::style::StyleValue::Opacity(#percent) }
        }
    }
}

fn length_unit_tokens(unit: zenui_style::LengthUnit) -> TokenStream {
    match unit {
        zenui_style::LengthUnit::Points => quote! { Points },
        zenui_style::LengthUnit::Em => quote! { Em },
    }
}

fn color_tokens(color: zenui_style::Color) -> TokenStream {
    match color {
        zenui_style::Color::Role(role) => {
            let role = role_tokens(role);
            quote! { ::zenui::style::Color::Role(::zenui::style::ColorRole::#role) }
        }
        zenui_style::Color::Literal(bytes) => {
            let [red, green, blue, alpha] = bytes;
            quote! { ::zenui::style::Color::Literal([#red, #green, #blue, #alpha]) }
        }
    }
}

fn role_tokens(role: zenui_style::ColorRole) -> TokenStream {
    match role {
        zenui_style::ColorRole::Surface => quote! { Surface },
        zenui_style::ColorRole::Control => quote! { Control },
        zenui_style::ColorRole::ControlHover => quote! { ControlHover },
        zenui_style::ColorRole::ControlPressed => quote! { ControlPressed },
        zenui_style::ColorRole::Accent => quote! { Accent },
        zenui_style::ColorRole::AccentHover => quote! { AccentHover },
        zenui_style::ColorRole::AccentPressed => quote! { AccentPressed },
        zenui_style::ColorRole::Danger => quote! { Danger },
        zenui_style::ColorRole::DangerHover => quote! { DangerHover },
        zenui_style::ColorRole::TextDefault => quote! { TextDefault },
        zenui_style::ColorRole::TextMuted => quote! { TextMuted },
        zenui_style::ColorRole::TextInverse => quote! { TextInverse },
        zenui_style::ColorRole::TextDisabled => quote! { TextDisabled },
        zenui_style::ColorRole::BorderDefault => quote! { BorderDefault },
        zenui_style::ColorRole::BorderAccent => quote! { BorderAccent },
    }
}

fn lower_attribute(attribute: &NodeAttribute) -> syn::Result<TokenStream> {
    let NodeAttribute::Attribute(attribute) = attribute else {
        return Err(syn::Error::new_spanned(
            attribute,
            "view! does not implement spread attributes",
        ));
    };
    let key = attribute.key.to_string();
    if let Some(event) = key.strip_prefix("on:") {
        let expression = expression(attribute)?;
        let constructor = match event {
            "click" => quote! { on_click },
            "input" => quote! { on_input },
            "change" => quote! { on_change },
            "submit" => quote! { on_submit },
            "keydown" => quote! { on_keydown },
            "focus" => quote! { on_focus },
            "blur" => quote! { on_blur },
            "compositionstart" => quote! { on_composition_start },
            "compositionupdate" => quote! { on_composition_update },
            "compositionend" => quote! { on_composition_end },
            "compositioncancel" => quote! { on_composition_cancel },
            other => {
                return Err(syn::Error::new_spanned(
                    &attribute.key,
                    format!("unsupported event `{other}`"),
                ));
            }
        };
        return Ok(quote_spanned! {expression.span()=>
            ::zenui::view::#constructor(#expression)
        });
    }
    match literal_string(attribute) {
        Some(string) => Ok(quote! { ::zenui::view::attr(#key, #string) }),
        None => {
            let expression = expression(attribute)?;
            Ok(quote_spanned! {expression.span()=>
                ::zenui::view::attr(#key, #expression)
            })
        }
    }
}

fn literal_string(attribute: &rstml::node::KeyedAttribute) -> Option<String> {
    match &attribute.possible_value {
        KeyedAttributeValue::Value(value) => match &value.value {
            KVAttributeValue::Expr(Expr::Lit(literal)) => match &literal.lit {
                syn::Lit::Str(string) => Some(string.value()),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

fn expression(attribute: &rstml::node::KeyedAttribute) -> syn::Result<&Expr> {
    match &attribute.possible_value {
        KeyedAttributeValue::Value(value) => match &value.value {
            KVAttributeValue::Expr(expression) => Ok(expression),
            _ => Err(syn::Error::new_spanned(
                value,
                "view! attributes must be Rust expressions",
            )),
        },
        _ => Err(syn::Error::new_spanned(
            attribute,
            "view! attributes must have a value",
        )),
    }
}

fn lower_component(
    element: &NodeElement<rstml::Infallible>,
    tag: &str,
) -> syn::Result<TokenStream> {
    let mut setters = Vec::new();
    for attribute in element.attributes() {
        let NodeAttribute::Attribute(attribute) = attribute else {
            return Err(syn::Error::new_spanned(
                attribute,
                "view! does not implement spread attributes",
            ));
        };
        let key = format_ident!("{}", attribute.key.to_string());
        if let Some(string) = literal_string(attribute) {
            setters.push(quote! { .#key(#string) });
        } else {
            let expression = expression(attribute)?;
            setters.push(quote_spanned! {expression.span()=> .#key(#expression) });
        }
    }
    let component = format_ident!("{tag}");
    let label = tag;
    let children = if element.children.is_empty() {
        quote! {}
    } else {
        let plan = children_plan(element)?;
        quote! { .children(#plan) }
    };
    Ok(quote! {
        ::zenui::view::component(#label, move || {
            #component::builder() #(#setters)* #children .mount()
        })
    })
}

fn lower_show(element: &NodeElement<rstml::Infallible>) -> syn::Result<TokenStream> {
    let mut when = None;
    let mut fallback = None;
    for attribute in element.attributes() {
        let NodeAttribute::Attribute(attribute) = attribute else {
            return Err(syn::Error::new_spanned(
                attribute,
                "Show does not implement spread attributes",
            ));
        };
        match attribute.key.to_string().as_str() {
            "when" => when = Some(expression(attribute)?),
            "fallback" => fallback = Some(expression(attribute)?),
            other => {
                return Err(syn::Error::new_spanned(
                    &attribute.key,
                    format!("unsupported Show attribute `{other}`"),
                ));
            }
        }
    }
    let when = when.ok_or_else(|| syn::Error::new_spanned(element.name(), "Show needs `when`"))?;
    let then = children_plan(element)?;
    let fallback = match fallback {
        Some(expression) => quote! {
            ::core::option::Option::Some(::core::convert::Into::into(#expression))
        },
        None => quote! { ::core::option::Option::None },
    };
    Ok(quote_spanned! {element.span()=>
        ::zenui::view::show(#when, #then, #fallback)
    })
}

fn lower_for(element: &NodeElement<rstml::Infallible>) -> syn::Result<TokenStream> {
    let mut each = None;
    let mut key = None;
    for attribute in element.attributes() {
        let NodeAttribute::Attribute(attribute) = attribute else {
            return Err(syn::Error::new_spanned(
                attribute,
                "For does not implement spread attributes",
            ));
        };
        match attribute.key.to_string().as_str() {
            "each" => each = Some(expression(attribute)?),
            "key" => key = Some(expression(attribute)?),
            other => {
                return Err(syn::Error::new_spanned(
                    &attribute.key,
                    format!("unsupported For attribute `{other}`"),
                ));
            }
        }
    }
    let each = each.ok_or_else(|| syn::Error::new_spanned(element.name(), "For needs `each`"))?;
    let key = key.ok_or_else(|| syn::Error::new_spanned(element.name(), "For needs `key`"))?;
    if element.children.len() != 1 {
        return Err(syn::Error::new_spanned(
            element.name(),
            "For expects one row factory child",
        ));
    }
    let Node::Block(NodeBlock::ValidBlock(block)) = &element.children[0] else {
        return Err(syn::Error::new_spanned(
            &element.children[0],
            "For expects one row factory closure",
        ));
    };
    let Some(Expr::Closure(_)) = single_expression(block)? else {
        return Err(syn::Error::new_spanned(
            block,
            "For expects one row factory closure",
        ));
    };
    Ok(quote_spanned! {element.span()=>
        ::zenui::view::for_each(#each, #key, #block)
    })
}

fn children_plan(element: &NodeElement<rstml::Infallible>) -> syn::Result<TokenStream> {
    let nodes = element
        .children
        .iter()
        .map(lower_node)
        .collect::<syn::Result<Vec<_>>>()?;
    Ok(quote! {
        move || ::core::result::Result::Ok(
            ::zenui::view::MountPlan::new(::std::vec![#(#nodes),*])
        )
    })
}

fn expand_component(item: TokenStream) -> syn::Result<TokenStream> {
    let function: ItemFn = syn::parse2(item)?;
    if !function.sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &function.sig.generics,
            "#[component] does not support generic parameters",
        ));
    }
    if let ReturnType::Type(_, ty) = &function.sig.output
        && !matches!(&**ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "MountResult"))
    {
        return Err(syn::Error::new_spanned(
            ty,
            "#[component] expects a MountResult<MountPlan> return type",
        ));
    }

    let name = &function.sig.ident;
    let visibility = &function.vis;
    let builder_name = format_ident!("{name}Builder");
    let render_name = format_ident!("__zenui_render_{name}");

    let mut fields = Vec::new();
    let mut defaults = Vec::new();
    let mut setters = Vec::new();
    let mut arguments = Vec::new();
    let mut parameters = Vec::new();
    for input in &function.sig.inputs {
        let FnArg::Typed(argument) = input else {
            return Err(syn::Error::new_spanned(
                input,
                "#[component] does not accept a receiver",
            ));
        };
        let Pat::Ident(pattern) = &*argument.pat else {
            return Err(syn::Error::new_spanned(
                &argument.pat,
                "#[component] parameters must be identifiers",
            ));
        };
        let field = format_ident!("{}", pattern.ident);
        let field_type = builder_type(&argument.ty)?;
        fields.push(quote! { #field: ::core::option::Option<#field_type> });
        defaults.push(quote! { #field: ::core::option::Option::None });
        setters.push(quote! {
            pub fn #field(mut self, value: impl ::core::convert::Into<#field_type>) -> Self {
                self.#field = ::core::option::Option::Some(value.into());
                self
            }
        });
        parameters.push(quote! { #argument });
        arguments.push(quote! {
            self.#field.ok_or_else(|| ::zenui::MountError::missing_prop(stringify!(#field)))?
        });
    }

    let body = &function.block;
    Ok(quote! {
        #visibility struct #name;

        #[allow(non_camel_case_types)]
        #visibility struct #builder_name {
            #(#fields),*
        }

        impl #name {
            pub fn builder() -> #builder_name {
                #builder_name { #(#defaults),* }
            }
        }

        impl #builder_name {
            #(#setters)*

            pub fn mount(self) -> ::zenui::MountResult<::zenui::MountPlan> {
                #render_name(#(#arguments),*)
            }
        }

        fn #render_name(#(#parameters),*) -> ::zenui::MountResult<::zenui::MountPlan> {
            #body
        }
    })
}

fn builder_type(ty: &Type) -> syn::Result<Type> {
    if let Type::ImplTrait(impl_trait) = ty {
        if impl_trait.bounds.len() == 1
            && let syn::TypeParamBound::Trait(bound) = &impl_trait.bounds[0]
            && bound
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "Into")
            && let syn::PathArguments::AngleBracketed(arguments) =
                &bound.path.segments.last().unwrap().arguments
            && arguments.args.len() == 1
            && let syn::GenericArgument::Type(inner) = &arguments.args[0]
        {
            return Ok(inner.clone());
        }
        return Err(syn::Error::new_spanned(
            ty,
            "#[component] supports only `impl Into<T>` parameters",
        ));
    }
    Ok(ty.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn unsupported_events_are_rejected() {
        let error = expand(quote! { <button on:paste={move || {}} /> }).unwrap_err();
        assert!(error.to_string().contains("unsupported event"));
    }

    #[test]
    fn focus_and_blur_events_are_supported() {
        expand(quote! {
            <button
                on:focus={move |_event: FocusEvent| {}}
                on:blur={move |_event: BlurEvent| {}}
            />
        })
        .expect("focus and blur lower to typed handlers");
    }

    #[test]
    fn composition_events_are_supported() {
        expand(quote! {
            <text-input
                on:compositionstart={move |_event: CompositionEvent| {}}
                on:compositionupdate={move |_event: CompositionEvent| {}}
                on:compositionend={move |_event: CompositionEvent| {}}
                on:compositioncancel={move |_event: CompositionEvent| {}}
            />
        })
        .expect("composition events lower to typed handlers");
    }

    #[test]
    fn show_requires_when() {
        let error = expand(quote! { <Show><text>"x"</text></Show> }).unwrap_err();
        assert!(error.to_string().contains("Show needs `when`"));
    }

    #[test]
    fn components_must_return_a_mount_result() {
        let error = expand_component(quote! { fn Bad() -> u8 { 0 } }).unwrap_err();
        assert!(error.to_string().contains("MountResult"));
    }
}
