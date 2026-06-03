use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use rustc_middle::middle::privacy::EffectiveVisibilities;
use rustc_middle::ty::TyCtxt;
use rustc_span::Span;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::LocalDefId;
use serde_json::to_vec_pretty;

use super::visit;
use crate::compiler::persistence;
use crate::compiler::persistence::CacheBuildKind;
use crate::compiler::persistence::FINDINGS_SCHEMA_VERSION;
use crate::compiler::persistence::FindingsSink;
use crate::compiler::persistence::StoredReport;
use crate::compiler::settings::DriverSettings;
use crate::compiler::source_cache;
use crate::compiler::source_cache::SourceCache;
use crate::compiler::visibility::field;
use crate::compiler::visibility::source;
use crate::compiler::visibility::use_sites;
use crate::reporting::CompilerWarningFacts;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemCategory {
    Module,
    NonModule,
}

pub struct VisibilityContext<'a, 'tcx> {
    pub tcx:                    TyCtxt<'tcx>,
    pub settings:               &'a DriverSettings,
    pub source_root:            &'a Path,
    pub root_module:            &'a Path,
    pub effective_visibilities: &'a EffectiveVisibilities,
    pub source_cache:           &'a SourceCache,
}

pub struct ItemInfo<'a> {
    pub def_id:         LocalDefId,
    pub file_path:      &'a Path,
    pub vis_text:       &'a str,
    pub kind_label:     Option<&'static str>,
    pub name:           Option<&'a str>,
    pub highlight_span: Span,
    pub category:       ItemCategory,
    pub impl_self_name: Option<String>,
}

pub fn collect_and_store_findings(tcx: TyCtxt<'_>, settings: &DriverSettings) -> Result<bool> {
    let crate_root_file = source::real_file_path(tcx, tcx.def_span(CRATE_DEF_ID))
        .context("failed to determine local crate root file")?;
    let Some(source_root) =
        source_cache::analysis_source_root_for(&crate_root_file, &settings.package_root)
    else {
        return Ok(false);
    };

    let mut sink = FindingsSink::default();
    let crate_items = tcx.hir_crate_items(());
    let cache_roots: Vec<&Path> = if settings.config_root == settings.package_root {
        vec![&source_root]
    } else {
        vec![&source_root, &settings.config_root]
    };
    let source_cache = SourceCache::build(&cache_roots)?;
    let ctx = VisibilityContext {
        tcx,
        settings,
        source_root: &source_root,
        root_module: &crate_root_file,
        effective_visibilities: tcx.effective_visibilities(()),
        source_cache: &source_cache,
    };

    for item_id in crate_items.free_items() {
        let item = tcx.hir_item(item_id);
        visit::visit_item(&ctx, item, &mut sink)?;
        field::check_item(&ctx, item, &mut sink)?;
    }

    for item_id in crate_items.impl_items() {
        visit::visit_impl_item(&ctx, tcx.hir_impl_item(item_id), &mut sink)?;
    }

    for item_id in crate_items.foreign_items() {
        visit::visit_foreign_item(&ctx, tcx.hir_foreign_item(item_id), &mut sink)?;
    }

    use_sites::collect_use_sites(tcx, &mut sink.use_sites);

    let build_kind = if tcx.sess.opts.test {
        CacheBuildKind::Test
    } else {
        CacheBuildKind::Library
    };
    let output_path = settings.findings_dir.join(persistence::cache_filename_for(
        &settings.package_root,
        &crate_root_file,
        build_kind,
    ));
    let stored_crate_root = if crate_root_file.is_absolute() {
        crate_root_file.clone()
    } else {
        settings.config_root.join(&crate_root_file)
    };
    if !sink.findings.is_empty() {
        sink.findings.sort_by(|a, b| {
            (
                &a.path,
                a.line,
                a.column,
                &a.diagnostic_code,
                &a.item,
                &a.message,
            )
                .cmp(&(
                    &b.path,
                    b.line,
                    b.column,
                    &b.diagnostic_code,
                    &b.item,
                    &b.message,
                ))
        });
        sink.findings.dedup_by(|a, b| {
            a.diagnostic_code == b.diagnostic_code
                && a.path == b.path
                && a.line == b.line
                && a.column == b.column
                && a.message == b.message
                && a.item == b.item
        });
    }

    let report = StoredReport {
        version:              FINDINGS_SCHEMA_VERSION,
        analysis_fingerprint: settings.analysis_fingerprint.clone(),
        scope_fingerprint:    settings.scope_fingerprint.clone(),
        package_root:         settings.package_root.to_string_lossy().into_owned(),
        crate_root_file:      stored_crate_root.to_string_lossy().into_owned(),
        config_fingerprint:   settings.config_fingerprint.clone(),
        findings:             sink.findings,
        pub_use_fix_facts:    sink.pub_use_fix_facts,
        compiler_warnings:    CompilerWarningFacts::None,
        use_sites:            sink.use_sites,
    };
    fs::write(&output_path, to_vec_pretty(&report)?)
        .with_context(|| format!("failed to write findings file {}", output_path.display()))?;
    Ok(true)
}
