use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use proc_macro2::LineColumn;
use syn::Arm;
use syn::Attribute;
use syn::Block;
use syn::Expr;
use syn::Item;
use syn::ItemMod;
use syn::ItemUse;
use syn::Stmt;
use syn::UseTree;
use syn::parse_file;
use syn::spanned::Spanned;
use syn::visit;
use syn::visit::Visit;
use walkdir::WalkDir;

use super::constants::IMPORTS_AT_TOP_MESSAGE;
use super::constants::IMPORTS_AT_TOP_SUGGESTION;
use super::imports::ImportGroup;
use super::imports::UseFix;
use super::imports::ValidatedFixSet;
use crate::compiler::SOURCE_DIR_SRC;
use crate::config::DiagnosticCode;
use crate::reporting::Finding;
use crate::reporting::FixSupport;
use crate::reporting::Severity;
use crate::selection::Selection;

pub(crate) struct ImportsAtTopScan {
    pub findings: Vec<Finding>,
    pub fixes:    ValidatedFixSet,
}

pub(crate) fn scan_selection(selection: &Selection) -> Result<ImportsAtTopScan> {
    let mut all_findings = Vec::new();
    let mut all_fixes = Vec::new();
    for package_root in &selection.package_roots {
        let source_root = package_root.join(SOURCE_DIR_SRC);
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
            let (findings, fixes) = scan_file(selection.analysis_root.as_path(), path)?;
            all_findings.extend(findings);
            all_fixes.extend(fixes);
        }
    }
    all_findings.sort_by(|left, right| {
        (&left.path, left.line, left.column).cmp(&(&right.path, right.line, right.column))
    });
    all_findings.dedup_by(|left, right| {
        left.path == right.path && left.line == right.line && left.column == right.column
    });
    Ok(ImportsAtTopScan {
        findings: all_findings,
        fixes:    ValidatedFixSet::try_from(all_fixes)?,
    })
}

fn scan_file(analysis_root: &Path, path: &Path) -> Result<(Vec<Finding>, Vec<UseFix>)> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let syntax =
        parse_file(&text).with_context(|| format!("failed to parse {}", path.display()))?;
    let offsets = line_offsets(&text);
    let display_path = path
        .strip_prefix(analysis_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    let root_scope = compute_scope_for_items(&syntax.items, &text, &offsets, "");

    let mut visitor = InBodyUseFinder {
        text: &text,
        offsets: &offsets,
        path,
        display_path: &display_path,
        scope_stack: vec![root_scope],
        gate_stack: Vec::new(),
        findings: Vec::new(),
        fixes: Vec::new(),
    };
    for item in &syntax.items {
        visitor.visit_item(item);
    }
    Ok((visitor.findings, visitor.fixes))
}

/// What the scope already exposes under a given bare name. Use-imports
/// track the resolved path so a same-path move dedupes cleanly, plus whether
/// they are `#[cfg]`-gated — a conditional import must never dedup against or
/// be added next to an unconditional one of the same name (both active at
/// once is a duplicate-import error). Other items (structs, fns, mods,
/// consts, …) carry no path — any move that would reintroduce their name at
/// this scope is treated as a hard collision and the in-body `use` is left
/// alone.
enum ExistingBinding {
    Use { full: String, gated: bool },
    Item,
}

struct Scope {
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

fn compute_scope_for_items(
    items: &[Item],
    text: &str,
    offsets: &[usize],
    default_indent: &str,
) -> Scope {
    let mut existing: BTreeMap<String, ExistingBinding> = BTreeMap::new();
    let mut last_use_end: Option<usize> = None;
    let mut first_item_start: Option<usize> = None;
    let mut detected_indent: Option<String> = None;

    for item in items {
        let item_start_lc = item.span().start();
        let item_start = offset(offsets, item_start_lc);
        first_item_start.get_or_insert(item_start);
        if detected_indent.is_none() {
            detected_indent = Some(indent_for_offset(text, item_start));
        }
        match item {
            Item::Use(item_use) => {
                let gated = any_cfg_attr(&item_use.attrs);
                for (bare, full) in flatten_use_to_bare_paths(&item_use.tree) {
                    existing.insert(bare, ExistingBinding::Use { full, gated });
                }
                let end = offset(offsets, item_use.span().end());
                let end = if text.as_bytes().get(end) == Some(&b'\n') {
                    end + 1
                } else {
                    end
                };
                last_use_end = Some(end);
            },
            other => {
                for name in item_defined_names(other) {
                    existing.entry(name).or_insert(ExistingBinding::Item);
                }
            },
        }
    }

    let insertion_offset = last_use_end.or(first_item_start).unwrap_or(0);
    let indent = detected_indent.unwrap_or_else(|| default_indent.to_string());
    Scope {
        insertion_offset,
        indent,
        existing,
    }
}

/// Bare identifiers that an item introduces at its containing module's
/// scope. Returns an empty vector for items that don't bind a name at
/// module level (`Item::Impl`, `Item::ExternCrate`, `Item::Use`,
/// `Item::Macro`, etc.).
fn item_defined_names(item: &Item) -> Vec<String> {
    match item {
        Item::Struct(s) => vec![s.ident.to_string()],
        Item::Enum(e) => vec![e.ident.to_string()],
        Item::Union(u) => vec![u.ident.to_string()],
        Item::Fn(f) => vec![f.sig.ident.to_string()],
        Item::Const(c) => vec![c.ident.to_string()],
        Item::Static(s) => vec![s.ident.to_string()],
        Item::Mod(m) => vec![m.ident.to_string()],
        Item::Trait(t) => vec![t.ident.to_string()],
        Item::TraitAlias(t) => vec![t.ident.to_string()],
        Item::Type(t) => vec![t.ident.to_string()],
        Item::ExternCrate(c) => c.rename.as_ref().map_or_else(
            || vec![c.ident.to_string()],
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

fn flatten_use_to_bare_paths(tree: &UseTree) -> Vec<(String, String)> {
    let mut prefix: Vec<String> = Vec::new();
    let mut out: Vec<(String, String)> = Vec::new();
    walk_use_tree(tree, &mut prefix, &mut out);
    out
}

fn walk_use_tree(tree: &UseTree, prefix: &mut Vec<String>, out: &mut Vec<(String, String)>) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            walk_use_tree(&path.tree, prefix, out);
            prefix.pop();
        },
        UseTree::Name(name) => {
            let bare = name.ident.to_string();
            let mut segments = prefix.clone();
            segments.push(bare.clone());
            out.push((bare, segments.join("::")));
        },
        UseTree::Rename(rename) => {
            let bare = rename.rename.to_string();
            let mut segments = prefix.clone();
            segments.push(rename.ident.to_string());
            out.push((bare, segments.join("::")));
        },
        UseTree::Group(group) => {
            for item in &group.items {
                walk_use_tree(item, prefix, out);
            }
        },
        UseTree::Glob(_) => {
            let mut segments = prefix.clone();
            segments.push("*".to_string());
            out.push(("*".to_string(), segments.join("::")));
        },
    }
}

struct InBodyUseFinder<'a> {
    text:         &'a str,
    offsets:      &'a [usize],
    path:         &'a Path,
    display_path: &'a str,
    scope_stack:  Vec<Scope>,
    /// Source text of the `#[cfg]`/`#[cfg_attr]` attributes on every enclosing
    /// construct the visitor is currently inside, outermost first. A nested
    /// `use` that moves to the module scope carries these attributes with it so
    /// the import stays conditionally compiled. Emptied on entering an inline
    /// module (an inner `use` moves only to that module's top, which the
    /// module's own gate already wraps).
    gate_stack:   Vec<String>,
    findings:     Vec<Finding>,
    fixes:        Vec<UseFix>,
}

/// Whether an attribute is `#[cfg(...)]` or `#[cfg_attr(...)]` — the two
/// attributes that conditionally remove the code they annotate. Moving a
/// `use` out of a construct carrying one would make a conditionally-compiled
/// import unconditional.
fn is_cfg_attr(attr: &Attribute) -> bool {
    attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr")
}

fn any_cfg_attr(attrs: &[Attribute]) -> bool { attrs.iter().any(is_cfg_attr) }

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
        Some(ExistingBinding::Item | ExistingBinding::Use { gated: true, .. }) => true,
        Some(ExistingBinding::Use {
            full: existing_full,
            gated: false,
        }) => existing_full != full,
        None => false,
    }
}

impl InBodyUseFinder<'_> {
    /// Source text of `attr`, sliced from the original file so the exact
    /// spelling (spacing, nested predicates) is preserved when it is carried
    /// onto a moved `use`.
    fn attr_text(&self, attr: &Attribute) -> String {
        let span = attr.span();
        let start = offset(self.offsets, span.start());
        let end = offset(self.offsets, span.end());
        self.text[start..end].to_string()
    }

    /// Source text of the `#[cfg]`/`#[cfg_attr]` attributes among `attrs`.
    fn cfg_texts(&self, attrs: &[Attribute]) -> Vec<String> {
        attrs
            .iter()
            .filter(|attr| is_cfg_attr(attr))
            .map(|attr| self.attr_text(attr))
            .collect()
    }

    fn try_emit_fix(&mut self, use_item: &ItemUse) {
        // A non-`cfg` attribute on the `use` itself (`#[allow(...)]`, a doc
        // comment, …) can change what the import means; leave those in place.
        if use_item.attrs.iter().any(|attr| !is_cfg_attr(attr)) {
            return;
        }

        let bare_paths = flatten_use_to_bare_paths(&use_item.tree);
        if bare_paths.is_empty() {
            return;
        }
        // Globs may shadow arbitrary names — don't move.
        if bare_paths.iter().any(|(bare, _)| bare == "*") {
            return;
        }

        // The gate the moved `use` must carry: every enclosing `#[cfg]` plus
        // any `#[cfg]` on the `use` itself. Non-empty means the import is
        // conditionally compiled and the attributes travel with it to the top.
        let mut gate_attrs = self.gate_stack.clone();
        gate_attrs.extend(self.cfg_texts(&use_item.attrs));
        let gated = !gate_attrs.is_empty();

        let Some(scope) = self.scope_stack.last() else {
            return;
        };

        // A gated `use` can't be reconciled with any existing binding of the
        // same name (even the same path double-imports when both are active),
        // so skip when the name is taken. An unconditional `use` skips only on
        // a real collision. Leaving it in place is safe on every target.
        let blocked = if gated {
            bare_paths
                .iter()
                .any(|(bare, _)| scope.existing.contains_key(bare))
        } else {
            bare_paths
                .iter()
                .any(|(bare, full)| unconditional_collision(scope.existing.get(bare), full))
        };
        if blocked {
            return;
        }

        // Duplicate: every bare name already imported unconditionally with the
        // same full path. The fix collapses to a pure deletion (no insertion).
        // A gated use is never a plain duplicate — its gate must be preserved.
        let all_duplicates = !gated
            && bare_paths.iter().all(|(bare, full)| {
                matches!(
                    scope.existing.get(bare),
                    Some(ExistingBinding::Use { full: existing, gated: false }) if existing == full
                )
            });

        self.emit_move(use_item, &bare_paths, &gate_attrs, gated, all_duplicates);
    }

    /// Push the finding and the insertion/deletion fixes for a movable `use`.
    /// `gate_attrs` are prepended to the inserted line so the import stays
    /// conditionally compiled; `all_duplicates` skips the insertion (the names
    /// are already imported at the scope top, so the fix is a pure deletion).
    fn emit_move(
        &mut self,
        use_item: &ItemUse,
        bare_paths: &[(String, String)],
        gate_attrs: &[String],
        gated: bool,
        all_duplicates: bool,
    ) {
        let use_span = use_item.span();
        // `use_span` starts at the first attribute; the moved line is built
        // from the `use` keyword onward so the carried gate is emitted cleanly.
        let attr_start = offset(self.offsets, use_span.start());
        let use_kw = use_item.use_token.span().start();
        let use_kw_start = offset(self.offsets, use_kw);
        let use_end = offset(self.offsets, use_span.end());
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

        let group = bare_paths.first().map(|(bare, full)| ImportGroup {
            bare_name: bare.clone(),
            full_path: full.clone(),
        });

        let source_line = self
            .text
            .lines()
            .nth(use_kw.line.saturating_sub(1))
            .unwrap_or_default()
            .to_string();
        self.findings.push(Finding {
            severity: Severity::Warning,
            diagnostic_code: DiagnosticCode::ImportsAtTop,
            path: self.display_path.to_string(),
            line: use_kw.line,
            column: use_kw.column + 1,
            highlight_len: (use_end - use_kw_start).max(1),
            source_line,
            item: None,
            message: IMPORTS_AT_TOP_MESSAGE.to_string(),
            suggestion: Some(IMPORTS_AT_TOP_SUGGESTION.to_string()),
            fix_support: FixSupport::ImportsAtTop,
            related: None,
        });

        if !all_duplicates {
            self.emit_insertion(
                bare_paths,
                gate_attrs,
                gated,
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
    }

    /// Insert the moved `use` (with its carried gate) at the scope top and
    /// record the newly imported names so later in-body uses in this pass see
    /// them as already imported. The gated flag rides along so a conditional
    /// move never dedups against an unconditional import of the same name.
    fn emit_insertion(
        &mut self,
        bare_paths: &[(String, String)],
        gate_attrs: &[String],
        gated: bool,
        use_kw_start: usize,
        use_end: usize,
        group: Option<&ImportGroup>,
    ) {
        let Some(scope) = self.scope_stack.last() else {
            return;
        };
        let indent = scope.indent.clone();
        let insertion_offset = scope.insertion_offset;

        let use_text = &self.text[use_kw_start..use_end];
        let mut insertion = String::new();
        for gate in gate_attrs {
            insertion.push_str(&indent);
            insertion.push_str(gate);
            insertion.push('\n');
        }
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

        if let Some(scope) = self.scope_stack.last_mut() {
            for (bare, full) in bare_paths {
                scope.existing.insert(
                    bare.clone(),
                    ExistingBinding::Use {
                        full: full.clone(),
                        gated,
                    },
                );
            }
        }
    }
}

impl<'ast> Visit<'ast> for InBodyUseFinder<'_> {
    fn visit_item(&mut self, node: &'ast Item) {
        let gates = self.cfg_texts(item_gate_attrs(node));
        let pushed = gates.len();
        self.gate_stack.extend(gates);
        visit::visit_item(self, node);
        self.gate_stack.truncate(self.gate_stack.len() - pushed);
    }

    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        let Some((brace, items)) = &node.content else {
            return;
        };
        let parent_indent = self
            .scope_stack
            .last()
            .map_or_else(String::new, |scope| scope.indent.clone());
        let nested_default = format!("{parent_indent}    ");
        let scope = compute_scope_for_items(items, self.text, self.offsets, &nested_default);
        // Empty inline mod (no items) — items list contributed no insertion
        // offset, so compute one from the position just after the opening
        // brace.
        let scope = if items.is_empty() {
            let brace_start = offset(self.offsets, brace.span.open().end());
            Scope {
                insertion_offset: brace_start,
                indent:           nested_default,
                existing:         BTreeMap::new(),
            }
        } else {
            scope
        };
        // The module opens its own scope: an inner `use` moves only to this
        // module's top, which the module's own gate already wraps, so gates
        // from enclosing constructs no longer apply. Clear the stack for the
        // body and restore it afterward.
        let outer_gate_stack = std::mem::take(&mut self.gate_stack);
        self.scope_stack.push(scope);
        for item in items {
            self.visit_item(item);
        }
        self.scope_stack.pop();
        self.gate_stack = outer_gate_stack;
    }

    fn visit_stmt(&mut self, node: &'ast Stmt) {
        let gates = self.cfg_texts(stmt_gate_attrs(node));
        let pushed = gates.len();
        self.gate_stack.extend(gates);
        visit::visit_stmt(self, node);
        self.gate_stack.truncate(self.gate_stack.len() - pushed);
    }

    fn visit_arm(&mut self, node: &'ast Arm) {
        let gates = self.cfg_texts(&node.attrs);
        let pushed = gates.len();
        self.gate_stack.extend(gates);
        visit::visit_arm(self, node);
        self.gate_stack.truncate(self.gate_stack.len() - pushed);
    }

    fn visit_block(&mut self, node: &'ast Block) {
        for stmt in &node.stmts {
            match stmt {
                Stmt::Item(Item::Use(use_item)) => self.try_emit_fix(use_item),
                _ => self.visit_stmt(stmt),
            }
        }
    }
}

fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (index, character) in text.char_indices() {
        if character == '\n' {
            offsets.push(index + 1);
        }
    }
    offsets
}

fn offset(line_offsets: &[usize], position: LineColumn) -> usize {
    line_offsets
        .get(position.line.saturating_sub(1))
        .copied()
        .unwrap_or(0)
        + position.column
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use syn::File;
    use syn::Item;
    use syn::UseTree;
    use syn::parse_str;

    use super::flatten_use_to_bare_paths;

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
    fn flatten_simple_name() {
        let tree = parse_tree("use crate::foo::Bar;");
        assert_eq!(
            flatten_use_to_bare_paths(&tree),
            vec![("Bar".to_string(), "crate::foo::Bar".to_string())]
        );
    }

    #[test]
    fn flatten_rename_uses_rename_as_bare() {
        let tree = parse_tree("use crate::foo::Bar as Renamed;");
        assert_eq!(
            flatten_use_to_bare_paths(&tree),
            vec![("Renamed".to_string(), "crate::foo::Bar".to_string())]
        );
    }

    #[test]
    fn flatten_group_expands_into_one_entry_per_leaf() {
        let tree = parse_tree("use crate::foo::{Bar, Baz};");
        let result = flatten_use_to_bare_paths(&tree);
        assert!(
            result.contains(&("Bar".to_string(), "crate::foo::Bar".to_string())),
            "{result:?}"
        );
        assert!(
            result.contains(&("Baz".to_string(), "crate::foo::Baz".to_string())),
            "{result:?}"
        );
    }

    #[test]
    fn flatten_glob_uses_sentinel_bare_name() {
        let tree = parse_tree("use crate::foo::*;");
        assert_eq!(
            flatten_use_to_bare_paths(&tree),
            vec![("*".to_string(), "crate::foo::*".to_string())]
        );
    }
}
