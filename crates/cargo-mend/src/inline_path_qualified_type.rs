use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use proc_macro2::LineColumn;
use syn::ExprPath;
use syn::ExprStruct;
use syn::Item;
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
use syn::TypePath;
use syn::UseTree;
use syn::spanned::Spanned;
use syn::visit::Visit;
use walkdir::WalkDir;

use super::config::DiagnosticCode;
use super::diagnostics::Finding;
use super::diagnostics::Severity;
use super::fix_support::FixSupport;
use super::imports::ImportGroup;
use super::imports::UseFix;
use super::imports::ValidatedFixSet;
use super::module_paths;
use super::selection::Selection;

pub(crate) struct InlinePathScan {
    pub findings: Vec<Finding>,
    pub fixes:    ValidatedFixSet,
}

pub(crate) fn scan_selection(selection: &Selection) -> Result<InlinePathScan> {
    let mut all_findings = Vec::new();
    let mut all_fixes = Vec::new();
    for package_root in &selection.package_roots {
        let source_root = package_root.join("src");
        if !source_root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&source_root)
            .into_iter()
            .filter_map(Result::ok)
        {
            let path = entry.path();
            if !entry.file_type().is_file()
                || path.extension().and_then(OsStr::to_str) != Some("rs")
            {
                continue;
            }
            let (findings, fixes) =
                scan_file(selection.analysis_root.as_path(), &source_root, path)?;
            all_findings.extend(findings);
            all_fixes.extend(fixes);
        }
    }
    all_findings.sort_by(|a, b| (&a.path, a.line, a.column).cmp(&(&b.path, b.line, b.column)));
    all_findings.dedup_by(|a, b| a.path == b.path && a.line == b.line && a.column == b.column);
    Ok(InlinePathScan {
        findings: all_findings,
        fixes:    ValidatedFixSet::try_from(all_fixes)?,
    })
}

struct InlinePathOccurrence {
    /// The original fully-qualified path as written (e.g.
    /// `crate::project::RustProject::Package`).
    full_path:   String,
    /// The path we intend to add as a `use` statement (e.g.
    /// `crate::project::RustProject`). For enum variants this is the parent
    /// type, not the variant itself.
    import_path: String,
    /// The bare last-segment of `import_path` — the name brought into scope
    /// by the `use` (e.g. `RustProject`). Used for collision detection.
    import_name: String,
    /// What replaces the inline fully-qualified path in the source. For an
    /// enum variant this is `Enum::Variant`; for a plain type it is `Type`.
    replacement: String,
    span_start:  LineColumn,
    span_end:    LineColumn,
}

struct ScopeInfo {
    span_start:       usize,
    span_end:         usize,
    insertion_offset: usize,
    indent:           String,
    module_path:      Vec<String>,
    existing_imports: BTreeSet<String>,
}

#[derive(Clone, Copy)]
struct ScopeSpan {
    start: usize,
    end:   usize,
}

impl ScopeSpan {
    const fn new(start: usize, end: usize) -> Self { Self { start, end } }
}

struct ScopeCollectionContext<'a> {
    text:    &'a str,
    offsets: &'a [usize],
    scopes:  &'a mut Vec<ScopeInfo>,
}

struct OccurrenceContext<'a> {
    path:            &'a Path,
    display_path:    &'a str,
    text:            &'a str,
    offsets:         &'a [usize],
    scopes:          &'a [ScopeInfo],
    collision_names: &'a BTreeSet<String>,
}

fn process_occurrence(
    occ: &InlinePathOccurrence,
    ctx: &OccurrenceContext<'_>,
    inserted_use_paths: &mut BTreeSet<(usize, String)>,
    findings: &mut Vec<Finding>,
    fixes: &mut Vec<UseFix>,
) {
    if ctx.collision_names.contains(&occ.import_name) {
        return;
    }

    // Importing a name that already has prelude meaning (`Result`, `Option`,
    // `Vec`, ...) at the top of a file silently changes what every future
    // bare reference to that name resolves to. Even when nothing in the file
    // currently writes bare `Result<T, E>`, adding `use io::Result;` is a
    // correctness footgun the moment anyone edits the file. Skip these
    // outright rather than rely on shadow detection.
    if shadows_prelude(&occ.import_name) {
        return;
    }

    let byte_start = offset(ctx.text, ctx.offsets, occ.span_start);
    let byte_end = offset(ctx.text, ctx.offsets, occ.span_end);

    let scope_id = find_innermost_scope(ctx.scopes, byte_start);
    let scope = scope_id.map(|id| &ctx.scopes[id]);

    // Resolve a partial path (`fmt::Display` written because `use std::fmt;`
    // is in scope) to its absolute form (`std::fmt::Display`) so the inserted
    // `use` is self-contained — i.e. it doesn't silently break if the parent
    // module import is later removed or reordered.
    let import_path = scope.map_or_else(
        || occ.import_path.clone(),
        |scope| absolutize_import_path(&occ.import_path, &scope.existing_imports),
    );

    let source_line = ctx
        .text
        .lines()
        .nth(occ.span_start.line.saturating_sub(1))
        .unwrap_or_default()
        .to_string();

    findings.push(Finding {
        severity: Severity::Warning,
        code: DiagnosticCode::InlinePathQualifiedType,
        path: ctx.display_path.to_string(),
        line: occ.span_start.line,
        column: occ.span_start.column + 1,
        highlight_len: occ.full_path.len().max(1),
        source_line,
        item: None,
        message: format!(
            "use a `use` import for `{}` instead of inline path",
            occ.import_name
        ),
        suggestion: Some(format!("consider adding: `use {import_path};`")),
        fixability: FixSupport::InlinePathQualifiedType,
        related: None,
    });

    // Group the rewrite and its companion `use` insertion so the combining
    // layer can drop them together on cross-pass name collisions.
    let group = Some(ImportGroup {
        bare_name: occ.import_name.clone(),
        full_path: import_path.clone(),
    });

    fixes.push(UseFix {
        path:         ctx.path.to_path_buf(),
        start:        byte_start,
        end:          byte_end,
        replacement:  occ.replacement.clone(),
        import_group: group.clone(),
    });

    let Some(scope_id) = scope_id else {
        return;
    };
    let scope = &ctx.scopes[scope_id];

    if !scope.existing_imports.contains(&import_path)
        && inserted_use_paths.insert((scope_id, import_path.clone()))
    {
        let use_path = canonicalize_inserted_use_path(scope, &import_path);
        let use_text = format!("{}use {use_path};\n", scope.indent);
        fixes.push(UseFix {
            path:         ctx.path.to_path_buf(),
            start:        scope.insertion_offset,
            end:          scope.insertion_offset,
            replacement:  use_text,
            import_group: group,
        });
    }
}

/// Names with prelude meaning. Importing any of these from a non-prelude
/// path silently shadows the prelude binding for the rest of the file —
/// `use io::Result;` makes future `Result<T, E>` references resolve to the
/// `std::io::Result<T>` type alias instead of the generic prelude `Result`.
/// Conservative list: prelude types and the most commonly-derived prelude
/// traits, all from std prelude v1 / 2021 / 2024.
fn shadows_prelude(name: &str) -> bool {
    matches!(
        name,
        "Box"
            | "Option"
            | "Result"
            | "String"
            | "Vec"
            | "Clone"
            | "Copy"
            | "Debug"
            | "Default"
            | "Drop"
            | "Eq"
            | "Fn"
            | "FnMut"
            | "FnOnce"
            | "From"
            | "Hash"
            | "Into"
            | "IntoIterator"
            | "Iterator"
            | "PartialEq"
            | "PartialOrd"
            | "Send"
            | "Sized"
            | "Sync"
            | "ToOwned"
            | "ToString"
            | "TryFrom"
            | "TryInto"
            | "Unpin"
    )
}

/// Resolve a partial path like `fmt::Display` against the file's existing
/// imports. If `use std::fmt;` is already in scope, `fmt::Display` becomes
/// `std::fmt::Display`. The returned import is self-contained — it doesn't
/// rely on a sibling module import staying in place.
fn absolutize_import_path(import_path: &str, existing_imports: &BTreeSet<String>) -> String {
    let Some((leading, rest)) = import_path.split_once("::") else {
        return import_path.to_string();
    };
    if leading == "crate" || leading == "super" || leading == "self" {
        return import_path.to_string();
    }
    // Look for an existing `use a::b::<leading>;` (i.e. an import whose final
    // segment matches `leading` and which has at least one parent segment).
    // Without a parent segment, the existing import is itself a top-level
    // crate name — already absolute.
    for existing in existing_imports {
        let Some((parent, last)) = existing.rsplit_once("::") else {
            continue;
        };
        if last == leading {
            return format!("{parent}::{leading}::{rest}");
        }
    }
    import_path.to_string()
}

fn scan_file(
    analysis_root: &Path,
    source_root: &Path,
    path: &Path,
) -> Result<(Vec<Finding>, Vec<UseFix>)> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let syntax =
        syn::parse_file(&text).with_context(|| format!("failed to parse {}", path.display()))?;
    let offsets = line_offsets(&text);
    let base_module_path = module_paths::file_module_path(source_root, path)
        .with_context(|| format!("failed to determine module path for {}", path.display()))?;
    let mut scopes = Vec::new();
    let mut scope_collection_context = ScopeCollectionContext {
        text:    &text,
        offsets: &offsets,
        scopes:  &mut scopes,
    };
    collect_scopes(
        &syntax.items,
        ScopeSpan::new(0, text.len()),
        &base_module_path,
        &mut scope_collection_context,
    );

    // Visit the AST to find inline path-qualified types
    let mut visitor = InlinePathVisitor {
        occurrences:     Vec::new(),
        bare_type_names: BTreeSet::new(),
        mod_depth:       0,
    };
    visitor.visit_file(&syntax);

    if visitor.occurrences.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let collision_names = find_collision_names(
        &visitor.occurrences,
        &visitor.bare_type_names,
        &scopes
            .iter()
            .flat_map(|scope| scope.existing_imports.iter().cloned())
            .collect(),
    );

    let display_path = path
        .strip_prefix(analysis_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    let mut findings = Vec::new();
    let mut fixes = Vec::new();
    let mut inserted_use_paths: BTreeSet<(usize, String)> = BTreeSet::new();

    let ctx = OccurrenceContext {
        path,
        display_path: &display_path,
        text: &text,
        offsets: &offsets,
        scopes: &scopes,
        collision_names: &collision_names,
    };

    for occ in &visitor.occurrences {
        process_occurrence(
            occ,
            &ctx,
            &mut inserted_use_paths,
            &mut findings,
            &mut fixes,
        );
    }

    Ok((findings, fixes))
}

fn collect_scopes(
    items: &[Item],
    span: ScopeSpan,
    module_path: &[String],
    scope_collection_context: &mut ScopeCollectionContext<'_>,
) {
    let mut existing_imports = BTreeSet::new();
    let mut last_use_start = None;
    let mut last_use_end = None;
    let mut first_item_start = None;

    for item in items {
        let item_start = offset(
            scope_collection_context.text,
            scope_collection_context.offsets,
            item.span().start(),
        );
        first_item_start.get_or_insert(item_start);

        if let Item::Use(item_use) = item {
            if let Some(import_path) = flatten_use_path(&item_use.tree) {
                existing_imports.insert(import_path);
            }
            last_use_start = Some(item_start);
            let item_end = offset(
                scope_collection_context.text,
                scope_collection_context.offsets,
                item_use.span().end(),
            );
            last_use_end = Some(
                if scope_collection_context.text.as_bytes().get(item_end) == Some(&b'\n') {
                    item_end + 1
                } else {
                    item_end
                },
            );
        }
    }

    let anchor_offset = last_use_start.or(first_item_start).unwrap_or(span.start);
    let insertion_offset = last_use_end.or(first_item_start).unwrap_or(span.end);
    let indent = indentation_at(scope_collection_context.text, anchor_offset);
    scope_collection_context.scopes.push(ScopeInfo {
        span_start: span.start,
        span_end: span.end,
        insertion_offset,
        indent,
        module_path: module_path.to_vec(),
        existing_imports,
    });

    for item in items {
        if let Item::Mod(item_mod) = item
            && let Some((_, child_items)) = &item_mod.content
        {
            let mut child_module_path = module_path.to_vec();
            child_module_path.push(item_mod.ident.to_string());
            collect_scopes(
                child_items,
                ScopeSpan::new(
                    offset(
                        scope_collection_context.text,
                        scope_collection_context.offsets,
                        item_mod.span().start(),
                    ),
                    offset(
                        scope_collection_context.text,
                        scope_collection_context.offsets,
                        item_mod.span().end(),
                    ),
                ),
                &child_module_path,
                scope_collection_context,
            );
        }
    }
}

fn find_innermost_scope(scopes: &[ScopeInfo], byte_offset: usize) -> Option<usize> {
    scopes
        .iter()
        .enumerate()
        .filter(|(_, scope)| scope.span_start <= byte_offset && byte_offset < scope.span_end)
        .max_by_key(|(_, scope)| (scope.span_start, Reverse(scope.span_end)))
        .map(|(scope_id, _)| scope_id)
}

fn indentation_at(text: &str, byte_offset: usize) -> String {
    let line_start = text[..byte_offset]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    text[line_start..byte_offset]
        .chars()
        .take_while(char::is_ascii_whitespace)
        .collect()
}

fn canonicalize_inserted_use_path(scope: &ScopeInfo, full_path: &str) -> String {
    let segments: Vec<&str> = full_path.split("::").collect();
    let super_count = segments
        .iter()
        .take_while(|segment| **segment == "super")
        .count();
    if super_count < 2 || super_count > scope.module_path.len() {
        return full_path.to_string();
    }

    let mut absolute_segments = Vec::with_capacity(1 + scope.module_path.len() + segments.len());
    absolute_segments.push("crate".to_string());
    absolute_segments.extend(
        scope.module_path[..scope.module_path.len() - super_count]
            .iter()
            .cloned(),
    );
    absolute_segments.extend(
        segments[super_count..]
            .iter()
            .map(|segment| (*segment).to_string()),
    );
    absolute_segments.join("::")
}

/// Finds type names that cannot be safely imported because they either:
/// - map to multiple distinct paths (ambiguous), or
/// - are already used bare in the file (importing would shadow the existing usage, e.g. prelude
///   `Result<T, E>` shadowed by `use crate::error::Result;`).
fn find_collision_names(
    occurrences: &[InlinePathOccurrence],
    bare_type_names: &BTreeSet<String>,
    existing_imports: &BTreeSet<String>,
) -> BTreeSet<String> {
    // Group by the name that will be brought into scope by the `use` (the
    // `import_name`), and track the set of distinct import paths per name.
    // If more than one distinct path maps to the same import name, the
    // imports would collide — skip them all.
    let mut name_to_paths: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for occ in occurrences {
        name_to_paths
            .entry(&occ.import_name)
            .or_default()
            .insert(&occ.import_path);
    }

    let mut collisions = BTreeSet::new();
    for (name, paths) in &name_to_paths {
        let ambiguous = paths.len() > 1;
        // If the name is already used bare somewhere in the file (e.g.
        // `use super::*` brings in a struct `Package`), introducing a new
        // `use crate::other::Package;` would shadow it.
        let would_shadow =
            bare_type_names.contains(*name) && !paths.iter().all(|p| existing_imports.contains(*p));
        if ambiguous || would_shadow {
            collisions.insert((*name).to_owned());
        }
    }
    collisions
}

// --- AST Visitor ---

struct InlinePathVisitor {
    occurrences:     Vec<InlinePathOccurrence>,
    bare_type_names: BTreeSet<String>,
    mod_depth:       usize,
}

impl InlinePathVisitor {
    fn check_path(&mut self, path: &syn::Path) {
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();

        if segments.len() < 2 {
            return;
        }

        let first = &segments[0];
        let is_intra_crate = first == "crate" || first == "super";

        // For intra-crate paths, require at least 3 segments — `crate::Foo` /
        // `super::Foo` are already short enough that hoisting them to a `use`
        // is churn. For external-crate paths (`ratatui::Frame`,
        // `std::collections::BTreeMap`), 2 segments is the minimum and worth
        // hoisting.
        if is_intra_crate && segments.len() < 3 {
            return;
        }

        // Filter obvious non-crate roots. `self::` is a same-module reference,
        // not a candidate for a `use`.
        if first == "self" || first == "Self" {
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
    fn record_bare_name_footprint(&mut self, path: &syn::Path) {
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
        syn::visit::visit_type_path(self, node);
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
        syn::visit::visit_expr_struct(self, node);
    }

    fn visit_pat_struct(&mut self, node: &PatStruct) {
        // `Foo { .. }` and `crate::foo::Bar { .. }` in pattern position
        // (`let Bar { .. } = ...`, match arms) — also not visited by
        // `visit_expr_path` / `visit_type_path`.
        if node.qself.is_none() {
            self.check_path(&node.path);
            self.record_bare_name_footprint(&node.path);
        }
        syn::visit::visit_pat_struct(self, node);
    }

    fn visit_item_mod(&mut self, node: &ItemMod) {
        // Track nesting depth so item-name registration below can gate on
        // "is this at the file's top level". Names defined inside nested
        // modules don't collide with imports added at the top level.
        self.mod_depth += 1;
        syn::visit::visit_item_mod(self, node);
        self.mod_depth -= 1;
    }

    fn visit_item_struct(&mut self, node: &ItemStruct) {
        if self.mod_depth == 0 {
            self.bare_type_names.insert(node.ident.to_string());
        }
        syn::visit::visit_item_struct(self, node);
    }

    fn visit_item_enum(&mut self, node: &ItemEnum) {
        if self.mod_depth == 0 {
            self.bare_type_names.insert(node.ident.to_string());
        }
        syn::visit::visit_item_enum(self, node);
    }

    fn visit_item_type(&mut self, node: &ItemType) {
        if self.mod_depth == 0 {
            self.bare_type_names.insert(node.ident.to_string());
        }
        syn::visit::visit_item_type(self, node);
    }

    fn visit_item_trait(&mut self, node: &ItemTrait) {
        if self.mod_depth == 0 {
            self.bare_type_names.insert(node.ident.to_string());
        }
        syn::visit::visit_item_trait(self, node);
    }

    fn visit_item_fn(&mut self, node: &ItemFn) {
        // A free function with a PascalCase name would also collide with an
        // imported type of the same name. Rare, but cheap to track.
        if self.mod_depth == 0 {
            self.bare_type_names.insert(node.sig.ident.to_string());
        }
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_item_const(&mut self, node: &ItemConst) {
        if self.mod_depth == 0 {
            self.bare_type_names.insert(node.ident.to_string());
        }
        syn::visit::visit_item_const(self, node);
    }

    fn visit_item_static(&mut self, node: &ItemStatic) {
        if self.mod_depth == 0 {
            self.bare_type_names.insert(node.ident.to_string());
        }
        syn::visit::visit_item_static(self, node);
    }

    fn visit_item_impl(&mut self, node: &ItemImpl) {
        // `impl Trait for Type` — the trait path is `ItemImpl::trait_`, a bare
        // `syn::Path` not visited as a `TypePath`. Inspect it directly.
        if let Some((_, trait_path, _)) = &node.trait_ {
            self.check_path(trait_path);
            if trait_path.segments.len() == 1 {
                let name = trait_path.segments[0].ident.to_string();
                if is_pascal_case(&name) {
                    self.bare_type_names.insert(name);
                }
            }
        }
        syn::visit::visit_item_impl(self, node);
    }

    fn visit_pat_tuple_struct(&mut self, node: &PatTupleStruct) {
        // `Foo(..)` in pattern position — e.g. `let Foo(x) = ...` or
        // `Some(Enum::Variant(x))` match arms.
        if node.qself.is_none() {
            self.check_path(&node.path);
            self.record_bare_name_footprint(&node.path);
        }
        syn::visit::visit_pat_tuple_struct(self, node);
    }
}

// --- Helpers ---

fn flatten_use_path(tree: &UseTree) -> Option<String> {
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

fn is_pascal_case(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };
    first.is_ascii_uppercase() && name.chars().any(|ch| ch.is_ascii_lowercase())
}

fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (idx, ch) in text.char_indices() {
        if ch == '\n' {
            offsets.push(idx + 1);
        }
    }
    offsets
}

fn offset(text: &str, line_offsets: &[usize], position: LineColumn) -> usize {
    let line_start = line_offsets
        .get(position.line.saturating_sub(1))
        .copied()
        .unwrap_or(0);
    // `proc_macro2::LineColumn::column` is a 0-based count of UTF-8 *characters*
    // from the start of the line, not bytes. Walk char_indices to convert to a
    // byte offset so multi-byte characters (em-dashes, accented letters, etc.)
    // earlier on the same line don't shift the replacement window.
    let line_text = text.get(line_start..).unwrap_or("");
    let byte_in_line = line_text
        .char_indices()
        .nth(position.column)
        .map_or(line_text.len(), |(byte_idx, _)| byte_idx);
    line_start + byte_in_line
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
