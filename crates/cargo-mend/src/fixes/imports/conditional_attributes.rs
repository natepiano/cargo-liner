use proc_macro2::LineColumn;
use proc_macro2::Span;
use syn::Attribute;
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
