use std::path::Path;

use anyhow::Context;
use anyhow::Result;

use super::in_body_use_finder;
use crate::fixes::imports::ModuleContext;
use crate::fixes::imports::ModuleImports;
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
        for path in ModuleImports::source_files(sources, package_root) {
            let ModuleContext::Known(module) =
                ModuleImports::file_module(sources, &selection.package_roots, &path)
            else {
                continue;
            };
            let (findings, fixes) =
                scan_file(selection.analysis_root.as_path(), &path, sources, &module)?;
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
    module: &[String],
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

    in_body_use_finder::scan_at_module(syntax, source.text(), path, &display_path, module)
}

#[cfg(test)]
mod tests {
    use crate::fixes::imports::ImportSource;
    use crate::rust_syntax::ParsedSources;

    #[test]
    fn file_scan_keeps_diagnostic_bytes_with_one_checked_index() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("lib.rs");
        let line = "fn f() { /* é */ use crate::a::Thing; }";
        std::fs::write(&path, format!("// header\n{line}\n"))?;
        let before = ImportSource::construction_count();
        let (findings, fixes) =
            super::scan_file(dir.path(), &path, &ParsedSources::default(), &[])?;
        assert_eq!(ImportSource::construction_count() - before, 1);
        assert_eq!(findings[0].source_line, line);
        assert_eq!(findings[0].path, "lib.rs");
        assert_eq!(fixes[0].replacement, "use crate::a::Thing;\n");
        Ok(())
    }
}
