use super::MendRunner;
use super::RunPlan;
use super::mend_runner::NextAnalysis;
use crate::compiler::BuildOutputMode;
use crate::fixes::imports;
use crate::reporting::AppliedFixCounts;
use crate::reporting::AppliedFixes;
use crate::reporting::CompilerFailureCause;
use crate::reporting::ExecutionOutcome;
use crate::reporting::FixValidationFailure;
use crate::reporting::MendFailure;
use crate::reporting::OutputFormat;
use crate::reporting::PassEdits;
use crate::reporting::RollbackStatus;
use crate::reporting::SubtreeReexportEdits;

impl MendRunner<'_> {
    pub(super) fn apply(&mut self, planned: RunPlan) -> Result<ExecutionOutcome, MendFailure> {
        let plan_check_duration = planned.check_duration;
        let compiler_warnings = planned.compiler_warnings;
        let compiler_fixable = planned.compiler_fixable;
        let fix_scans = planned.fix_scans();
        let applied_pub_use = fix_scans
            .pub_use
            .map_or(0, |pub_use_fix_scan| pub_use_fix_scan.applied);
        let subtree_reexport_edits = fix_scans.subtree_reexports.map_or(
            SubtreeReexportEdits::Unchanged,
            |subtree_reexport_scan| {
                if subtree_reexport_scan.fixes.is_empty() {
                    SubtreeReexportEdits::Unchanged
                } else {
                    SubtreeReexportEdits::Written
                }
            },
        );
        let fixes = Self::combined_fixes(fix_scans)?;
        if fixes.is_empty() {
            let notice = Self::build_fix_notice(
                planned.operation_mode.intent,
                Some(&planned.report),
                fix_scans,
                AppliedFixCounts::default(),
            );
            let warning_facts = planned.report.facts.compiler_warning_facts;
            return Ok(ExecutionOutcome {
                report: planned.report,
                notice,
                check_duration: plan_check_duration,
                compiler_warnings,
                compiler_fixable,
                applied_pub_use: 0,
                subtree_reexport_edits: SubtreeReexportEdits::Unchanged,
                pass_edits: PassEdits::Unchanged,
                compiler_warning_facts: warning_facts,
            });
        }

        // `self.session_snapshot.record` runs before the write, into a snapshot the runner keeps
        // for the whole invocation: an earlier pass's edits are already on disk and
        // a rollback owes the user the tree they started with.
        self.session_snapshot
            .record(&fixes)
            .map_err(MendFailure::Unexpected)?;
        let applied_fix_counts = imports::apply_fixes(&fixes).map_err(MendFailure::Unexpected)?;
        let validation_output_mode = if self.output_format == OutputFormat::Json {
            BuildOutputMode::Json
        } else {
            BuildOutputMode::Validation
        };
        match self.build_selection(validation_output_mode) {
            Ok(checked_selection) => {
                let validation = &checked_selection.result;
                let check_duration = plan_check_duration + validation.check_duration;
                let notice = Self::build_fix_notice(
                    planned.operation_mode.intent,
                    Some(&validation.report),
                    fix_scans,
                    applied_fix_counts,
                );
                let warning_facts = validation.report.facts.compiler_warning_facts;
                let report = validation.report.clone();
                self.next_analysis = NextAnalysis::Validated(Box::new(checked_selection));
                Ok(ExecutionOutcome {
                    report,
                    notice,
                    check_duration,
                    compiler_warnings,
                    compiler_fixable,
                    applied_pub_use,
                    subtree_reexport_edits,
                    pass_edits: PassEdits::Written,
                    compiler_warning_facts: warning_facts,
                })
            },
            Err(mend_failure) => {
                let rollback_status = self
                    .session_snapshot
                    .restore()
                    .map_or(RollbackStatus::RestoreFailed, |()| RollbackStatus::Restored);
                let compiler_failure_cause = match mend_failure {
                    MendFailure::Analysis(analysis_failure) => analysis_failure.cause,
                    MendFailure::Unexpected(error) => CompilerFailureCause::Unexpected(error),
                    MendFailure::FixValidation(fix_validation_failure) => {
                        fix_validation_failure.cause
                    },
                };
                Err(MendFailure::FixValidation(FixValidationFailure {
                    rollback_status,
                    cause: compiler_failure_cause,
                    applied_fixes: AppliedFixes::Mend,
                }))
            },
        }
    }
}
