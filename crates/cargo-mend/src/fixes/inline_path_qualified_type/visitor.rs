use std::collections::BTreeSet;

use proc_macro2::LineColumn;
use syn::ExprPath;
use syn::ExprStruct;
use syn::File;
use syn::GenericParam;
use syn::Generics;
use syn::Ident;
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
use syn::visit;
use syn::visit::Visit;

use crate::fixes::imports;
use crate::fixes::imports::ConditionalAttributes;
use crate::fixes::imports::GateSource;
use crate::fixes::imports::GateTracking;
use crate::fixes::imports::ImportSource;
use crate::rust_syntax::PathAnchor;

enum PathOrigin {
    IntraCrate,
    External,
    ExplicitSelf,
}

impl From<PathAnchor> for PathOrigin {
    fn from(anchor: PathAnchor) -> Self {
        match anchor {
            PathAnchor::Crate | PathAnchor::Super => Self::IntraCrate,
            PathAnchor::SelfMod | PathAnchor::SelfType => Self::ExplicitSelf,
            PathAnchor::Name => Self::External,
        }
    }
}

enum BareNameFootprint {
    SingleType(String),
    QualifiedType(String),
    SelfReference,
}

impl BareNameFootprint {
    fn for_path(path: &Path, name: String) -> Self {
        match (path.segments.len(), name.as_str()) {
            (1, _) => Self::SingleType(name),
            (_, "Self") => Self::SelfReference,
            _ => Self::QualifiedType(name),
        }
    }
}

#[derive(Clone, Copy)]
enum ModuleNesting {
    FileRoot,
    Nested { depth: usize },
}

impl ModuleNesting {
    const fn depth(self) -> usize {
        match self {
            Self::FileRoot => 0,
            Self::Nested { depth } => depth,
        }
    }
}

impl From<usize> for ModuleNesting {
    fn from(depth: usize) -> Self {
        match depth {
            0 => Self::FileRoot,
            _ => Self::Nested { depth },
        }
    }
}

pub(super) struct InlinePathOccurrence {
    /// The original fully-qualified path as written (e.g.
    /// `crate::project::RustProject::Package`).
    pub(super) full_path:              String,
    /// The path we intend to add as a `use` statement (e.g.
    /// `crate::project::RustProject`). For enum variants this is the parent
    /// type, not the variant itself.
    pub(super) import_path:            String,
    /// The bare last-segment of `import_path` — the name brought into scope
    /// by the `use` (e.g. `RustProject`). Used for collision detection.
    pub(super) import_name:            String,
    /// What replaces the inline fully-qualified path in the source. For an
    /// enum variant this is `Enum::Variant`; for a plain type it is `Type`.
    pub(super) replacement:            String,
    pub(super) span_start:             LineColumn,
    pub(super) span_end:               LineColumn,
    pub(super) conditional_attributes: ConditionalAttributes,
}

pub(super) struct InlinePathVisitor {
    pub(super) occurrences:            Vec<InlinePathOccurrence>,
    pub(super) bare_type_names:        BTreeSet<String>,
    pub(super) mod_depth:              usize,
    /// Stack of generic type-parameter names that are in scope at the
    /// current point of traversal. A path whose first segment matches an
    /// active generic (`S::Ok` inside `fn serialize<S>(...)`) is an
    /// associated-item reference, not a crate-qualified path — skip it.
    pub(super) generic_scopes:         Vec<BTreeSet<String>>,
    pub(super) conditional_attributes: ConditionalAttributes,
}

impl InlinePathVisitor {
    fn register_root_name(&mut self, name: &Ident) {
        match ModuleNesting::from(self.mod_depth) {
            ModuleNesting::FileRoot => {
                self.bare_type_names.insert(name.to_string());
            },
            ModuleNesting::Nested { .. } => {},
        }
    }

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

impl GateTracking for GatedInlinePaths<'_, '_> {
    fn gate_source(&self) -> GateSource<'_, '_> {
        GateSource {
            indexed: self.source,
        }
    }

    fn conditional_attributes_mut(&mut self) -> &mut ConditionalAttributes {
        &mut self.visitor.conditional_attributes
    }
}

impl InlinePathVisitor {
    fn check_path(&mut self, path: &Path) {
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();

        if segments.len() < 2 {
            return;
        }

        let first = &segments[0];
        // Intra-crate paths need three segments; external paths need two and a crate name.
        // Generic associated items and explicit self references are never import candidates.
        match PathOrigin::from(PathAnchor::from(first.as_str())) {
            PathOrigin::IntraCrate => {
                if segments.len() < 3 {
                    return;
                }
            },
            PathOrigin::External => {
                if is_pascal_case(first) || self.is_active_generic(first) {
                    return;
                }
            },
            PathOrigin::ExplicitSelf => return,
        }

        let leaf = &segments[segments.len() - 1];
        if !is_pascal_case(leaf) {
            return;
        }

        let full_path = segments.join("::");

        // `is_pascal_case(penultimate)` is the heuristic: if the penultimate segment is also
        // PascalCase, the leaf is almost certainly an enum variant (or associated
        // type/const) of that type. Import the parent type, not the leaf, so that variants
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

        // `first_ident_span` and `last_ident_span` use identifier spans to exclude generic
        // arguments from the replacement range. path.span() includes generic args (e.g.,
        // `<T>`), but we only want to replace the path portion, leaving generic args in
        // place. `check_path` already required at least three intra-crate segments or two
        // external-crate segments.
        let first_ident_span = path.segments[0].ident.span();
        let last_ident_span = path.segments[segments.len() - 1].ident.span();

        self.occurrences.push(InlinePathOccurrence {
            full_path,
            import_path,
            import_name,
            replacement,
            span_start: first_ident_span.start(),
            span_end: last_ident_span.end(),
            conditional_attributes: self.conditional_attributes.clone(),
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
        match BareNameFootprint::for_path(path, name) {
            BareNameFootprint::SingleType(name) | BareNameFootprint::QualifiedType(name) => {
                self.bare_type_names.insert(name);
            },
            BareNameFootprint::SelfReference => {},
        }
    }
}

/// A traversal borrowing the file scan's checked index for all gate extraction.
struct GatedInlinePaths<'visit, 'source> {
    visitor: &'visit mut InlinePathVisitor,
    source:  &'visit ImportSource<'source>,
}

impl InlinePathVisitor {
    pub(super) fn visit_file(&mut self, node: &File, source: &ImportSource<'_>) {
        GatedInlinePaths {
            visitor: self,
            source,
        }
        .visit_file(node);
    }
}

impl Visit<'_> for GatedInlinePaths<'_, '_> {
    imports::gate_tracking_visit!();

    fn visit_item_use(&mut self, _: &ItemUse) {
        // `GatedInlinePaths::visit_item_use` skips `ItemUse` because use statements are imports,
        // not inline code.
    }

    fn visit_type_path(&mut self, node: &TypePath) {
        if node.qself.is_none() {
            self.visitor.check_path(&node.path);
            self.visitor.record_bare_name_footprint(&node.path);
        }
        visit::visit_type_path(self, node);
    }

    fn visit_expr_path(&mut self, node: &ExprPath) {
        if node.qself.is_none() {
            self.visitor.check_path(&node.path);
            self.visitor.record_bare_name_footprint(&node.path);
        }
        // `GatedInlinePaths::visit_expr_path` does not recurse because `node.path` segments do not
        // contain sub-expressions.
    }

    fn visit_expr_struct(&mut self, node: &ExprStruct) {
        // `GatedInlinePaths::visit_expr_struct` handles `node.path` in `Foo { .. }` and
        // `crate::foo::Bar { .. }` — the path of a struct literal isn't reached by
        // `visit_expr_path` / `visit_type_path`, so handle it explicitly.
        if node.qself.is_none() {
            self.visitor.check_path(&node.path);
            self.visitor.record_bare_name_footprint(&node.path);
        }
        visit::visit_expr_struct(self, node);
    }

    fn visit_pat_struct(&mut self, node: &PatStruct) {
        // `GatedInlinePaths::visit_pat_struct` handles `node.path` in `Foo { .. }` and
        // `crate::foo::Bar { .. }` in pattern position (`let Bar { .. } = ...`, match arms)
        // — also not visited by `visit_expr_path` / `visit_type_path`.
        if node.qself.is_none() {
            self.visitor.check_path(&node.path);
            self.visitor.record_bare_name_footprint(&node.path);
        }
        visit::visit_pat_struct(self, node);
    }

    fn visit_item_mod(&mut self, node: &ItemMod) {
        // `InlinePathVisitor::mod_depth` tracks nesting depth so item-name registration below can
        // gate on "is this at the file's top level". Names defined inside nested
        // modules don't collide with imports added at the top level.
        let nesting = ModuleNesting::from(self.visitor.mod_depth);
        self.visitor.mod_depth = nesting.depth() + 1;
        visit::visit_item_mod(self, node);
        self.visitor.mod_depth = nesting.depth();
    }

    fn visit_item_struct(&mut self, node: &ItemStruct) {
        self.visitor.register_root_name(&node.ident);
        self.visitor.push_generics(&node.generics);
        visit::visit_item_struct(self, node);
        self.visitor.pop_generics();
    }

    fn visit_item_enum(&mut self, node: &ItemEnum) {
        self.visitor.register_root_name(&node.ident);
        self.visitor.push_generics(&node.generics);
        visit::visit_item_enum(self, node);
        self.visitor.pop_generics();
    }

    fn visit_item_type(&mut self, node: &ItemType) {
        self.visitor.register_root_name(&node.ident);
        self.visitor.push_generics(&node.generics);
        visit::visit_item_type(self, node);
        self.visitor.pop_generics();
    }

    fn visit_item_trait(&mut self, node: &ItemTrait) {
        self.visitor.register_root_name(&node.ident);
        self.visitor.push_generics(&node.generics);
        visit::visit_item_trait(self, node);
        self.visitor.pop_generics();
    }

    fn visit_item_fn(&mut self, node: &ItemFn) {
        self.visitor.register_root_name(&node.sig.ident);
        // `GatedInlinePaths::visit_item_fn` calls `push_generics` and `pop_generics` around the
        // entire item — signature AND body. Bodies contain closure parameter types and
        // locals that may reference these generics (e.g. `let f = |x: &T::Item| ...` inside
        // `fn foo<T>`). A `visit_signature`-only push pops before the body is visited, so
        // those references would be misread as crate paths.
        self.visitor.push_generics(&node.sig.generics);
        visit::visit_item_fn(self, node);
        self.visitor.pop_generics();
    }

    fn visit_impl_item_fn(&mut self, node: &ImplItemFn) {
        self.visitor.push_generics(&node.sig.generics);
        visit::visit_impl_item_fn(self, node);
        self.visitor.pop_generics();
    }

    fn visit_trait_item_fn(&mut self, node: &TraitItemFn) {
        self.visitor.push_generics(&node.sig.generics);
        visit::visit_trait_item_fn(self, node);
        self.visitor.pop_generics();
    }

    fn visit_item_const(&mut self, node: &ItemConst) {
        self.visitor.register_root_name(&node.ident);
        visit::visit_item_const(self, node);
    }

    fn visit_item_static(&mut self, node: &ItemStatic) {
        self.visitor.register_root_name(&node.ident);
        visit::visit_item_static(self, node);
    }

    fn visit_item_impl(&mut self, node: &ItemImpl) {
        // `impl Trait for Type` — the trait path is `ItemImpl::trait_`, a bare
        // `syn::Path` not visited as a `TypePath`. Inspect it directly.
        self.visitor.push_generics(&node.generics);
        if let Some((trait_path, _)) = &node.trait_ {
            self.visitor.check_path(trait_path);
            if trait_path.segments.len() == 1 {
                let name = trait_path.segments[0].ident.to_string();
                if is_pascal_case(&name) {
                    self.visitor.bare_type_names.insert(name);
                }
            }
        }
        visit::visit_item_impl(self, node);
        self.visitor.pop_generics();
    }

    fn visit_pat_tuple_struct(&mut self, node: &PatTupleStruct) {
        // `GatedInlinePaths::visit_pat_tuple_struct` checks `node.path` in `Foo(..)` patterns —
        // e.g. `let Foo(x) = ...` or `Some(Enum::Variant(x))` match arms.
        if node.qself.is_none() {
            self.visitor.check_path(&node.path);
            self.visitor.record_bare_name_footprint(&node.path);
        }
        visit::visit_pat_tuple_struct(self, node);
    }
}

pub(super) fn is_pascal_case(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };
    first.is_ascii_uppercase() && name.chars().any(|ch| ch.is_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::InlinePathVisitor;
    use super::is_pascal_case;
    use crate::fixes::imports::ConditionalAttributes;

    fn inspect(text: &str) -> syn::Result<InlinePathVisitor> {
        let syntax = syn::parse_file(text)?;
        let mut visitor = InlinePathVisitor {
            occurrences:            Vec::new(),
            bare_type_names:        BTreeSet::new(),
            mod_depth:              0,
            generic_scopes:         Vec::new(),
            conditional_attributes: ConditionalAttributes::default(),
        };
        visitor.visit_file(&syntax, &crate::fixes::imports::ImportSource::new(text));
        Ok(visitor)
    }

    #[test]
    fn many_ungated_nodes_share_one_checked_source_index() -> syn::Result<()> {
        let statements = "crate::types::Widget;".repeat(512);
        let text = format!("fn f() {{ {statements} }}");
        let before = crate::fixes::imports::ImportSource::construction_count();
        let visitor = inspect(&text)?;
        assert_eq!(
            crate::fixes::imports::ImportSource::construction_count() - before,
            1
        );
        assert_eq!(visitor.occurrences.len(), 512);
        assert!(
            visitor
                .occurrences
                .iter()
                .all(|site| site.conditional_attributes.is_empty())
        );

        let gated = format!("#[cfg(unix)] fn f() {{ {statements} }}");
        let before = crate::fixes::imports::ImportSource::construction_count();
        let visitor = inspect(&gated)?;
        assert_eq!(
            crate::fixes::imports::ImportSource::construction_count() - before,
            1
        );
        assert_eq!(visitor.occurrences.len(), 512);
        assert!(
            visitor
                .occurrences
                .iter()
                .all(|site| { site.conditional_attributes.render("") == "#[cfg(unix)]\n" })
        );
        Ok(())
    }

    #[test]
    fn origin_policy_keeps_length_self_and_generic_exclusions() -> syn::Result<()> {
        let visitor = inspect(concat!(
            "fn f<lower>() { crate::Widget; crate::types::Widget; ",
            "super::Widget; super::types::Widget; self::types::Widget; ",
            "Self::Assoc; ext::Widget; Type::Assoc; lower::Assoc; }",
        ))?;
        let paths: Vec<_> = visitor
            .occurrences
            .iter()
            .map(|occ| occ.full_path.as_str())
            .collect();
        assert_eq!(
            paths,
            [
                "crate::types::Widget",
                "super::types::Widget",
                "ext::Widget"
            ]
        );
        assert!(visitor.bare_type_names.contains("Type"));
        assert!(!visitor.bare_type_names.contains("Self"));
        Ok(())
    }

    #[test]
    fn root_registration_resumes_after_nested_modules() -> syn::Result<()> {
        let visitor = inspect(concat!(
            "struct Root; mod child { struct Nested; mod deeper { enum Deep {} } } ",
            "enum After {} type Alias = u8; trait Trait {} fn function() {} ",
            "const VALUE: u8 = 0; static STATIC: u8 = 0;",
        ))?;
        assert_eq!(visitor.mod_depth, 0);
        assert_eq!(
            visitor.bare_type_names,
            BTreeSet::from_iter(
                [
                    "Root", "After", "Alias", "Trait", "function", "VALUE", "STATIC"
                ]
                .map(str::to_string)
            )
        );
        Ok(())
    }

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
