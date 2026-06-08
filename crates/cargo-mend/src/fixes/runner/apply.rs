use super::MendRunner;
use super::RunPlan;
use crate::compiler::BuildOutputMode;
use crate::fixes::imports;
use crate::reporting::CompilerFailureCause;
use crate::reporting::ExecutionOutcome;
use crate::reporting::FixValidationFailure;
use crate::reporting::MendFailure;
use crate::reporting::OutputFormat;
use crate::reporting::RollbackStatus;

impl MendRunner<'_> {
    pub(super) fn apply(&self, planned: RunPlan) -> Result<ExecutionOutcome, MendFailure> {
        let plan_check_duration = planned.check_duration;
        let compiler_warnings = planned.compiler_warnings;
        let compiler_fixable = planned.compiler_fixable;
        let fix_scans = planned.fix_scans();
        let applied_pub_use = fix_scans.pub_use.map_or(0, |scan| scan.applied);
        let fixes = Self::combined_fixes(fix_scans)?;
        if fixes.is_empty() {
            let notice = Self::build_fix_notice(
                planned.operation_mode.intent,
                Some(&planned.report),
                fix_scans,
            );
            let warning_facts = planned.report.facts.compiler_warning_facts;
            return Ok(ExecutionOutcome {
                report: planned.report,
                notice,
                check_duration: plan_check_duration,
                compiler_warnings,
                compiler_fixable,
                applied_pub_use: 0,
                compiler_warning_facts: warning_facts,
            });
        }

        let snapshots = imports::snapshot_files(&fixes).map_err(MendFailure::Unexpected)?;
        imports::apply_fixes(&fixes).map_err(MendFailure::Unexpected)?;
        let validation_output_mode = if self.output_format == OutputFormat::Json {
            BuildOutputMode::Json
        } else {
            BuildOutputMode::Quiet
        };
        match self.build_selection(validation_output_mode) {
            Ok(validation) => {
                let check_duration = plan_check_duration + validation.check_duration;
                let notice = Self::build_fix_notice(
                    planned.operation_mode.intent,
                    Some(&validation.report),
                    fix_scans,
                );
                let warning_facts = validation.report.facts.compiler_warning_facts;
                Ok(ExecutionOutcome {
                    report: validation.report,
                    notice,
                    check_duration,
                    compiler_warnings,
                    compiler_fixable,
                    applied_pub_use,
                    compiler_warning_facts: warning_facts,
                })
            },
            Err(err) => {
                let rollback_status = match imports::restore_files(&snapshots) {
                    Ok(()) => RollbackStatus::Restored,
                    Err(_) => RollbackStatus::RestoreFailed,
                };
                let cause = match err {
                    MendFailure::Analysis(a) => a.cause,
                    MendFailure::Unexpected(e) => CompilerFailureCause::Unexpected(e),
                    MendFailure::FixValidation(f) => f.cause,
                };
                Err(MendFailure::FixValidation(FixValidationFailure {
                    rollback_status,
                    cause,
                }))
            },
        }
    }
}
