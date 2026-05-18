use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use syn::visit::Visit;
use walkdir::WalkDir;

use super::offsets;
use super::process;
use super::process::OccurrenceContext;
use super::scope;
use super::scope::ScopeCollectionContext;
use super::scope::ScopeSpan;
use super::visitor::InlinePathVisitor;
use crate::compiler::RUST_SOURCE_FILE_EXTENSION;
use crate::compiler::SOURCE_DIR_SRC;
use crate::fixes::imports::UseFix;
use crate::fixes::imports::ValidatedFixSet;
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
    let offsets = offsets::line_offsets(&text);
    let base_module_path = rust_syntax::file_module_path(source_root, path)
        .with_context(|| format!("failed to determine module path for {}", path.display()))?;
    let mut scopes = Vec::new();
    let mut scope_collection_context = ScopeCollectionContext {
        text:    &text,
        offsets: &offsets,
        scopes:  &mut scopes,
    };
    scope::collect_scopes(
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

    let collision_names = process::find_collision_names(
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
        process::process_occurrence(
            occ,
            &ctx,
            &mut inserted_use_paths,
            &mut findings,
            &mut fixes,
        );
    }

    Ok((findings, fixes))
}
