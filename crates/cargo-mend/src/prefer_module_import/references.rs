use std::collections::BTreeSet;

use syn::Expr;
use syn::ItemUse;
use syn::visit::Visit;

use super::shared;

pub(super) struct BareReference {
    pub(super) name:       String,
    pub(super) byte_start: usize,
    pub(super) byte_end:   usize,
}

pub(super) struct ReferenceCollector<'a> {
    pub(super) offsets:        &'a [usize],
    pub(super) imported_names: &'a BTreeSet<String>,
    pub(super) references:     Vec<BareReference>,
}

impl Visit<'_> for ReferenceCollector<'_> {
    fn visit_item_use(&mut self, _: &ItemUse) {}

    fn visit_expr(&mut self, node: &Expr) {
        match node {
            Expr::Path(expr_path) => {
                if expr_path.qself.is_none() && expr_path.path.segments.len() == 1 {
                    let segment = &expr_path.path.segments[0];
                    let name = segment.ident.to_string();
                    if self.imported_names.contains(&name) {
                        let span = segment.ident.span();
                        let start = shared::offset(self.offsets, span.start());
                        let end = shared::offset(self.offsets, span.end());
                        self.references.push(BareReference {
                            name,
                            byte_start: start,
                            byte_end: end,
                        });
                    }
                }
            },
            _ => syn::visit::visit_expr(self, node),
        }
    }

    fn visit_macro(&mut self, node: &syn::Macro) {
        collect_bare_refs_from_tokens(
            &node.tokens,
            self.offsets,
            self.imported_names,
            &mut self.references,
        );
        syn::visit::visit_macro(self, node);
    }
}

pub(super) fn collect_bare_refs_from_tokens(
    tokens: &proc_macro2::TokenStream,
    offsets: &[usize],
    imported_names: &BTreeSet<String>,
    references: &mut Vec<BareReference>,
) {
    let mut prev_colon_joint = false;
    let mut prev_is_colon_colon = false;
    for token_tree in tokens.clone() {
        match token_tree {
            proc_macro2::TokenTree::Ident(ref ident) => {
                let name = ident.to_string();
                if !prev_is_colon_colon && imported_names.contains(&name) {
                    let span = ident.span();
                    let start = shared::offset(offsets, span.start());
                    let end = shared::offset(offsets, span.end());
                    references.push(BareReference {
                        name,
                        byte_start: start,
                        byte_end: end,
                    });
                }
                prev_colon_joint = false;
                prev_is_colon_colon = false;
            },
            proc_macro2::TokenTree::Punct(ref punct) => {
                if punct.as_char() == ':' {
                    if prev_colon_joint {
                        prev_is_colon_colon = true;
                        prev_colon_joint = false;
                    } else if punct.spacing() == proc_macro2::Spacing::Joint {
                        prev_colon_joint = true;
                        prev_is_colon_colon = false;
                    } else {
                        prev_colon_joint = false;
                        prev_is_colon_colon = false;
                    }
                } else {
                    prev_colon_joint = false;
                    prev_is_colon_colon = false;
                }
            },
            proc_macro2::TokenTree::Group(ref group) => {
                collect_bare_refs_from_tokens(&group.stream(), offsets, imported_names, references);
                prev_is_colon_colon = false;
            },
            proc_macro2::TokenTree::Literal(_) => {
                prev_colon_joint = false;
                prev_is_colon_colon = false;
            },
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::BTreeSet;

    use super::BareReference;
    use super::collect_bare_refs_from_tokens;
    use super::shared;

    #[test]
    fn collect_bare_refs_finds_ident_in_macro_tokens() {
        let src = r"matches!(do_thing(x), MyEnum::Variant)";
        let offsets = shared::line_offsets(src);
        let mut names = BTreeSet::new();
        names.insert("do_thing".to_string());
        let tokens: proc_macro2::TokenStream = src.parse().expect("parse tokens");
        let mut refs: Vec<BareReference> = Vec::new();
        collect_bare_refs_from_tokens(&tokens, &offsets, &names, &mut refs);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "do_thing");
        assert_eq!(&src[refs[0].byte_start..refs[0].byte_end], "do_thing");
    }

    #[test]
    fn collect_bare_refs_skips_qualified_ident_in_macro_tokens() {
        let src = r"matches!(module::do_thing(x), MyEnum::Variant)";
        let offsets = shared::line_offsets(src);
        let mut names = BTreeSet::new();
        names.insert("do_thing".to_string());
        let tokens: proc_macro2::TokenStream = src.parse().expect("parse tokens");
        let mut refs: Vec<BareReference> = Vec::new();
        collect_bare_refs_from_tokens(&tokens, &offsets, &names, &mut refs);
        assert!(refs.is_empty(), "qualified path should not match");
    }

    #[test]
    fn collect_bare_refs_finds_nested_in_group() {
        let src = r"assert!(do_thing(foo(bar())))";
        let offsets = shared::line_offsets(src);
        let mut names = BTreeSet::new();
        names.insert("do_thing".to_string());
        let tokens: proc_macro2::TokenStream = src.parse().expect("parse tokens");
        let mut refs: Vec<BareReference> = Vec::new();
        collect_bare_refs_from_tokens(&tokens, &offsets, &names, &mut refs);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "do_thing");
    }
}
