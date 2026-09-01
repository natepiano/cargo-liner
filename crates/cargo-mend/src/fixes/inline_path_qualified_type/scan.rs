use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use syn::parse_file;
use syn::visit::Visit;
use walkdir::WalkDir;

use super::offsets;
use super::process;
use super::process::OccurrenceContext;
use super::scope;
use super::scope::ScopeCollectionContext;
use super::scope::ScopeSpan;
use super::visitor::InlinePathVisitor;
use crate::compiler::SOURCE_DIR_SRC;
use crate::fixes::imports::ConditionalAttributes;
use crate::fixes::imports::UseFix;
use crate::fixes::imports::ValidatedFixSet;
use crate::reporting::Finding;
use crate::rust_syntax::ModuleMap;
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
        let module_map = ModuleMap::resolve(&source_root);
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
) -> Result<(Vec<Finding>, Vec<UseFix>)> {
    let Some(base_module_path) = module_map.scannable_module_path(source_root, path)? else {
        return Ok((Vec::new(), Vec::new()));
    };
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let syntax =
        parse_file(&text).with_context(|| format!("failed to parse {}", path.display()))?;
    let offsets = offsets::line_offsets(&text);
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
        occurrences:            Vec::new(),
        bare_type_names:        BTreeSet::new(),
        mod_depth:              0,
        generic_scopes:         Vec::new(),
        text:                   &text,
        offsets:                &offsets,
        conditional_attributes: ConditionalAttributes::default(),
    };
    visitor.visit_file(&syntax);

    if visitor.occurrences.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let display_path = path
        .strip_prefix(analysis_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");

    // Collision detection needs every occurrence's resolved import path, and
    // resolution never consults the collision set, so resolve once against a
    // context with no collisions recorded yet.
    let no_collision_names = BTreeSet::new();
    let resolve_ctx = OccurrenceContext {
        path,
        display_path: &display_path,
        text: &text,
        offsets: &offsets,
        scopes: &scopes,
        collision_names: &no_collision_names,
    };
    let resolved_import_paths: Vec<String> = visitor
        .occurrences
        .iter()
        .map(|occurrence| process::resolve_occurrence(occurrence, &resolve_ctx).import_path)
        .collect();
    let collision_names = process::find_collision_names(
        &visitor.occurrences,
        &resolved_import_paths,
        &visitor.bare_type_names,
        &scopes
            .iter()
            .flat_map(|scope| scope.existing_imports.keys().cloned())
            .collect(),
    );

    let mut findings = Vec::new();
    let mut fixes = Vec::new();
    let mut inserted_use_paths: BTreeSet<(usize, String)> = BTreeSet::new();

    let ctx = OccurrenceContext {
        collision_names: &collision_names,
        ..resolve_ctx
    };
    let import_attribute_plan = process::plan_import_attributes(&visitor.occurrences, &ctx);

    for occ in &visitor.occurrences {
        process::process_occurrence(
            occ,
            &ctx,
            &import_attribute_plan,
            &mut inserted_use_paths,
            &mut findings,
            &mut fixes,
        );
    }

    Ok((findings, fixes))
}
