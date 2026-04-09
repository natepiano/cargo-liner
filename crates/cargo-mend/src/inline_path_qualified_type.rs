use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use proc_macro2::LineColumn;
use syn::Item;
use syn::ItemUse;
use syn::TypePath;
use syn::UseTree;
use syn::parse_file;
use syn::spanned::Spanned;
use syn::visit::Visit;
use walkdir::WalkDir;

use super::config::DiagnosticCode;
use super::diagnostics::Finding;
use super::diagnostics::Severity;
use super::fix_support::FixSupport;
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
        let src_root = package_root.join("src");
        if !src_root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&src_root).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if !entry.file_type().is_file()
                || path.extension().and_then(|ext| ext.to_str()) != Some("rs")
            {
                continue;
            }
            let (findings, fixes) = scan_file(selection.analysis_root.as_path(), &src_root, path)?;
            all_findings.extend(findings);
            all_fixes.extend(fixes);
        }
    }
    all_findings.sort_by(|a, b| (&a.path, a.line, a.column).cmp(&(&b.path, b.line, b.column)));
    all_findings.dedup_by(|a, b| a.path == b.path && a.line == b.line && a.column == b.column);
    Ok(InlinePathScan {
        findings: all_findings,
        fixes:    ValidatedFixSet::from_vec(all_fixes)?,
    })
}

struct InlinePathOccurrence {
    full_path:  String,
    type_name:  String,
    span_start: LineColumn,
    span_end:   LineColumn,
}

struct ScopeInfo {
    span_start:       usize,
    span_end:         usize,
    insertion_offset: usize,
    indent:           String,
    module_path:      Vec<String>,
    existing_imports: BTreeSet<String>,
}

fn scan_file(
    analysis_root: &Path,
    src_root: &Path,
    path: &Path,
) -> Result<(Vec<Finding>, Vec<UseFix>)> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let syntax =
        parse_file(&text).with_context(|| format!("failed to parse {}", path.display()))?;
    let offsets = line_offsets(&text);
    let base_module_path = module_paths::file_module_path(src_root, path)
        .with_context(|| format!("failed to determine module path for {}", path.display()))?;
    let mut scopes = Vec::new();
    collect_scopes(
        &syntax.items,
        &text,
        &offsets,
        0,
        text.len(),
        &base_module_path,
        &mut scopes,
    );

    // Visit the AST to find inline path-qualified types
    let mut visitor = InlinePathVisitor {
        occurrences:     Vec::new(),
        bare_type_names: BTreeSet::new(),
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

    for occ in &visitor.occurrences {
        // Skip collisions
        if collision_names.contains(&occ.type_name) {
            continue;
        }

        let byte_start = offset(&offsets, occ.span_start);
        let byte_end = offset(&offsets, occ.span_end);

        let source_line = text
            .lines()
            .nth(occ.span_start.line.saturating_sub(1))
            .unwrap_or_default()
            .to_string();

        findings.push(Finding {
            severity: Severity::Warning,
            code: DiagnosticCode::InlinePathQualifiedType,
            path: display_path.clone(),
            line: occ.span_start.line,
            column: occ.span_start.column + 1,
            highlight_len: occ.full_path.len().max(1),
            source_line,
            item: None,
            message: format!(
                "use a `use` import for `{}` instead of inline path",
                occ.type_name
            ),
            suggestion: Some(format!("consider adding: `use {};`", occ.full_path)),
            fix_support: FixSupport::InlinePathQualifiedType,
            related: None,
        });

        // Replace the inline path with just the type name
        fixes.push(UseFix {
            path:        path.to_path_buf(),
            start:       byte_start,
            end:         byte_end,
            replacement: occ.type_name.clone(),
        });

        let Some(scope_id) = find_innermost_scope(&scopes, byte_start) else {
            continue;
        };
        let scope = &scopes[scope_id];

        // Insert a `use` statement in the containing scope, not always at file scope.
        if !scope.existing_imports.contains(&occ.full_path)
            && inserted_use_paths.insert((scope_id, occ.full_path.clone()))
        {
            let use_path = canonicalize_inserted_use_path(scope, &occ.full_path);
            let use_text = format!("{}use {};\n", scope.indent, use_path);
            fixes.push(UseFix {
                path:        path.to_path_buf(),
                start:       scope.insertion_offset,
                end:         scope.insertion_offset,
                replacement: use_text,
            });
        }
    }

    Ok((findings, fixes))
}

fn collect_scopes(
    items: &[Item],
    text: &str,
    offsets: &[usize],
    span_start: usize,
    span_end: usize,
    module_path: &[String],
    scopes: &mut Vec<ScopeInfo>,
) {
    let mut existing_imports = BTreeSet::new();
    let mut last_use_start = None;
    let mut last_use_end = None;
    let mut first_item_start = None;

    for item in items {
        let item_start = offset(offsets, item.span().start());
        first_item_start.get_or_insert(item_start);

        if let Item::Use(item_use) = item {
            if let Some(import_path) = flatten_use_path(&item_use.tree) {
                existing_imports.insert(import_path);
            }
            last_use_start = Some(item_start);
            let item_end = offset(offsets, item_use.span().end());
            last_use_end = Some(if text.as_bytes().get(item_end) == Some(&b'\n') {
                item_end + 1
            } else {
                item_end
            });
        }
    }

    let anchor_offset = last_use_start.or(first_item_start).unwrap_or(span_start);
    let insertion_offset = last_use_end.or(first_item_start).unwrap_or(span_end);
    let indent = indentation_at(text, anchor_offset);
    scopes.push(ScopeInfo {
        span_start,
        span_end,
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
                text,
                offsets,
                offset(offsets, item_mod.span().start()),
                offset(offsets, item_mod.span().end()),
                &child_module_path,
                scopes,
            );
        }
    }
}

fn find_innermost_scope(scopes: &[ScopeInfo], byte_offset: usize) -> Option<usize> {
    scopes
        .iter()
        .enumerate()
        .filter(|(_, scope)| scope.span_start <= byte_offset && byte_offset < scope.span_end)
        .max_by_key(|(_, scope)| (scope.span_start, std::cmp::Reverse(scope.span_end)))
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
    let mut name_to_paths: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for occ in occurrences {
        name_to_paths
            .entry(&occ.type_name)
            .or_default()
            .insert(&occ.full_path);
    }

    let mut collisions = BTreeSet::new();
    for (name, paths) in &name_to_paths {
        let ambiguous = paths.len() > 1;
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
}

impl InlinePathVisitor {
    fn check_path(&mut self, path: &syn::Path) {
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();

        if segments.len() < 3 {
            return;
        }

        let first = &segments[0];
        if first != "crate" && first != "super" {
            return;
        }

        let leaf = &segments[segments.len() - 1];
        if !is_pascal_case(leaf) {
            return;
        }

        let full_path = segments.join("::");

        // Use ident spans to exclude generic arguments from the replacement range.
        // path.span() includes generic args (e.g., `<T>`), but we only want to
        // replace the path portion, leaving generic args in place.
        // Safety: segments.len() >= 3, checked above.
        let first_ident_span = path.segments[0].ident.span();
        let last_ident_span = path.segments[segments.len() - 1].ident.span();

        self.occurrences.push(InlinePathOccurrence {
            full_path,
            type_name: leaf.clone(),
            span_start: first_ident_span.start(),
            span_end: last_ident_span.end(),
        });
    }
}

impl Visit<'_> for InlinePathVisitor {
    fn visit_item_use(&mut self, _: &ItemUse) {
        // Skip use statements — they are imports, not inline code
    }

    fn visit_type_path(&mut self, node: &TypePath) {
        if node.qself.is_none() {
            self.check_path(&node.path);
            // Track bare type names to detect potential shadowing
            if node.path.segments.len() == 1 {
                let name = node.path.segments[0].ident.to_string();
                if is_pascal_case(&name) {
                    self.bare_type_names.insert(name);
                }
            }
        }
        syn::visit::visit_type_path(self, node);
    }

    fn visit_expr_path(&mut self, node: &syn::ExprPath) {
        if node.qself.is_none() {
            self.check_path(&node.path);
            if node.path.segments.len() == 1 {
                let name = node.path.segments[0].ident.to_string();
                if is_pascal_case(&name) {
                    self.bare_type_names.insert(name);
                }
            }
        }
        // Don't recurse — path segments don't contain sub-expressions
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

fn offset(line_offsets: &[usize], position: LineColumn) -> usize {
    line_offsets
        .get(position.line.saturating_sub(1))
        .copied()
        .unwrap_or(0)
        + position.column
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
