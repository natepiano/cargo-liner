use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use proc_macro2::LineColumn;
use syn::ItemUse;
use syn::TypePath;
use syn::UseTree;
use syn::parse_file;
use syn::spanned::Spanned;
use syn::visit::Visit;
use walkdir::WalkDir;

use super::diagnostics::Finding;
use super::diagnostics::Severity;
use super::fix_support::FixSupport;
use super::imports::UseFix;
use super::imports::ValidatedFixSet;
use super::selection::Selection;

pub struct InlinePathScan {
    pub findings: Vec<Finding>,
    pub fixes:    ValidatedFixSet,
}

pub fn scan_selection(selection: &Selection) -> Result<InlinePathScan> {
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
            let (findings, fixes) = scan_file(selection.analysis_root.as_path(), path)?;
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

fn scan_file(analysis_root: &Path, path: &Path) -> Result<(Vec<Finding>, Vec<UseFix>)> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let syntax =
        parse_file(&text).with_context(|| format!("failed to parse {}", path.display()))?;
    let offsets = line_offsets(&text);

    // Collect existing use imports to avoid duplicates
    let mut existing_imports: BTreeSet<String> = BTreeSet::new();
    let mut last_use_byte_end: Option<usize> = None;
    for item in &syntax.items {
        if let syn::Item::Use(item_use) = item {
            if let Some(import_path) = flatten_use_path(&item_use.tree) {
                existing_imports.insert(import_path);
            }
            let end = offset(&offsets, item_use.span().end());
            // Move past the trailing newline if present
            let end_with_newline = if text.as_bytes().get(end) == Some(&b'\n') {
                end + 1
            } else {
                end
            };
            last_use_byte_end = Some(end_with_newline);
        }
    }

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
        &existing_imports,
    );

    let display_path = path
        .strip_prefix(analysis_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    let mut findings = Vec::new();
    let mut fixes = Vec::new();
    let mut inserted_use_paths: BTreeSet<String> = BTreeSet::new();

    // Compute insertion point for new use statements
    let insertion_offset = last_use_byte_end.unwrap_or(0);

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
            code: "inline_path_qualified_type".to_string(),
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

        // Insert use statement (only once per unique full path, and only if not already imported)
        if !existing_imports.contains(&occ.full_path)
            && inserted_use_paths.insert(occ.full_path.clone())
        {
            let use_text = format!("use {};\n", occ.full_path);
            fixes.push(UseFix {
                path:        path.to_path_buf(),
                start:       insertion_offset,
                end:         insertion_offset,
                replacement: use_text,
            });
        }
    }

    Ok((findings, fixes))
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
    fn visit_item_use(&mut self, _node: &ItemUse) {
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
