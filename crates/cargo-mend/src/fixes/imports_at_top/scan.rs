use std::ffi::OsStr;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use walkdir::WalkDir;

use super::in_body_use_finder;
use crate::compiler::SOURCE_DIR_SRC;
use crate::fixes::imports::UseFix;
use crate::fixes::imports::ValidatedFixSet;
use crate::reporting::Finding;
use crate::rust_syntax::ParsedSources;
use crate::selection::Selection;

pub(crate) struct ImportsAtTopScan {
    pub findings: Vec<Finding>,
    pub fixes:    ValidatedFixSet,
}

pub(crate) fn scan_selection(
    selection: &Selection,
    sources: &ParsedSources,
) -> Result<ImportsAtTopScan> {
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
            let (findings, fixes) = scan_file(selection.analysis_root.as_path(), path, sources)?;
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

fn scan_file(
    analysis_root: &Path,
    path: &Path,
    sources: &ParsedSources,
) -> Result<(Vec<Finding>, Vec<UseFix>)> {
    let source = sources
        .source(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let syntax = source
        .syntax()
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let display_path = path
        .strip_prefix(analysis_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    Ok(in_body_use_finder::scan(
        syntax,
        source.text(),
        path,
        &display_path,
    ))
}
