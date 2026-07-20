use proc_macro2::LineColumn;
use syn::Attribute;
use syn::spanned::Spanned;

#[derive(Clone, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct ConditionalAttributes {
    source: Vec<String>,
}

impl ConditionalAttributes {
    pub fn from_attributes(text: &str, line_offsets: &[usize], attributes: &[Attribute]) -> Self {
        let source = attributes
            .iter()
            .filter(|attribute| is_conditional(attribute))
            .map(|attribute| {
                let span = attribute.span();
                let start = byte_offset(text, line_offsets, span.start());
                let end = byte_offset(text, line_offsets, span.end());
                text[start..end].to_string()
            })
            .collect();
        Self { source }
    }

    pub fn contains(attributes: &[Attribute]) -> bool { attributes.iter().any(is_conditional) }

    pub fn extend(&mut self, other: Self) { self.source.extend(other.source); }

    pub const fn is_empty(&self) -> bool { self.source.is_empty() }

    pub const fn len(&self) -> usize { self.source.len() }

    pub fn render(&self, indent: &str) -> String {
        let mut rendered = String::new();
        for attribute in &self.source {
            rendered.push_str(indent);
            rendered.push_str(attribute);
            rendered.push('\n');
        }
        rendered
    }

    pub fn truncate(&mut self, len: usize) { self.source.truncate(len); }
}

pub fn is_conditional(attribute: &Attribute) -> bool {
    attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr")
}

fn byte_offset(text: &str, line_offsets: &[usize], position: LineColumn) -> usize {
    line_offsets
        .get(position.line.saturating_sub(1))
        .copied()
        .unwrap_or(text.len())
        .saturating_add(position.column)
        .min(text.len())
}
