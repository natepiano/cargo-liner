use std::collections::BTreeMap;
use std::collections::BTreeSet;

use super::StoredReport;

pub(super) fn apply_caller_aware_suppression(reports: &mut [StoredReport]) {
    let mut callers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for report in reports.iter() {
        for site in &report.use_sites {
            callers
                .entry(site.target_def_path.clone())
                .or_default()
                .insert(site.caller_module_def_path.clone());
        }
    }

    for report in reports.iter_mut() {
        report.findings.retain(|finding| {
            let Some(item_path) = finding.item_def_path.as_deref() else {
                return true;
            };
            let Some(narrower_scope) = finding.narrower_scope_def_path.as_deref() else {
                return true;
            };
            let Some(caller_set) = callers.get(item_path) else {
                return true;
            };
            caller_set
                .iter()
                .all(|caller| def_path_is_descendant(caller, narrower_scope))
        });
    }
}

fn def_path_is_descendant(caller_path: &str, narrower_scope: &str) -> bool {
    if caller_path == narrower_scope {
        return true;
    }
    if let Some(rest) = caller_path.strip_prefix(narrower_scope)
        && rest.starts_with("::")
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::apply_caller_aware_suppression;
    use crate::compiler::constants::FINDINGS_SCHEMA_VERSION;
    use crate::compiler::persistence::StoredFinding;
    use crate::compiler::persistence::StoredReport;
    use crate::compiler::persistence::UseSite;
    use crate::compiler::settings;
    use crate::config::DiagnosticCode;
    use crate::reporting::CompilerWarningFacts;
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
            }],
            ..report_for_test()
        }];

        apply_caller_aware_suppression(&mut reports);

        assert!(reports[0].findings.is_empty());
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
            source_files:           Vec::new(),
            findings:               Vec::new(),
            pub_use_fix_facts:      Vec::new(),
            compiler_warning_facts: CompilerWarningFacts::None,
            use_sites:              Vec::new(),
        }
    }
}
