use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use serde_json::from_str;

use super::StoredAncestorReexport;
use super::StoredFinding;
use super::StoredPubUseFixFact;
use super::StoredReport;
use super::StoredSubtreeReexportFixFact;
use super::caller_aware;
use super::intersection;
use super::visibility_constraint;
use super::visibility_priority;
use crate::compiler::constants::FINDINGS_SCHEMA_VERSION;
use crate::compiler::settings;
use crate::config::DiagnosticCode;
use crate::reporting::AllFeaturesCoverage;
use crate::reporting::AncestorReexport;
use crate::reporting::AncestorReexportInsertion;
use crate::reporting::CompilerWarningFacts;
use crate::reporting::Finding;
use crate::reporting::FixSupport;
use crate::reporting::ItemVisibility;
use crate::reporting::ModuleMountFact;
use crate::reporting::NarrowerScope;
use crate::reporting::PubUseFixFact;
use crate::reporting::Report;
use crate::reporting::ReportFacts;
use crate::reporting::ReportSummary;
use crate::reporting::SubtreeReexportFixFact;
use crate::selection::Selection;

/// Whether any stored report reached this run.
///
/// An empty `Report::findings` means the code is clean only when at least one
/// `StoredReport` matched the selection and passed `stored_report_is_compatible`.
/// With none, the compiler examined no code at all — cargo found nothing to
/// rebuild, so the driver never ran, and no report beside the units cargo listed
/// survived the compatibility check — and the empty findings list is
/// indistinguishable from a clean crate without this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler) enum AnalysisEvidence {
    Present,
    Absent,
}

/// `load_report`'s result: the merged report, and whether anything produced it.
pub(in crate::compiler) struct LoadedReport {
    pub report:            Report,
    pub analysis_evidence: AnalysisEvidence,
}

/// The child declaration a `--fix pub-use` narrowing would rewrite.
///
/// The driver writes a `StoredPubUseFixFact` next to the finding that
/// authorizes it — `SuspiciousPub` on the child declaration, or
/// `InternalParentPubUseFacade` on the parent `pub use` line. The four
/// suppression stages inside `reconcile_cross_target_reports` can drop that
/// finding afterwards; a fact whose site no longer appears in
/// `StoredReport::findings` would let the fixer apply a narrowing the
/// cross-target analysis already rejected.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct PubUseNarrowingSite {
    path:      String,
    line:      usize,
    item_name: String,
}

impl PubUseNarrowingSite {
    /// The two lines a single `pub use` fix can be advertised on. One edit,
    /// two diagnostics: `suspicious_pub` reports it on the child declaration,
    /// `internal_parent_pub_use_facade` reports it on the parent `pub use`
    /// line. Either surviving finding keeps the fact.
    fn advertising_sites(fact: &StoredPubUseFixFact) -> [Self; 2] {
        [
            Self {
                path:      fact.child_path.clone(),
                line:      fact.child_line,
                item_name: fact.child_item_name.clone(),
            },
            Self {
                path:      fact.parent_path.clone(),
                line:      fact.parent_line,
                item_name: fact.child_item_name.clone(),
            },
        ]
    }
}

/// The `pub_use_outside_subtree` finding a `StoredSubtreeReexportFixFact`
/// belongs to: the fact is applicable only while a finding advertising
/// `FixSupport::PubUseOutsideSubtree` survives at this position.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct SubtreeReexportSite {
    path:   String,
    line:   usize,
    column: usize,
}

impl From<&StoredSubtreeReexportFixFact> for SubtreeReexportSite {
    fn from(fact: &StoredSubtreeReexportFixFact) -> Self {
        Self {
            path:   fact.use_path.clone(),
            line:   fact.use_line,
            column: fact.use_column,
        }
    }
}

/// The fix facts every surviving report contributes, relativized to the
/// analysis root.
#[derive(Default)]
struct MergedFixFacts {
    pub_use:           Vec<PubUseFixFact>,
    subtree_reexports: Vec<SubtreeReexportFixFact>,
    module_mounts:     Vec<ModuleMountFact>,
}

/// Merges the reports at `report_paths`, the `StoredReport` files beside the
/// units cargo listed for this run (see `report_path_for_metadata`).
///
/// A listed path with no file is a unit the driver did not analyze — a
/// dependency, or a workspace member outside the selection — and is skipped
/// without affecting `AllFeaturesCoverage`. Reports from other units in the same
/// target directory are never read, so another mend build's reports and reports
/// from a different feature set or scope cannot enter the merge.
pub(in crate::compiler) fn load_report(
    report_paths: &[PathBuf],
    selection: &Selection,
    config_fingerprint: &str,
) -> LoadedReport {
    let selected_roots: Vec<PathBuf> = selection.package_roots.clone();
    let selected_root_strings: Vec<String> = selected_roots
        .iter()
        .map(|root| root.to_string_lossy().into_owned())
        .collect();
    let selected_canonical_roots: Vec<PathBuf> = selected_roots
        .iter()
        .filter_map(|root| fs::canonicalize(root).ok())
        .collect();
    let mut matched_reports: Vec<StoredReport> = Vec::new();
    let mut all_features_coverage = AllFeaturesCoverage::Superset;

    for report_path in report_paths {
        let text = match fs::read_to_string(report_path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(_) => {
                all_features_coverage = AllFeaturesCoverage::NotGuaranteed;
                continue;
            },
        };
        let Ok(stored) = from_str::<StoredReport>(&text) else {
            all_features_coverage = AllFeaturesCoverage::NotGuaranteed;
            continue;
        };
        if !stored_matches_selected_root(
            &stored,
            &selected_roots,
            &selected_root_strings,
            &selected_canonical_roots,
        ) {
            continue;
        }
        if !stored_report_is_compatible(&stored, config_fingerprint) {
            all_features_coverage = AllFeaturesCoverage::NotGuaranteed;
            continue;
        }
        all_features_coverage = all_features_coverage.merge(stored.all_features_coverage);
        matched_reports.push(stored);
    }

    reconcile_cross_target_reports(&mut matched_reports);
    discard_fix_facts_for_suppressed_findings(&mut matched_reports);

    let analysis_evidence = if matched_reports.is_empty() {
        all_features_coverage = AllFeaturesCoverage::default();
        AnalysisEvidence::Absent
    } else {
        AnalysisEvidence::Present
    };

    let mut findings = Vec::new();
    let mut merged_fix_facts = MergedFixFacts::default();
    for stored in matched_reports {
        extend_report_from_stored(
            &mut findings,
            &mut merged_fix_facts,
            stored,
            selection.analysis_root.as_path(),
        );
    }

    sort_and_dedup_findings(&mut findings);
    let MergedFixFacts {
        pub_use: mut pub_use_fix_facts,
        subtree_reexports: mut subtree_reexport_fix_facts,
        module_mounts: mut module_mount_facts,
    } = merged_fix_facts;
    sort_and_dedup_pub_use_fix_facts(&mut pub_use_fix_facts);
    drop_conflicting_subtree_reexport_fix_facts(&mut subtree_reexport_fix_facts);
    retain_mount_facts_of_fixable_crates(&mut module_mount_facts, &subtree_reexport_fix_facts);

    LoadedReport {
        report: Report {
            root: selection_root_string(selection.analysis_root.as_path()),
            summary: ReportSummary::default(),
            findings,
            facts: ReportFacts {
                pub_use_fix_facts: pub_use_fix_facts.into(),
                all_features_coverage,
                compiler_warning_facts: CompilerWarningFacts::None,
                subtree_reexport_fix_facts: subtree_reexport_fix_facts.into(),
                module_mount_facts: module_mount_facts.into(),
            },
        },
        analysis_evidence,
    }
}

fn reconcile_cross_target_reports(reports: &mut [StoredReport]) {
    intersection::apply_cross_compilation_intersection(reports);
    let callers = caller_aware::apply_caller_aware_suppression(reports);
    visibility_constraint::reconcile_visibility_constraints(reports, &callers);
    visibility_priority::apply_visibility_narrowing_priority(reports);
}

/// Drops every stored fix fact whose authorizing finding did not survive
/// `reconcile_cross_target_reports`. Pruning happens once, at that boundary:
/// the surviving finding set is settled the moment those four stages return,
/// and the facts are still untouched here. A report's `module_mount_facts`
/// serve only its subtree re-export facts, so they go with the last of those.
fn discard_fix_facts_for_suppressed_findings(reports: &mut [StoredReport]) {
    for report in reports {
        let subtree_reexport_sites: BTreeSet<SubtreeReexportSite> = report
            .findings
            .iter()
            .filter(|finding| {
                finding.diagnostic_code == DiagnosticCode::PubUseOutsideSubtree
                    && finding.fix_support == FixSupport::PubUseOutsideSubtree
            })
            .map(|finding| SubtreeReexportSite {
                path:   finding.path.clone(),
                line:   finding.line,
                column: finding.column,
            })
            .collect();
        report
            .subtree_reexport_fix_facts
            .retain(|fact| subtree_reexport_sites.contains(&SubtreeReexportSite::from(fact)));
        if report.subtree_reexport_fix_facts.is_empty() {
            report.module_mount_facts.clear();
        }

        let narrowing_sites: BTreeSet<PubUseNarrowingSite> = report
            .findings
            .iter()
            .filter(|finding| finding.fix_support == FixSupport::PubUse)
            .filter_map(|finding| {
                finding.item.as_deref().map(|item| PubUseNarrowingSite {
                    path:      finding.path.clone(),
                    line:      finding.line,
                    // `StoredFinding::item` is a rendering; the fact stores the
                    // bare name, so recover it the sanctioned way.
                    item_name: StoredFinding::item_name(item).to_string(),
                })
            })
            .collect();
        report.pub_use_fix_facts.retain(|fact| {
            PubUseNarrowingSite::advertising_sites(fact)
                .iter()
                .any(|site| narrowing_sites.contains(site))
        });
    }
}

fn sort_and_dedup_findings(findings: &mut Vec<Finding>) {
    findings.sort_by(|a, b| {
        (
            a.severity,
            &a.path,
            a.line,
            a.column,
            &a.diagnostic_code,
            &a.item,
            &a.message,
        )
            .cmp(&(
                b.severity,
                &b.path,
                b.line,
                b.column,
                &b.diagnostic_code,
                &b.item,
                &b.message,
            ))
    });
    findings.dedup_by(|a, b| {
        a.severity == b.severity
            && a.diagnostic_code == b.diagnostic_code
            && a.path == b.path
            && a.line == b.line
            && a.column == b.column
            && a.message == b.message
            && a.item == b.item
    });
    retain_one_restricted_annotation_fix_per_site(findings);
}

/// Leaves one `restricted_annotation` fix advertised per declaration site.
///
/// `suspicious_pub` words its message differently for a binary than for a
/// library, so a declaration compiled into both targets keeps two findings
/// through the dedup above — correctly, since they are two distinct
/// diagnostics. Both would otherwise claim the fix, and the summary would
/// advertise two available fixes where `fixes::restricted_annotation` rewrites
/// the declaration once. The findings are already sorted by position, so the
/// duplicates of a site are adjacent.
fn retain_one_restricted_annotation_fix_per_site(findings: &mut [Finding]) {
    let sites =
        findings.chunk_by_mut(|a, b| a.path == b.path && a.line == b.line && a.column == b.column);
    for site in sites {
        site.iter_mut()
            .filter(|finding| finding.fix_support == FixSupport::RestrictedAnnotation)
            .skip(1)
            .for_each(|finding| finding.fix_support = FixSupport::None);
    }
}

fn sort_and_dedup_pub_use_fix_facts(pub_use_fix_facts: &mut Vec<PubUseFixFact>) {
    pub_use_fix_facts.sort_by(|a, b| {
        (
            &a.child_path,
            a.child_line,
            &a.child_item_name,
            &a.parent_path,
            a.parent_line,
            &a.child_module,
        )
            .cmp(&(
                &b.child_path,
                b.child_line,
                &b.child_item_name,
                &b.parent_path,
                b.parent_line,
                &b.child_module,
            ))
    });
    pub_use_fix_facts.dedup_by(|a, b| {
        a.child_path == b.child_path
            && a.child_line == b.child_line
            && a.child_item_name == b.child_item_name
            && a.parent_path == b.parent_path
            && a.parent_line == b.parent_line
            && a.child_module == b.child_module
    });
}

/// Sorts and dedups on full content, then drops every use site that still
/// carries more than one distinct fact. Two compilation units that disagree on
/// how to rewrite one re-export leave the fixer nothing it can trust, so it
/// skips the site; the finding keeps advertising the fix.
fn drop_conflicting_subtree_reexport_fix_facts(facts: &mut Vec<SubtreeReexportFixFact>) {
    facts.sort();
    facts.dedup();
    let agreed = facts
        .chunk_by(|a, b| {
            a.use_path == b.use_path && a.use_line == b.use_line && a.use_column == b.use_column
        })
        .filter(|site| site.len() == 1)
        .flatten()
        .cloned()
        .collect();
    *facts = agreed;
}

/// Keeps a mount fact only while some surviving subtree re-export fact comes
/// from the same crate root, since the fixer reads mounts only for those, then
/// sorts and dedups the survivors.
fn retain_mount_facts_of_fixable_crates(
    module_mount_facts: &mut Vec<ModuleMountFact>,
    subtree_reexport_fix_facts: &[SubtreeReexportFixFact],
) {
    let fixable_crate_roots: BTreeSet<&str> = subtree_reexport_fix_facts
        .iter()
        .map(|fact| fact.crate_root_file.as_str())
        .collect();
    module_mount_facts.retain(|fact| fixable_crate_roots.contains(fact.crate_root_file.as_str()));
    module_mount_facts.sort();
    module_mount_facts.dedup();
}

fn stored_report_is_compatible(stored: &StoredReport, config_fingerprint: &str) -> bool {
    stored.version == FINDINGS_SCHEMA_VERSION
        && stored.analysis_fingerprint == settings::current_analysis_fingerprint()
        && stored.config_fingerprint == config_fingerprint
        && stored_crate_root_exists(stored)
}

fn stored_crate_root_exists(stored: &StoredReport) -> bool {
    stored.crate_root_file.is_empty() || {
        let crate_root = Path::new(&stored.crate_root_file);
        if crate_root.is_absolute() {
            crate_root.exists()
        } else {
            Path::new(&stored.package_root).join(crate_root).exists()
        }
    }
}

fn stored_matches_selected_root(
    stored: &StoredReport,
    selected_roots: &[PathBuf],
    selected_root_strings: &[String],
    selected_canonical_roots: &[PathBuf],
) -> bool {
    selected_root_strings
        .iter()
        .any(|root| root == &stored.package_root)
        || fs::canonicalize(Path::new(&stored.package_root)).is_ok_and(|stored_root| {
            selected_canonical_roots
                .iter()
                .any(|selected_root| selected_root == &stored_root)
        })
        || (stored.package_root.is_empty() && selected_roots.len() == 1)
}

fn extend_report_from_stored(
    findings: &mut Vec<Finding>,
    merged_fix_facts: &mut MergedFixFacts,
    stored: StoredReport,
    analysis_root: &Path,
) {
    for finding in stored.findings {
        findings.push(Finding {
            severity:        finding.severity,
            diagnostic_code: finding.diagnostic_code,
            path:            relativize_path(&finding.path, analysis_root),
            line:            finding.line,
            column:          finding.column,
            highlight_len:   finding.highlight_len,
            source_line:     finding.source_line,
            item:            finding.item,
            message:         finding.message,
            suggestion:      finding.suggestion,
            fix_support:     finding.fix_support,
            related:         finding
                .related
                .map(|related| relativize_path(&related, analysis_root)),
            item_visibility: ItemVisibility {
                written:        finding.visibility_annotation.into(),
                narrower_scope: NarrowerScope::resolve(
                    finding.narrower_scope_def_path,
                    finding.fix_support,
                    finding.exact_boundary_spelling,
                ),
            },
        });
    }
    for fact in stored.pub_use_fix_facts {
        merged_fix_facts.pub_use.push(PubUseFixFact {
            child_path:      relativize_path(&fact.child_path, analysis_root),
            child_line:      fact.child_line,
            child_item_name: fact.child_item_name,
            parent_path:     relativize_path(&fact.parent_path, analysis_root),
            parent_line:     fact.parent_line,
            child_module:    fact.child_module,
        });
    }
    for fact in stored.subtree_reexport_fix_facts {
        merged_fix_facts
            .subtree_reexports
            .push(subtree_reexport_fix_fact(fact, analysis_root));
    }
    for fact in stored.module_mount_facts {
        merged_fix_facts.module_mounts.push(ModuleMountFact {
            crate_root_file: relativize_path(&fact.crate_root_file, analysis_root),
            file:            relativize_path(&fact.file, analysis_root),
            module_path:     fact.module_path,
        });
    }
}

fn subtree_reexport_fix_fact(
    fact: StoredSubtreeReexportFixFact,
    analysis_root: &Path,
) -> SubtreeReexportFixFact {
    let ancestor_reexport = match fact.ancestor_reexport {
        StoredAncestorReexport::NotRequired => AncestorReexport::NotRequired,
        StoredAncestorReexport::Insert(insertion) => {
            AncestorReexport::Insert(AncestorReexportInsertion {
                reexport_visibility: insertion.reexport_visibility,
                relative_path:       insertion.relative_path,
                cfg_attributes:      insertion.cfg_attributes,
                file:                relativize_path(&insertion.file, analysis_root),
                offset:              insertion.offset,
                indent:              insertion.indent,
            })
        },
    };
    SubtreeReexportFixFact {
        use_path: relativize_path(&fact.use_path, analysis_root),
        use_line: fact.use_line,
        use_column: fact.use_column,
        exported_name: fact.exported_name,
        written_path: fact.written_path,
        owner_module: fact.owner_module,
        owner_module_form: fact.owner_module_form,
        source_module: fact.source_module,
        target_scope: fact.target_scope,
        common_ancestor: fact.common_ancestor,
        crate_root_file: relativize_path(&fact.crate_root_file, analysis_root),
        ancestor_reexport,
    }
}

fn selection_root_string(root: &Path) -> String { root.display().to_string() }

fn relativize_path(path: &str, analysis_root: &Path) -> String {
    let absolute = Path::new(path);
    absolute.strip_prefix(analysis_root).map_or_else(
        |_| path.to_string(),
        |relative| relative.to_string_lossy().replace('\\', "/"),
    )
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;

    use serde_json::to_vec_pretty;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::AnalysisEvidence;
    use super::discard_fix_facts_for_suppressed_findings;
    use super::drop_conflicting_subtree_reexport_fix_facts;
    use super::load_report;
    use crate::compiler::constants::FINDINGS_SCHEMA_VERSION;
    use crate::compiler::persistence::StoredAncestorReexport;
    use crate::compiler::persistence::StoredFinding;
    use crate::compiler::persistence::StoredModuleMountFact;
    use crate::compiler::persistence::StoredPubUseFixFact;
    use crate::compiler::persistence::StoredReport;
    use crate::compiler::persistence::StoredSubtreeReexportFixFact;
    use crate::compiler::settings;
    use crate::config::DiagnosticCode;
    use crate::reporting::AllFeaturesCoverage;
    use crate::reporting::AncestorReexport;
    use crate::reporting::AncestorReexportInsertion;
    use crate::reporting::CompilerWarningFacts;
    use crate::reporting::ExactBoundarySpelling;
    use crate::reporting::FixSupport;
    use crate::reporting::ModuleForm;
    use crate::reporting::ReexportVisibility;
    use crate::reporting::Severity;
    use crate::reporting::SubtreeReexportFixFact;
    use crate::selection::Selection;
    use crate::selection::SelectionScope;

    const CONFIG_FINGERPRINT: &str = "config-fingerprint";

    struct PersistenceFixture {
        temp:         TempDir,
        reports_dir:  PathBuf,
        package_root: PathBuf,
        crate_root:   PathBuf,
    }

    impl PersistenceFixture {
        fn new() -> Self {
            let temp = tempdir().expect("create persistence fixture");
            let package_root = temp.path().join("package");
            let source_dir = package_root.join("src");
            fs::create_dir_all(&source_dir).expect("create package src dir");
            let crate_root = source_dir.join("lib.rs");
            fs::write(&crate_root, "pub fn item() {}\n").expect("write crate root");
            let reports_dir = temp.path().join("deps");
            fs::create_dir_all(&reports_dir).expect("create reports dir");
            Self {
                temp,
                reports_dir,
                package_root,
                crate_root,
            }
        }

        fn selection(&self) -> Selection {
            self.selection_with_roots(vec![self.package_root.clone()])
        }

        fn selection_with_roots(&self, package_roots: Vec<PathBuf>) -> Selection {
            Selection {
                manifest_path: self.package_root.join("Cargo.toml"),
                manifest_dir: self.package_root.clone(),
                workspace_root: self.package_root.clone(),
                target_directory: self.temp.path().join("target"),
                analysis_root: self.package_root.clone(),
                scope: SelectionScope::SinglePackage,
                package_roots,
                packages: Vec::new(),
            }
        }

        fn write_report(&self, file_name: &str, report: &StoredReport) -> PathBuf {
            let report_path = self.reports_dir.join(file_name);
            fs::write(
                &report_path,
                to_vec_pretty(report).expect("serialize stored report"),
            )
            .expect("write stored report");
            report_path
        }

        fn write_malformed_json(&self, file_name: &str) -> PathBuf {
            let report_path = self.reports_dir.join(file_name);
            fs::write(&report_path, b"{ not json").expect("write malformed report");
            report_path
        }

        fn report_with_findings(&self, findings: Vec<StoredFinding>) -> StoredReport {
            StoredReport {
                version: FINDINGS_SCHEMA_VERSION,
                analysis_fingerprint: settings::current_analysis_fingerprint(),
                scope_fingerprint: "scope".to_string(),
                package_root: self.package_root.to_string_lossy().into_owned(),
                crate_root_file: self.crate_root.to_string_lossy().into_owned(),
                config_fingerprint: CONFIG_FINGERPRINT.to_string(),
                source_files: Vec::new(),
                findings,
                visibility_constraints: Vec::new(),
                pub_use_fix_facts: Vec::new(),
                subtree_reexport_fix_facts: Vec::new(),
                module_mount_facts: Vec::new(),
                all_features_coverage: AllFeaturesCoverage::default(),
                compiler_warning_facts: CompilerWarningFacts::None,
                use_sites: Vec::new(),
            }
        }
    }

    #[test]
    fn malformed_json_prevents_all_features_coverage() {
        let fixture = PersistenceFixture::new();
        let finding = stored_finding(
            DiagnosticCode::OverbroadPubCrate,
            &fixture.crate_root,
            "item",
            1,
        );
        let mut report = fixture.report_with_findings(vec![finding]);
        report.all_features_coverage = AllFeaturesCoverage::Superset;
        let report_paths = [
            fixture.write_malformed_json("broken.mend.json"),
            fixture.write_report("valid.mend.json", &report),
        ];

        let loaded = load_report(&report_paths, &fixture.selection(), CONFIG_FINGERPRINT);

        assert_eq!(loaded.report.findings.len(), 1);
        assert_eq!(
            loaded.report.facts.all_features_coverage,
            AllFeaturesCoverage::NotGuaranteed
        );
    }

    #[test]
    fn a_missing_report_is_not_a_clean_crate() {
        let fixture = PersistenceFixture::new();
        let report_paths = [fixture
            .reports_dir
            .join("libabsent-0000000000000000.mend.json")];

        let loaded = load_report(&report_paths, &fixture.selection(), CONFIG_FINGERPRINT);

        assert!(loaded.report.findings.is_empty());
        assert_eq!(
            loaded.analysis_evidence,
            AnalysisEvidence::Absent,
            "no stored report means nothing was analyzed, not that the crate is clean"
        );
        assert_eq!(
            loaded.report.facts.all_features_coverage,
            AllFeaturesCoverage::default(),
            "a listed unit the driver did not analyze must not lower coverage"
        );
    }

    #[test]
    fn a_report_outside_the_listed_paths_is_not_loaded() {
        let fixture = PersistenceFixture::new();
        let listed = fixture.report_with_findings(vec![stored_finding(
            DiagnosticCode::OverbroadPubCrate,
            &fixture.crate_root,
            "listed",
            1,
        )]);
        let sibling = fixture.report_with_findings(vec![stored_finding(
            DiagnosticCode::OverbroadPubCrate,
            &fixture.crate_root,
            "sibling",
            2,
        )]);
        let report_paths = [fixture.write_report("liblisted-0000000000000001.mend.json", &listed)];
        fixture.write_report("liblisted-0000000000000002.mend.json", &sibling);

        let loaded = load_report(&report_paths, &fixture.selection(), CONFIG_FINGERPRINT);

        assert_eq!(loaded.report.findings.len(), 1);
        assert_eq!(loaded.report.findings[0].item.as_deref(), Some("listed"));
    }

    #[test]
    fn wrong_schema_or_fingerprint_reports_are_rejected() {
        let fixture = PersistenceFixture::new();
        let mut wrong_schema = fixture.report_with_findings(vec![stored_finding(
            DiagnosticCode::OverbroadPubCrate,
            &fixture.crate_root,
            "item",
            1,
        )]);
        wrong_schema.pub_use_fix_facts.push(StoredPubUseFixFact {
            child_path:      fixture.crate_root.to_string_lossy().into_owned(),
            child_line:      2,
            child_item_name: "Child".to_string(),
            parent_path:     fixture.crate_root.to_string_lossy().into_owned(),
            parent_line:     3,
            child_module:    "child".to_string(),
        });
        wrong_schema.version = FINDINGS_SCHEMA_VERSION - 1;
        let mut wrong_analysis = fixture.report_with_findings(vec![stored_finding(
            DiagnosticCode::OverbroadPubCrate,
            &fixture.crate_root,
            "item",
            1,
        )]);
        wrong_analysis.analysis_fingerprint = "old-analysis".to_string();
        let mut wrong_config = fixture.report_with_findings(vec![stored_finding(
            DiagnosticCode::OverbroadPubCrate,
            &fixture.crate_root,
            "item",
            1,
        )]);
        wrong_config.config_fingerprint = "old-config".to_string();
        let report_paths = [
            fixture.write_report("schema.mend.json", &wrong_schema),
            fixture.write_report("analysis.mend.json", &wrong_analysis),
            fixture.write_report("config.mend.json", &wrong_config),
        ];

        let loaded = load_report(&report_paths, &fixture.selection(), CONFIG_FINGERPRINT);

        assert!(loaded.report.findings.is_empty());
        assert!(
            loaded
                .report
                .facts
                .pub_use_fix_facts
                .iter()
                .next()
                .is_none()
        );
        assert_eq!(loaded.analysis_evidence, AnalysisEvidence::Absent);
    }

    #[test]
    fn missing_crate_root_report_is_rejected() {
        let fixture = PersistenceFixture::new();
        let finding = stored_finding(
            DiagnosticCode::OverbroadPubCrate,
            &fixture.crate_root,
            "item",
            1,
        );
        let mut report = fixture.report_with_findings(vec![finding]);
        report.crate_root_file = "src/missing.rs".to_string();
        let report_paths = [fixture.write_report("missing-root.mend.json", &report)];

        let loaded = load_report(&report_paths, &fixture.selection(), CONFIG_FINGERPRINT);

        assert!(loaded.report.findings.is_empty());
        assert_eq!(loaded.analysis_evidence, AnalysisEvidence::Absent);
    }

    #[test]
    fn canonical_selected_roots_are_accepted() {
        let fixture = PersistenceFixture::new();
        let finding = stored_finding(
            DiagnosticCode::OverbroadPubCrate,
            &fixture.crate_root,
            "item",
            1,
        );
        let report = fixture.report_with_findings(vec![finding]);
        let report_paths = [fixture.write_report("canonical-root.mend.json", &report)];
        let selected_root = fixture.package_root.join("src").join("..");
        let selection = fixture.selection_with_roots(vec![selected_root]);

        let loaded = load_report(&report_paths, &selection, CONFIG_FINGERPRINT);

        assert_eq!(loaded.report.findings.len(), 1);
    }

    #[test]
    fn empty_package_root_compatibility_is_retained() {
        let fixture = PersistenceFixture::new();
        let finding = stored_finding(
            DiagnosticCode::OverbroadPubCrate,
            &fixture.crate_root,
            "item",
            1,
        );
        let mut report = fixture.report_with_findings(vec![finding]);
        report.package_root.clear();
        report.crate_root_file.clear();
        let report_paths = [fixture.write_report("legacy-root.mend.json", &report)];

        let loaded = load_report(&report_paths, &fixture.selection(), CONFIG_FINGERPRINT);

        assert_eq!(loaded.report.findings.len(), 1);
    }

    #[test]
    fn serialized_driver_report_loads_back_through_load_report() {
        let fixture = PersistenceFixture::new();
        // The fact rides with the finding that authorizes it: same file, same
        // line, same item name.
        let mut finding = stored_finding(
            DiagnosticCode::SuspiciousPub,
            &fixture.crate_root,
            "Child",
            2,
        );
        finding.item = Some(StoredFinding::render_item("struct", "Child"));
        finding.fix_support = FixSupport::PubUse;
        let mut report = fixture.report_with_findings(vec![finding]);
        report.pub_use_fix_facts.push(StoredPubUseFixFact {
            child_path:      fixture.crate_root.to_string_lossy().into_owned(),
            child_line:      2,
            child_item_name: "Child".to_string(),
            parent_path:     fixture.crate_root.to_string_lossy().into_owned(),
            parent_line:     3,
            child_module:    "child".to_string(),
        });
        let report_paths = [fixture.write_report("driver-report.mend.json", &report)];

        let loaded = load_report(&report_paths, &fixture.selection(), CONFIG_FINGERPRINT);
        let facts = loaded
            .report
            .facts
            .pub_use_fix_facts
            .iter()
            .collect::<Vec<_>>();

        assert_eq!(loaded.analysis_evidence, AnalysisEvidence::Present);
        assert_eq!(loaded.report.findings.len(), 1);
        assert_eq!(loaded.report.findings[0].path, "src/lib.rs");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].child_path, "src/lib.rs");
    }

    #[test]
    fn suppressed_finding_leaves_no_applicable_pub_use_fix_fact() {
        let fixture = PersistenceFixture::new();
        let mut suspicious = stored_finding(
            DiagnosticCode::SuspiciousPub,
            &fixture.crate_root,
            "Child",
            2,
        );
        suspicious.item = Some(StoredFinding::render_item("struct", "Child"));
        suspicious.fix_support = FixSupport::PubUse;
        // `apply_visibility_narrowing_priority` drops a `suspicious_pub`
        // finding that shares its path/line/column with an `unused_pub` one.
        let unused = stored_finding(DiagnosticCode::UnusedPub, &fixture.crate_root, "Child", 2);
        let mut report = fixture.report_with_findings(vec![suspicious, unused]);
        report.pub_use_fix_facts.push(StoredPubUseFixFact {
            child_path:      fixture.crate_root.to_string_lossy().into_owned(),
            child_line:      2,
            child_item_name: "Child".to_string(),
            parent_path:     fixture.crate_root.to_string_lossy().into_owned(),
            parent_line:     3,
            child_module:    "child".to_string(),
        });
        let report_paths = [fixture.write_report("suppressed.mend.json", &report)];

        let loaded = load_report(&report_paths, &fixture.selection(), CONFIG_FINGERPRINT);

        assert_eq!(loaded.report.findings.len(), 1);
        assert_eq!(
            loaded.report.findings[0].diagnostic_code,
            DiagnosticCode::UnusedPub
        );
        assert!(
            loaded
                .report
                .facts
                .pub_use_fix_facts
                .iter()
                .next()
                .is_none(),
            "a suppressed finding must not leave its pub-use fix fact behind"
        );
    }

    #[test]
    fn surviving_finding_keeps_its_pub_use_fix_fact() {
        let fixture = PersistenceFixture::new();
        let mut suspicious = stored_finding(
            DiagnosticCode::SuspiciousPub,
            &fixture.crate_root,
            "Child",
            2,
        );
        suspicious.item = Some(StoredFinding::render_item("struct", "Child"));
        suspicious.fix_support = FixSupport::PubUse;
        let mut report = fixture.report_with_findings(vec![suspicious]);
        report.pub_use_fix_facts.push(StoredPubUseFixFact {
            child_path:      fixture.crate_root.to_string_lossy().into_owned(),
            child_line:      2,
            child_item_name: "Child".to_string(),
            parent_path:     fixture.crate_root.to_string_lossy().into_owned(),
            parent_line:     3,
            child_module:    "child".to_string(),
        });
        let report_paths = [fixture.write_report("surviving.mend.json", &report)];

        let loaded = load_report(&report_paths, &fixture.selection(), CONFIG_FINGERPRINT);

        assert_eq!(loaded.report.findings.len(), 1);
        assert_eq!(loaded.report.facts.pub_use_fix_facts.iter().count(), 1);
    }

    #[test]
    fn subtree_reexport_fix_fact_lives_only_while_its_fixable_finding_survives() {
        let fixture = PersistenceFixture::new();
        let mut fixable = stored_finding(
            DiagnosticCode::PubUseOutsideSubtree,
            &fixture.crate_root,
            "N",
            2,
        );
        fixable.fix_support = FixSupport::PubUseOutsideSubtree;
        let unfixable = stored_finding(
            DiagnosticCode::PubUseOutsideSubtree,
            &fixture.crate_root,
            "M",
            3,
        );
        let mut report = fixture.report_with_findings(vec![fixable, unfixable]);
        report.subtree_reexport_fix_facts = vec![
            stored_subtree_reexport_fix_fact(&fixture.crate_root, 2),
            stored_subtree_reexport_fix_fact(&fixture.crate_root, 3),
            stored_subtree_reexport_fix_fact(&fixture.crate_root, 4),
        ];
        report.module_mount_facts = vec![stored_module_mount_fact(&fixture.crate_root)];
        let mut reports = [report];

        discard_fix_facts_for_suppressed_findings(&mut reports);

        let lines = reports[0]
            .subtree_reexport_fix_facts
            .iter()
            .map(|fact| fact.use_line)
            .collect::<Vec<_>>();
        assert_eq!(lines, [2], "only the fixable finding's fact survives");
        assert_eq!(reports[0].module_mount_facts.len(), 1);
    }

    #[test]
    fn mount_facts_go_with_the_last_subtree_reexport_fix_fact() {
        let fixture = PersistenceFixture::new();
        let unfixable = stored_finding(
            DiagnosticCode::PubUseOutsideSubtree,
            &fixture.crate_root,
            "N",
            2,
        );
        let mut report = fixture.report_with_findings(vec![unfixable]);
        report.subtree_reexport_fix_facts =
            vec![stored_subtree_reexport_fix_fact(&fixture.crate_root, 2)];
        report.module_mount_facts = vec![stored_module_mount_fact(&fixture.crate_root)];
        let mut reports = [report];

        discard_fix_facts_for_suppressed_findings(&mut reports);

        assert!(reports[0].subtree_reexport_fix_facts.is_empty());
        assert!(
            reports[0].module_mount_facts.is_empty(),
            "mount facts serve only subtree re-export facts"
        );
    }

    #[test]
    fn conflicting_subtree_reexport_fix_facts_are_both_dropped() {
        let agreed = subtree_reexport_fix_fact(2, AncestorReexport::NotRequired);
        let conflicting_first = subtree_reexport_fix_fact(5, AncestorReexport::NotRequired);
        let conflicting_second = subtree_reexport_fix_fact(
            5,
            AncestorReexport::Insert(AncestorReexportInsertion {
                reexport_visibility: ReexportVisibility::Crate,
                relative_path:       "child::N".to_string(),
                cfg_attributes:      Vec::new(),
                file:                "src/lib.rs".to_string(),
                offset:              0,
                indent:              String::new(),
            }),
        );
        let mut facts = vec![
            conflicting_first,
            agreed.clone(),
            agreed.clone(),
            conflicting_second,
        ];

        drop_conflicting_subtree_reexport_fix_facts(&mut facts);

        assert_eq!(
            facts,
            [agreed],
            "identical facts merge; differing facts for one use site are both dropped"
        );
    }

    fn stored_subtree_reexport_fix_fact(
        path: &Path,
        use_line: usize,
    ) -> StoredSubtreeReexportFixFact {
        StoredSubtreeReexportFixFact {
            use_path: path.to_string_lossy().into_owned(),
            use_line,
            use_column: 1,
            exported_name: "N".to_string(),
            written_path: "super::sibling::N".to_string(),
            owner_module: vec!["owner".to_string()],
            owner_module_form: ModuleForm::File,
            source_module: vec!["sibling".to_string()],
            target_scope: Vec::new(),
            common_ancestor: Vec::new(),
            crate_root_file: path.to_string_lossy().into_owned(),
            ancestor_reexport: StoredAncestorReexport::NotRequired,
        }
    }

    fn stored_module_mount_fact(crate_root: &Path) -> StoredModuleMountFact {
        StoredModuleMountFact {
            crate_root_file: crate_root.to_string_lossy().into_owned(),
            file:            crate_root
                .with_file_name("generated.rs")
                .to_string_lossy()
                .into_owned(),
            module_path:     vec!["generated".to_string()],
        }
    }

    fn subtree_reexport_fix_fact(
        use_line: usize,
        ancestor_reexport: AncestorReexport,
    ) -> SubtreeReexportFixFact {
        SubtreeReexportFixFact {
            use_path: "src/owner.rs".to_string(),
            use_line,
            use_column: 1,
            exported_name: "N".to_string(),
            written_path: "super::sibling::N".to_string(),
            owner_module: vec!["owner".to_string()],
            owner_module_form: ModuleForm::File,
            source_module: vec!["sibling".to_string()],
            target_scope: Vec::new(),
            common_ancestor: Vec::new(),
            crate_root_file: "src/lib.rs".to_string(),
            ancestor_reexport,
        }
    }

    fn stored_finding(
        diagnostic_code: DiagnosticCode,
        path: &Path,
        item: &str,
        line: usize,
    ) -> StoredFinding {
        StoredFinding {
            severity: Severity::Warning,
            diagnostic_code,
            path: path.to_string_lossy().into_owned(),
            line,
            column: 1,
            highlight_len: 3,
            source_line: "pub fn item() {}".to_string(),
            item: Some(item.to_string()),
            message: format!("{item} should change visibility"),
            suggestion: Some("use narrower visibility".to_string()),
            fix_support: FixSupport::None,
            related: None,
            visibility_annotation: None,
            item_def_path: None,
            narrower_scope_def_path: None,
            exact_boundary_spelling: ExactBoundarySpelling::CratePath,
        }
    }
}
