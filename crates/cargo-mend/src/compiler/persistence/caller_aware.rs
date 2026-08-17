use std::collections::BTreeMap;
use std::collections::BTreeSet;

use super::StoredReport;
use super::schema::UseSiteReference;
use crate::compiler::visibility;
use crate::config::DiagnosticCode;
use crate::reporting::FixSupport;

pub(super) type CallerMap = BTreeMap<CallerKey, ItemCallers>;

/// The modules reaching one item across every compilation target, split by
/// whether a re-export of the item could serve them. `naming` is a subset of
/// `reaching`: a module that writes the item's path also reaches it.
#[derive(Default)]
pub(super) struct ItemCallers {
    /// Modules that write a path to the item.
    pub naming:             BTreeSet<String>,
    /// Every module that reaches the item, including those that reach it only
    /// through the signature of some other item they named.
    pub reaching:           BTreeSet<String>,
    /// Reach backed by an actual expression, type, or signature rather than
    /// an import declaration without a semantic reference.
    pub semantic_reaching:  BTreeSet<String>,
    /// Private imports must delay a visibility rewrite until the import is
    /// removed, but they do not prove that the item is used.
    pub private_imports:    BTreeSet<String>,
    /// Restricted imports impose their written boundary even when no semantic
    /// reference uses the imported name.
    pub restricted_imports: BTreeSet<String>,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct CallerKey {
    package_root:    String,
    target_def_path: String,
}

impl CallerKey {
    fn for_package(package_root: &str, target_def_path: &str) -> Self {
        Self {
            package_root:    package_root.to_string(),
            target_def_path: target_def_path.to_string(),
        }
    }
}

pub(super) fn apply_caller_aware_suppression(reports: &mut [StoredReport]) -> CallerMap {
    let callers = collect_callers(reports);
    for report in reports {
        suppress_invalid_narrowings(report, &callers);
    }
    callers
}

fn collect_callers(reports: &[StoredReport]) -> CallerMap {
    let mut callers = CallerMap::new();
    for report in reports {
        for site in &report.use_sites {
            let item_callers = callers
                .entry(CallerKey::for_package(
                    &report.package_root,
                    &site.target_def_path,
                ))
                .or_default();
            item_callers
                .reaching
                .insert(site.caller_module_def_path.clone());
            match site.reference {
                UseSiteReference::Named => {
                    item_callers
                        .naming
                        .insert(site.caller_module_def_path.clone());
                    item_callers
                        .semantic_reaching
                        .insert(site.caller_module_def_path.clone());
                },
                UseSiteReference::ThroughSignature => {
                    item_callers
                        .semantic_reaching
                        .insert(site.caller_module_def_path.clone());
                },
                UseSiteReference::PrivateImport => {
                    item_callers
                        .naming
                        .insert(site.caller_module_def_path.clone());
                    item_callers
                        .private_imports
                        .insert(site.caller_module_def_path.clone());
                },
                UseSiteReference::RestrictedImport => {
                    item_callers
                        .naming
                        .insert(site.caller_module_def_path.clone());
                    item_callers
                        .restricted_imports
                        .insert(site.caller_module_def_path.clone());
                },
                // `reaching` only: a declaration's own signature caps how far
                // the annotation may narrow without proving anyone uses the
                // item, so it must not suppress the narrowing finding.
                UseSiteReference::DeclarationInterface => {},
            }
        }
    }
    callers
}

fn suppress_invalid_narrowings(report: &mut StoredReport, callers: &CallerMap) {
    let package_root = report.package_root.clone();
    let constrained_sites = report
        .visibility_constraints
        .iter()
        .map(|constraint| {
            (
                constraint.diagnostic_code,
                constraint.source.path.clone(),
                constraint.source.line,
                constraint.source.column,
            )
        })
        .collect::<BTreeSet<_>>();
    report.findings.retain_mut(|finding| {
        if matches!(
            finding.diagnostic_code,
            DiagnosticCode::OverbroadPubCrate | DiagnosticCode::ForbiddenPubInCrate
        ) || constrained_sites.contains(&(
            finding.diagnostic_code,
            finding.path.clone(),
            finding.line,
            finding.column,
        )) {
            true
        } else {
            retain_narrowing_finding(finding, callers, &package_root)
        }
    });
}

fn retain_narrowing_finding(
    finding: &mut super::StoredFinding,
    callers: &CallerMap,
    package_root: &str,
) -> bool {
    let Some(item_path) = finding.item_def_path.as_deref() else {
        return true;
    };
    let Some(narrower_scope) = finding.narrower_scope_def_path.as_deref() else {
        return true;
    };
    let Some(item_callers) = callers_for_package(callers, package_root, item_path) else {
        return true;
    };
    if item_callers
        .semantic_reaching
        .iter()
        .any(|caller| !visibility::def_path_is_descendant(caller, narrower_scope))
    {
        return false;
    }
    if item_callers
        .restricted_imports
        .iter()
        .any(|caller| !visibility::def_path_is_descendant(caller, narrower_scope))
    {
        return finding.diagnostic_code == DiagnosticCode::SuspiciousPub
            && (finding.related.is_some()
                || matches!(
                    finding.fix_support,
                    crate::reporting::FixSupport::PubUse
                        | crate::reporting::FixSupport::NeedsManualPubUseCleanup
                ));
    }
    if item_callers
        .private_imports
        .iter()
        .any(|caller| !visibility::def_path_is_descendant(caller, narrower_scope))
    {
        finding.fix_support = FixSupport::BlockedByPrivateImport;
        finding.related = Some(String::from(
            "remove the unused private import before narrowing this item",
        ));
    }
    true
}

pub(super) fn callers_for_package<'a>(
    callers: &'a CallerMap,
    package_root: &str,
    item_def_path: &str,
) -> Option<&'a ItemCallers> {
    callers.get(&CallerKey::for_package(package_root, item_def_path))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::apply_caller_aware_suppression;
    use crate::compiler::constants::FINDINGS_SCHEMA_VERSION;
    use crate::compiler::persistence::StoredFinding;
    use crate::compiler::persistence::StoredReport;
    use crate::compiler::persistence::schema::UseSite;
    use crate::compiler::persistence::schema::UseSiteReference;
    use crate::compiler::settings;
    use crate::config::DiagnosticCode;
    use crate::reporting::AllFeaturesCoverage;
    use crate::reporting::CompilerWarningFacts;
    use crate::reporting::ExactBoundarySpelling;
    use crate::reporting::FixSupport;
    use crate::reporting::Severity;

    const CONFIG_FINGERPRINT: &str = "config-fingerprint";

    #[test]
    fn caller_aware_suppression_drops_narrowing_outside_scope() {
        let item_path = "crate::module::item";
        let narrower_scope = "crate::module";
        let mut reports = vec![StoredReport {
            findings: vec![narrowing_finding(
                DiagnosticCode::SuspiciousPub,
                item_path,
                narrower_scope,
            )],
            use_sites: vec![UseSite {
                target_def_path:        item_path.to_string(),
                caller_module_def_path: "crate::other".to_string(),
                reference:              UseSiteReference::Named,
            }],
            ..report_for_test()
        }];

        apply_caller_aware_suppression(&mut reports);

        assert!(reports[0].findings.is_empty());
    }

    #[test]
    fn private_import_keeps_the_warning_but_blocks_its_rewrite() {
        let item_path = "crate::module::item";
        let mut finding = narrowing_finding(DiagnosticCode::UnusedPub, item_path, "crate::module");
        finding.fix_support = FixSupport::UnusedPub;
        let mut reports = vec![StoredReport {
            findings: vec![finding],
            use_sites: vec![UseSite {
                target_def_path:        item_path.to_string(),
                caller_module_def_path: "crate::other".to_string(),
                reference:              UseSiteReference::PrivateImport,
            }],
            ..report_for_test()
        }];

        apply_caller_aware_suppression(&mut reports);

        assert_eq!(reports[0].findings.len(), 1);
        assert_eq!(
            reports[0].findings[0].fix_support,
            FixSupport::BlockedByPrivateImport
        );
    }

    #[test]
    fn restricted_import_suppresses_a_direct_narrowing_but_keeps_facade_cleanup() {
        let item_path = "crate::module::item";
        let direct = narrowing_finding(DiagnosticCode::UnusedPub, item_path, "crate::module");
        let mut cleanup =
            narrowing_finding(DiagnosticCode::SuspiciousPub, item_path, "crate::module");
        cleanup.fix_support = FixSupport::PubUse;
        let mut reports = vec![StoredReport {
            findings: vec![direct, cleanup],
            use_sites: vec![UseSite {
                target_def_path:        item_path.to_string(),
                caller_module_def_path: "crate".to_string(),
                reference:              UseSiteReference::RestrictedImport,
            }],
            ..report_for_test()
        }];

        apply_caller_aware_suppression(&mut reports);

        assert_eq!(reports[0].findings.len(), 1);
        assert_eq!(reports[0].findings[0].fix_support, FixSupport::PubUse);
    }

    fn narrowing_finding(
        diagnostic_code: DiagnosticCode,
        item_path: &str,
        narrower_scope: &str,
    ) -> StoredFinding {
        StoredFinding {
            item_def_path: Some(item_path.to_string()),
            narrower_scope_def_path: Some(narrower_scope.to_string()),
            ..stored_finding(diagnostic_code, Path::new("/package/src/lib.rs"), "item", 1)
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

    fn report_for_test() -> StoredReport {
        StoredReport {
            version:                FINDINGS_SCHEMA_VERSION,
            analysis_fingerprint:   settings::current_analysis_fingerprint(),
            scope_fingerprint:      "scope".to_string(),
            package_root:           "/package".to_string(),
            crate_root_file:        "/package/src/lib.rs".to_string(),
            config_fingerprint:     CONFIG_FINGERPRINT.to_string(),
            source_files:           Vec::new(),
            findings:               Vec::new(),
            visibility_constraints: Vec::new(),
            pub_use_fix_facts:      Vec::new(),
            all_features_coverage:  AllFeaturesCoverage::default(),
            compiler_warning_facts: CompilerWarningFacts::None,
            use_sites:              Vec::new(),
        }
    }
}
