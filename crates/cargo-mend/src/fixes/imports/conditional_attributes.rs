use std::collections::BTreeSet;

use proc_macro2::LineColumn;
use proc_macro2::Span;
use syn::Attribute;
use syn::Expr;
use syn::Item;
use syn::Meta;
use syn::Token;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;

#[derive(Clone, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::fixes) struct ConditionalAttributes {
    source: Vec<String>,
}

impl ConditionalAttributes {
    pub(in crate::fixes) fn from_attributes(
        text: &str,
        line_offsets: &[usize],
        attributes: &[Attribute],
    ) -> Self {
        let source = attributes
            .iter()
            .filter_map(|attribute| conditional_attribute_source(text, line_offsets, attribute))
            .collect();
        Self { source }
    }

    pub(in crate::fixes) fn contains(attributes: &[Attribute]) -> bool {
        attributes.iter().any(is_conditional)
    }

    /// The gate a synthesized import must carry to cover every occurrence in
    /// `candidates`. One ungated occurrence forces an ungated import; a single
    /// agreed gate is reproduced verbatim; gates that disagree yield `None`,
    /// leaving the caller to skip the rewrite rather than insert an import
    /// that resolves in only some configurations.
    pub(in crate::fixes) fn covering(candidates: &BTreeSet<Self>) -> Option<Self> {
        let ungated = Self::default();
        if candidates.contains(&ungated) {
            return Some(ungated);
        }
        if candidates.len() == 1 {
            return candidates.first().cloned();
        }
        None
    }

    pub(in crate::fixes) fn extend(&mut self, other: Self) { self.source.extend(other.source); }

    /// True when every gate in `self` also appears in `other` — an item gated
    /// by `self` is compiled in every configuration where a site gated by
    /// `other` is. Gates are compared by source text, so differently spelled
    /// but equivalent predicates conservatively compare unequal.
    pub(in crate::fixes) fn is_subset_of(&self, other: &Self) -> bool {
        self.source
            .iter()
            .all(|attribute| other.source.contains(attribute))
    }

    pub(in crate::fixes) const fn is_empty(&self) -> bool { self.source.is_empty() }

    pub(in crate::fixes) const fn len(&self) -> usize { self.source.len() }

    pub(in crate::fixes) fn render(&self, indent: &str) -> String {
        let mut rendered = String::new();
        for attribute in &self.source {
            rendered.push_str(indent);
            rendered.push_str(attribute);
            rendered.push('\n');
        }
        rendered
    }

    pub(in crate::fixes) fn truncate(&mut self, len: usize) { self.source.truncate(len); }
}

pub(in crate::fixes) fn is_conditional(attribute: &Attribute) -> bool {
    attribute.path().is_ident("cfg") || cfg_attr_applies_cfg(attribute)
}

/// Attributes written on a statement attach to the statement's expression:
/// `#[cfg(test)] app.init_resource::<Injected>();` parses as a `Stmt::Expr`
/// holding an `ExprMethodCall` whose `attrs` carry the `cfg`. `syn::Expr`
/// publishes no accessor for those, so every variant that owns an `attrs`
/// field is named here. `Expr` is `#[non_exhaustive]`; a variant syn adds
/// later reports no gates, which keeps the enclosing occurrence fully
/// qualified rather than importing it under the wrong `cfg`.
pub(in crate::fixes) fn expr_attributes(expr: &Expr) -> &[Attribute] {
    match expr {
        Expr::Array(node) => &node.attrs,
        Expr::Assign(node) => &node.attrs,
        Expr::Async(node) => &node.attrs,
        Expr::Await(node) => &node.attrs,
        Expr::Binary(node) => &node.attrs,
        Expr::Block(node) => &node.attrs,
        Expr::Break(node) => &node.attrs,
        Expr::Call(node) => &node.attrs,
        Expr::Cast(node) => &node.attrs,
        Expr::Closure(node) => &node.attrs,
        Expr::Const(node) => &node.attrs,
        Expr::Continue(node) => &node.attrs,
        Expr::Field(node) => &node.attrs,
        Expr::ForLoop(node) => &node.attrs,
        Expr::Group(node) => &node.attrs,
        Expr::If(node) => &node.attrs,
        Expr::Index(node) => &node.attrs,
        Expr::Infer(node) => &node.attrs,
        Expr::Let(node) => &node.attrs,
        Expr::Lit(node) => &node.attrs,
        Expr::Loop(node) => &node.attrs,
        Expr::Macro(node) => &node.attrs,
        Expr::Match(node) => &node.attrs,
        Expr::MethodCall(node) => &node.attrs,
        Expr::Paren(node) => &node.attrs,
        Expr::Path(node) => &node.attrs,
        Expr::Range(node) => &node.attrs,
        Expr::RawAddr(node) => &node.attrs,
        Expr::Reference(node) => &node.attrs,
        Expr::Repeat(node) => &node.attrs,
        Expr::Return(node) => &node.attrs,
        Expr::Struct(node) => &node.attrs,
        Expr::Try(node) => &node.attrs,
        Expr::TryBlock(node) => &node.attrs,
        Expr::Tuple(node) => &node.attrs,
        Expr::Unary(node) => &node.attrs,
        Expr::Unsafe(node) => &node.attrs,
        Expr::While(node) => &node.attrs,
        Expr::Yield(node) => &node.attrs,
        _ => &[],
    }
}

/// Attributes on an item, for the same reason `expr_attributes` exists:
/// `syn::Item` publishes no accessor and is `#[non_exhaustive]`.
pub(in crate::fixes) fn item_attributes(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(node) => &node.attrs,
        Item::Enum(node) => &node.attrs,
        Item::ExternCrate(node) => &node.attrs,
        Item::Fn(node) => &node.attrs,
        Item::ForeignMod(node) => &node.attrs,
        Item::Impl(node) => &node.attrs,
        Item::Macro(node) => &node.attrs,
        Item::Mod(node) => &node.attrs,
        Item::Static(node) => &node.attrs,
        Item::Struct(node) => &node.attrs,
        Item::Trait(node) => &node.attrs,
        Item::TraitAlias(node) => &node.attrs,
        Item::Type(node) => &node.attrs,
        Item::Union(node) => &node.attrs,
        Item::Use(node) => &node.attrs,
        _ => &[],
    }
}

fn cfg_attr_applies_cfg(attribute: &Attribute) -> bool {
    cfg_attr_meta_applies_cfg(&attribute.meta)
}

fn cfg_attr_meta_applies_cfg(meta: &Meta) -> bool {
    if !meta.path().is_ident("cfg_attr") {
        return false;
    }
    let Meta::List(list) = meta else {
        return false;
    };
    list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        .is_ok_and(|metas| metas.iter().skip(1).any(meta_applies_cfg))
}

fn meta_applies_cfg(meta: &Meta) -> bool {
    meta.path().is_ident("cfg") || cfg_attr_meta_applies_cfg(meta)
}

fn conditional_attribute_source(
    text: &str,
    line_offsets: &[usize],
    attribute: &Attribute,
) -> Option<String> {
    if attribute.path().is_ident("cfg") {
        return Some(source_for_span(text, line_offsets, attribute.span()).to_string());
    }
    gating_meta_source(text, line_offsets, &attribute.meta).map(|meta| format!("#[{meta}]"))
}

fn gating_meta_source(text: &str, line_offsets: &[usize], meta: &Meta) -> Option<String> {
    if meta.path().is_ident("cfg") {
        return Some(source_for_span(text, line_offsets, meta.span()).to_string());
    }
    if !meta.path().is_ident("cfg_attr") {
        return None;
    }
    let Meta::List(list) = meta else {
        return None;
    };
    let metas = list
        .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        .ok()?;
    let predicate = metas.first()?;
    let gating_attributes = metas
        .iter()
        .skip(1)
        .filter_map(|nested| gating_meta_source(text, line_offsets, nested))
        .collect::<Vec<_>>();
    if gating_attributes.is_empty() {
        return None;
    }
    Some(format!(
        "cfg_attr({}, {})",
        source_for_span(text, line_offsets, predicate.span()),
        gating_attributes.join(", ")
    ))
}

fn source_for_span<'a>(text: &'a str, line_offsets: &[usize], span: Span) -> &'a str {
    let start = byte_offset(text, line_offsets, span.start());
    let end = byte_offset(text, line_offsets, span.end());
    &text[start..end]
}

fn byte_offset(text: &str, line_offsets: &[usize], position: LineColumn) -> usize {
    line_offsets
        .get(position.line.saturating_sub(1))
        .copied()
        .unwrap_or(text.len())
        .saturating_add(position.column)
        .min(text.len())
}
