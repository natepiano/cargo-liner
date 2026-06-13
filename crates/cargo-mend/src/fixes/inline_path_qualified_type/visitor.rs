use std::collections::BTreeSet;

use proc_macro2::LineColumn;
use syn::ExprPath;
use syn::ExprStruct;
use syn::GenericParam;
use syn::Generics;
use syn::ImplItemFn;
use syn::ItemConst;
use syn::ItemEnum;
use syn::ItemFn;
use syn::ItemImpl;
use syn::ItemMod;
use syn::ItemStatic;
use syn::ItemStruct;
use syn::ItemTrait;
use syn::ItemType;
use syn::ItemUse;
use syn::PatStruct;
use syn::PatTupleStruct;
use syn::Path;
use syn::TraitItemFn;
use syn::TypePath;
use syn::UseTree;
use syn::visit;
use syn::visit::Visit;

use crate::rust_syntax::PathAnchor;

pub(super) struct InlinePathOccurrence {
    /// The original fully-qualified path as written (e.g.
    /// `crate::project::RustProject::Package`).
    pub(super) full_path:   String,
    /// The path we intend to add as a `use` statement (e.g.
    /// `crate::project::RustProject`). For enum variants this is the parent
    /// type, not the variant itself.
    pub(super) import_path: String,
    /// The bare last-segment of `import_path` — the name brought into scope
    /// by the `use` (e.g. `RustProject`). Used for collision detection.
    pub(super) import_name: String,
    /// What replaces the inline fully-qualified path in the source. For an
    /// enum variant this is `Enum::Variant`; for a plain type it is `Type`.
    pub(super) replacement: String,
    pub(super) span_start:  LineColumn,
    pub(super) span_end:    LineColumn,
}

pub(super) struct InlinePathVisitor {
    pub(super) occurrences:     Vec<InlinePathOccurrence>,
    pub(super) bare_type_names: BTreeSet<String>,
    pub(super) mod_depth:       usize,
    /// Stack of generic type-parameter names that are in scope at the
    /// current point of traversal. A path whose first segment matches an
    /// active generic (`S::Ok` inside `fn serialize<S>(...)`) is an
    /// associated-item reference, not a crate-qualified path — skip it.
    pub(super) generic_scopes:  Vec<BTreeSet<String>>,
}

impl InlinePathVisitor {
    fn push_generics(&mut self, generics: &Generics) {
        let mut params = BTreeSet::new();
        for param in &generics.params {
            if let GenericParam::Type(type_param) = param {
                params.insert(type_param.ident.to_string());
            }
        }
        self.generic_scopes.push(params);
    }

    fn pop_generics(&mut self) { self.generic_scopes.pop(); }

    fn is_active_generic(&self, name: &str) -> bool {
        self.generic_scopes.iter().any(|scope| scope.contains(name))
    }
}

impl InlinePathVisitor {
    fn check_path(&mut self, path: &Path) {
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();

        if segments.len() < 2 {
            return;
        }

        let first = &segments[0];
        let path_anchor = PathAnchor::from(first.as_str());
        let is_intra_crate = path_anchor.is_crate_relative();

        // For intra-crate paths, require at least 3 segments: `crate::Foo` and
        // `super::Foo` are already short, so adding a `use` would not shorten
        // the call site. For external-crate paths (`ratatui::Frame`,
        // `std::collections::BTreeMap`), 2 segments is the minimum rewrite.
        if is_intra_crate && segments.len() < 3 {
            return;
        }

        // Filter obvious non-crate roots. `self::` is a same-module reference,
        // not a candidate for a `use`.
        if path_anchor.is_explicit_self() {
            return;
        }

        // For non-intra-crate paths, the first segment must look like a crate
        // name (snake_case / lowercase). A PascalCase first segment means this
        // is `Type::Variant` (enum variant pattern), `Type::AssocType`, or
        // `Type::CONST` — not a crate-qualified path. Suggesting `use Type;`
        // for those cases is wrong and confusing.
        if !is_intra_crate && is_pascal_case(first) {
            return;
        }

        // Associated-item references on a generic type parameter in scope
        // (`S::Ok` inside `fn serialize<S>(...)`, `B::Item` inside
        // `impl<B: Bucket>`) are not crate paths — `use S::Ok;` would be
        // nonsense. Skip if the first segment matches a generic param visible
        // at this point of the traversal. This is robust regardless of the
        // generic's naming convention (`T`, `B`, `Idx`, lowercase, etc.).
        if !is_intra_crate && self.is_active_generic(first) {
            return;
        }

        let leaf = &segments[segments.len() - 1];
        if !is_pascal_case(leaf) {
            return;
        }

        let full_path = segments.join("::");

        // Heuristic: if the penultimate segment is also PascalCase, the leaf
        // is almost certainly an enum variant (or associated type/const) of
        // that type. Import the parent type, not the leaf, so that variants
        // stay disambiguated by their enum container (`RustProject::Package`
        // rather than bare `Package`). This avoids collisions with
        // same-named structs or other variants that share a leaf name.
        let penultimate = &segments[segments.len() - 2];
        let (import_segments, import_name, replacement) =
            if is_pascal_case(penultimate) && penultimate != "Self" {
                let import_segments = segments[..segments.len() - 1].to_vec();
                let replacement = format!("{penultimate}::{leaf}");
                (import_segments, penultimate.clone(), replacement)
            } else {
                (segments.clone(), leaf.clone(), leaf.clone())
            };
        let import_path = import_segments.join("::");

        // Use ident spans to exclude generic arguments from the replacement range.
        // path.span() includes generic args (e.g., `<T>`), but we only want to
        // replace the path portion, leaving generic args in place.
        // Safety: segments.len() >= 3, checked above.
        let first_ident_span = path.segments[0].ident.span();
        let last_ident_span = path.segments[segments.len() - 1].ident.span();

        self.occurrences.push(InlinePathOccurrence {
            full_path,
            import_path,
            import_name,
            replacement,
            span_start: first_ident_span.start(),
            span_end: last_ident_span.end(),
        });
    }
}

impl InlinePathVisitor {
    /// Register a path's bare-name footprint for shadow detection.
    ///
    /// A single-segment path (`Result<T, E>`) clearly puts `Result` in use as
    /// a type. But a multi-segment path that *starts* with a `PascalCase`
    /// segment (`Result::ok`, `Alignment::Center`, `MyEnum::Variant`) also
    /// references `Result` / `Alignment` / `MyEnum` as a type — so adding
    /// `use other_crate::Result;` would silently change which type those
    /// expressions resolve through. Register the leading `PascalCase` segment
    /// in either case.
    fn record_bare_name_footprint(&mut self, path: &Path) {
        let Some(first) = path.segments.first() else {
            return;
        };
        let name = first.ident.to_string();
        if !is_pascal_case(&name) {
            return;
        }
        if path.segments.len() == 1 {
            self.bare_type_names.insert(name);
        } else if name != "Self" {
            // A bare `Self::method` is fine — `Self` isn't a name we'd ever
            // import. Anything else PascalCase as the first segment is a real
            // type reference whose meaning would change under a same-named
            // import.
            self.bare_type_names.insert(name);
        }
    }
}

impl Visit<'_> for InlinePathVisitor {
    fn visit_item_use(&mut self, _: &ItemUse) {
        // Skip use statements — they are imports, not inline code
    }

    fn visit_type_path(&mut self, node: &TypePath) {
        if node.qself.is_none() {
            self.check_path(&node.path);
            self.record_bare_name_footprint(&node.path);
        }
        visit::visit_type_path(self, node);
    }

    fn visit_expr_path(&mut self, node: &ExprPath) {
        if node.qself.is_none() {
            self.check_path(&node.path);
            self.record_bare_name_footprint(&node.path);
        }
        // Don't recurse — path segments don't contain sub-expressions
    }

    fn visit_expr_struct(&mut self, node: &ExprStruct) {
        // `Foo { .. }` and `crate::foo::Bar { .. }` — the path of a struct
        // literal isn't reached by `visit_expr_path` / `visit_type_path`,
        // so handle it explicitly.
        if node.qself.is_none() {
            self.check_path(&node.path);
            self.record_bare_name_footprint(&node.path);
        }
        visit::visit_expr_struct(self, node);
    }

    fn visit_pat_struct(&mut self, node: &PatStruct) {
        // `Foo { .. }` and `crate::foo::Bar { .. }` in pattern position
        // (`let Bar { .. } = ...`, match arms) — also not visited by
        // `visit_expr_path` / `visit_type_path`.
        if node.qself.is_none() {
            self.check_path(&node.path);
            self.record_bare_name_footprint(&node.path);
        }
        visit::visit_pat_struct(self, node);
    }

    fn visit_item_mod(&mut self, node: &ItemMod) {
        // Track nesting depth so item-name registration below can gate on
        // "is this at the file's top level". Names defined inside nested
        // modules don't collide with imports added at the top level.
        self.mod_depth += 1;
        visit::visit_item_mod(self, node);
        self.mod_depth -= 1;
    }

    fn visit_item_struct(&mut self, node: &ItemStruct) {
        if self.mod_depth == 0 {
            self.bare_type_names.insert(node.ident.to_string());
        }
        self.push_generics(&node.generics);
        visit::visit_item_struct(self, node);
        self.pop_generics();
    }

    fn visit_item_enum(&mut self, node: &ItemEnum) {
        if self.mod_depth == 0 {
            self.bare_type_names.insert(node.ident.to_string());
        }
        self.push_generics(&node.generics);
        visit::visit_item_enum(self, node);
        self.pop_generics();
    }

    fn visit_item_type(&mut self, node: &ItemType) {
        if self.mod_depth == 0 {
            self.bare_type_names.insert(node.ident.to_string());
        }
        self.push_generics(&node.generics);
        visit::visit_item_type(self, node);
        self.pop_generics();
    }

    fn visit_item_trait(&mut self, node: &ItemTrait) {
        if self.mod_depth == 0 {
            self.bare_type_names.insert(node.ident.to_string());
        }
        self.push_generics(&node.generics);
        visit::visit_item_trait(self, node);
        self.pop_generics();
    }

    fn visit_item_fn(&mut self, node: &ItemFn) {
        if self.mod_depth == 0 {
            self.bare_type_names.insert(node.sig.ident.to_string());
        }
        // Push the fn's generics around the entire item — signature AND body.
        // Bodies contain closure parameter types and locals that may reference
        // these generics (e.g. `let f = |x: &T::Item| ...` inside `fn foo<T>`).
        // A `visit_signature`-only push pops before the body is visited, so
        // those references would be misread as crate paths.
        self.push_generics(&node.sig.generics);
        visit::visit_item_fn(self, node);
        self.pop_generics();
    }

    fn visit_impl_item_fn(&mut self, node: &ImplItemFn) {
        self.push_generics(&node.sig.generics);
        visit::visit_impl_item_fn(self, node);
        self.pop_generics();
    }

    fn visit_trait_item_fn(&mut self, node: &TraitItemFn) {
        self.push_generics(&node.sig.generics);
        visit::visit_trait_item_fn(self, node);
        self.pop_generics();
    }

    fn visit_item_const(&mut self, node: &ItemConst) {
        if self.mod_depth == 0 {
            self.bare_type_names.insert(node.ident.to_string());
        }
        visit::visit_item_const(self, node);
    }

    fn visit_item_static(&mut self, node: &ItemStatic) {
        if self.mod_depth == 0 {
            self.bare_type_names.insert(node.ident.to_string());
        }
        visit::visit_item_static(self, node);
    }

    fn visit_item_impl(&mut self, node: &ItemImpl) {
        // `impl Trait for Type` — the trait path is `ItemImpl::trait_`, a bare
        // `syn::Path` not visited as a `TypePath`. Inspect it directly.
        self.push_generics(&node.generics);
        if let Some((_, trait_path, _)) = &node.trait_ {
            self.check_path(trait_path);
            if trait_path.segments.len() == 1 {
                let name = trait_path.segments[0].ident.to_string();
                if is_pascal_case(&name) {
                    self.bare_type_names.insert(name);
                }
            }
        }
        visit::visit_item_impl(self, node);
        self.pop_generics();
    }

    fn visit_pat_tuple_struct(&mut self, node: &PatTupleStruct) {
        // `Foo(..)` in pattern position — e.g. `let Foo(x) = ...` or
        // `Some(Enum::Variant(x))` match arms.
        if node.qself.is_none() {
            self.check_path(&node.path);
            self.record_bare_name_footprint(&node.path);
        }
        visit::visit_pat_tuple_struct(self, node);
    }
}

pub(super) fn flatten_use_path(tree: &UseTree) -> Option<String> {
    let mut segments = Vec::new();
    let mut cursor = tree;
    loop {
        match cursor {
            UseTree::Path(path) => {
                segments.push(path.ident.to_string());
                cursor = &path.tree;
            },
            UseTree::Name(name) => {
                segments.push(name.ident.to_string());
                break;
            },
            _ => return None,
        }
    }
    Some(segments.join("::"))
}

pub(super) fn is_pascal_case(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };
    first.is_ascii_uppercase() && name.chars().any(|ch| ch.is_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::is_pascal_case;

    #[test]
    fn pascal_case_detects_types() {
        assert!(is_pascal_case("MyType"));
        assert!(is_pascal_case("Thing"));
        assert!(is_pascal_case("PublicContainer"));
        assert!(is_pascal_case("Foo"));
    }

    #[test]
    fn pascal_case_rejects_functions() {
        assert!(!is_pascal_case("do_thing"));
        assert!(!is_pascal_case("func_a"));
    }

    #[test]
    fn pascal_case_rejects_constants() {
        assert!(!is_pascal_case("MAX_SIZE"));
        assert!(!is_pascal_case("A"));
    }

    #[test]
    fn pascal_case_rejects_empty() {
        assert!(!is_pascal_case(""));
    }
}
