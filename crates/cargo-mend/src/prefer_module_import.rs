use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use proc_macro2::LineColumn;
use syn::Expr;
use syn::ItemUse;
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

pub struct PreferModuleImportScan {
    pub findings: Vec<Finding>,
    pub fixes:    ValidatedFixSet,
}

pub fn scan_selection(selection: &Selection) -> Result<PreferModuleImportScan> {
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
    Ok(PreferModuleImportScan {
        findings: all_findings,
        fixes:    ValidatedFixSet::from_vec(all_fixes)?,
    })
}

struct RawCandidate {
    function_name:   String,
    module_name:     String,
    module_path:     String,
    replacement_use: String,
    span_start:      LineColumn,
    span_end:        LineColumn,
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
    let current_module_path = module_paths::file_module_path(src_root, path)
        .with_context(|| format!("failed to determine module path for {}", path.display()))?;
    let offsets = line_offsets(&text);

    // Collect `mod` declarations in this file to avoid conflicts
    let declared_modules: BTreeSet<String> = syntax
        .items
        .iter()
        .filter_map(|item| {
            if let syn::Item::Mod(item_mod) = item {
                // Only external `mod foo;` declarations (no inline body)
                if item_mod.content.is_none() {
                    return Some(item_mod.ident.to_string());
                }
            }
            None
        })
        .collect();

    // Pass 1: detect candidate function imports
    let mut detector = ImportDetector {
        src_root,
        current_module_path: &current_module_path,
        declared_modules: &declared_modules,
        candidates: Vec::new(),
    };
    detector.visit_file(&syntax);

    if detector.candidates.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    // Group by module path: multiple functions from the same module get one module import
    let mut module_to_functions: BTreeMap<String, Vec<RawCandidate>> = BTreeMap::new();
    for candidate in detector.candidates {
        module_to_functions
            .entry(candidate.module_path.clone())
            .or_default()
            .push(candidate);
    }

    // Collect all imported function names for reference collection
    let imported_names: BTreeSet<String> = module_to_functions
        .values()
        .flatten()
        .map(|c| c.function_name.clone())
        .collect();

    // Pass 2: find bare references to these function names
    let mut collector = ReferenceCollector {
        offsets:        &offsets,
        imported_names: &imported_names,
        references:     Vec::new(),
    };
    collector.visit_file(&syntax);

    // Build lookup: function name → module name
    let mut func_to_module: BTreeMap<&str, &str> = BTreeMap::new();
    for functions in module_to_functions.values() {
        for func in functions {
            func_to_module.insert(func.function_name.as_str(), func.module_name.as_str());
        }
    }

    let (findings, fixes) = build_findings_and_fixes(
        analysis_root,
        path,
        &text,
        &offsets,
        &module_to_functions,
        &func_to_module,
        &collector.references,
    );

    Ok((findings, fixes))
}

fn build_findings_and_fixes(
    analysis_root: &Path,
    path: &Path,
    text: &str,
    offsets: &[usize],
    module_to_functions: &BTreeMap<String, Vec<RawCandidate>>,
    func_to_module: &BTreeMap<&str, &str>,
    references: &[BareReference],
) -> (Vec<Finding>, Vec<UseFix>) {
    let display_path = path
        .strip_prefix(analysis_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    let mut findings = Vec::new();
    let mut fixes = Vec::new();

    let mut rewritten_modules: BTreeSet<String> = BTreeSet::new();
    for functions in module_to_functions.values() {
        for func in functions {
            let byte_start = offset(offsets, func.span_start);
            let byte_end = offset(offsets, func.span_end);
            let byte_end_with_newline = if text.as_bytes().get(byte_end) == Some(&b'\n') {
                byte_end + 1
            } else {
                byte_end
            };

            let source_line = text
                .lines()
                .nth(func.span_start.line.saturating_sub(1))
                .unwrap_or_default()
                .to_string();

            findings.push(Finding {
                severity: Severity::Warning,
                code: DiagnosticCode::PreferModuleImport,
                path: display_path.clone(),
                line: func.span_start.line,
                column: func.span_start.column + 1,
                highlight_len: func.function_name.len().max(1),
                source_line,
                item: None,
                message: format!(
                    "import the module `{}` instead of the function `{}`",
                    func.module_name, func.function_name
                ),
                suggestion: Some(format!("consider using: `{}`", func.replacement_use)),
                fix_support: FixSupport::PreferModuleImport,
                related: None,
            });

            if rewritten_modules.insert(func.module_path.clone()) {
                fixes.push(UseFix {
                    path:        path.to_path_buf(),
                    start:       byte_start,
                    end:         byte_end,
                    replacement: func.replacement_use.clone(),
                });
            } else {
                fixes.push(UseFix {
                    path:        path.to_path_buf(),
                    start:       byte_start,
                    end:         byte_end_with_newline,
                    replacement: String::new(),
                });
            }
        }
    }

    for reference in references {
        if let Some(&module_name) = func_to_module.get(reference.name.as_str()) {
            fixes.push(UseFix {
                path:        path.to_path_buf(),
                start:       reference.byte_start,
                end:         reference.byte_end,
                replacement: format!("{module_name}::{}", reference.name),
            });
        }
    }

    (findings, fixes)
}

// --- Pass 1: Detect function imports ---

struct ImportDetector<'a> {
    src_root:            &'a Path,
    current_module_path: &'a [String],
    declared_modules:    &'a BTreeSet<String>,
    candidates:          Vec<RawCandidate>,
}

impl Visit<'_> for ImportDetector<'_> {
    fn visit_item_use(&mut self, node: &ItemUse) {
        if let Some(candidate) = analyze_function_import(
            self.src_root,
            self.current_module_path,
            self.declared_modules,
            node,
        ) {
            self.candidates.push(candidate);
        }
    }
}

fn analyze_function_import(
    src_root: &Path,
    current_module_path: &[String],
    declared_modules: &BTreeSet<String>,
    node: &ItemUse,
) -> Option<RawCandidate> {
    let flat = flatten_use_tree(&node.tree)?;

    // Skip renames
    if flat.rename.is_some() {
        return None;
    }

    // Must start with crate or super
    let first = flat.segments.first()?;
    if first != "crate" && first != "super" {
        return None;
    }

    // Need at least 3 segments: crate::module::function or super::module::function
    // With only 2 segments (e.g., `super::mesh`), the leaf is the module itself, not a function
    if flat.segments.len() < 3 {
        return None;
    }

    // The leaf must be snake_case (function)
    let leaf = flat.segments.last()?;
    if !is_snake_case_function_name(leaf) {
        return None;
    }

    // Resolve the import to absolute module segments for filesystem checks
    let absolute_segments = resolve_to_absolute(&flat.segments, current_module_path)?;

    // Check if the leaf is actually a module on the filesystem — module names are also
    // snake_case, so naming alone cannot distinguish them from functions
    if leaf_is_module(src_root, &absolute_segments) {
        return None;
    }

    // Build the module import path (everything except the leaf)
    let module_segments = &flat.segments[..flat.segments.len() - 1];
    let module_name = flat.segments[flat.segments.len() - 2].clone();

    // Skip if the module name is `super` — `use super::super;` is nonsensical
    if module_name == "super" || module_name == "crate" {
        return None;
    }

    // Skip if the module has a `mod` declaration in the same file — `use crate::foo;`
    // would conflict with `mod foo;`
    if declared_modules.contains(&module_name) {
        return None;
    }

    // Apply path shortening: try to use super:: instead of crate:: when possible
    let shortened_module_segments = shorten_module_path(current_module_path, module_segments);
    let module_path = shortened_module_segments.join("::");

    // Reconstruct the replacement use statement preserving visibility
    let vis_prefix = extract_visibility_prefix(node);
    let replacement_use = format!("{vis_prefix}use {module_path};");

    let span = node.span();

    Some(RawCandidate {
        function_name: leaf.clone(),
        module_name,
        module_path,
        replacement_use,
        span_start: span.start(),
        span_end: span.end(),
    })
}

fn resolve_to_absolute(segments: &[String], current_module_path: &[String]) -> Option<Vec<String>> {
    let first = segments.first()?;
    if first == "crate" {
        Some(segments[1..].to_vec())
    } else if first == "super" {
        let super_count = segments.iter().take_while(|s| *s == "super").count();
        if super_count > current_module_path.len() {
            return None;
        }
        let mut absolute = current_module_path[..current_module_path.len() - super_count].to_vec();
        absolute.extend(segments[super_count..].iter().cloned());
        Some(absolute)
    } else {
        None
    }
}

fn leaf_is_module(src_root: &Path, absolute_segments: &[String]) -> bool {
    if absolute_segments.is_empty() {
        return false;
    }
    // The parent segments form the directory path, the leaf is the potential module
    let parent_segments = &absolute_segments[..absolute_segments.len() - 1];
    let leaf = &absolute_segments[absolute_segments.len() - 1];

    let mut parent_dir = src_root.to_path_buf();
    for seg in parent_segments {
        parent_dir.push(seg);
    }

    // Check for leaf.rs or leaf/mod.rs under the parent directory
    parent_dir.join(format!("{leaf}.rs")).is_file()
        || parent_dir.join(leaf).join("mod.rs").is_file()
}

fn shorten_module_path(current_module_path: &[String], module_segments: &[String]) -> Vec<String> {
    // If the path already starts with super, keep it
    if module_segments.first().is_some_and(|s| s == "super") {
        return module_segments.to_vec();
    }

    // Only try to shorten crate:: paths
    let Some(first) = module_segments.first() else {
        return module_segments.to_vec();
    };
    if first != "crate" {
        return module_segments.to_vec();
    }

    // The target segments are everything after "crate"
    let target = &module_segments[1..];
    if target.is_empty() {
        return module_segments.to_vec();
    }

    let common = common_prefix_len(current_module_path, target);
    if common == 0 {
        return module_segments.to_vec();
    }

    let up_count = current_module_path.len().saturating_sub(common);
    // Only shorten if we go up at most 1 level
    if up_count > 1 {
        return module_segments.to_vec();
    }

    let mut relative = Vec::new();
    if up_count == 1 {
        relative.push("super".to_string());
    }
    relative.extend(target[common..].iter().cloned());

    // Only use shortened form if it's actually shorter or starts with super
    if relative.is_empty() || relative == module_segments[1..] {
        return module_segments.to_vec();
    }

    relative
}

fn common_prefix_len(left: &[String], right: &[String]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(l, r)| l == r)
        .count()
}

fn extract_visibility_prefix(node: &ItemUse) -> String {
    match &node.vis {
        syn::Visibility::Public(_) => "pub ".to_string(),
        syn::Visibility::Restricted(vis) => {
            let path = &vis.path;
            format!("pub({}) ", quote::quote!(#path))
        },
        syn::Visibility::Inherited => String::new(),
    }
}

fn flatten_use_tree(tree: &UseTree) -> Option<FlattenedImport> {
    let mut segments = Vec::new();
    let mut rename = None;
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
            UseTree::Rename(rename_tree) => {
                segments.push(rename_tree.ident.to_string());
                rename = Some(rename_tree.rename.to_string());
                break;
            },
            // Grouped imports (`UseTree::Group`) or glob (`UseTree::Glob`) — skip
            _ => return None,
        }
    }
    Some(FlattenedImport { segments, rename })
}

struct FlattenedImport {
    segments: Vec<String>,
    rename:   Option<String>,
}

fn is_snake_case_function_name(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && first != '_' {
        return false;
    }
    // Reject UPPER_SNAKE_CASE constants (all uppercase + underscores + digits)
    if name
        .chars()
        .all(|ch| ch.is_ascii_uppercase() || ch == '_' || ch.is_ascii_digit())
    {
        return false;
    }
    name.chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

// --- Pass 2: Collect bare references ---

struct BareReference {
    name:       String,
    byte_start: usize,
    byte_end:   usize,
}

struct ReferenceCollector<'a> {
    offsets:        &'a [usize],
    imported_names: &'a BTreeSet<String>,
    references:     Vec<BareReference>,
}

impl Visit<'_> for ReferenceCollector<'_> {
    fn visit_item_use(&mut self, _: &ItemUse) {
        // Skip use statements — don't qualify references inside them
    }

    fn visit_expr(&mut self, node: &Expr) {
        match node {
            Expr::Path(expr_path) => {
                if expr_path.qself.is_none() && expr_path.path.segments.len() == 1 {
                    let seg = &expr_path.path.segments[0];
                    let name = seg.ident.to_string();
                    if self.imported_names.contains(&name) {
                        let span = seg.ident.span();
                        let start = offset(self.offsets, span.start());
                        let end = offset(self.offsets, span.end());
                        self.references.push(BareReference {
                            name,
                            byte_start: start,
                            byte_end: end,
                        });
                    }
                }
            },
            _ => syn::visit::visit_expr(self, node),
        }
    }
}

// --- Utilities ---

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
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use super::is_snake_case_function_name;

    #[test]
    fn snake_case_detects_functions() {
        assert!(is_snake_case_function_name("do_thing"));
        assert!(is_snake_case_function_name("func_a"));
        assert!(is_snake_case_function_name("process_data"));
        assert!(is_snake_case_function_name("a"));
    }

    #[test]
    fn snake_case_rejects_types() {
        assert!(!is_snake_case_function_name("MyType"));
        assert!(!is_snake_case_function_name("Thing"));
        assert!(!is_snake_case_function_name("PublicContainer"));
    }

    #[test]
    fn snake_case_rejects_constants() {
        assert!(!is_snake_case_function_name("MAX_SIZE"));
        assert!(!is_snake_case_function_name("DEFAULT_PORT"));
    }

    #[test]
    fn snake_case_rejects_empty() {
        assert!(!is_snake_case_function_name(""));
    }
}
