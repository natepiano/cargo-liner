use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;
use proc_macro2::LineColumn;
use syn::Arm;
use syn::Attribute;
use syn::Block;
use syn::Expr;
use syn::File;
use syn::Item;
use syn::ItemMod;
use syn::ItemUse;
use syn::Stmt;
use syn::UseTree;
use syn::spanned::Spanned;
use syn::visit;
use syn::visit::Visit;

use crate::config::DiagnosticCode;
use crate::fixes::constants::IMPORTS_AT_TOP_MESSAGE;
use crate::fixes::constants::IMPORTS_AT_TOP_SUGGESTION;
use crate::fixes::imports;
use crate::fixes::imports::ConditionalAttributes;
use crate::fixes::imports::ImportGroup;
use crate::fixes::imports::ImportSource;
use crate::fixes::imports::UseFix;
use crate::reporting::Finding;
use crate::reporting::FixSupport;
use crate::reporting::ItemVisibility;
use crate::reporting::Severity;

/// What the scope already exposes under a given bare name. Use-imports
/// track the resolved path so a same-path move dedupes cleanly, plus whether
/// they are `#[cfg]`-gated — a conditional import must never dedup against or
/// be added next to an unconditional one of the same name (both active at
/// once is a duplicate-import error). Other items (structs, fns, mods,
/// consts, …) carry no path — any move that would reintroduce their name at
/// this scope is treated as a hard collision and the in-body `use` is left
/// alone.
enum ExistingBinding {
    Use { full: String, gate: ImportGate },
    Item,
}

/// The collision and duplicate policy carried by an import's compile gate.
#[derive(Clone, Copy)]
enum ImportGate {
    Conditional,
    Unconditional,
}

impl From<&ConditionalAttributes> for ImportGate {
    fn from(attributes: &ConditionalAttributes) -> Self {
        if attributes.is_empty() {
            Self::Unconditional
        } else {
            Self::Conditional
        }
    }
}

impl From<&[Attribute]> for ImportGate {
    fn from(attributes: &[Attribute]) -> Self {
        if ConditionalAttributes::contains(attributes) {
            Self::Conditional
        } else {
            Self::Unconditional
        }
    }
}

impl ImportGate {
    fn blocks(self, existing: Option<&ExistingBinding>, full: &str) -> bool {
        match self {
            Self::Conditional => existing.is_some(),
            Self::Unconditional => unconditional_collision(existing, full),
        }
    }

    fn all_duplicates(self, scope: &ScopedUseFinder, bare_paths: &[NamedImport]) -> bool {
        match self {
            Self::Conditional => false,
            Self::Unconditional => bare_paths.iter().all(|binding| {
                let NamedImport {
                    name: bare,
                    path: full,
                } = binding;
                matches!(scope.existing.get(bare),
                    Some(ExistingBinding::Use { full: existing, gate: Self::Unconditional })
                        if existing == full)
            }),
        }
    }
}

/// An active module scope required by every import move and insertion.
/// The file scan installs the root scope, and entering a module replaces it
/// until that module's traversal finishes; emission never has an absent scope.
struct ScopedUseFinder {
    /// Byte offset where new `use` lines insert at the top of the scope.
    /// Sits just after the last top-of-scope `use`, or at the first
    /// non-use item, or at the start of the scope when neither exists.
    insertion_offset: usize,
    /// Indent (spaces and/or tabs) to prepend to inserted `use` lines.
    indent:           String,
    /// `bare_name -> what's already in scope under that name`. Used to
    /// detect collisions and duplicates before moving an in-body `use`.
    existing:         BTreeMap<String, ExistingBinding>,
}

struct InBodyUseFinder<'a> {
    source:                 &'a ImportSource<'a>,
    text:                   &'a str,
    path:                   &'a Path,
    display_path:           &'a str,
    scope:                  ScopedUseFinder,
    /// Source text of the `#[cfg]`/`#[cfg_attr]` attributes on every enclosing
    /// construct the visitor is currently inside, outermost first. A nested
    /// `use` that moves to the module scope carries these attributes with it so
    /// the import stays conditionally compiled. Emptied on entering an inline
    /// module (an inner `use` moves only to that module's top, which the
    /// module's own gate already wraps).
    conditional_attributes: ConditionalAttributes,
    result:                 anyhow::Result<()>,
    findings:               Vec<Finding>,
    fixes:                  Vec<UseFix>,
}

impl InBodyUseFinder<'_> {
    fn attributes(&self, attributes: &[Attribute]) -> ConditionalAttributes {
        ConditionalAttributes::from_attributes(self.source, attributes)
    }

    fn try_emit_fix(&mut self, use_item: &ItemUse) -> anyhow::Result<()> {
        // `InBodyUseFinder::try_emit_fix` leaves `use_item` in place when a non-`cfg`
        // attribute on the `use` itself (`#[allow(...)]`, a doc comment, …) can change
        // what the import means.
        if use_item
            .attrs
            .iter()
            .any(|attribute| !imports::is_conditional(attribute))
        {
            return Ok(());
        }

        let Ok(bare_paths) = flatten_use_to_bare_paths(&use_item.tree)
            .map(ImportLeaf::into_named)
            .collect::<Result<Vec<_>, _>>()
        else {
            return Ok(());
        };
        if bare_paths.is_empty() {
            return Ok(());
        }

        // `conditional_attributes` combines every enclosing `#[cfg]` plus
        // any `#[cfg]` on the `use` itself. Non-empty means the import is
        // conditionally compiled and the attributes travel with it to the top.
        let mut conditional_attributes = self.conditional_attributes.clone();
        conditional_attributes.extend(self.attributes(&use_item.attrs));
        let gate = ImportGate::from(&conditional_attributes);

        let scope = &self.scope;

        // `blocked` rejects a gated `use` with any `scope.existing` binding of the
        // same name (even the same path double-imports when both are active),
        // so skip when the name is taken. An unconditional `use` skips only on
        // a real collision. Leaving it in place is safe on every target.
        let blocked = bare_paths
            .iter()
            .any(|binding| gate.blocks(scope.existing.get(&binding.name), &binding.path));
        if blocked {
            return Ok(());
        }

        // `all_duplicates` means every bare name is already imported unconditionally with the
        // same full path. The fix collapses to a pure deletion (no insertion).
        // A gated use is never a plain duplicate — its gate must be preserved.
        let all_duplicates = gate.all_duplicates(scope, &bare_paths);

        self.emit_move(
            use_item,
            &bare_paths,
            &conditional_attributes,
            gate,
            all_duplicates,
        )
    }

    /// Push the finding and the insertion/deletion fixes for a movable `use`.
    /// `gate_attrs` are prepended to the inserted line so the import stays
    /// conditionally compiled; `all_duplicates` skips the insertion (the names
    /// are already imported at the scope top, so the fix is a pure deletion).
    fn emit_move(
        &mut self,
        use_item: &ItemUse,
        bare_paths: &[NamedImport],
        conditional_attributes: &ConditionalAttributes,
        gate: ImportGate,
        all_duplicates: bool,
    ) -> anyhow::Result<()> {
        let use_span = use_item.span();
        // `use_span` includes the first attribute. Build the moved text from
        // `use_item.use_token` so `conditional_attributes` are emitted once.
        let attr_start = SourcePosition::new(self.source, use_span.start())?.offset;
        let use_kw = use_item.use_token.span().start();
        let use_location = SourcePosition::new(self.source, use_kw)?;
        let use_kw_start = use_location.offset;
        let use_end = SourcePosition::new(self.source, use_span.end())?.offset;
        let use_end_with_nl = if self.text.as_bytes().get(use_end) == Some(&b'\n') {
            use_end + 1
        } else {
            use_end
        };
        let line_start = line_start_offset(self.text, attr_start);
        let leading = &self.text[line_start..attr_start];
        let delete_start = if leading
            .chars()
            .all(|character| character == ' ' || character == '\t')
        {
            line_start
        } else {
            attr_start
        };

        let group = bare_paths.first().map(|binding| ImportGroup {
            bare_name: binding.name.clone(),
            full_path: binding.path.clone(),
        });

        self.findings.push(Finding {
            disposition:     Finding::disposition(
                DiagnosticCode::ImportsAtTop,
                Severity::Warning,
                FixSupport::ImportsAtTop,
            ),
            diagnostic_code: DiagnosticCode::ImportsAtTop,
            path:            self.display_path.to_string(),
            line:            use_kw.line,
            column:          use_kw.column + 1,
            highlight_len:   (use_end - use_kw_start).max(1),
            source_line:     use_location.source_line.to_string(),
            item:            None,
            message:         IMPORTS_AT_TOP_MESSAGE.to_string(),
            suggestion:      Some(IMPORTS_AT_TOP_SUGGESTION.to_string()),
            related:         None,
            item_visibility: ItemVisibility::default(),
        });

        if !all_duplicates {
            self.emit_insertion(
                bare_paths,
                conditional_attributes,
                gate,
                use_kw_start,
                use_end,
                group.as_ref(),
            );
        }

        self.fixes.push(UseFix {
            path:         self.path.to_path_buf(),
            start:        delete_start,
            end:          use_end_with_nl,
            replacement:  String::new(),
            import_group: group,
        });
        Ok(())
    }

    /// Insert the moved `use` (with its carried gate) at the scope top and
    /// record the newly imported names so later in-body uses in this pass see
    /// them as already imported. The `gate` stays with the binding so a
    /// conditional move never dedups against an unconditional import of the same name.
    fn emit_insertion(
        &mut self,
        bare_paths: &[NamedImport],
        conditional_attributes: &ConditionalAttributes,
        gate: ImportGate,
        use_kw_start: usize,
        use_end: usize,
        group: Option<&ImportGroup>,
    ) {
        let indent = self.scope.indent.clone();
        let insertion_offset = self.scope.insertion_offset;

        let use_text = &self.text[use_kw_start..use_end];
        let mut insertion = conditional_attributes.render(&indent);
        insertion.push_str(&indent);
        insertion.push_str(use_text);
        insertion.push('\n');
        self.fixes.push(UseFix {
            path:         self.path.to_path_buf(),
            start:        insertion_offset,
            end:          insertion_offset,
            replacement:  insertion,
            import_group: group.cloned(),
        });

        for binding in bare_paths {
            self.scope.existing.insert(
                binding.name.clone(),
                ExistingBinding::Use {
                    full: binding.path.clone(),
                    gate,
                },
            );
        }
    }
}

impl<'ast> Visit<'ast> for InBodyUseFinder<'_> {
    fn visit_item(&mut self, node: &'ast Item) {
        if self.result.is_err() {
            return;
        }
        let attributes = self.attributes(item_gate_attrs(node));
        let pushed = attributes.len();
        self.conditional_attributes.extend(attributes);
        visit::visit_item(self, node);
        self.conditional_attributes
            .truncate(self.conditional_attributes.len() - pushed);
    }

    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        let Some((brace, items)) = &node.content else {
            return;
        };
        let nested_default = format!("{}    ", self.scope.indent);
        let scope = SourcePosition::new(self.source, brace.span.open().end()).and_then(|opening| {
            compute_scope_for_items(
                items,
                self.text,
                self.source,
                &nested_default,
                opening.offset,
            )
        });
        let scope = match scope {
            Ok(scope) => scope,
            Err(error) => {
                self.result = Err(error);
                return;
            },
        };
        // `visit_item_mod` clears `self.conditional_attributes` because `node` opens its own scope:
        // an inner `use` moves only to this module's top, which the module's own gate
        // already wraps, so gates from enclosing constructs no longer apply. Clear the
        // stack for the body and restore it afterward.
        let outer_attributes = std::mem::take(&mut self.conditional_attributes);
        let parent_scope = std::mem::replace(&mut self.scope, scope);
        for item in items {
            self.visit_item(item);
        }
        self.scope = parent_scope;
        self.conditional_attributes = outer_attributes;
    }

    fn visit_stmt(&mut self, node: &'ast Stmt) {
        let attributes = self.attributes(stmt_gate_attrs(node));
        let pushed = attributes.len();
        self.conditional_attributes.extend(attributes);
        visit::visit_stmt(self, node);
        self.conditional_attributes
            .truncate(self.conditional_attributes.len() - pushed);
    }

    fn visit_arm(&mut self, node: &'ast Arm) {
        let attributes = self.attributes(&node.attrs);
        let pushed = attributes.len();
        self.conditional_attributes.extend(attributes);
        visit::visit_arm(self, node);
        self.conditional_attributes
            .truncate(self.conditional_attributes.len() - pushed);
    }

    fn visit_block(&mut self, node: &'ast Block) {
        for stmt in &node.stmts {
            if self.result.is_err() {
                return;
            }
            match stmt {
                Stmt::Item(Item::Use(use_item)) => {
                    if let Err(error) = self.try_emit_fix(use_item) {
                        self.result = Err(error);
                        return;
                    }
                },
                Stmt::Item(_) | Stmt::Local(_) | Stmt::Expr(_, _) | Stmt::Macro(_) => {
                    self.visit_stmt(stmt);
                },
            }
        }
    }
}

pub(super) fn scan(
    syntax: &File,
    text: &str,
    path: &Path,
    display_path: &str,
) -> anyhow::Result<(Vec<Finding>, Vec<UseFix>)> {
    let indexed = ImportSource::new(text);
    let root_scope = compute_scope_for_items(&syntax.items, text, &indexed, "", 0)?;
    let mut visitor = InBodyUseFinder {
        source: &indexed,
        text,
        path,
        display_path,
        scope: root_scope,
        conditional_attributes: ConditionalAttributes::default(),
        result: Ok(()),
        findings: Vec::new(),
        fixes: Vec::new(),
    };
    for item in &syntax.items {
        visitor.visit_item(item);
    }
    visitor.result?;
    Ok((visitor.findings, visitor.fixes))
}

fn compute_scope_for_items(
    items: &[Item],
    text: &str,
    source: &ImportSource<'_>,
    default_indent: &str,
    empty_offset: usize,
) -> anyhow::Result<ScopedUseFinder> {
    let mut existing: BTreeMap<String, ExistingBinding> = BTreeMap::new();
    let mut insertion = ScopeInsertion::EmptyModule(empty_offset);
    let mut detected_indent: Option<String> = None;

    for item in items {
        let item_start_lc = item.span().start();
        let item_start = SourcePosition::new(source, item_start_lc)?.offset;
        if let ScopeInsertion::EmptyModule(_) = insertion {
            insertion = ScopeInsertion::BeforeItem(item_start);
        }
        if detected_indent.is_none() {
            detected_indent = Some(indent_for_offset(text, item_start));
        }
        match item {
            Item::Use(item_use) => {
                let gate = ImportGate::from(item_use.attrs.as_slice());
                for leaf in flatten_use_to_bare_paths(&item_use.tree) {
                    if let ImportLeaf::Named(binding) = leaf {
                        existing.insert(
                            binding.name,
                            ExistingBinding::Use {
                                full: binding.path,
                                gate,
                            },
                        );
                    }
                }
                let end = SourcePosition::new(source, item_use.span().end())?.offset;
                let end = if text.as_bytes().get(end) == Some(&b'\n') {
                    end + 1
                } else {
                    end
                };
                insertion = ScopeInsertion::AfterUse(end);
            },
            other => {
                for name in item_defined_names(other) {
                    existing.entry(name).or_insert(ExistingBinding::Item);
                }
            },
        }
    }

    let insertion_offset = insertion.offset();
    let indent = detected_indent.unwrap_or_else(|| default_indent.to_string());
    Ok(ScopedUseFinder {
        insertion_offset,
        indent,
        existing,
    })
}

/// Bare identifiers that an item introduces at its containing module's
/// scope. Returns an empty vector for items that don't bind a name at
/// module level (`Item::Impl`, `Item::ExternCrate`, `Item::Use`,
/// `Item::Macro`, etc.).
fn item_defined_names(item: &Item) -> Vec<String> {
    match item {
        Item::Struct(item) => vec![item.ident.to_string()],
        Item::Enum(item) => vec![item.ident.to_string()],
        Item::Union(item) => vec![item.ident.to_string()],
        Item::Fn(item) => vec![item.sig.ident.to_string()],
        Item::Const(item) => vec![item.ident.to_string()],
        Item::Static(item) => vec![item.ident.to_string()],
        Item::Mod(item) => vec![item.ident.to_string()],
        Item::Trait(item) => vec![item.ident.to_string()],
        Item::TraitAlias(item) => vec![item.ident.to_string()],
        Item::Type(item) => vec![item.ident.to_string()],
        Item::ExternCrate(item) => item.rename.as_ref().map_or_else(
            || vec![item.ident.to_string()],
            |(_, rename)| vec![rename.to_string()],
        ),
        _ => Vec::new(),
    }
}

fn indent_for_offset(text: &str, offset: usize) -> String {
    let line_start = line_start_offset(text, offset);
    let leading = &text[line_start..offset];
    if leading
        .chars()
        .all(|character| character == ' ' || character == '\t')
    {
        leading.to_string()
    } else {
        String::new()
    }
}

fn line_start_offset(text: &str, offset: usize) -> usize {
    text[..offset]
        .rfind('\n')
        .map_or(0, |position| position + 1)
}

/// A named import that can participate in move and duplicate planning.
#[derive(Debug, PartialEq)]
struct NamedImport {
    name: String,
    path: String,
}

/// Globs may shadow arbitrary names, so only the named phase can be moved.
#[derive(Debug, PartialEq)]
enum ImportLeaf {
    Named(NamedImport),
    Glob { path: String },
}

impl ImportLeaf {
    fn into_named(self) -> Result<NamedImport, String> {
        match self {
            Self::Named(binding) => Ok(binding),
            Self::Glob { path } => Err(path),
        }
    }
}

/// An empty module inserts at its real opening position, supplied by the caller.
/// It never starts at a placeholder zero that needs repair after scope collection.
enum ScopeInsertion {
    AfterUse(usize),
    BeforeItem(usize),
    EmptyModule(usize),
}

impl ScopeInsertion {
    const fn offset(self) -> usize {
        match self {
            Self::AfterUse(offset) | Self::BeforeItem(offset) | Self::EmptyModule(offset) => offset,
        }
    }
}

fn flatten_use_to_bare_paths(tree: &UseTree) -> impl Iterator<Item = ImportLeaf> {
    imports::collect_use_bindings(tree)
        .into_iter()
        .map(|binding| {
            binding.name().map_or_else(
                || ImportLeaf::Glob {
                    path: binding.path().to_string(),
                },
                |name| {
                    ImportLeaf::Named(NamedImport {
                        name: name.to_string(),
                        path: binding.path().to_string(),
                    })
                },
            )
        })
}

/// Whether an attribute is `#[cfg(...)]` or `#[cfg_attr(...)]` — the two
/// attributes that conditionally remove the code they annotate. Moving a
/// `use` out of a construct carrying one would make a conditionally-compiled
/// import unconditional.
/// Attributes on a statement that gate a `use` nested inside it. Items are
/// handled in [`InBodyUseFinder::visit_item`]; a `use` statement's own
/// attributes are handled in [`InBodyUseFinder::try_emit_fix`].
fn stmt_gate_attrs(stmt: &Stmt) -> &[Attribute] {
    match stmt {
        Stmt::Local(local) => &local.attrs,
        Stmt::Macro(stmt_macro) => &stmt_macro.attrs,
        Stmt::Expr(expr, _) => expr_gate_attrs(expr),
        Stmt::Item(_) => &[],
    }
}

/// Attributes on a block-bearing expression. Only expressions that introduce a
/// block can hold a nested `use`, so other variants never gate one.
fn expr_gate_attrs(expr: &Expr) -> &[Attribute] {
    match expr {
        Expr::Block(block) => &block.attrs,
        Expr::Unsafe(block) => &block.attrs,
        Expr::If(branch) => &branch.attrs,
        Expr::Match(branch) => &branch.attrs,
        Expr::Loop(loop_expr) => &loop_expr.attrs,
        Expr::While(loop_expr) => &loop_expr.attrs,
        Expr::ForLoop(loop_expr) => &loop_expr.attrs,
        _ => &[],
    }
}

/// Attributes on an item that can contain a `use` moving past it to the module
/// scope. Modules are excluded: they open their own scope, so an inner `use`
/// moves to the module top and stays within any gate on the module itself.
fn item_gate_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Fn(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::Const(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        _ => &[],
    }
}

/// Whether moving an unconditional `use` of `full` would collide with what
/// the scope already exposes under that bare name: an item, a gated use of any
/// path (an unconditional sibling would double-import when the gate holds), or
/// an unconditional use of a different path.
fn unconditional_collision(existing: Option<&ExistingBinding>, full: &str) -> bool {
    match existing {
        Some(
            ExistingBinding::Item
            | ExistingBinding::Use {
                gate: ImportGate::Conditional,
                ..
            },
        ) => true,
        Some(ExistingBinding::Use {
            full: existing_full,
            gate: ImportGate::Unconditional,
        }) => existing_full != full,
        None => false,
    }
}

/// A source-bound character position checked before move or insertion planning.
/// Invalid lines and columns are rejected instead of becoming byte zero; diagnostics
/// carry the original source line rather than substituting empty text for a missing line.
/// Character columns are converted to byte offsets against that same source.
struct SourcePosition<'a> {
    offset:      usize,
    source_line: &'a str,
}

impl<'a> SourcePosition<'a> {
    fn new(source: &ImportSource<'a>, position: LineColumn) -> anyhow::Result<Self> {
        let location = source
            .locate(position, position)
            .with_context(|| format!("invalid import source position: {position:?}"))?;
        Ok(Self {
            offset:      location.start(),
            source_line: location.source_line(),
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use proc_macro2::LineColumn;
    use syn::File;
    use syn::Item;
    use syn::UseTree;
    use syn::parse_str;

    use super::ImportLeaf;
    use super::NamedImport;
    use super::flatten_use_to_bare_paths;

    fn named(name: &str, path: &str) -> ImportLeaf {
        ImportLeaf::Named(NamedImport {
            name: name.to_string(),
            path: path.to_string(),
        })
    }

    fn parse_tree(source: &str) -> UseTree {
        let file: File = parse_str(source).expect("parse");
        let item = file
            .items
            .into_iter()
            .next()
            .expect("fixture should produce one item");
        let Item::Use(item_use) = item else {
            panic!("expected use item")
        };
        item_use.tree
    }

    #[test]
    fn checked_positions_reject_invalid_lines_and_columns_and_keep_source_text() {
        let text = "// é\nfn f() { use crate::a::Thing; }\n";
        let source = crate::fixes::imports::ImportSource::new(text);
        let at = |line, column| LineColumn { line, column };
        for position in [at(0, 0), at(4, 0), at(2, 99)] {
            assert!(super::SourcePosition::new(&source, position).is_err());
        }
        let position = super::SourcePosition::new(&source, at(2, 9)).expect("use keyword");
        assert_eq!(position.source_line, "fn f() { use crate::a::Thing; }");
        assert_eq!(&text[position.offset..position.offset + 3], "use");
        let end = super::SourcePosition::new(&source, at(3, 0)).expect("final empty line");
        assert_eq!(end.offset, text.len());
        assert_eq!(end.source_line, "");
    }

    #[test]
    fn finder_rejects_mismatched_source_before_returning_edits() {
        let syntax = syn::parse_file("fn f() { use crate::a::Thing; }").expect("fixture");
        assert!(super::scan(&syntax, "", std::path::Path::new("lib.rs"), "lib.rs").is_err());
    }

    #[test]
    fn moved_import_keeps_unicode_diagnostic_and_edit_bytes() -> anyhow::Result<()> {
        let line = "fn f() { /* é */ use crate::a::Thing; }";
        let text = format!("// é\n{line}\n");
        let syntax = syn::parse_file(&text)?;
        let (findings, fixes) =
            super::scan(&syntax, &text, std::path::Path::new("lib.rs"), "lib.rs")?;
        assert_eq!(findings[0].source_line, line);
        assert_eq!(findings[0].line, 2);
        assert_eq!(fixes[0].replacement, "use crate::a::Thing;\n");
        assert_eq!(&text[fixes[1].start..fixes[1].end], "use crate::a::Thing;");
        Ok(())
    }

    #[test]
    fn full_file_scan_uses_one_index_for_gated_and_ungated_nodes() -> anyhow::Result<()> {
        for gate in ["", "#[cfg(any(unix, windows))]\n"] {
            let statements = "value();".repeat(128);
            let text =
                format!("mod nested {{ {gate}fn f() {{ use crate::a::Thing; {statements} }} }}");
            let syntax = syn::parse_file(&text)?;
            let before = crate::fixes::imports::ImportSource::construction_count();
            let (findings, fixes) = super::scan(
                &syntax,
                &text,
                std::path::Path::new("fixture.rs"),
                "fixture.rs",
            )?;
            assert_eq!(
                crate::fixes::imports::ImportSource::construction_count() - before,
                1
            );
            assert_eq!(findings.len(), 1);
            assert!(fixes.iter().any(|fix| {
                fix.replacement
                    .contains(&format!("{gate}use crate::a::Thing;"))
            }));
        }
        Ok(())
    }

    #[test]
    fn scope_insertion_uses_the_actual_empty_module_position() {
        let source = crate::fixes::imports::ImportSource::new("// head\nmod a {}");
        let scope = super::compute_scope_for_items(&[], "// head\nmod a {}", &source, "    ", 15)
            .expect("valid empty scope");
        assert_eq!(scope.insertion_offset, 15);
        assert_eq!(scope.indent, "    ");
        assert_eq!(super::ScopeInsertion::AfterUse(7).offset(), 7);
        assert_eq!(super::ScopeInsertion::BeforeItem(3).offset(), 3);
    }

    #[test]
    fn scoped_moves_restore_the_parent_and_leave_globs_in_place() -> anyhow::Result<()> {
        let source = concat!(
            "mod a {\n    fn f() { use crate::a::A; }\n}\n",
            "fn g() { use crate::b::B; }\n",
            "fn h() { use crate::c::*; }\n",
        );
        let syntax = syn::parse_file(source)?;
        let (findings, fixes) = super::scan(
            &syntax,
            source,
            std::path::Path::new("fixture.rs"),
            "fixture.rs",
        )
        .expect("valid source positions");
        assert_eq!(findings.len(), 2);
        assert_eq!(fixes.len(), 4);
        assert!(fixes[0].replacement.starts_with("    use crate::a::A;"));
        assert!(fixes[2].replacement.starts_with("use crate::b::B;"));
        assert_eq!(fixes[2].start, 0);
        Ok(())
    }

    #[test]
    fn flatten_simple_name() {
        let tree = parse_tree("use crate::foo::Bar;");
        assert_eq!(
            flatten_use_to_bare_paths(&tree).collect::<Vec<_>>(),
            vec![named("Bar", "crate::foo::Bar")]
        );
    }

    #[test]
    fn flatten_rename_uses_rename_as_bare() {
        let tree = parse_tree("use crate::foo::Bar as Renamed;");
        assert_eq!(
            flatten_use_to_bare_paths(&tree).collect::<Vec<_>>(),
            vec![named("Renamed", "crate::foo::Bar")]
        );
    }

    #[test]
    fn flatten_group_expands_into_one_entry_per_leaf() {
        let tree = parse_tree("use crate::foo::{Bar, Baz};");
        let result = flatten_use_to_bare_paths(&tree).collect::<Vec<_>>();
        assert!(
            result.contains(&named("Bar", "crate::foo::Bar")),
            "{result:?}"
        );
        assert!(
            result.contains(&named("Baz", "crate::foo::Baz")),
            "{result:?}"
        );
    }

    #[test]
    fn flatten_glob_names_its_phase() {
        let tree = parse_tree("use crate::foo::*;");
        assert_eq!(
            flatten_use_to_bare_paths(&tree).collect::<Vec<_>>(),
            vec![ImportLeaf::Glob {
                path: "crate::foo".to_string(),
            }]
        );
    }
    #[test]
    fn moving_imports_preserves_gate_collisions_and_duplicate_deletion() -> anyhow::Result<()> {
        for (existing, nested, findings, fixes) in [
            ("use crate::a::Thing;", "use crate::a::Thing;", 1, 1),
            (
                "use crate::a::Thing;",
                "#[cfg(test)] use crate::a::Thing;",
                0,
                0,
            ),
            (
                "#[cfg(test)] use crate::a::Thing;",
                "use crate::a::Thing;",
                0,
                0,
            ),
            ("use crate::b::Thing;", "use crate::a::Thing;", 0, 0),
            ("struct Thing;", "use crate::a::Thing;", 0, 0),
            ("", "#[cfg(test)] use crate::a::Thing;", 1, 2),
            ("", "use crate::a::Thing;", 1, 2),
        ] {
            let source = format!("{existing}\nfn f() {{ {nested} }}\n");
            let syntax = syn::parse_file(&source).expect("parse import fixture");
            let (actual_findings, actual_fixes) = super::scan(
                &syntax,
                &source,
                std::path::Path::new("fixture.rs"),
                "fixture.rs",
            )?;
            assert_eq!(
                (actual_findings.len(), actual_fixes.len()),
                (findings, fixes),
                "{source}"
            );
            if existing.is_empty() {
                let insertion = &actual_fixes[0].replacement;
                assert!(insertion.contains("use crate::a::Thing;"), "{insertion}");
                assert_eq!(
                    insertion.contains("#[cfg(test)]"),
                    nested.starts_with("#[cfg(test)]")
                );
            }
        }
        Ok(())
    }

    #[test]
    fn a_moved_gated_import_blocks_a_later_import_of_the_same_name() {
        let source =
            "fn f() { #[cfg(test)] use crate::a::Thing; }\nfn g() { use crate::a::Thing; }\n";
        let syntax = syn::parse_file(source).expect("parse import fixture");
        let (findings, fixes) = super::scan(
            &syntax,
            source,
            std::path::Path::new("fixture.rs"),
            "fixture.rs",
        )
        .expect("valid source positions");
        assert_eq!(findings.len(), 1);
        assert_eq!(fixes.len(), 2);
        assert!(
            fixes[0]
                .replacement
                .starts_with("#[cfg(test)]\nuse crate::a::Thing;")
        );
    }
}
