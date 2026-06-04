use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use proc_macro2::LineColumn;
use syn::Block;
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

use super::imports::ImportGroup;
use super::imports::UseFix;
use super::imports::ValidatedFixSet;
use crate::compiler::RUST_SOURCE_FILE_EXTENSION;
use crate::compiler::SOURCE_DIR_SRC;
use crate::config::DiagnosticCode;
use crate::reporting::Finding;
use crate::reporting::FixSupport;
use crate::reporting::Severity;
use crate::rust_syntax::MODULE_GLOB_SEGMENT;
use crate::rust_syntax::MODULE_PATH_SEPARATOR;
use crate::selection::Selection;

const MESSAGE: &str = "lift this `use` to the top of its enclosing module";
const SUGGESTION: &str = "move this `use` to the top of the file or inline module";

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
                || path.extension().and_then(OsStr::to_str) != Some(RUST_SOURCE_FILE_EXTENSION)
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
        findings: Vec::new(),
        fixes: Vec::new(),
    };
    for item in &syntax.items {
        visitor.visit_item(item);
    }
    Ok((visitor.findings, visitor.fixes))
}

/// What the scope already exposes under a given bare name. Use-imports
/// track the resolved path so a same-path lift dedupes cleanly. Other items
/// (structs, fns, mods, consts, …) carry no path — any lift that would
/// reintroduce their name at this scope is treated as a hard collision and
/// the in-body `use` is left alone.
enum ExistingBinding {
    Use(String),
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
    /// detect collisions and duplicates before lifting an in-body `use`.
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
                for (bare, full) in flatten_use_to_bare_paths(&item_use.tree) {
                    existing.insert(bare, ExistingBinding::Use(full));
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
            out.push((bare, segments.join(MODULE_PATH_SEPARATOR)));
        },
        UseTree::Rename(rename) => {
            let bare = rename.rename.to_string();
            let mut segments = prefix.clone();
            segments.push(rename.ident.to_string());
            out.push((bare, segments.join(MODULE_PATH_SEPARATOR)));
        },
        UseTree::Group(group) => {
            for item in &group.items {
                walk_use_tree(item, prefix, out);
            }
        },
        UseTree::Glob(_) => {
            let mut segments = prefix.clone();
            segments.push(MODULE_GLOB_SEGMENT.to_string());
            out.push((
                MODULE_GLOB_SEGMENT.to_string(),
                segments.join(MODULE_PATH_SEPARATOR),
            ));
        },
    }
}

struct InBodyUseFinder<'a> {
    text:         &'a str,
    offsets:      &'a [usize],
    path:         &'a Path,
    display_path: &'a str,
    scope_stack:  Vec<Scope>,
    findings:     Vec<Finding>,
    fixes:        Vec<UseFix>,
}

impl InBodyUseFinder<'_> {
    fn try_emit_fix(&mut self, use_item: &ItemUse) {
        // Skip any attributed `use` — `#[cfg(...)]` and similar attributes
        // can change whether the import is in scope, so lifting risks
        // changing visibility. Conservative: skip every attributed use.
        if !use_item.attrs.is_empty() {
            return;
        }

        let bare_paths = flatten_use_to_bare_paths(&use_item.tree);
        if bare_paths.is_empty() {
            return;
        }
        // Globs may shadow arbitrary names — don't lift.
        if bare_paths
            .iter()
            .any(|(bare, _)| bare == MODULE_GLOB_SEGMENT)
        {
            return;
        }

        let Some(scope) = self.scope_stack.last() else {
            return;
        };

        // Collision: same bare name already in scope from a different
        // source (an item definition, or a use with a different full path).
        for (bare, full) in &bare_paths {
            match scope.existing.get(bare) {
                Some(ExistingBinding::Use(existing_full)) if existing_full != full => return,
                Some(ExistingBinding::Item) => return,
                _ => {},
            }
        }

        // Duplicate: every bare name already imported with the same full path.
        // The fix collapses to a pure deletion (no insertion).
        let all_duplicates = bare_paths.iter().all(|(bare, full)| {
            matches!(scope.existing.get(bare), Some(ExistingBinding::Use(existing)) if existing == full)
        });

        let use_span = use_item.span();
        let use_start = offset(self.offsets, use_span.start());
        let use_end = offset(self.offsets, use_span.end());
        let use_end_with_nl = if self.text.as_bytes().get(use_end) == Some(&b'\n') {
            use_end + 1
        } else {
            use_end
        };
        let line_start = line_start_offset(self.text, use_start);
        let leading = &self.text[line_start..use_start];
        let delete_start = if leading
            .chars()
            .all(|character| character == ' ' || character == '\t')
        {
            line_start
        } else {
            use_start
        };

        let group = bare_paths.first().map(|(bare, full)| ImportGroup {
            bare_name: bare.clone(),
            full_path: full.clone(),
        });

        let source_line = self
            .text
            .lines()
            .nth(use_span.start().line.saturating_sub(1))
            .unwrap_or_default()
            .to_string();
        self.findings.push(Finding {
            severity: Severity::Warning,
            diagnostic_code: DiagnosticCode::ImportsAtTop,
            path: self.display_path.to_string(),
            line: use_span.start().line,
            column: use_span.start().column + 1,
            highlight_len: (use_end - use_start).max(1),
            source_line,
            item: None,
            message: MESSAGE.to_string(),
            suggestion: Some(SUGGESTION.to_string()),
            fixability: FixSupport::ImportsAtTop,
            related: None,
        });

        if !all_duplicates {
            let raw_use_text = &self.text[use_start..use_end];
            let insertion = format!("{}{raw_use_text}\n", scope.indent);
            self.fixes.push(UseFix {
                path:         self.path.to_path_buf(),
                start:        scope.insertion_offset,
                end:          scope.insertion_offset,
                replacement:  insertion,
                import_group: group.clone(),
            });
            // Record the newly lifted names so later in-body uses in this
            // pass see them as already imported.
            if let Some(scope) = self.scope_stack.last_mut() {
                for (bare, full) in &bare_paths {
                    scope
                        .existing
                        .insert(bare.clone(), ExistingBinding::Use(full.clone()));
                }
            }
        }

        self.fixes.push(UseFix {
            path:         self.path.to_path_buf(),
            start:        delete_start,
            end:          use_end_with_nl,
            replacement:  String::new(),
            import_group: group,
        });
    }
}

impl<'ast> Visit<'ast> for InBodyUseFinder<'_> {
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
        self.scope_stack.push(scope);
        for item in items {
            self.visit_item(item);
        }
        self.scope_stack.pop();
    }

    fn visit_block(&mut self, node: &'ast Block) {
        for stmt in &node.stmts {
            match stmt {
                Stmt::Item(Item::Use(use_item)) => self.try_emit_fix(use_item),
                _ => visit::visit_stmt(self, stmt),
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
