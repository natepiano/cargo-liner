use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use walkdir::WalkDir;

use super::process;
use super::process::OccurrenceResolutionContext;
use super::scope;
use super::scope::ScopeCollectionContext;
use super::scope::ScopeSpan;
use super::visitor::InlinePathVisitor;
use crate::compiler::SOURCE_DIR_SRC;
use crate::fixes::imports::ConditionalAttributes;
use crate::fixes::imports::ImportSource;
use crate::fixes::imports::UseFix;
use crate::fixes::imports::ValidatedFixSet;
use crate::reporting::Finding;
use crate::rust_syntax::ModuleMap;
use crate::rust_syntax::ParsedSources;
use crate::selection::Selection;

pub(crate) struct InlinePathScan {
    pub findings: Vec<Finding>,
    pub fixes:    ValidatedFixSet,
}

pub(crate) fn scan_selection(
    selection: &Selection,
    sources: &ParsedSources,
) -> Result<InlinePathScan> {
    let mut all_findings = Vec::new();
    let mut all_fixes = Vec::new();
    for package_root in &selection.package_roots {
        let source_root = package_root.join(SOURCE_DIR_SRC);
        if !source_root.is_dir() {
            continue;
        }
        let module_map = sources.module_map(&source_root);
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
            let (findings, fixes) = scan_file(
                selection.analysis_root.as_path(),
                &source_root,
                path,
                &module_map,
                sources,
            )?;
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
    module_map: &ModuleMap,
    sources: &ParsedSources,
) -> Result<(Vec<Finding>, Vec<UseFix>)> {
    let Some(base_module_path) = module_map.scannable_module_path(source_root, path)? else {
        return Ok((Vec::new(), Vec::new()));
    };
    let source = sources
        .source(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let syntax = source
        .syntax()
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let text = source.text();
    let indexed = ImportSource::new(text);
    let mut scopes = Vec::new();
    let mut scope_collection_context = ScopeCollectionContext {
        text,
        scopes: &mut scopes,
    };
    scope::collect_scopes(
        &syntax.items,
        ScopeSpan::new(0, text.len()),
        &base_module_path,
        &mut scope_collection_context,
        &indexed,
    )?;

    let mut visitor = InlinePathVisitor {
        occurrences:            Vec::new(),
        bare_type_names:        BTreeSet::new(),
        mod_depth:              0,
        generic_scopes:         Vec::new(),
        conditional_attributes: ConditionalAttributes::default(),
    };
    visitor.visit_file(syntax, &indexed);

    if visitor.occurrences.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let display_path = path
        .strip_prefix(analysis_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    let resolve_ctx = OccurrenceResolutionContext { scopes: &scopes };
    let resolved = visitor
        .occurrences
        .iter()
        .map(|occurrence| process::resolve_occurrence(occurrence, &resolve_ctx, &indexed))
        .collect::<Result<Vec<_>>>()?;
    let mut findings = Vec::new();
    let mut fixes = Vec::new();
    let mut inserted_use_paths: BTreeSet<(usize, String)> = BTreeSet::new();

    let ctx = resolve_ctx.analyze_collisions(
        path,
        &display_path,
        &visitor.occurrences,
        &resolved,
        &visitor.bare_type_names,
    );
    let import_attribute_plan = process::plan_import_attributes(&resolved);

    for resolved in &resolved {
        process::process_occurrence(
            resolved,
            &ctx,
            &import_attribute_plan,
            &mut inserted_use_paths,
            &mut findings,
            &mut fixes,
        );
    }

    Ok((findings, fixes))
}

#[cfg(test)]
mod tests {
    use crate::fixes::imports::ImportSource;
    use crate::rust_syntax::ParsedSources;

    #[test]
    fn completed_scan_rejects_ambiguous_and_already_bare_names() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("lib.rs");
        for text in [
            "fn f() { a::Widget; b::Widget; }",
            "fn f() { a::Widget; Widget; }",
        ] {
            std::fs::write(&path, text)?;
            let sources = ParsedSources::default();
            let module_map = sources.module_map(dir.path());
            let before = ImportSource::construction_count();
            let (findings, fixes) =
                super::scan_file(dir.path(), dir.path(), &path, &module_map, &sources)?;
            assert_eq!(ImportSource::construction_count() - before, 1);
            assert!(findings.is_empty());
            assert!(fixes.is_empty());
        }
        Ok(())
    }

    #[test]
    fn full_file_scan_shares_one_index_across_scopes_and_paths() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("lib.rs");
        let statements = "ext::Widget;".repeat(128);
        for gate in ["", "#[cfg(any(unix, windows))]\n"] {
            let text = format!(
                "mod nested {{ {gate}use ext::Existing; {gate}fn f() {{ {statements} }} \
                 mod deeper {{ {gate}fn g() {{ {statements} }} }} }}"
            );
            std::fs::write(&path, &text)?;
            let sources = ParsedSources::default();
            let module_map = sources.module_map(dir.path());
            let before = ImportSource::construction_count();
            let (findings, fixes) =
                super::scan_file(dir.path(), dir.path(), &path, &module_map, &sources)?;
            assert_eq!(ImportSource::construction_count() - before, 1);
            assert_eq!(findings.len(), 256);
            assert!(!fixes.is_empty());
            assert!(
                fixes
                    .iter()
                    .any(|fix| { fix.replacement.contains(&format!("{gate}use ext::Widget;")) })
            );
        }
        Ok(())
    }

    #[test]
    fn resolved_scan_keeps_unicode_diagnostic_line_and_edit_bytes() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("lib.rs");
        let line = "fn f() { /* é */ ext::Widget; }";
        let text = format!("// header\n{line}\n");
        std::fs::write(&path, &text)?;
        let sources = ParsedSources::default();
        let module_map = sources.module_map(dir.path());
        let before = ImportSource::construction_count();
        let (findings, fixes) =
            super::scan_file(dir.path(), dir.path(), &path, &module_map, &sources)?;
        assert_eq!(ImportSource::construction_count() - before, 1);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].source_line, line);
        assert_eq!(findings[0].line, 2);
        assert_eq!(findings[0].column, 18);
        let rewrites = fixes
            .iter()
            .filter(|fix| fix.start != fix.end)
            .collect::<Vec<_>>();
        assert_eq!(rewrites.len(), 1);
        assert_eq!(&text[rewrites[0].start..rewrites[0].end], "ext::Widget");
        assert_eq!(rewrites[0].replacement, "Widget");
        Ok(())
    }
}
