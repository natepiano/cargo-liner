use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use super::StoredReport;
use super::schema::DeclarationIdentity;
use super::schema::UseSiteReference;
use crate::compiler::visibility;
use crate::config::DiagnosticCode;
use crate::reporting::FixSupport;
use crate::rust_syntax::ParsedSources;

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
    package_root: String,
    declaration:  DeclarationIdentity,
}

impl CallerKey {
    fn for_package(package_root: &str, declaration: &DeclarationIdentity) -> Self {
        Self {
            package_root: package_root.to_string(),
            declaration:  declaration.clone(),
        }
    }
}

pub(super) fn apply_caller_aware_suppression(reports: &mut [StoredReport]) -> CallerMap {
    let caller_map = collect_callers(reports);
    for stored_report in reports {
        suppress_invalid_narrowings(stored_report, &caller_map);
    }
    caller_map
}

fn collect_callers(reports: &[StoredReport]) -> CallerMap {
    let mut caller_map = CallerMap::new();
    for stored_report in reports {
        for use_site in &stored_report.use_sites {
            let item_callers = caller_map
                .entry(CallerKey::for_package(
                    &stored_report.package_root,
                    &use_site.declaration,
                ))
                .or_default();
            item_callers
                .reaching
                .insert(use_site.caller_module_def_path.clone());
            match use_site.reference {
                UseSiteReference::Named => {
                    item_callers
                        .naming
                        .insert(use_site.caller_module_def_path.clone());
                    item_callers
                        .semantic_reaching
                        .insert(use_site.caller_module_def_path.clone());
                },
                UseSiteReference::ThroughSignature => {
                    item_callers
                        .semantic_reaching
                        .insert(use_site.caller_module_def_path.clone());
                },
                UseSiteReference::PrivateImport => {
                    item_callers
                        .naming
                        .insert(use_site.caller_module_def_path.clone());
                    item_callers
                        .private_imports
                        .insert(use_site.caller_module_def_path.clone());
                },
                UseSiteReference::RestrictedImport => {
                    item_callers
                        .naming
                        .insert(use_site.caller_module_def_path.clone());
                    item_callers
                        .restricted_imports
                        .insert(use_site.caller_module_def_path.clone());
                },
                // `reaching` only: a declaration's own signature caps how far
                // the annotation may narrow without proving anyone uses the
                // item, so it must not suppress the narrowing finding.
                UseSiteReference::DeclarationInterface => {},
            }
        }
    }
    caller_map
}

fn suppress_invalid_narrowings(report: &mut StoredReport, caller_map: &CallerMap) {
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
    let parsed = ParsedSources::default();
    let declaration_sources: BTreeMap<_, _> = report
        .use_sites
        .iter()
        .map(|site| (site.declaration.def_path.as_str(), &site.declaration))
        .collect();
    let mut facade_sources: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for fact in &report.pub_use_fix_facts {
        facade_sources
            .entry((
                fact.parent_path.as_str(),
                fact.parent_line,
                fact.child_item_name.as_str(),
            ))
            .or_default()
            .insert(fact.child_path.as_str());
    }
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
            let evidence =
                finding
                    .item_def_path
                    .as_ref()
                    .map_or(CallerEvidence::NoneRecorded, |item_path| {
                        if let Some(declaration) = declaration_sources.get(item_path.as_str()) {
                            return callers_for_package(caller_map, &package_root, declaration);
                        }
                        if finding.diagnostic_code == DiagnosticCode::InternalParentPubUseFacade {
                            let sources = finding.item.as_deref().and_then(|item| {
                                facade_sources.get(&(
                                    finding.path.as_str(),
                                    finding.line,
                                    super::StoredFinding::item_name(item),
                                ))
                            });
                            // Separate namespaces can export the same name from different files.
                            // Match each file with the definition path before selecting its
                            // callers.
                            return sources
                                .into_iter()
                                .flatten()
                                .find_map(|source| {
                                    let declaration = DeclarationIdentity::new(
                                        &parsed.path_key(Path::new(source)),
                                        item_path.clone(),
                                    );
                                    match callers_for_package(
                                        caller_map,
                                        &package_root,
                                        &declaration,
                                    ) {
                                        CallerEvidence::Recorded(callers) => {
                                            Some(CallerEvidence::Recorded(callers))
                                        },
                                        CallerEvidence::NoneRecorded => None,
                                    }
                                })
                                .unwrap_or(CallerEvidence::NoneRecorded);
                        }
                        let declaration = DeclarationIdentity::new(
                            &parsed.path_key(Path::new(&finding.path)),
                            item_path.clone(),
                        );
                        callers_for_package(caller_map, &package_root, &declaration)
                    });
            retain_narrowing_finding(finding, evidence)
        }
    });
}

fn retain_narrowing_finding(
    finding: &mut super::StoredFinding,
    evidence: CallerEvidence<'_>,
) -> bool {
    let Some(narrower_scope) = finding.narrower_scope_def_path.as_deref() else {
        return true;
    };
    let CallerEvidence::Recorded(item_callers) = evidence else {
        return true;
    };
    if finding.diagnostic_code == DiagnosticCode::InternalParentPubUseFacade {
        return facade_serves_only_its_subtree(item_callers, narrower_scope);
    }
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

/// Whether every module holding an expression, type, or signature reference to
/// the item sits inside the facade's own subtree. A reference from outside can
/// only be reaching the item through the parent `pub use`, which makes that
/// re-export a real entry point rather than an internal convenience — removing
/// it would stop the crate compiling.
///
/// Only `semantic_reaching` answers this. `reaching` also carries
/// `DeclarationInterface` sites, which never prove anyone uses the item, and
/// `naming` carries the facade's own `pub(crate) use` under the boundary it
/// writes — the crate root — so every `pub(crate)` facade would look like it
/// had an outside caller.
fn facade_serves_only_its_subtree(item_callers: &ItemCallers, subtree: &str) -> bool {
    item_callers
        .semantic_reaching
        .iter()
        .all(|caller| visibility::def_path_is_descendant(caller, subtree))
}

/// An absent record does not prove that an item has no callers.
#[derive(Clone, Copy)]
pub(super) enum CallerEvidence<'a> {
    Recorded(&'a ItemCallers),
    NoneRecorded,
}

pub(super) fn callers_for_package<'a>(
    caller_map: &'a CallerMap,
    package_root: &str,
    declaration: &DeclarationIdentity,
) -> CallerEvidence<'a> {
    caller_map
        .get(&CallerKey::for_package(package_root, declaration))
        .map_or(CallerEvidence::NoneRecorded, CallerEvidence::Recorded)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::apply_caller_aware_suppression;
    use crate::compiler::constants::FINDINGS_SCHEMA_VERSION;
    use crate::compiler::persistence::StoredFinding;
    use crate::compiler::persistence::StoredReport;
    use crate::compiler::persistence::schema::DeclarationIdentity;
    use crate::compiler::persistence::schema::UseSite;
    use crate::compiler::persistence::schema::UseSiteReference;
    use crate::compiler::settings;
    use crate::config::DiagnosticCode;
    use crate::reporting::AllFeaturesCoverage;
    use crate::reporting::CompilerWarningFacts;
    use crate::reporting::ExactBoundarySpelling;
    use crate::reporting::FixSupport;
    use crate::reporting::Severity;
    use crate::rust_syntax::ParsedSources;

    const CONFIG_FINGERPRINT: &str = "config-fingerprint";
    const CRATE_ROOT_FILE: &str = "/package/src/lib.rs";
    const FINDING_LINE: usize = 1;
    const ITEM_DEF_PATH: &str = "crate::module::item";
    const ITEM_NAME: &str = "item";
    const ITEM_SOURCE_LINE: &str = "pub fn item() {}";
    const NARROWER_SCOPE_DEF_PATH: &str = "crate::module";
    const NARROWER_VISIBILITY_SUGGESTION: &str = "use narrower visibility";
    const OUTSIDE_CALLER_MODULE_DEF_PATH: &str = "crate::other";
    const PACKAGE_ROOT: &str = "/package";
    const SCOPE_FINGERPRINT: &str = "scope";

    #[test]
    fn caller_aware_suppression_drops_narrowing_outside_scope() {
        let item_path = ITEM_DEF_PATH;
        let narrower_scope = NARROWER_SCOPE_DEF_PATH;
        let mut reports = vec![StoredReport {
            findings: vec![narrowing_finding(
                DiagnosticCode::SuspiciousPub,
                item_path,
                narrower_scope,
            )],
            use_sites: vec![UseSite {
                declaration:            DeclarationIdentity::new(
                    &ParsedSources::default().path_key(Path::new(CRATE_ROOT_FILE)),
                    item_path.to_string(),
                ),
                caller_module_def_path: OUTSIDE_CALLER_MODULE_DEF_PATH.to_string(),
                reference:              UseSiteReference::Named,
            }],
            ..report_for_test()
        }];

        apply_caller_aware_suppression(&mut reports);

        assert_eq!(reports[0].findings, [] as [StoredFinding; 0]);
    }

    #[test]
    fn private_import_keeps_the_warning_but_blocks_its_rewrite() {
        let item_path = ITEM_DEF_PATH;
        let mut finding = narrowing_finding(
            DiagnosticCode::UnusedPub,
            item_path,
            NARROWER_SCOPE_DEF_PATH,
        );
        finding.fix_support = FixSupport::UnusedPub;
        let mut reports = vec![StoredReport {
            findings: vec![finding],
            use_sites: vec![UseSite {
                declaration:            DeclarationIdentity::new(
                    &ParsedSources::default().path_key(Path::new(CRATE_ROOT_FILE)),
                    item_path.to_string(),
                ),
                caller_module_def_path: OUTSIDE_CALLER_MODULE_DEF_PATH.to_string(),
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
        let item_path = ITEM_DEF_PATH;
        let direct = narrowing_finding(
            DiagnosticCode::UnusedPub,
            item_path,
            NARROWER_SCOPE_DEF_PATH,
        );
        let mut cleanup = narrowing_finding(
            DiagnosticCode::SuspiciousPub,
            item_path,
            NARROWER_SCOPE_DEF_PATH,
        );
        cleanup.fix_support = FixSupport::PubUse;
        let mut reports = vec![StoredReport {
            findings: vec![direct, cleanup],
            use_sites: vec![UseSite {
                declaration:            DeclarationIdentity::new(
                    &ParsedSources::default().path_key(Path::new(CRATE_ROOT_FILE)),
                    item_path.to_string(),
                ),
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
            ..stored_finding(
                diagnostic_code,
                Path::new(CRATE_ROOT_FILE),
                ITEM_NAME,
                FINDING_LINE,
            )
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
            source_line: ITEM_SOURCE_LINE.to_string(),
            item: Some(item.to_string()),
            message: format!("{item} should change visibility"),
            suggestion: Some(NARROWER_VISIBILITY_SUGGESTION.to_string()),
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
            version:                    FINDINGS_SCHEMA_VERSION,
            analysis_fingerprint:       settings::current_analysis_fingerprint(),
            scope_fingerprint:          SCOPE_FINGERPRINT.to_string(),
            package_root:               PACKAGE_ROOT.to_string(),
            crate_root_file:            CRATE_ROOT_FILE.to_string(),
            config_fingerprint:         CONFIG_FINGERPRINT.to_string(),
            source_files:               Vec::new(),
            findings:                   Vec::new(),
            visibility_constraints:     Vec::new(),
            pub_use_fix_facts:          Vec::new(),
            subtree_reexport_fix_facts: Vec::new(),
            module_mount_facts:         Vec::new(),
            all_features_coverage:      AllFeaturesCoverage::default(),
            compiler_warning_facts:     CompilerWarningFacts::None,
            use_sites:                  Vec::new(),
        }
    }
}
