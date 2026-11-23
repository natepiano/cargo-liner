use std::collections::BTreeSet;

use proc_macro2::LineColumn;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use syn::Block;
use syn::ExprPath;
use syn::ExprStruct;
use syn::File;
use syn::GenericParam;
use syn::Generics;
use syn::Ident;
use syn::ItemImpl;
use syn::ItemMod;
use syn::ItemStatic;
use syn::ItemUse;
use syn::Macro;
use syn::PatStruct;
use syn::PatTupleStruct;
use syn::Path;
use syn::TypePath;
use syn::ext::IdentExt;
use syn::spanned::Spanned;
use syn::visit;
use syn::visit::Visit;

use super::block_bindings::BlockBinding;
use super::block_bindings::BlockBindings;
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

/// An active generic can bind the first segment or the name from the new import.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GenericBinding {
    Active,
    Absent,
}

#[derive(Clone, Copy)]
enum ModuleSite {
    ImportScope,
    BlockLocalModule,
}

/// The ASCII type-name heuristic accepts an uppercase first character and at least one
/// lowercase character. A Pascal penultimate segment keeps the leaf qualified by its enum
/// container, avoiding collisions with same-named structs or variants from another enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IdentifierCase {
    Pascal,
    NonPascal,
}

/// The token before an identifier in a macro body. An identifier after a `PathSeparator`
/// (`::`) or a `MemberDot` (`.`) is a later path segment or a member, not a path start.
/// `SelfModule` is a `self` that starts a path, and `SelfModuleColon` is the first `:` after
/// it. The identifier after `self::` resolves through the module scope, like a path start.
#[derive(Clone, Copy)]
enum PrecedingMacroToken {
    Other,
    Colon,
    PathSeparator,
    MemberDot,
    SelfModule,
    SelfModuleColon,
}

impl PrecedingMacroToken {
    /// The preceding token after `punct`. A second `:` makes `::`, except after a path-start
    /// `self`, where it returns to `Other`. A second `.` makes the `..` of a range, whose end
    /// resolves through scope.
    const fn after_punct(self, punct: char) -> Self {
        match (self, punct) {
            (Self::SelfModule, ':') => Self::SelfModuleColon,
            (Self::SelfModuleColon, ':') | (Self::MemberDot, '.') => Self::Other,
            (Self::Colon, ':') => Self::PathSeparator,
            (_, ':') => Self::Colon,
            (_, '.') => Self::MemberDot,
            _ => Self::Other,
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
    /// The import segments before resolution (e.g. `crate::project::RustProject`).
    /// For enum variants, these name the parent type.
    /// Resolution keeps the absolute anchor from `full_path`.
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
    /// `check_path` checks type and const parameters in the enclosing scopes.
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
            let name = match param {
                GenericParam::Type(param) => &param.ident,
                GenericParam::Const(param) => &param.ident,
                GenericParam::Lifetime(_) => continue,
            };
            params.insert(name.unraw().to_string());
        }
        self.generic_scopes.push(params);
    }

    fn pop_generics(&mut self) { self.generic_scopes.pop(); }

    fn generic_binding(&self, name: &Ident) -> GenericBinding {
        let name = name.unraw().to_string();
        if self
            .generic_scopes
            .iter()
            .any(|scope| scope.contains(&name))
        {
            GenericBinding::Active
        } else {
            GenericBinding::Absent
        }
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
    fn check_path(&mut self, path: &Path, block_scopes: &[BlockBindings]) {
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();

        if segments.len() < 2 {
            return;
        }

        let first = &segments[0];
        // `PathOrigin::IntraCrate` requires at least three segments.
        // `PathOrigin::External` can start with a local name.
        match PathOrigin::from(PathAnchor::from(first.as_str())) {
            PathOrigin::IntraCrate => {
                if segments.len() < 3 {
                    return;
                }
            },
            PathOrigin::External => {
                if is_pascal_case(first) == IdentifierCase::Pascal {
                    return;
                }
                let first_ident = &path.segments[0].ident;
                if path.leading_colon.is_none()
                    && (self.generic_binding(first_ident) == GenericBinding::Active
                        || BlockBindings::lookup(block_scopes, first_ident)
                            == BlockBinding::MayBind)
                {
                    return;
                }
            },
            PathOrigin::ExplicitSelf => return,
        }

        let leaf = &segments[segments.len() - 1];
        if is_pascal_case(leaf) == IdentifierCase::NonPascal {
            return;
        }

        let mut full_path = segments.join("::");
        if path.leading_colon.is_some() {
            full_path.insert_str(0, "::");
        }

        // `IdentifierCase::Pascal` on the penultimate segment can indicate an enum type.
        // Keep enum variants qualified by their type to avoid collisions with other items.
        let penultimate = &segments[segments.len() - 2];
        let (import_segments, import_name, replacement) =
            if is_pascal_case(penultimate) == IdentifierCase::Pascal && penultimate != "Self" {
                let import_segments = &segments[..segments.len() - 1];
                let replacement = format!("{penultimate}::{leaf}");
                (import_segments, penultimate.clone(), replacement)
            } else {
                (segments.as_slice(), leaf.clone(), leaf.clone())
            };
        let import_ident = &path.segments[import_segments.len() - 1].ident;
        if self.generic_binding(import_ident) == GenericBinding::Active
            || BlockBindings::lookup(block_scopes, import_ident) == BlockBinding::MayBind
        {
            return;
        }
        let import_path = import_segments.join("::");

        // Include the leading `::` when the path has one.
        // Stop at the last identifier to keep its generic arguments.
        let last_ident_span = path.segments[segments.len() - 1].ident.span();

        self.occurrences.push(InlinePathOccurrence {
            full_path,
            import_path,
            import_name,
            replacement,
            span_start: path.span().start(),
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
    /// in either case. After a leading `self`, the second segment resolves
    /// through the module scope, glob imports included, so `self::Foo::X`
    /// registers `Foo`.
    fn record_bare_name_footprint(&mut self, path: &Path) {
        let mut segments = path.segments.iter();
        let Some(first) = segments.next() else {
            return;
        };
        let first_name = first.ident.unraw().to_string();
        let name = match PathAnchor::from(first_name.as_str()) {
            PathAnchor::SelfMod => {
                let Some(second) = segments.next() else {
                    return;
                };
                second.ident.unraw().to_string()
            },
            PathAnchor::Crate | PathAnchor::Super | PathAnchor::SelfType | PathAnchor::Name => {
                first_name
            },
        };
        if is_pascal_case(&name) == IdentifierCase::NonPascal {
            return;
        }
        match BareNameFootprint::for_path(path, name) {
            BareNameFootprint::SingleType(name) | BareNameFootprint::QualifiedType(name) => {
                self.bare_type_names.insert(name);
            },
            BareNameFootprint::SelfReference => {},
        }
    }

    /// Register each `PascalCase` identifier that starts a path in a macro body, nested
    /// groups included. `syn` does not parse the body, so `record_bare_name_footprint` never
    /// sees `Foo` in `assert_eq!(Foo::X, 1)`. An import of `Foo` would hide the `Foo` the body
    /// uses. Like `record_bare_name_footprint`, it skips a later path segment or a member: the
    /// `Foo` in `crate::other::Foo::X` or `value.Foo` does not resolve through scope. The `Foo`
    /// in `self::Foo::X` does, so a path-start `self` keeps the next identifier a path start.
    fn record_macro_footprint(&mut self, tokens: TokenStream) {
        let mut preceding = PrecedingMacroToken::Other;
        for token in tokens {
            preceding = match token {
                TokenTree::Ident(ident) => match preceding {
                    PrecedingMacroToken::Other
                    | PrecedingMacroToken::Colon
                    | PrecedingMacroToken::SelfModule
                    | PrecedingMacroToken::SelfModuleColon => {
                        let name = ident.unraw().to_string();
                        match (PathAnchor::from(name.as_str()), is_pascal_case(&name)) {
                            (PathAnchor::SelfMod, _) => PrecedingMacroToken::SelfModule,
                            (_, IdentifierCase::Pascal) => {
                                self.bare_type_names.insert(name);
                                PrecedingMacroToken::Other
                            },
                            (_, IdentifierCase::NonPascal) => PrecedingMacroToken::Other,
                        }
                    },
                    PrecedingMacroToken::PathSeparator | PrecedingMacroToken::MemberDot => {
                        PrecedingMacroToken::Other
                    },
                },
                TokenTree::Punct(punct) => preceding.after_punct(punct.as_char()),
                TokenTree::Group(group) => {
                    self.record_macro_footprint(group.stream());
                    PrecedingMacroToken::Other
                },
                TokenTree::Literal(_) => PrecedingMacroToken::Other,
            };
        }
    }
}

/// A traversal borrowing the file scan's checked index for all gate extraction.
struct GatedInlinePaths<'visit, 'source> {
    visitor:      &'visit mut InlinePathVisitor,
    source:       &'visit ImportSource<'source>,
    block_scopes: Vec<BlockBindings>,
    module_site:  ModuleSite,
}

impl GatedInlinePaths<'_, '_> {
    fn in_generic_scope(&mut self, generics: &Generics, visit: impl FnOnce(&mut Self)) {
        self.visitor.push_generics(generics);
        visit(self);
        self.visitor.pop_generics();
    }

    fn inspect_path(&mut self, path: &Path) {
        self.visitor.record_bare_name_footprint(path);
        if matches!(self.module_site, ModuleSite::ImportScope) {
            self.visitor.check_path(path, &self.block_scopes);
        }
    }
}

impl InlinePathVisitor {
    pub(super) fn visit_file(&mut self, node: &File, source: &ImportSource<'_>) {
        GatedInlinePaths {
            visitor: self,
            source,
            block_scopes: Vec::new(),
            module_site: ModuleSite::ImportScope,
        }
        .visit_file(node);
    }
}

// Keep each owner's generic parameters in scope throughout its subtree.
macro_rules! visit_generic_items {
    ($($method:ident($node:ident: $item:ident) => $generics:expr $(, $root_name:expr)?;)+) => {
        $(
            fn $method(&mut self, $node: &syn::$item) {
                $(self.visitor.register_root_name($root_name);)?
                self.in_generic_scope($generics, |visitor| visit::$method(visitor, $node));
            }
        )+
    };
}

impl Visit<'_> for GatedInlinePaths<'_, '_> {
    imports::gate_tracking_visit!();

    visit_generic_items! {
        visit_item_const(node: ItemConst) => &node.generics, &node.ident;
        visit_item_enum(node: ItemEnum) => &node.generics, &node.ident;
        visit_item_fn(node: ItemFn) => &node.sig.generics, &node.sig.ident;
        visit_item_struct(node: ItemStruct) => &node.generics, &node.ident;
        visit_item_trait(node: ItemTrait) => &node.generics, &node.ident;
        visit_item_trait_alias(node: ItemTraitAlias) => &node.generics, &node.ident;
        visit_item_type(node: ItemType) => &node.generics, &node.ident;
        visit_item_union(node: ItemUnion) => &node.generics, &node.ident;
        visit_foreign_item_fn(node: ForeignItemFn) => &node.sig.generics;
        visit_foreign_item_type(node: ForeignItemType) => &node.generics;
        visit_impl_item_const(node: ImplItemConst) => &node.generics;
        visit_impl_item_fn(node: ImplItemFn) => &node.sig.generics;
        visit_impl_item_type(node: ImplItemType) => &node.generics;
        visit_trait_item_const(node: TraitItemConst) => &node.generics;
        visit_trait_item_fn(node: TraitItemFn) => &node.sig.generics;
        visit_trait_item_type(node: TraitItemType) => &node.generics;
    }

    fn visit_block(&mut self, node: &Block) {
        self.block_scopes.push(BlockBindings::collect(node));
        visit::visit_block(self, node);
        self.block_scopes.pop();
    }

    fn visit_item_use(&mut self, _: &ItemUse) {
        // `GatedInlinePaths::visit_item_use` skips `ItemUse` because use statements are imports,
        // not inline code.
    }

    fn visit_type_path(&mut self, node: &TypePath) {
        if node.qself.is_none() {
            self.inspect_path(&node.path);
        }
        visit::visit_type_path(self, node);
    }

    fn visit_expr_path(&mut self, node: &ExprPath) {
        if node.qself.is_none() {
            self.inspect_path(&node.path);
        }
        // `GatedInlinePaths::visit_expr_path` does not recurse because `node.path` segments do not
        // contain sub-expressions.
    }

    fn visit_expr_struct(&mut self, node: &ExprStruct) {
        // `GatedInlinePaths::visit_expr_struct` handles `node.path` in `Foo { .. }` and
        // `crate::foo::Bar { .. }` — the path of a struct literal isn't reached by
        // `visit_expr_path` / `visit_type_path`, so handle it explicitly.
        if node.qself.is_none() {
            self.inspect_path(&node.path);
        }
        visit::visit_expr_struct(self, node);
    }

    fn visit_pat_struct(&mut self, node: &PatStruct) {
        // `GatedInlinePaths::visit_pat_struct` handles `node.path` in `Foo { .. }` and
        // `crate::foo::Bar { .. }` in pattern position (`let Bar { .. } = ...`, match arms)
        // — also not visited by `visit_expr_path` / `visit_type_path`.
        if node.qself.is_none() {
            self.inspect_path(&node.path);
        }
        visit::visit_pat_struct(self, node);
    }

    fn visit_item_mod(&mut self, node: &ItemMod) {
        // `collect_scopes` has no insertion site for a module in a block.
        // `inspect_path` still records bare names in that module.
        let module_site = self.module_site;
        if !self.block_scopes.is_empty() {
            self.module_site = ModuleSite::BlockLocalModule;
        }
        let nesting = ModuleNesting::from(self.visitor.mod_depth);
        self.visitor.mod_depth = nesting.depth() + 1;
        visit::visit_item_mod(self, node);
        self.visitor.mod_depth = nesting.depth();
        self.module_site = module_site;
    }

    fn visit_macro(&mut self, node: &Macro) {
        self.visitor.record_macro_footprint(node.tokens.clone());
        visit::visit_macro(self, node);
    }

    fn visit_item_static(&mut self, node: &ItemStatic) {
        self.visitor.register_root_name(&node.ident);
        visit::visit_item_static(self, node);
    }

    fn visit_item_impl(&mut self, node: &ItemImpl) {
        self.in_generic_scope(&node.generics, |visitor| {
            // Check `ItemImpl::trait_` directly because it is a `syn::Path`.
            if let Some((trait_path, _)) = &node.trait_ {
                visitor.inspect_path(trait_path);
            }
            visit::visit_item_impl(visitor, node);
        });
    }

    fn visit_pat_tuple_struct(&mut self, node: &PatTupleStruct) {
        // `GatedInlinePaths::visit_pat_tuple_struct` checks `node.path` in `Foo(..)` patterns —
        // e.g. `let Foo(x) = ...` or `Some(Enum::Variant(x))` match arms.
        if node.qself.is_none() {
            self.inspect_path(&node.path);
        }
        visit::visit_pat_tuple_struct(self, node);
    }
}

pub(super) fn is_pascal_case(name: &str) -> IdentifierCase {
    let Some(first) = name.chars().next() else {
        return IdentifierCase::NonPascal;
    };
    if first.is_ascii_uppercase() && name.chars().any(|ch| ch.is_ascii_lowercase()) {
        IdentifierCase::Pascal
    } else {
        IdentifierCase::NonPascal
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use anyhow::Context;

    use super::IdentifierCase;
    use super::InlinePathVisitor;
    use super::is_pascal_case;
    use crate::fixes::imports::ConditionalAttributes;
    use crate::fixes::imports::ImportSource;

    const ABSOLUTE_PATH_REPLACEMENTS: [(&str, &str); 3] = [
        (
            "fn f() { ::dep::events::PointerCancel {}; }",
            "fn f() { PointerCancel {}; }",
        ),
        (
            "fn f(_: ::dep::events::Widget<u8>) {}",
            "fn f(_: Widget<u8>) {}",
        ),
        (
            "fn f() { ::dep::events::Event::Cancel; }",
            "fn f() { Event::Cancel; }",
        ),
    ];

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
    fn absolute_path_replacements_remove_the_anchor_and_keep_generic_arguments()
    -> anyhow::Result<()> {
        for (text, expected) in ABSOLUTE_PATH_REPLACEMENTS {
            let visitor = inspect(text)?;
            assert_eq!(visitor.occurrences.len(), 1);
            let occurrence = &visitor.occurrences[0];
            let source = ImportSource::new(text);
            let location = source
                .locate(occurrence.span_start, occurrence.span_end)
                .context("locate the absolute path replacement")?;
            let mut rewritten = text.to_string();
            rewritten.replace_range(location.start()..location.end(), &occurrence.replacement);
            assert_eq!(rewritten, expected);
        }
        Ok(())
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
        assert_eq!(is_pascal_case("MyType"), IdentifierCase::Pascal);
        assert_eq!(is_pascal_case("Thing"), IdentifierCase::Pascal);
        assert_eq!(is_pascal_case("PublicContainer"), IdentifierCase::Pascal);
        assert_eq!(is_pascal_case("Foo"), IdentifierCase::Pascal);
    }

    #[test]
    fn pascal_case_rejects_functions() {
        assert_eq!(is_pascal_case("do_thing"), IdentifierCase::NonPascal);
        assert_eq!(is_pascal_case("func_a"), IdentifierCase::NonPascal);
    }

    #[test]
    fn pascal_case_rejects_constants() {
        assert_eq!(is_pascal_case("MAX_SIZE"), IdentifierCase::NonPascal);
        assert_eq!(is_pascal_case("A"), IdentifierCase::NonPascal);
    }

    #[test]
    fn pascal_case_rejects_empty() {
        assert_eq!(is_pascal_case(""), IdentifierCase::NonPascal);
    }
}
