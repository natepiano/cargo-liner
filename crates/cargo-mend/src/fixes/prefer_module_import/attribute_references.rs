use std::collections::BTreeSet;

use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use quote::ToTokens;
use syn::Attribute;
use syn::File;
use syn::LitStr;
use syn::visit::Visit;

/// Every name the file's attributes mention, either as a bare ident or as a
/// string literal holding a single identifier.
///
/// A prefer-module-import rewrite has to rename every reference to the imported
/// function, and it can only rename references it can see as paths. Attribute
/// payloads are neither: syn exposes `Meta::List` contents as an opaque token
/// stream that no visitor descends into, and macros like serde name a function
/// with a *string* — `#[serde(default = "default_scale")]`. Deleting
/// `use super::defaults::default_scale;` leaves that string naming something
/// that is no longer in scope (E0425).
///
/// Qualifying the mention instead is not an option: the attribute macro decides
/// what its own tokens mean, so a string that reads as an identifier is only
/// sometimes a path. The caller drops matching candidates and leaves the import
/// alone.
pub(super) fn collect(syntax: &File) -> BTreeSet<String> {
    let mut collector = AttributeNames {
        names: BTreeSet::new(),
    };
    collector.visit_file(syntax);
    collector.names
}

struct AttributeNames {
    names: BTreeSet<String>,
}

impl Visit<'_> for AttributeNames {
    fn visit_attribute(&mut self, node: &Attribute) {
        // Doc comments desugar to `#[doc = "..."]`. Prose that happens to be a
        // single word is not a reference, and treating it as one would silence
        // the lint across most documented files.
        if node.path().is_ident("doc") {
            return;
        }
        collect_from_tokens(&node.meta.to_token_stream(), &mut self.names);
    }
}

fn collect_from_tokens(tokens: &TokenStream, names: &mut BTreeSet<String>) {
    for token in tokens.clone() {
        match token {
            TokenTree::Ident(ident) => {
                names.insert(ident.to_string());
            },
            TokenTree::Literal(literal) => {
                if let Ok(text) = syn::parse_str::<LitStr>(&literal.to_string()) {
                    let value = text.value();
                    if is_plain_ident(&value) {
                        names.insert(value);
                    }
                }
            },
            TokenTree::Group(group) => collect_from_tokens(&group.stream(), names),
            TokenTree::Punct(_) => {},
        }
    }
}

/// True for a string that is exactly one identifier. A qualified string such as
/// `"defaults::default_scale"` resolves through the module, not through the
/// function import, so a prefer-module-import rewrite cannot break it.
fn is_plain_ident(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use syn::parse_file;

    use super::collect;

    fn names(source: &str) -> Vec<String> {
        let syntax = parse_file(source).expect("parse fixture");
        collect(&syntax).into_iter().collect()
    }

    #[test]
    fn collects_single_ident_string_literal() {
        let found = names(
            "struct Thing {\n    #[serde(default = \"default_scale\")]\n    scale: \
                           f64,\n}\n",
        );
        assert!(
            found.iter().any(|name| name == "default_scale"),
            "serde string path should be collected, got: {found:?}"
        );
    }

    #[test]
    fn collects_bare_ident_in_attribute_tokens() {
        let found = names(
            "struct Thing {\n    #[arg(default_value_t = default_scale())]\n    \
                           scale: f64,\n}\n",
        );
        assert!(
            found.iter().any(|name| name == "default_scale"),
            "bare ident in attribute tokens should be collected, got: {found:?}"
        );
    }

    #[test]
    fn skips_qualified_string_literal() {
        let found = names(
            "struct Thing {\n    #[serde(default = \
                           \"defaults::default_scale\")]\n    scale: f64,\n}\n",
        );
        assert!(
            !found.iter().any(|name| name == "default_scale"),
            "qualified string resolves through the module, got: {found:?}"
        );
    }

    #[test]
    fn skips_doc_attributes() {
        let found = names("/// default_scale\nfn documented() {}\n");
        assert!(
            !found.iter().any(|name| name == "default_scale"),
            "doc prose is not a reference, got: {found:?}"
        );
    }

    #[test]
    fn collects_from_nested_attribute_groups() {
        let found = names(
            "#[cfg_attr(feature = \"serde\", serde(default = \"default_scale\"))]\nstruct \
             Thing;\n",
        );
        assert!(
            found.iter().any(|name| name == "default_scale"),
            "nested attribute group should be walked, got: {found:?}"
        );
    }
}
