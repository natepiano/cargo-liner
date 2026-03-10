use std::collections::BTreeMap;
use std::collections::BTreeSet;
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
use super::fix_support::FixSupport;
use super::module_paths::file_module_path;
use super::selection::Selection;

pub struct ImportScan {
    pub findings: Vec<Finding>,
    pub fixes:    ValidatedFixSet,
}

#[derive(Debug, Clone)]
struct ShortenImportFact {
    path:          String,
    line:          usize,
    column:        usize,
    highlight_len: usize,
    source_line:   String,
    replacement:   String,
}

#[derive(Debug, Clone)]
pub struct UseFix {
    pub path:        PathBuf,
    pub start:       usize,
    pub end:         usize,
    pub replacement: String,
}

#[derive(Debug, Clone)]
pub struct ValidatedFixSet {
    fixes: Vec<UseFix>,
}

impl ValidatedFixSet {
    pub fn from_vec(mut fixes: Vec<UseFix>) -> Result<Self> {
        fixes.sort_by(|left, right| {
            (&left.path, left.start, left.end, &left.replacement).cmp(&(
                &right.path,
                right.start,
                right.end,
                &right.replacement,
            ))
        });
        fixes.dedup_by(|left, right| {
            left.path == right.path
                && left.start == right.start
                && left.end == right.end
                && left.replacement == right.replacement
        });

        let mut by_file: BTreeMap<&Path, Vec<&UseFix>> = BTreeMap::new();
        for fix in &fixes {
            by_file.entry(fix.path.as_path()).or_default().push(fix);
        }

        for (path, mut file_fixes) in by_file {
            file_fixes.sort_by_key(|fix| (fix.start, fix.end));
            let mut previous_fix: Option<&UseFix> = None;
            for fix in file_fixes {
                if fix.start > fix.end {
                    anyhow::bail!(
                        "invalid fix range {}..{} for {}",
                        fix.start,
                        fix.end,
                        path.display()
                    );
                }
                if let Some(previous) = previous_fix
                    && fix.start < previous.end
                {
                    anyhow::bail!(
                        "overlapping fixes detected for {}: {}..{} ({:?}) overlaps {}..{} ({:?})",
                        path.display(),
                        previous.start,
                        previous.end,
                        previous.replacement,
                        fix.start,
                        fix.end,
                        fix.replacement
                    );
                }
                previous_fix = Some(fix);
            }
        }

        Ok(Self { fixes })
    }

    pub const fn is_empty(&self) -> bool { self.fixes.is_empty() }

    pub const fn len(&self) -> usize { self.fixes.len() }

    pub fn iter(&self) -> impl Iterator<Item = &UseFix> { self.fixes.iter() }
}

#[derive(Debug)]
struct ImportFinding {
    fact: ShortenImportFact,
    fix:  UseFix,
}

impl ShortenImportFact {
    fn into_finding(self) -> Finding {
        let replacement = self.replacement;
        Finding {
            severity:      Severity::Warning,
            code:          "shorten_local_crate_import".to_string(),
            path:          self.path,
            line:          self.line,
            column:        self.column,
            highlight_len: self.highlight_len,
            source_line:   self.source_line,
            item:          None,
            message:       "it stays within the same local module boundary".to_string(),
            suggestion:    Some(format!("consider using: `{replacement}`")),
            fix_support:   FixSupport::ShortenImport,
            related:       None,
        }
    }
}

pub fn scan_selection(selection: &Selection) -> Result<ImportScan> {
    let findings_with_fixes = scan_selection_with_fixes(selection)?;
    let fixes = ValidatedFixSet::from_vec(
        findings_with_fixes
            .iter()
            .map(|finding| finding.fix.clone())
            .collect(),
    )?;
    Ok(ImportScan {
        findings: findings_with_fixes
            .iter()
            .map(|finding| finding.fact.clone().into_finding())
            .collect(),
        fixes,
    })
}

pub fn apply_fixes(fixes: &ValidatedFixSet) -> Result<usize> {
    let mut by_file: BTreeMap<&Path, Vec<&UseFix>> = BTreeMap::new();
    for fix in fixes.iter() {
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

pub fn snapshot_files(fixes: &ValidatedFixSet) -> Result<Vec<(PathBuf, String)>> {
    let mut unique_paths = BTreeSet::new();
    for fix in fixes.iter() {
        unique_paths.insert(fix.path.clone());
    }

    let mut snapshots = Vec::new();
    for path in unique_paths {
        let text = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        snapshots.push((path, text));
    }
    Ok(snapshots)
}

pub fn restore_files(snapshots: &[(PathBuf, String)]) -> Result<()> {
    for (path, text) in snapshots {
        fs::write(path, text).with_context(|| format!("failed to restore {}", path.display()))?;
    }
    Ok(())
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
            &a.fact.path,
            a.fact.line,
            a.fact.column,
            "shorten_local_crate_import",
        )
            .cmp(&(
                &b.fact.path,
                b.fact.line,
                b.fact.column,
                "shorten_local_crate_import",
            ))
    });
    findings.dedup_by(|a, b| {
        a.fact.path == b.fact.path && a.fact.line == b.fact.line && a.fact.column == b.fact.column
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
            let original_item = &self.text[start_offset..end_offset];
            let replacement =
                original_item.replacen(&candidate.original, &candidate.replacement, 1);
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
                fact: ShortenImportFact {
                    path: display_path,
                    line: start.line,
                    column: start.column + 1,
                    highlight_len: candidate.original.len().max(1),
                    source_line,
                    replacement: replacement.clone(),
                },
                fix:  UseFix {
                    path: self.path.to_path_buf(),
                    start: start_offset,
                    end: end_offset,
                    replacement,
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
    if common == 0 {
        return None;
    }
    let up_count = current_len.saturating_sub(common);
    if up_count > 1 {
        return None;
    }

    let relative = build_relative_path(current_module_path, target_segments, &import)?;
    if relative == import.original
        || !(relative.starts_with("super::") || target_segments.starts_with(current_module_path))
    {
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
        0 => {},
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
    use std::path::PathBuf;

    use super::UseFix;
    use super::ValidatedFixSet;

    #[test]
    fn validated_fix_set_allows_adjacent_non_overlapping_ranges() {
        let path = PathBuf::from("src/lib.rs");
        let fixes = vec![
            UseFix {
                path:        path.clone(),
                start:       100,
                end:         110,
                replacement: "first".to_string(),
            },
            UseFix {
                path,
                start: 110,
                end: 120,
                replacement: "second".to_string(),
            },
        ];

        let validated_result = ValidatedFixSet::from_vec(fixes);
        assert!(
            validated_result.is_ok(),
            "adjacent edits should be valid: {}",
            validated_result
                .as_ref()
                .err()
                .map_or_else(String::new, |err| format!("{err:#}"))
        );
        let Ok(validated) = validated_result else {
            return;
        };
        assert_eq!(validated.len(), 2);
    }
}
