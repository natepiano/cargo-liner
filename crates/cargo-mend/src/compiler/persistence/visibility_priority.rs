use std::collections::BTreeSet;

use super::StoredFinding;
use super::StoredReport;
use crate::config::DiagnosticCode;

pub(super) fn apply_visibility_narrowing_priority(reports: &mut [StoredReport]) {
    let mut unused_pub_keys = BTreeSet::new();
    for report in reports.iter() {
        for finding in &report.findings {
            if finding.diagnostic_code == DiagnosticCode::UnusedPub {
                unused_pub_keys.insert(visibility_priority_key(finding));
            }
        }
    }

    if unused_pub_keys.is_empty() {
        return;
    }

    for report in reports.iter_mut() {
        report.findings.retain(|finding| {
            if !matches!(
                finding.diagnostic_code,
                DiagnosticCode::NarrowToPubCrate | DiagnosticCode::SuspiciousPub
            ) {
                return true;
            }
            !unused_pub_keys.contains(&visibility_priority_key(finding))
        });
    }
}

fn visibility_priority_key(finding: &StoredFinding) -> (String, usize, usize) {
    (finding.path.clone(), finding.line, finding.column)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::apply_visibility_narrowing_priority;
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
    fn visibility_priority_prefers_unused_pub_for_same_item() {
        let mut reports = vec![StoredReport {
            findings: vec![
                stored_finding(
                    DiagnosticCode::UnusedPub,
                    Path::new("/package/src/lib.rs"),
                    "item",
                    7,
                ),
                stored_finding(
                    DiagnosticCode::NarrowToPubCrate,
                    Path::new("/package/src/lib.rs"),
                    "item",
                    7,
                ),
            ],
            ..report_for_test()
        }];

        apply_visibility_narrowing_priority(&mut reports);

        assert_eq!(reports[0].findings.len(), 1);
        assert_eq!(
            reports[0].findings[0].diagnostic_code,
            DiagnosticCode::UnusedPub
        );
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
            source_files:           Vec::new(),
            findings:               Vec::new(),
            pub_use_fix_facts:      Vec::new(),
            compiler_warning_facts: CompilerWarningFacts::None,
            use_sites:              Vec::new(),
        }
    }
}
