use std::collections::BTreeMap;
use std::collections::BTreeSet;

use super::StoredFinding;
use super::StoredReport;
use crate::config::DiagnosticCode;

pub(super) fn apply_cross_compilation_intersection(reports: &mut [StoredReport]) {
    if reports.len() < 2 {
        return;
    }

    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, report) in reports.iter().enumerate() {
        groups
            .entry(report.crate_root_file.clone())
            .or_default()
            .push(idx);
    }

    for indices in groups.into_values() {
        if indices.len() < 2 {
            continue;
        }

        let mut emission_count: BTreeMap<(DiagnosticCode, String, usize, usize), usize> =
            BTreeMap::new();
        for &idx in &indices {
            let mut seen_in_this_report: BTreeSet<(DiagnosticCode, String, usize, usize)> =
                BTreeSet::new();
            for finding in &reports[idx].findings {
                if requires_cross_compilation_agreement(finding.diagnostic_code) {
                    let key = finding_intersection_key(finding);
                    if seen_in_this_report.insert(key.clone()) {
                        *emission_count.entry(key).or_default() += 1;
                    }
                }
            }
        }

        let group_size = indices.len();

        for &idx in &indices {
            reports[idx].findings.retain(|finding| {
                if !requires_cross_compilation_agreement(finding.diagnostic_code) {
                    return true;
                }
                let key = finding_intersection_key(finding);
                emission_count.get(&key).copied().unwrap_or(0) == group_size
            });
        }
    }
}

const fn requires_cross_compilation_agreement(code: DiagnosticCode) -> bool {
    matches!(
        code,
        DiagnosticCode::SuspiciousPub | DiagnosticCode::InternalParentPubUseFacade
    )
}

fn finding_intersection_key(finding: &StoredFinding) -> (DiagnosticCode, String, usize, usize) {
    (
        finding.diagnostic_code,
        finding.path.clone(),
        finding.line,
        finding.column,
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::apply_cross_compilation_intersection;
    use crate::compiler::constants::FINDINGS_SCHEMA_VERSION;
    use crate::compiler::persistence::StoredFinding;
    use crate::compiler::persistence::StoredReport;
    use crate::compiler::settings;
    use crate::config::DiagnosticCode;
    use crate::reporting::CompilerWarningFacts;
    use crate::reporting::FixSupport;
    use crate::reporting::Severity;

    const CONFIG_FINGERPRINT: &str = "config-fingerprint";

    #[test]
    fn cross_compilation_intersection_requires_all_sibling_reports() {
        let item_path = "crate::module::item";
        let narrower_scope = "crate::module";
        let mut first = StoredReport {
            findings: vec![narrowing_finding(
                DiagnosticCode::SuspiciousPub,
                item_path,
                narrower_scope,
            )],
            crate_root_file: "src/lib.rs".to_string(),
            ..report_for_test()
        };
        let second = StoredReport {
            findings: Vec::new(),
            crate_root_file: "src/lib.rs".to_string(),
            ..report_for_test()
        };
        let mut reports = vec![first, second];

        apply_cross_compilation_intersection(&mut reports);
        first = reports.remove(0);

        assert!(first.findings.is_empty());
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
            item_def_path: None,
            narrower_scope_def_path: None,
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
            findings:               Vec::new(),
            pub_use_fix_facts:      Vec::new(),
            compiler_warning_facts: CompilerWarningFacts::None,
            use_sites:              Vec::new(),
        }
    }
}
