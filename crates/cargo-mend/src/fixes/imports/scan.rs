use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use syn::ItemMod;
use syn::ItemUse;
use syn::parse_file;
use syn::spanned::Spanned;
use syn::visit::Visit;
use walkdir::WalkDir;

use super::ImportScan;
use super::UseFix;
use super::ValidatedFixSet;
use super::path as import_path;
use crate::compiler::SOURCE_DIR_SRC;
use crate::config::DiagnosticCode;
use crate::reporting::Finding;
use crate::reporting::FixSupport;
use crate::reporting::Severity;
use crate::rust_syntax;
use crate::selection::Selection;

#[derive(Debug, Clone)]
struct ShortenImportFact {
    diagnostic_code: DiagnosticCode,
    message:         &'static str,
    path:            String,
    line:            usize,
    column:          usize,
    highlight_len:   usize,
    source_line:     String,
    replacement:     String,
}

#[derive(Debug)]
struct ImportFinding {
    shorten_import_fact: ShortenImportFact,
    use_fix:             UseFix,
}

impl From<ShortenImportFact> for Finding {
    fn from(fact: ShortenImportFact) -> Self {
        let replacement = fact.replacement;
        Self {
            severity:        Severity::Warning,
            diagnostic_code: fact.diagnostic_code,
            path:            fact.path,
            line:            fact.line,
            column:          fact.column,
            highlight_len:   fact.highlight_len,
            source_line:     fact.source_line,
            item:            None,
            message:         fact.message.to_string(),
            suggestion:      Some(format!("consider using: `{replacement}`")),
            fix_support:     FixSupport::ShortenImport,
            related:         None,
        }
    }
}

pub(super) fn scan_selection(selection: &Selection) -> Result<ImportScan> {
    let findings_with_fixes = scan_selection_with_fixes(selection)?;
    let fixes = ValidatedFixSet::try_from(
        findings_with_fixes
            .iter()
            .map(|finding| finding.use_fix.clone())
            .collect::<Vec<_>>(),
    )?;
    Ok(ImportScan {
        findings: findings_with_fixes
            .iter()
            .map(|finding| Finding::from(finding.shorten_import_fact.clone()))
            .collect(),
        fixes,
    })
}

fn scan_selection_with_fixes(selection: &Selection) -> Result<Vec<ImportFinding>> {
    let mut findings = Vec::new();
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
            findings.extend(scan_file(
                selection.analysis_root.as_path(),
                &source_root,
                path,
            )?);
        }
    }
    findings.sort_by(|a, b| {
        (
            &a.shorten_import_fact.path,
            a.shorten_import_fact.line,
            a.shorten_import_fact.column,
            a.shorten_import_fact.diagnostic_code,
        )
            .cmp(&(
                &b.shorten_import_fact.path,
                b.shorten_import_fact.line,
                b.shorten_import_fact.column,
                b.shorten_import_fact.diagnostic_code,
            ))
    });
    findings.dedup_by(|a, b| {
        a.shorten_import_fact.path == b.shorten_import_fact.path
            && a.shorten_import_fact.line == b.shorten_import_fact.line
            && a.shorten_import_fact.column == b.shorten_import_fact.column
    });
    Ok(findings)
}

fn scan_file(analysis_root: &Path, source_root: &Path, path: &Path) -> Result<Vec<ImportFinding>> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let syntax =
        parse_file(&text).with_context(|| format!("failed to parse {}", path.display()))?;
    let base_module_path = rust_syntax::file_module_path(source_root, path)
        .with_context(|| format!("failed to determine module path for {}", path.display()))?;
    let offsets = import_path::line_offsets(&text);
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
        let candidate = import_path::analyze_use_tree(&self.current_module_path, &node.tree)
            .or_else(|| import_path::analyze_deep_super(&self.current_module_path, &node.tree));
        if let Some(candidate) = candidate {
            let span = node.span();
            let start = span.start();
            let end = span.end();
            let start_offset = import_path::offset(self.offsets, start);
            let end_offset = import_path::offset(self.offsets, end);
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
                shorten_import_fact: ShortenImportFact {
                    diagnostic_code: candidate.diagnostic_code,
                    message: candidate.message,
                    path: display_path,
                    line: start.line,
                    column: start.column + 1,
                    highlight_len: candidate.original.len().max(1),
                    source_line,
                    replacement: replacement.clone(),
                },
                use_fix:             UseFix {
                    path: self.path.to_path_buf(),
                    start: start_offset,
                    end: end_offset,
                    replacement,
                    import_group: None,
                },
            });
        }
    }
}
