use std::collections::BTreeSet;

use proc_macro2::Delimiter;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use quote::ToTokens;
use syn::Attribute;
use syn::File;
use syn::LitStr;
use syn::Macro;
use syn::MacroDelimiter;
use syn::visit::Visit;

/// Every name whose import a prefer-module-import rewrite must keep.
///
/// The rewrite renames only the references it can see as paths. Attribute
/// payloads and macro bodies are opaque token streams that the attribute or the
/// macro gives meaning to. Serde names a function with a string, as in
/// `#[serde(default = "default_scale")]`. A macro body can use a name as a struct
/// field, where `module::device` is not valid syntax. If the rewrite deletes the
/// import, the name is no longer in scope (E0425). If it qualifies the token, the
/// code can stop parsing. The caller keeps these imports.
pub(super) fn collect(syntax: &File) -> BTreeSet<String> {
    let mut collector = PreservedImports {
        names: BTreeSet::new(),
    };
    collector.visit_file(syntax);
    collector.names
}

struct PreservedImports {
    names: BTreeSet<String>,
}

impl Visit<'_> for PreservedImports {
    fn visit_attribute(&mut self, node: &Attribute) {
        // Doc comments desugar to `#[doc = "..."]`. A doc comment that is one word
        // is prose, not a reference. If it counted, most documented files would
        // keep every import.
        if node.path().is_ident("doc") {
            return;
        }
        collect_from_tokens(&node.meta.to_token_stream(), &mut self.names);
    }

    fn visit_macro(&mut self, node: &Macro) {
        if let Some(name) = node.path.get_ident() {
            self.names.insert(name.to_string());
        }
        if matches!(node.delimiter, MacroDelimiter::Brace(_)) {
            collect_from_tokens(&node.tokens, &mut self.names);
        } else {
            self.collect_braced_names(&node.tokens);
        }
    }
}

impl PreservedImports {
    fn collect_braced_names(&mut self, tokens: &TokenStream) {
        for token in tokens.clone() {
            if let TokenTree::Group(group) = token {
                if group.delimiter() == Delimiter::Brace {
                    collect_from_tokens(&group.stream(), &mut self.names);
                } else {
                    self.collect_braced_names(&group.stream());
                }
            }
        }
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
/// function import, so the rewrite cannot break it.
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
