use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use proc_macro2::LineColumn;
use syn::ItemMod;
use syn::ItemUse;
use syn::UseTree;
use syn::parse_file;
use syn::spanned::Spanned;
use syn::visit::Visit;
use walkdir::WalkDir;

use super::diagnostics::Finding;
use super::diagnostics::Severity;
use super::selection::Selection;

pub(super) struct ImportScan {
    pub(super) findings: Vec<Finding>,
    pub(super) fixes:    Vec<UseFix>,
}

#[derive(Debug, Clone)]
pub(super) struct UseFix {
    pub(super) path:        PathBuf,
    pub(super) start:       usize,
    pub(super) end:         usize,
    pub(super) replacement: String,
}

#[derive(Debug)]
struct ImportFinding {
    finding: Finding,
    fix:     UseFix,
}

pub(super) fn scan_selection(selection: &Selection) -> Result<ImportScan> {
    let findings_with_fixes = scan_selection_with_fixes(selection)?;
    Ok(ImportScan {
        findings: findings_with_fixes
            .iter()
            .map(|finding| finding.finding.clone())
            .collect(),
        fixes:    findings_with_fixes
            .into_iter()
            .map(|finding| finding.fix)
            .collect(),
    })
}

pub(super) fn apply_fixes(fixes: &[UseFix]) -> Result<usize> {
    let mut by_file: BTreeMap<&Path, Vec<&UseFix>> = BTreeMap::new();
    for fix in fixes {
        by_file.entry(fix.path.as_path()).or_default().push(fix);
    }
    let mut applied = 0usize;
    for (path, mut file_fixes) in by_file {
        let mut text = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        file_fixes.sort_by_key(|fix| std::cmp::Reverse(fix.start));
        for fix in file_fixes {
            if fix.end <= text.len() && fix.start <= fix.end {
                text.replace_range(fix.start..fix.end, &fix.replacement);
                applied += 1;
            }
        }
        fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))?;
    }

    Ok(applied)
}

fn scan_selection_with_fixes(selection: &Selection) -> Result<Vec<ImportFinding>> {
    let mut findings = Vec::new();
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
            findings.extend(scan_file(
                selection.analysis_root.as_path(),
                &src_root,
                path,
            )?);
        }
    }
    findings.sort_by(|a, b| {
        (
            &a.finding.path,
            a.finding.line,
            a.finding.column,
            &a.finding.code,
        )
            .cmp(&(
                &b.finding.path,
                b.finding.line,
                b.finding.column,
                &b.finding.code,
            ))
    });
    findings.dedup_by(|a, b| {
        a.finding.path == b.finding.path
            && a.finding.line == b.finding.line
            && a.finding.column == b.finding.column
            && a.finding.code == b.finding.code
    });
    Ok(findings)
}

fn scan_file(analysis_root: &Path, src_root: &Path, path: &Path) -> Result<Vec<ImportFinding>> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let syntax =
        parse_file(&text).with_context(|| format!("failed to parse {}", path.display()))?;
    let base_module_path = file_module_path(src_root, path)
        .with_context(|| format!("failed to determine module path for {}", path.display()))?;
    let offsets = line_offsets(&text);
    let mut visitor = UseVisitor {
        analysis_root,
        path,
        text: &text,
        offsets: &offsets,
        current_module_path: base_module_path,
        findings: Vec::new(),
    };
    visitor.visit_file(&syntax);
    Ok(visitor.findings)
}

struct UseVisitor<'a> {
    analysis_root:       &'a Path,
    path:                &'a Path,
    text:                &'a str,
    offsets:             &'a [usize],
    current_module_path: Vec<String>,
    findings:            Vec<ImportFinding>,
}

impl Visit<'_> for UseVisitor<'_> {
    fn visit_item_mod(&mut self, node: &ItemMod) {
        if let Some((_, items)) = &node.content {
            self.current_module_path.push(node.ident.to_string());
            for item in items {
                self.visit_item(item);
            }
            self.current_module_path.pop();
        }
    }

    fn visit_item_use(&mut self, node: &ItemUse) {
        if let Some(candidate) = analyze_use_tree(&self.current_module_path, &node.tree) {
            let span = node.span();
            let start = span.start();
            let end = span.end();
            let start_offset = offset(self.offsets, start);
            let end_offset = offset(self.offsets, end);
            let source_line = self
                .text
                .lines()
                .nth(start.line.saturating_sub(1))
                .unwrap_or_default()
                .to_string();
            let display_path = self
                .path
                .strip_prefix(self.analysis_root)
                .unwrap_or(self.path)
                .to_string_lossy()
                .replace('\\', "/");
            self.findings.push(ImportFinding {
                finding: Finding {
                    severity: Severity::Warning,
                    code: "shorten_local_crate_import".to_string(),
                    path: display_path,
                    line: start.line,
                    column: start.column + 1,
                    highlight_len: candidate.original.len().max(1),
                    source_line,
                    item: None,
                    message: "it stays within the same local module boundary".to_string(),
                    suggestion: Some(format!("consider using: `use {};`", candidate.replacement)),
                },
                fix:     UseFix {
                    path:        self.path.to_path_buf(),
                    start:       start_offset,
                    end:         end_offset,
                    replacement: format!("use {};", candidate.replacement),
                },
            });
        }
    }
}

struct ImportCandidate {
    original:    String,
    replacement: String,
}

fn analyze_use_tree(current_module_path: &[String], tree: &UseTree) -> Option<ImportCandidate> {
    let import = flatten_use_tree(tree)?;
    if import.segments.first()? != "crate" {
        return None;
    }

    let target_segments = &import.segments[1..];
    if target_segments.len() < 2 {
        return None;
    }

    let current_len = current_module_path.len();
    let common = common_prefix_len(current_module_path, target_segments);
    let up_count = current_len.saturating_sub(common);
    if up_count > 1 {
        return None;
    }

    let relative = build_relative_path(current_module_path, target_segments, &import)?;
    if !matches!(
        relative.as_str(),
        path if path.starts_with("self::") || path.starts_with("super::")
    ) {
        return None;
    }

    Some(ImportCandidate {
        original:    import.original,
        replacement: relative,
    })
}

struct FlattenedImport {
    segments: Vec<String>,
    original: String,
    rename:   Option<String>,
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
            _ => return None,
        }
    }
    Some(FlattenedImport {
        original: format_path(&segments, rename.as_deref()),
        segments,
        rename,
    })
}

fn build_relative_path(
    current_module_path: &[String],
    target_segments: &[String],
    import: &FlattenedImport,
) -> Option<String> {
    let common = common_prefix_len(current_module_path, target_segments);
    let up_count = current_module_path.len().saturating_sub(common);
    let mut relative_segments = Vec::new();
    match up_count {
        0 => relative_segments.push("self".to_string()),
        1 => relative_segments.push("super".to_string()),
        _ => return None,
    }
    relative_segments.extend(target_segments[common..].iter().cloned());
    Some(format_path(&relative_segments, import.rename.as_deref()))
}

fn common_prefix_len(left: &[String], right: &[String]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(l, r)| l == r)
        .count()
}

fn format_path(segments: &[String], rename: Option<&str>) -> String {
    let mut path = segments.join("::");
    if let Some(rename) = rename {
        path.push_str(" as ");
        path.push_str(rename);
    }
    path
}

fn file_module_path(src_root: &Path, path: &Path) -> Option<Vec<String>> {
    let relative = path.strip_prefix(src_root).ok()?;
    let stem = path.file_stem()?.to_str()?;
    let mut result: Vec<String> = relative
        .parent()
        .into_iter()
        .flat_map(|parent| parent.iter())
        .filter_map(|segment| segment.to_str().map(str::to_string))
        .collect();
    if stem != "lib" && stem != "main" && stem != "mod" {
        result.push(stem.to_string());
    }
    Some(result)
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
