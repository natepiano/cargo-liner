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
use syn::Ident;
use syn::ImplItemFn;
use syn::Item;
use syn::ItemFn;
use syn::ItemMod;
use syn::ItemUse;
use syn::Local;
use syn::Macro;
use syn::Pat;
use syn::Stmt;
use syn::TraitItemFn;
use syn::visit;
use syn::visit::Visit;

use super::support;

pub(super) struct BareReference {
    pub(super) name:             String,
    pub(super) byte_start:       usize,
    pub(super) byte_end:         usize,
    /// Inline `mod` nesting at the reference site. Each level pushes `super`
    /// one module further away, so a parent-module rewrite needs
    /// `inline_mod_depth + 1` `super` segments to reach the file's parent.
    pub(super) inline_mod_depth: usize,
}

pub(super) struct ReferenceCollector<'a> {
    pub(super) offsets:          &'a [usize],
    pub(super) imported_names:   &'a BTreeSet<String>,
    pub(super) references:       Vec<BareReference>,
    pub(super) scopes:           Vec<BTreeSet<String>>,
    pub(super) inline_mod_depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviousToken {
    Other,
    JointColon,
    ColonColon,
    JointDot,
    Dot,
}

impl PreviousToken {
    const fn allows_bare_reference(self) -> bool { !matches!(self, Self::ColonColon | Self::Dot) }

    const fn after_colon(self, spacing: Spacing) -> Self {
        match self {
            Self::JointColon => Self::ColonColon,
            _ if matches!(spacing, Spacing::Joint) => Self::JointColon,
            _ => Self::Other,
        }
    }

    /// A single `.` introduces a field access or a method name — `progress.normalized()`
    /// names an inherent method, never a path root, so rewriting the ident to
    /// `ledger::normalized` produces `progress.ledger::normalized()`, which the
    /// macro matcher rejects with "no rules expected `::`". `..` and `..=` are
    /// range operators whose endpoint *is* a value path, so they must keep
    /// allowing the ident that follows.
    const fn after_dot(self, spacing: Spacing) -> Self {
        match self {
            Self::JointDot => Self::Other,
            _ if matches!(spacing, Spacing::Joint) => Self::JointDot,
            _ => Self::Dot,
        }
    }

    const fn after_group(self) -> Self {
        match self {
            Self::ColonColon | Self::Dot => Self::Other,
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
            inline_mod_depth: 0,
        }
    }

    fn is_shadowed(&self, name: &str) -> bool {
        self.scopes.iter().any(|scope| scope.contains(name))
    }

    fn enter_scope_with(&mut self, bindings: BTreeSet<String>) { self.scopes.push(bindings); }

    fn exit_scope(&mut self) { self.scopes.pop(); }
}

impl Visit<'_> for ReferenceCollector<'_> {
    fn visit_item_use(&mut self, _: &ItemUse) {}

    fn visit_item_mod(&mut self, node: &ItemMod) {
        let Some((_, items)) = &node.content else {
            visit::visit_item_mod(self, node);
            return;
        };
        // An inline `mod` does not inherit the file's imports: a bare name
        // inside it resolves to the module's own items and `use` bindings.
        // `#[cfg(test)] mod tests { fn reconcile(..) {..} }` next to a
        // file-scope `use crate::reconcile::reconcile;` is the case that
        // matters — rewriting the call to `reconcile::reconcile()` resolves
        // to the local fn, not a module (E0433).
        self.inline_mod_depth += 1;
        self.enter_scope_with(item_bindings(items));
        visit::visit_item_mod(self, node);
        self.exit_scope();
        self.inline_mod_depth -= 1;
    }

    fn visit_block(&mut self, block: &Block) {
        self.enter_scope_with(block_item_bindings(block));
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
        self.visit_block(&item.block);
        self.exit_scope();
    }

    fn visit_impl_item_fn(&mut self, item: &ImplItemFn) {
        for attr in &item.attrs {
            self.visit_attribute(attr);
        }
        let mut params = BTreeSet::new();
        collect_fn_param_bindings(item.sig.inputs.iter(), &mut params);
        self.enter_scope_with(params);
        self.visit_block(&item.block);
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
            self.visit_block(body);
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
        self.visit_block(&for_loop.body);
        self.exit_scope();
    }

    fn visit_arm(&mut self, arm: &Arm) {
        for attr in &arm.attrs {
            self.visit_attribute(attr);
        }
        let mut bindings = BTreeSet::new();
        collect_pat_bindings(&arm.pat, &mut bindings);
        self.enter_scope_with(bindings);
        if let Pat::Guard(pat_guard) = &arm.pat {
            self.visit_expr(&pat_guard.guard);
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
                        let start = support::offset(self.offsets, span.start());
                        let end = support::offset(self.offsets, span.end());
                        self.references.push(BareReference {
                            name,
                            byte_start: start,
                            byte_end: end,
                            inline_mod_depth: self.inline_mod_depth,
                        });
                    }
                }
            },
            _ => visit::visit_expr(self, node),
        }
    }

    fn visit_macro(&mut self, node: &Macro) {
        // The token scanner sees idents, not scopes, so filter its hits
        // against the bindings in scope at the macro call site.
        let mut collected = Vec::new();
        collect_bare_refs_from_tokens(
            &node.tokens,
            self.offsets,
            self.imported_names,
            self.inline_mod_depth,
            &mut collected,
        );
        collected.retain(|reference| !self.is_shadowed(&reference.name));
        self.references.append(&mut collected);
        visit::visit_macro(self, node);
    }
}

/// Names the items in an inline `mod` body bind in that module.
fn item_bindings(items: &[Item]) -> BTreeSet<String> {
    let mut bindings = BTreeSet::new();
    for item in items {
        collect_item_bindings(item, &mut bindings);
    }
    bindings
}

/// Names the items declared directly in `block` bind in it.
fn block_item_bindings(block: &Block) -> BTreeSet<String> {
    let mut bindings = BTreeSet::new();
    for stmt in &block.stmts {
        if let Stmt::Item(item) = stmt {
            collect_item_bindings(item, &mut bindings);
        }
    }
    bindings
}

/// The name `item` declares in its enclosing module or block. Declarations are
/// in scope throughout that whole scope no matter where they sit in it, so
/// callers collect these before visiting the scope's contents.
///
/// `use` is deliberately absent. A `use` binding is exactly what this pass
/// rewrites, so counting one as a shadow would suppress the reference rewrites
/// while the import itself still changed — `use crate::disk::worker::channels;`
/// with the call left as bare `contents_match(..)` (E0425). Inline modules that
/// re-import a name from the file's own top level are handled by dropping the
/// candidate instead, in `scan::drop_candidates_reimported_by_inline_modules`.
///
/// `Item` is `#[non_exhaustive]`; a variant syn adds later declares no name
/// here, which at worst rewrites a reference this collector should have left
/// alone — the exposure the collector had before declarations were tracked.
fn collect_item_bindings(item: &Item, bindings: &mut BTreeSet<String>) {
    match item {
        Item::Const(node) => insert_ident(&node.ident, bindings),
        Item::Enum(node) => insert_ident(&node.ident, bindings),
        Item::ExternCrate(node) => insert_ident(&node.ident, bindings),
        Item::Fn(node) => insert_ident(&node.sig.ident, bindings),
        Item::Macro(node) => {
            if let Some(ident) = &node.ident {
                insert_ident(ident, bindings);
            }
        },
        Item::Mod(node) => insert_ident(&node.ident, bindings),
        Item::Static(node) => insert_ident(&node.ident, bindings),
        Item::Struct(node) => insert_ident(&node.ident, bindings),
        Item::Trait(node) => insert_ident(&node.ident, bindings),
        Item::TraitAlias(node) => insert_ident(&node.ident, bindings),
        Item::Type(node) => insert_ident(&node.ident, bindings),
        Item::Union(node) => insert_ident(&node.ident, bindings),
        _ => {},
    }
}

fn insert_ident(ident: &Ident, bindings: &mut BTreeSet<String>) {
    bindings.insert(ident.to_string());
}

fn collect_pat_bindings(pat: &Pat, bindings: &mut BTreeSet<String>) {
    match pat {
        Pat::Ident(pat_ident) => {
            bindings.insert(pat_ident.ident.to_string());
            if let Some((_, sub)) = &pat_ident.subpat {
                collect_pat_bindings(sub, bindings);
            }
        },
        Pat::Guard(pat_guard) => collect_pat_bindings(&pat_guard.pat, bindings),
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
    inline_mod_depth: usize,
    references: &mut Vec<BareReference>,
) {
    let mut previous_token = PreviousToken::Other;
    for token_tree in tokens.clone() {
        match token_tree {
            TokenTree::Ident(ref ident) => {
                let name = ident.to_string();
                if previous_token.allows_bare_reference() && imported_names.contains(&name) {
                    let span = ident.span();
                    let start = support::offset(offsets, span.start());
                    let end = support::offset(offsets, span.end());
                    references.push(BareReference {
                        name,
                        byte_start: start,
                        byte_end: end,
                        inline_mod_depth,
                    });
                }
                previous_token = PreviousToken::Other;
            },
            TokenTree::Punct(ref punct) => {
                previous_token = match punct.as_char() {
                    ':' => previous_token.after_colon(punct.spacing()),
                    '.' => previous_token.after_dot(punct.spacing()),
                    _ => PreviousToken::Other,
                };
            },
            TokenTree::Group(ref group) => {
                collect_bare_refs_from_tokens(
                    &group.stream(),
                    offsets,
                    imported_names,
                    inline_mod_depth,
                    references,
                );
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
    use syn::visit::Visit;

    use super::BareReference;
    use super::ReferenceCollector;
    use super::collect_bare_refs_from_tokens;
    use super::support;

    #[test]
    fn collect_bare_refs_finds_ident_in_macro_tokens() {
        let src = r"matches!(do_thing(x), MyEnum::Variant)";
        let offsets = support::line_offsets(src);
        let mut names = BTreeSet::new();
        names.insert("do_thing".to_string());
        let tokens: TokenStream = src.parse().expect("parse tokens");
        let mut refs: Vec<BareReference> = Vec::new();
        collect_bare_refs_from_tokens(&tokens, &offsets, &names, 0, &mut refs);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "do_thing");
        assert_eq!(&src[refs[0].byte_start..refs[0].byte_end], "do_thing");
    }

    #[test]
    fn collect_bare_refs_skips_qualified_ident_in_macro_tokens() {
        let src = r"matches!(module::do_thing(x), MyEnum::Variant)";
        let offsets = support::line_offsets(src);
        let mut names = BTreeSet::new();
        names.insert("do_thing".to_string());
        let tokens: TokenStream = src.parse().expect("parse tokens");
        let mut refs: Vec<BareReference> = Vec::new();
        collect_bare_refs_from_tokens(&tokens, &offsets, &names, 0, &mut refs);
        assert!(refs.is_empty(), "qualified path should not match");
    }

    #[test]
    fn collect_bare_refs_finds_nested_in_group() {
        let src = r"assert!(do_thing(foo(bar())))";
        let offsets = support::line_offsets(src);
        let mut names = BTreeSet::new();
        names.insert("do_thing".to_string());
        let tokens: TokenStream = src.parse().expect("parse tokens");
        let mut refs: Vec<BareReference> = Vec::new();
        collect_bare_refs_from_tokens(&tokens, &offsets, &names, 0, &mut refs);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "do_thing");
    }

    #[test]
    fn match_guard_uses_pattern_bindings_as_local_scope() {
        let source = r"
fn inspect(value: i32) {
    match value {
        threshold if helper(threshold) => helper(threshold),
    }
}
";
        let syntax = syn::parse_file(source).expect("parse fixture");
        let offsets = support::line_offsets(source);
        let imported_names = BTreeSet::from(["helper".to_string(), "threshold".to_string()]);
        let mut collector = ReferenceCollector::new(&offsets, &imported_names);

        collector.visit_file(&syntax);

        assert_eq!(collector.references.len(), 2);
        assert!(
            collector
                .references
                .iter()
                .all(|reference| reference.name == "helper")
        );
    }
}
