use super::MendRunner;
use super::RunPlan;
use crate::compiler;
use crate::compiler::BuildOutputMode;
use crate::compiler::SelectionResult;
use crate::config::DiagnosticCode;
use crate::config::DiagnosticStatus;
use crate::config::OperationIntent;
use crate::fixes::imports;
use crate::fixes::imports::ImportScan;
use crate::fixes::imports_at_top;
use crate::fixes::imports_at_top::ImportsAtTopScan;
use crate::fixes::inline_path_qualified_type;
use crate::fixes::inline_path_qualified_type::InlinePathScan;
use crate::fixes::prefer_module_import;
use crate::fixes::prefer_module_import::PreferModuleImportScan;
use crate::reporting::ExecutionOutcome;
use crate::reporting::Finding;
use crate::reporting::MendFailure;
use crate::reporting::PassEdits;
use crate::reporting::SubtreeReexportEdits;
use crate::rust_syntax::ParsedSources;

/// A checked selection, with the syntax scans that added findings to its
/// report and the sources they read, kept so the fix scans that follow reuse
/// both instead of reading the tree again.
pub(super) struct CheckedSelection {
    pub(super) result:  SelectionResult,
    pub(super) scans:   SyntaxScans,
    pub(super) sources: ParsedSources,
}

/// The syntax scans whose diagnostics are enabled; a fix kind is enabled only
/// with one of its diagnostics, so each fix scan finds its scan here.
pub(super) struct SyntaxScans {
    pub(super) prefer_module_import: Option<PreferModuleImportScan>,
    pub(super) inline_path:          Option<InlinePathScan>,
    pub(super) imports:              Option<ImportScan>,
    pub(super) imports_at_top:       Option<ImportsAtTopScan>,
}

impl MendRunner<'_> {
    pub(super) fn execute(&mut self, planned: RunPlan) -> Result<ExecutionOutcome, MendFailure> {
        let check_duration = planned.check_duration;
        let compiler_warnings = planned.compiler_warnings;
        let compiler_fixable = planned.compiler_fixable;
        match planned.operation_mode.intent {
            OperationIntent::ReadOnly => Ok(ExecutionOutcome {
                compiler_warning_facts: planned.report.facts.compiler_warning_facts,
                report: planned.report,
                notice: None,
                check_duration,
                compiler_warnings,
                compiler_fixable,
                applied_pub_use: 0,
                subtree_reexport_edits: SubtreeReexportEdits::Unchanged,
                pass_edits: PassEdits::Unchanged,
            }),
            OperationIntent::DryRun => {
                // Count `previewed` from `Self::combined_fixes`, not the scans: a dry run must
                // preview the edits an apply would write, and combining is
                // where conflicting groups are dropped and duplicates collapse.
                let fix_scans = planned.fix_scans();
                let previewed = Self::combined_fixes(fix_scans)?;
                let notice = Self::build_fix_notice(
                    planned.operation_mode.intent,
                    Some(&planned.report),
                    fix_scans,
                    previewed.counts(),
                );
                Ok(ExecutionOutcome {
                    compiler_warning_facts: planned.report.facts.compiler_warning_facts,
                    report: planned.report,
                    notice,
                    check_duration,
                    compiler_warnings,
                    compiler_fixable,
                    applied_pub_use: 0,
                    subtree_reexport_edits: SubtreeReexportEdits::Unchanged,
                    pass_edits: PassEdits::Unchanged,
                })
            },
            OperationIntent::Apply => self.apply(planned),
        }
    }

    pub(super) fn build_selection(
        &self,
        output_mode: BuildOutputMode,
    ) -> Result<CheckedSelection, MendFailure> {
        let mut result = compiler::run_selection(
            self.selection,
            self.cargo_plan,
            self.loaded_config,
            output_mode,
            self.color_mode,
            self.toolchain_probes,
        )?;
        // Create `ParsedSources` after `compiler::run_selection` because `cargo fix`
        // may rewrite files during that check.
        let sources = ParsedSources::for_selection(self.selection);
        let diagnostics_config = &self.loaded_config.diagnostics_config;
        let enabled = |codes: &[DiagnosticCode]| {
            codes
                .iter()
                .any(|&code| diagnostics_config.is_enabled(code) == DiagnosticStatus::Enabled)
        };
        let scans = SyntaxScans {
            imports:              enabled(&[
                DiagnosticCode::ShortenLocalCrateImport,
                DiagnosticCode::ReplaceDeepSuperImport,
            ])
            .then(|| imports::scan_selection(self.selection, &sources))
            .transpose()
            .map_err(MendFailure::Unexpected)?,
            prefer_module_import: enabled(&[DiagnosticCode::PreferModuleImport])
                .then(|| prefer_module_import::scan_selection(self.selection, &sources))
                .transpose()
                .map_err(MendFailure::Unexpected)?,
            inline_path:          enabled(&[DiagnosticCode::InlinePathQualifiedType])
                .then(|| inline_path_qualified_type::scan_selection(self.selection, &sources))
                .transpose()
                .map_err(MendFailure::Unexpected)?,
            imports_at_top:       enabled(&[DiagnosticCode::ImportsAtTop])
                .then(|| imports_at_top::scan_selection(self.selection, &sources))
                .transpose()
                .map_err(MendFailure::Unexpected)?,
        };
        let report = &mut result.report;
        let scan_findings = [
            scans
                .prefer_module_import
                .as_ref()
                .map(|scan| &scan.findings),
            scans.inline_path.as_ref().map(|scan| &scan.findings),
            scans.imports.as_ref().map(|scan| &scan.findings),
            scans.imports_at_top.as_ref().map(|scan| &scan.findings),
        ];
        report
            .findings
            .extend(scan_findings.into_iter().flatten().flatten().cloned());
        sort_and_dedup_findings(&mut report.findings);
        // `report.findings.retain` removes findings whose `diagnostic_code` is disabled in
        // `self.loaded_config.diagnostics_config`.
        report.findings.retain(|f| {
            self.loaded_config
                .diagnostics_config
                .is_enabled(f.diagnostic_code)
                == DiagnosticStatus::Enabled
        });
        report.refresh_summary();
        Ok(CheckedSelection {
            result,
            scans,
            sources,
        })
    }
}

/// Preserve compiler input order and the runner's suggestion-sensitive duplicate policy.
fn sort_and_dedup_findings(findings: &mut Vec<Finding>) {
    findings.sort_by(|a, b| {
        (
            a.input_order(),
            &a.path,
            a.line,
            a.column,
            &a.diagnostic_code,
            &a.item,
            &a.message,
            &a.suggestion,
        )
            .cmp(&(
                b.input_order(),
                &b.path,
                b.line,
                b.column,
                &b.diagnostic_code,
                &b.item,
                &b.message,
                &b.suggestion,
            ))
    });
    findings.dedup_by(|a, b| {
        a.input_order() == b.input_order()
            && a.diagnostic_code == b.diagnostic_code
            && a.path == b.path
            && a.line == b.line
            && a.column == b.column
            && a.message == b.message
            && a.item == b.item
            && a.suggestion == b.suggestion
    });
}

#[cfg(test)]
mod tests {

    use crate::reporting::Finding;
    use crate::reporting::FixSupport;
    use crate::reporting::Severity;
    #[test]
    fn input_order_and_dedup_preserve_origins_and_shared_annotation_sites() -> anyhow::Result<()> {
        let error = ordering_finding(Severity::Error, FixSupport::PubUse, "origin")?;
        let warning = ordering_finding(Severity::Warning, FixSupport::PubUse, "origin")?;
        let manual = ordering_finding(Severity::Error, FixSupport::None, "manual")?;
        let first = ordering_finding(Severity::Error, FixSupport::RestrictedAnnotation, "site-a")?;
        let second = ordering_finding(
            Severity::Warning,
            FixSupport::RestrictedAnnotation,
            "site-b",
        )?;
        let mut findings = vec![
            warning.clone(),
            second,
            error.clone(),
            first,
            manual,
            error,
            warning,
        ];
        super::sort_and_dedup_findings(&mut findings);
        assert_eq!(
            findings
                .iter()
                .map(|finding| finding.message.as_str())
                .collect::<Vec<_>>(),
            ["manual", "origin", "site-a", "origin", "site-b"]
        );
        let wires = findings
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(
            wires
                .iter()
                .map(|wire| wire["severity"].as_str())
                .collect::<Vec<_>>(),
            [
                Some("error"),
                Some("error"),
                Some("error"),
                Some("warning"),
                Some("warning")
            ]
        );
        // Runner dedup preserves both distinct compiler diagnostics at the site.
        assert_eq!(findings[2].fix_support(), FixSupport::RestrictedAnnotation);
        assert_eq!(findings[4].fix_support(), FixSupport::RestrictedAnnotation);
        let mut distinct = findings[1].clone();
        distinct.suggestion = Some("different suggestion".to_string());
        findings.push(distinct);
        super::sort_and_dedup_findings(&mut findings);
        assert_eq!(findings.len(), 6);
        Ok(())
    }

    fn ordering_finding(
        severity: Severity,
        support: FixSupport,
        message: &str,
    ) -> anyhow::Result<Finding> {
        Ok(serde_json::from_value(serde_json::json!({
            "severity": severity, "fixability": support,
            "diagnostic_code": "suspicious_pub", "path": "src/lib.rs",
            "line": 1, "column": 1, "highlight_len": 3,
            "source_line": "pub fn item() {}", "item": "item", "message": message,
        }))?)
    }
}
