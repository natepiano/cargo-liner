use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use rustc_middle::middle::privacy::EffectiveVisibilities;
use rustc_middle::ty::TyCtxt;
use rustc_span::FileName;
use rustc_span::Span;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::LOCAL_CRATE;
use rustc_span::def_id::LocalDefId;
use serde_json::to_vec_pretty;

use super::visit;
use crate::compiler::constants::FINDINGS_SCHEMA_VERSION;
use crate::compiler::exposure::SignatureExposureCache;
#[cfg(feature = "test-counters")]
use crate::compiler::facade;
use crate::compiler::facade::ModuleSourceMap;
use crate::compiler::persistence;
use crate::compiler::persistence::CacheBuildKind;
use crate::compiler::persistence::FindingsSink;
use crate::compiler::persistence::StoredReport;
use crate::compiler::settings::DriverSettings;
use crate::compiler::source_cache;
use crate::compiler::source_cache::SourceCache;
use crate::compiler::visibility::field;
use crate::compiler::visibility::interface_ceiling;
use crate::compiler::visibility::interface_ceiling::InterfaceCeiling;
use crate::compiler::visibility::source;
use crate::compiler::visibility::use_sites;
use crate::compiler::visibility::use_sites::ParentFacadeAnalysis;
use crate::compiler::visibility::use_sites::ReexportIndex;
use crate::reporting::CompilerWarningFacts;

pub(in crate::compiler::visibility) struct VisibilityContext<'a, 'tcx> {
    pub tcx:                       TyCtxt<'tcx>,
    pub settings:                  &'a DriverSettings,
    pub source_root:               &'a Path,
    pub root_module:               &'a Path,
    pub effective_visibilities:    &'a EffectiveVisibilities,
    pub source_cache:              &'a SourceCache,
    pub public_visibility_targets: &'a FxHashSet<LocalDefId>,
    /// Per ADT, how far it may be widened before one of its trait impls leaves
    /// a narrower type in its interface. Absent means nothing caps it.
    pub interface_ceilings:        &'a FxHashMap<LocalDefId, InterfaceCeiling>,
    pub reexport_index:            &'a ReexportIndex,
    pub module_sources:            &'a ModuleSourceMap,
    parent_facade_analyses:        RefCell<FxHashMap<LocalDefId, Option<ParentFacadeAnalysis<'a>>>>,
    /// Shared by every `ExposureContext` built during the scan, so the exposure
    /// walk answers each item once for the whole crate rather than once per
    /// analyzed item.
    pub signature_exposure_cache:  SignatureExposureCache,
}

impl<'a> VisibilityContext<'a, '_> {
    pub(in crate::compiler::visibility) fn resolve_parent_facade(
        &self,
        item_def_id: LocalDefId,
    ) -> Option<ParentFacadeAnalysis<'a>> {
        #[cfg(feature = "test-counters")]
        facade::record_facade_resolution_request(item_def_id);
        let cached = self
            .parent_facade_analyses
            .borrow()
            .get(&item_def_id)
            .cloned();
        if let Some(parent_facade_analysis) = cached {
            return parent_facade_analysis;
        }

        #[cfg(feature = "test-counters")]
        facade::record_facade_resolution(item_def_id);
        let facade_subject = self.reexport_index.facade_subject(item_def_id);
        let parent_facade_analysis =
            self.reexport_index
                .parent_facade_analysis(self.tcx, item_def_id, facade_subject);
        self.parent_facade_analyses
            .borrow_mut()
            .insert(item_def_id, parent_facade_analysis.clone());
        parent_facade_analysis
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler::visibility) enum ItemCategory {
    Module,
    Declaration,
    Use,
}

pub(in crate::compiler::visibility) struct ItemInfo<'a> {
    pub def_id:          LocalDefId,
    pub file_path:       &'a Path,
    pub visibility_text: &'a str,
    pub kind_label:      Option<&'static str>,
    pub name:            Option<&'a str>,
    pub highlight_span:  Span,
    pub category:        ItemCategory,
    pub facade_subject:  LocalDefId,
}

pub(in crate::compiler::visibility) fn collect_and_store_findings(
    tcx: TyCtxt<'_>,
    settings: &DriverSettings,
) -> Result<bool> {
    let crate_root_file = source::real_file_path(tcx, tcx.def_span(CRATE_DEF_ID))
        .context("failed to determine local crate root file")?;
    let Some(source_root) =
        source_cache::analysis_source_root_for(&crate_root_file, &settings.package_root)
    else {
        return Ok(false);
    };

    let mut sink = FindingsSink::default();
    let crate_items = tcx.hir_crate_items(());
    let source_cache = build_source_cache(tcx, &crate_root_file)?;
    let reexport_index = use_sites::reexport_index(tcx);
    let module_sources = ModuleSourceMap::new(tcx, &source_cache);
    let mut public_visibility_targets = FxHashSet::default();
    sink.use_sites = use_sites::collect_use_sites(tcx, &mut public_visibility_targets);
    let interface_ceilings = interface_ceiling::collect_interface_ceilings(tcx);
    let ctx = VisibilityContext {
        tcx,
        settings,
        source_root: &source_root,
        root_module: &crate_root_file,
        effective_visibilities: tcx.effective_visibilities(()),
        source_cache: &source_cache,
        public_visibility_targets: &public_visibility_targets,
        interface_ceilings: &interface_ceilings,
        reexport_index: &reexport_index,
        module_sources: &module_sources,
        parent_facade_analyses: RefCell::new(FxHashMap::default()),
        signature_exposure_cache: SignatureExposureCache::default(),
    };

    let mut source_files = BTreeSet::new();
    for item_id in crate_items.free_items() {
        let item = tcx.hir_item(item_id);
        if let Some(path) = source::real_file_path(tcx, item.span) {
            source_files.insert(path.to_string_lossy().into_owned());
        }
        visit::visit_item(&ctx, item, &mut sink)?;
        field::check_item(&ctx, item, &mut sink)?;
    }

    for item_id in crate_items.impl_items() {
        visit::visit_impl_item(&ctx, tcx.hir_impl_item(item_id), &mut sink)?;
    }

    for item_id in crate_items.foreign_items() {
        visit::visit_foreign_item(&ctx, tcx.hir_foreign_item(item_id), &mut sink)?;
    }

    let build_kind = cache_build_kind(tcx);
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
    sort_and_dedupe(&mut sink);

    let report = StoredReport {
        version:                FINDINGS_SCHEMA_VERSION,
        analysis_fingerprint:   settings.analysis_fingerprint.clone(),
        scope_fingerprint:      settings.scope_fingerprint.clone(),
        package_root:           settings.package_root.to_string_lossy().into_owned(),
        crate_root_file:        stored_crate_root.to_string_lossy().into_owned(),
        config_fingerprint:     settings.config_fingerprint.clone(),
        source_files:           source_files.into_iter().collect(),
        findings:               sink.findings,
        visibility_constraints: sink.visibility_constraints,
        pub_use_fix_facts:      sink.pub_use_fix_facts,
        all_features_coverage:  source_cache.all_features_coverage(),
        compiler_warning_facts: CompilerWarningFacts::None,
        use_sites:              sink.use_sites.into_use_sites(),
    };
    fs::write(&output_path, to_vec_pretty(&report)?)
        .with_context(|| format!("failed to write findings file {}", output_path.display()))?;
    Ok(true)
}

/// Orders the sink's findings by source position so the stored report is
/// reproducible, then drops the duplicates that separate scans of the same
/// item produce.
fn sort_and_dedupe(sink: &mut FindingsSink) {
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
    sink.visibility_constraints.sort();
    sink.visibility_constraints.dedup();
}

fn cache_build_kind(tcx: TyCtxt<'_>) -> CacheBuildKind {
    if tcx.sess.opts.test {
        CacheBuildKind::Test
    } else {
        CacheBuildKind::Library
    }
}

fn build_source_cache(tcx: TyCtxt<'_>, crate_root_file: &Path) -> Result<SourceCache> {
    let source_map = tcx.sess.source_map();
    let crate_source_files = source_map
        .files()
        .iter()
        .filter(|source_file| source_file.cnum == LOCAL_CRATE)
        .filter_map(|source_file| {
            let FileName::Real(real_file_name) = &source_file.name else {
                return None;
            };
            real_file_name.local_path()
        })
        .filter(|path| path.extension().and_then(OsStr::to_str) == Some("rs"))
        .map(|path| fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path)))
        .collect::<Vec<_>>();
    SourceCache::build_crate(crate_root_file, &crate_source_files)
}

#[cfg(all(test, feature = "test-counters"))]
mod counter_tests {
    use std::fs;
    use std::path::Path;

    use anyhow::Context;
    use anyhow::Result;
    use anyhow::anyhow;
    use rustc_driver::Callbacks;
    use rustc_driver::Compilation;
    use rustc_hir::def::DefKind;
    use rustc_hir::def::Res;
    use rustc_interface::interface::Compiler;
    use rustc_middle::ty::TyCtxt;
    use rustc_span::def_id::CRATE_DEF_ID;
    use rustc_span::def_id::LocalDefId;
    use tempfile::tempdir;

    use super::collect_and_store_findings;
    use crate::compiler::facade;
    use crate::compiler::persistence::StoredReport;
    use crate::compiler::settings;
    use crate::compiler::settings::DriverSettings;
    use crate::compiler::visibility::use_sites;
    use crate::config::DiagnosticCode;
    use crate::config::VisibilityConfig;

    #[test]
    fn facade_counters_bound_resolution_and_occurrence_usage_scans() -> Result<()> {
        let temp = tempdir()?;
        write_fixture(temp.path())?;
        let source = temp.path().join("src/main.rs");
        let target_directory = temp.path().join("target");
        let findings_dir = target_directory.join("mend-findings");
        fs::create_dir_all(&findings_dir)?;
        let output = target_directory.join("fixture.rmeta");
        let driver_settings = DriverSettings {
            config_root: temp.path().to_path_buf(),
            visibility_config: VisibilityConfig::default(),
            config_fingerprint: String::from("test"),
            analysis_fingerprint: settings::current_analysis_fingerprint(),
            scope_fingerprint: String::from("scope"),
            findings_dir,
            package_root: temp.path().to_path_buf(),
        };
        let arguments = vec![
            String::from("rustc"),
            source.display().to_string(),
            String::from("--crate-name"),
            String::from("facade_counter_fixture"),
            String::from("--edition=2024"),
            String::from("--emit=metadata"),
            String::from("-o"),
            output.display().to_string(),
        ];
        let mut callbacks = CounterAssertions {
            driver_settings,
            result: None,
        };
        rustc_driver::catch_with_exit_code(|| {
            rustc_driver::run_compiler(&arguments, &mut callbacks);
        });

        callbacks.result.context("counter assertions did not run")?
    }

    fn write_fixture(root: &Path) -> Result<()> {
        fs::create_dir_all(root.join("src/a/b/c"))?;
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"facade-counter-fixture\"\nversion = \"0.1.0\"\n\
             edition = \"2024\"\n",
        )?;
        fs::write(root.join("src/main.rs"), "mod a;\nfn main() {}\n")?;
        fs::write(root.join("src/a.rs"), "mod b;\n")?;
        fs::write(root.join("src/a/b.rs"), "mod c;\n")?;
        fs::write(
            root.join("src/a/b/c.rs"),
            "mod blocked_child;\nmod consumer;\nmod glob_child;\nmod review_child;\n\
             pub(super) use blocked_child::*;\npub(super) use glob_child::*;\n\
             pub(crate) use review_child::Reviewed;\n",
        )?;
        fs::write(
            root.join("src/a/b/c/blocked_child.rs"),
            "pub(crate) struct Blocked;\n",
        )?;
        fs::write(
            root.join("src/a/b/c/consumer.rs"),
            "use super::{Reviewed, Second};\nfn use_exports(_: Reviewed, _: Second) {}\n",
        )?;
        fs::write(
            root.join("src/a/b/c/glob_child.rs"),
            "pub struct First;\npub struct Second;\n",
        )?;
        fs::write(
            root.join("src/a/b/c/review_child.rs"),
            "pub struct Reviewed;\npub struct Exposed;\n\
             impl Reviewed { pub fn expose(_: Exposed) {} }\n",
        )?;
        Ok(())
    }

    struct CounterAssertions {
        driver_settings: DriverSettings,
        result:          Option<Result<()>>,
    }

    impl Callbacks for CounterAssertions {
        fn after_analysis(&mut self, _: &Compiler, tcx: TyCtxt<'_>) -> Compilation {
            self.result = Some(assert_counter_behavior(tcx, &self.driver_settings));
            Compilation::Stop
        }
    }

    fn assert_counter_behavior(tcx: TyCtxt<'_>, driver_settings: &DriverSettings) -> Result<()> {
        facade::reset_performance_counters();
        collect_and_store_findings(tcx, driver_settings).context("collect visibility findings")?;

        let crate_module = CRATE_DEF_ID;
        let a_module = child_module(tcx, crate_module, "a")?;
        let b_module = child_module(tcx, a_module, "b")?;
        let c_module = child_module(tcx, b_module, "c")?;
        let review_child = child_module(tcx, c_module, "review_child")?;
        let blocked_child = child_module(tcx, c_module, "blocked_child")?;
        let glob_child = child_module(tcx, c_module, "glob_child")?;
        let reviewed = child_item(tcx, review_child, "Reviewed")?;
        let blocked = child_item(tcx, blocked_child, "Blocked")?;
        let first = child_item(tcx, glob_child, "First")?;
        let second = child_item(tcx, glob_child, "Second")?;
        let index = use_sites::reexport_index(tcx);
        let review_analysis = index
            .parent_facade_analysis(tcx, reviewed, reviewed)
            .context("missing reviewed facade analysis")?;
        let reviewed_occurrence = review_analysis.nearest.selected;
        let blocked_analysis = index
            .parent_facade_analysis(tcx, blocked, blocked)
            .context("missing blocked facade analysis")?;
        let blocked_occurrence = blocked_analysis.nearest.selected;
        let first_analysis = index
            .parent_facade_analysis(tcx, first, first)
            .context("missing first glob facade analysis")?;
        let first_occurrence = first_analysis.nearest.selected;
        let second_analysis = index
            .parent_facade_analysis(tcx, second, second)
            .context("missing second glob facade analysis")?;
        let second_occurrence = second_analysis.nearest.selected;

        assert_eq!(facade::facade_resolution_count(reviewed), 1);
        assert!(facade::facade_resolution_request_count(reviewed) >= 2);
        assert_eq!(facade::facade_resolution_count(blocked), 1);
        assert_eq!(facade::facade_resolution_count(first), 1);
        assert_eq!(facade::facade_resolution_count(second), 1);
        assert_eq!(reviewed_occurrence.export_names, ["Reviewed"]);
        assert_eq!(
            facade::facade_usage_scan_count(reviewed_occurrence.use_def_id),
            1
        );
        assert_eq!(
            facade::facade_usage_scan_count(blocked_occurrence.use_def_id),
            0
        );
        assert_eq!(first_occurrence.use_def_id, second_occurrence.use_def_id);
        assert_eq!(first_occurrence.export_names, ["First", "Second"]);
        assert_eq!(
            facade::facade_usage_scan_count(first_occurrence.use_def_id),
            1
        );
        assert_usage_findings(driver_settings)?;
        Ok(())
    }

    fn assert_usage_findings(driver_settings: &DriverSettings) -> Result<()> {
        let report_path = fs::read_dir(&driver_settings.findings_dir)?
            .find_map(|entry| {
                let entry = entry.ok()?;
                (entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    == Some("json"))
                .then(|| entry.path())
            })
            .context("missing counter report")?;
        let report_bytes = fs::read(&report_path)
            .with_context(|| format!("read counter report {}", report_path.display()))?;
        let report: StoredReport = serde_json::from_slice(&report_bytes)?;
        if !report
            .findings
            .iter()
            .any(|finding| finding.diagnostic_code == DiagnosticCode::InternalParentPubUseFacade)
        {
            return Err(anyhow!(
                "missing internal parent facade finding: {:?}",
                report.findings
            ));
        }
        let first_is_stale = report.findings.iter().any(|finding| {
            finding.diagnostic_code == DiagnosticCode::SuspiciousPub
                && finding.item.as_deref() == Some("struct First")
        });
        let second_is_stale = report.findings.iter().any(|finding| {
            finding.diagnostic_code == DiagnosticCode::SuspiciousPub
                && finding.item.as_deref() == Some("struct Second")
        });
        if !first_is_stale || second_is_stale {
            return Err(anyhow!(
                "glob usage was not attributed by export name: {:?}",
                report.findings
            ));
        }
        Ok(())
    }

    fn child_module(tcx: TyCtxt<'_>, parent: LocalDefId, name: &str) -> Result<LocalDefId> {
        tcx.module_children_local(parent)
            .iter()
            .find_map(|child| match child.res {
                Res::Def(DefKind::Mod, def_id) if child.ident.name.as_str() == name => {
                    def_id.as_local()
                },
                _ => None,
            })
            .ok_or_else(|| anyhow!("missing module {name}"))
    }

    fn child_item(tcx: TyCtxt<'_>, parent: LocalDefId, name: &str) -> Result<LocalDefId> {
        tcx.module_children_local(parent)
            .iter()
            .find_map(|child| match child.res {
                Res::Def(_, def_id) if child.ident.name.as_str() == name => def_id.as_local(),
                _ => None,
            })
            .ok_or_else(|| anyhow!("missing item {name}"))
    }
}
