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
    parsed_sources: &ParsedSources,
) -> Result<ImportsAtTopScan> {
    let mut all_findings = Vec::new();
    let mut all_fixes = Vec::new();
    for package_root in &selection.package_roots {
        for path in ModuleImports::source_files(parsed_sources, package_root) {
            let ModuleContext::Known(module) =
                ModuleImports::file_module(parsed_sources, &selection.package_roots, &path)
            else {
                continue;
            };
            let (findings, fixes) = scan_file(
                selection.analysis_root.as_path(),
                &path,
                parsed_sources,
                &module,
            )?;
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
        fixes:    ValidatedFixSet::try_from((all_fixes, parsed_sources))?,
    })
}

fn scan_file(
    analysis_root: &Path,
    path: &Path,
    parsed_sources: &ParsedSources,
    module: &[String],
) -> Result<(Vec<Finding>, Vec<UseFix>)> {
    let parsed_source = parsed_sources
        .source(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let file = parsed_source
        .syntax()
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let display_path = parsed_sources.display_path(analysis_root, path);

    in_body_use_finder::scan_at_module(file, parsed_source.text(), path, &display_path, module)
}

#[cfg(test)]
mod tests {
    use crate::fixes::imports::ImportSource;
    use crate::rust_syntax::ParsedSources;
    #[cfg(unix)]
    use crate::selection::Selection;
    #[cfg(unix)]
    use crate::selection::SelectionScope;

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
        #[cfg(unix)]
        assert_declared_symlink_import(line)?;
        Ok(())
    }
    #[cfg(unix)]
    fn assert_declared_symlink_import(line: &str) -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        std::fs::create_dir(root.join("src"))?;
        std::fs::write(root.join("src/lib.rs"), "mod alias;\n")?;
        let path = root.join("external.rs");
        std::fs::write(&path, format!("// header\n{line}\n"))?;
        std::os::unix::fs::symlink(&path, root.join("src/alias.rs"))?;
        let selection = Selection {
            manifest_path:    root.join("Cargo.toml"),
            manifest_dir:     root.to_path_buf(),
            workspace_root:   root.to_path_buf(),
            target_directory: root.join("target"),
            analysis_root:    root.to_path_buf(),
            scope:            SelectionScope::SinglePackage,
            package_roots:    vec![root.to_path_buf()],
            packages:         Vec::new(),
        };
        let sources = ParsedSources::for_selection(&selection);
        let scanned = super::scan_selection(&selection, &sources)?;
        assert_eq!(
            scanned.findings.len(),
            1,
            "the declared symlink retains its module"
        );
        crate::fixes::imports::apply_fixes(&scanned.fixes)?;
        assert_eq!(
            std::fs::read_to_string(path)?,
            "// header\nuse crate::a::Thing;\nfn f() { /* é */  }\n"
        );
        Ok(())
    }
}
