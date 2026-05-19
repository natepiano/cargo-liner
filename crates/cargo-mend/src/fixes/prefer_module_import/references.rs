use std::collections::BTreeSet;

use proc_macro2::Spacing;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use syn::Arm;
use syn::Block;
use syn::Expr;
use syn::ExprClosure;
use syn::ExprForLoop;
use syn::FieldValue;
use syn::FnArg;
use syn::ImplItemFn;
use syn::ItemFn;
use syn::ItemUse;
use syn::Local;
use syn::Macro;
use syn::Pat;
use syn::TraitItemFn;
use syn::visit;
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
    pub(super) scopes:         Vec<BTreeSet<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviousToken {
    Other,
    JointColon,
    ColonColon,
}

impl PreviousToken {
    const fn allows_bare_reference(self) -> bool { !matches!(self, Self::ColonColon) }

    const fn after_colon(self, spacing: Spacing) -> Self {
        match self {
            Self::JointColon => Self::ColonColon,
            _ if matches!(spacing, Spacing::Joint) => Self::JointColon,
            _ => Self::Other,
        }
    }

    const fn after_group(self) -> Self {
        match self {
            Self::ColonColon => Self::Other,
            other => other,
        }
    }
}

impl<'a> ReferenceCollector<'a> {
    pub(super) fn new(offsets: &'a [usize], imported_names: &'a BTreeSet<String>) -> Self {
        Self {
            offsets,
            imported_names,
            references: Vec::new(),
            scopes: vec![BTreeSet::new()],
        }
    }

    fn is_shadowed(&self, name: &str) -> bool {
        self.scopes.iter().any(|scope| scope.contains(name))
    }

    fn enter_scope_with(&mut self, bindings: BTreeSet<String>) { self.scopes.push(bindings); }

    fn enter_scope(&mut self) { self.scopes.push(BTreeSet::new()); }

    fn exit_scope(&mut self) { self.scopes.pop(); }
}

impl Visit<'_> for ReferenceCollector<'_> {
    fn visit_item_use(&mut self, _: &ItemUse) {}

    fn visit_block(&mut self, block: &Block) {
        self.enter_scope();
        visit::visit_block(self, block);
        self.exit_scope();
    }

    fn visit_local(&mut self, local: &Local) {
        for attr in &local.attrs {
            self.visit_attribute(attr);
        }
        if let Some(init) = &local.init {
            self.visit_expr(&init.expr);
            if let Some((_, diverge)) = &init.diverge {
                self.visit_expr(diverge);
            }
        }
        let mut bindings = BTreeSet::new();
        collect_pat_bindings(&local.pat, &mut bindings);
        if let Some(scope) = self.scopes.last_mut() {
            scope.extend(bindings);
        }
    }

    fn visit_item_fn(&mut self, item: &ItemFn) {
        for attr in &item.attrs {
            self.visit_attribute(attr);
        }
        let mut params = BTreeSet::new();
        collect_fn_param_bindings(item.sig.inputs.iter(), &mut params);
        self.enter_scope_with(params);
        visit::visit_block(self, &item.block);
        self.exit_scope();
    }

    fn visit_impl_item_fn(&mut self, item: &ImplItemFn) {
        for attr in &item.attrs {
            self.visit_attribute(attr);
        }
        let mut params = BTreeSet::new();
        collect_fn_param_bindings(item.sig.inputs.iter(), &mut params);
        self.enter_scope_with(params);
        visit::visit_block(self, &item.block);
        self.exit_scope();
    }

    fn visit_trait_item_fn(&mut self, item: &TraitItemFn) {
        for attr in &item.attrs {
            self.visit_attribute(attr);
        }
        if let Some(body) = &item.default {
            let mut params = BTreeSet::new();
            collect_fn_param_bindings(item.sig.inputs.iter(), &mut params);
            self.enter_scope_with(params);
            visit::visit_block(self, body);
            self.exit_scope();
        }
    }

    fn visit_expr_closure(&mut self, closure: &ExprClosure) {
        for attr in &closure.attrs {
            self.visit_attribute(attr);
        }
        let mut params = BTreeSet::new();
        for input in &closure.inputs {
            collect_pat_bindings(input, &mut params);
        }
        self.enter_scope_with(params);
        self.visit_expr(&closure.body);
        self.exit_scope();
    }

    fn visit_expr_for_loop(&mut self, for_loop: &ExprForLoop) {
        for attr in &for_loop.attrs {
            self.visit_attribute(attr);
        }
        if let Some(label) = &for_loop.label {
            self.visit_label(label);
        }
        self.visit_expr(&for_loop.expr);
        let mut bindings = BTreeSet::new();
        collect_pat_bindings(&for_loop.pat, &mut bindings);
        self.enter_scope_with(bindings);
        visit::visit_block(self, &for_loop.body);
        self.exit_scope();
    }

    fn visit_arm(&mut self, arm: &Arm) {
        for attr in &arm.attrs {
            self.visit_attribute(attr);
        }
        let mut bindings = BTreeSet::new();
        collect_pat_bindings(&arm.pat, &mut bindings);
        self.enter_scope_with(bindings);
        if let Some((_, guard)) = &arm.guard {
            self.visit_expr(guard);
        }
        self.visit_expr(&arm.body);
        self.exit_scope();
    }

    fn visit_field_value(&mut self, field_value: &FieldValue) {
        for attr in &field_value.attrs {
            self.visit_attribute(attr);
        }
        if field_value.colon_token.is_none() {
            // Struct literal field shorthand `Foo { name }`. The expression
            // is required to be a bare ident matching `name`; replacing it
            // with `module::name` produces a parse error. Either way the
            // value resolves to a local binding (otherwise the expansion
            // `name: name` wouldn't compile), so leave the bare ident
            // alone.
            return;
        }
        self.visit_expr(&field_value.expr);
    }

    fn visit_expr(&mut self, node: &Expr) {
        match node {
            Expr::Path(expr_path) => {
                if expr_path.qself.is_none() && expr_path.path.segments.len() == 1 {
                    let segment = &expr_path.path.segments[0];
                    let name = segment.ident.to_string();
                    if self.imported_names.contains(&name) && !self.is_shadowed(&name) {
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
            _ => visit::visit_expr(self, node),
        }
    }

    fn visit_macro(&mut self, node: &Macro) {
        collect_bare_refs_from_tokens(
            &node.tokens,
            self.offsets,
            self.imported_names,
            &mut self.references,
        );
        visit::visit_macro(self, node);
    }
}

fn collect_pat_bindings(pat: &Pat, bindings: &mut BTreeSet<String>) {
    match pat {
        Pat::Ident(pat_ident) => {
            bindings.insert(pat_ident.ident.to_string());
            if let Some((_, sub)) = &pat_ident.subpat {
                collect_pat_bindings(sub, bindings);
            }
        },
        Pat::Tuple(tuple) => {
            for elem in &tuple.elems {
                collect_pat_bindings(elem, bindings);
            }
        },
        Pat::TupleStruct(tuple_struct) => {
            for elem in &tuple_struct.elems {
                collect_pat_bindings(elem, bindings);
            }
        },
        Pat::Struct(pat_struct) => {
            for field in &pat_struct.fields {
                collect_pat_bindings(&field.pat, bindings);
            }
        },
        Pat::Or(pat_or) => {
            for case in &pat_or.cases {
                collect_pat_bindings(case, bindings);
            }
        },
        Pat::Reference(pat_ref) => collect_pat_bindings(&pat_ref.pat, bindings),
        Pat::Slice(slice) => {
            for elem in &slice.elems {
                collect_pat_bindings(elem, bindings);
            }
        },
        Pat::Type(pat_type) => collect_pat_bindings(&pat_type.pat, bindings),
        Pat::Paren(paren) => collect_pat_bindings(&paren.pat, bindings),
        _ => {},
    }
}

fn collect_fn_param_bindings<'a>(
    inputs: impl IntoIterator<Item = &'a FnArg>,
    bindings: &mut BTreeSet<String>,
) {
    for input in inputs {
        if let FnArg::Typed(pat_type) = input {
            collect_pat_bindings(&pat_type.pat, bindings);
        }
    }
}

pub(super) fn collect_bare_refs_from_tokens(
    tokens: &TokenStream,
    offsets: &[usize],
    imported_names: &BTreeSet<String>,
    references: &mut Vec<BareReference>,
) {
    let mut previous_token = PreviousToken::Other;
    for token_tree in tokens.clone() {
        match token_tree {
            TokenTree::Ident(ref ident) => {
                let name = ident.to_string();
                if previous_token.allows_bare_reference() && imported_names.contains(&name) {
                    let span = ident.span();
                    let start = shared::offset(offsets, span.start());
                    let end = shared::offset(offsets, span.end());
                    references.push(BareReference {
                        name,
                        byte_start: start,
                        byte_end: end,
                    });
                }
                previous_token = PreviousToken::Other;
            },
            TokenTree::Punct(ref punct) => {
                if punct.as_char() == ':' {
                    previous_token = previous_token.after_colon(punct.spacing());
                } else {
                    previous_token = PreviousToken::Other;
                }
            },
            TokenTree::Group(ref group) => {
                collect_bare_refs_from_tokens(&group.stream(), offsets, imported_names, references);
                previous_token = previous_token.after_group();
            },
            TokenTree::Literal(_) => {
                previous_token = PreviousToken::Other;
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

    use proc_macro2::TokenStream;

    use super::BareReference;
    use super::collect_bare_refs_from_tokens;
    use super::shared;

    #[test]
    fn collect_bare_refs_finds_ident_in_macro_tokens() {
        let src = r"matches!(do_thing(x), MyEnum::Variant)";
        let offsets = shared::line_offsets(src);
        let mut names = BTreeSet::new();
        names.insert("do_thing".to_string());
        let tokens: TokenStream = src.parse().expect("parse tokens");
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
        let tokens: TokenStream = src.parse().expect("parse tokens");
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
        let tokens: TokenStream = src.parse().expect("parse tokens");
        let mut refs: Vec<BareReference> = Vec::new();
        collect_bare_refs_from_tokens(&tokens, &offsets, &names, &mut refs);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "do_thing");
    }
}
