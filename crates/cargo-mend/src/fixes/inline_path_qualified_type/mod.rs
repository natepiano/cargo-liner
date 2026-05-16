mod process;
mod scope;
mod visitor;

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use proc_macro2::LineColumn;
use process::OccurrenceContext;
use process::find_collision_names;
use process::process_occurrence;
use scope::ScopeCollectionContext;
use scope::ScopeSpan;
use scope::collect_scopes;
use syn::visit::Visit;
use visitor::InlinePathVisitor;
use walkdir::WalkDir;

use super::imports::UseFix;
use super::imports::ValidatedFixSet;
use crate::compiler::RUST_SOURCE_FILE_EXTENSION;
use crate::compiler::SOURCE_DIR_SRC;
use crate::reporting::Finding;
use crate::rust_syntax;
use crate::selection::Selection;

pub(crate) struct InlinePathScan {
    pub findings: Vec<Finding>,
    pub fixes:    ValidatedFixSet,
}

pub(crate) fn scan_selection(selection: &Selection) -> Result<InlinePathScan> {
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
    let base_module_path = rust_syntax::file_module_path(source_root, path)
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

    let mut visitor = InlinePathVisitor {
        occurrences:     Vec::new(),
        bare_type_names: BTreeSet::new(),
        mod_depth:       0,
        generic_scopes:  Vec::new(),
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

fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (idx, ch) in text.char_indices() {
        if ch == '\n' {
            offsets.push(idx + 1);
        }
    }
    offsets
}

pub(super) fn offset(text: &str, line_offsets: &[usize], position: LineColumn) -> usize {
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
    use super::visitor::is_pascal_case;

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
