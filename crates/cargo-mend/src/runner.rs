use anyhow::Result;

use crate::compiler;
use crate::config;
use crate::diagnostics;
use crate::imports;
use crate::outcome::AnalysisFailure;
use crate::outcome::CompilerFailureCause;
use crate::outcome::ExecutionNotice;
use crate::outcome::ExecutionOutcome;
use crate::outcome::FixNotice;
use crate::outcome::FixValidationFailure;
use crate::outcome::MendFailure;
use crate::outcome::NoticeKind;
use crate::outcome::PubUseNotice;
use crate::outcome::RollbackStatus;
use crate::pub_use_fixes;
use crate::run_mode::FixKind;
use crate::run_mode::OperationIntent;
use crate::run_mode::OperationMode;
use crate::selection;

pub struct MendRunner<'a> {
    selection: &'a selection::Selection,
    config:    &'a config::LoadedConfig,
}

enum BuildReportFailure {
    Analysis(AnalysisFailure),
    Unexpected(anyhow::Error),
}

struct RunPlan {
    mode:         OperationMode,
    report:       diagnostics::Report,
    import_scan:  Option<imports::ImportScan>,
    pub_use_scan: Option<pub_use_fixes::PubUseFixScan>,
}

impl<'a> MendRunner<'a> {
    pub const fn new(
        selection: &'a selection::Selection,
        config: &'a config::LoadedConfig,
    ) -> Self {
        Self { selection, config }
    }

    pub fn run(&self, mode: OperationMode) -> Result<ExecutionOutcome, MendFailure> {
        let planned = self.plan(mode)?;
        self.execute(planned)
    }

    fn plan(&self, mode: OperationMode) -> Result<RunPlan, MendFailure> {
        let output_mode = if mode.fixes.contains(FixKind::FixPubUse) {
            compiler::BuildOutputMode::SuppressUnusedImportWarnings
        } else {
            compiler::BuildOutputMode::Full
        };
        let report = self
            .build_report(output_mode)
            .map_err(Self::build_report_failure_into_mend_failure)?;
        let import_scan = mode
            .fixes
            .contains(FixKind::ShortenImport)
            .then(|| imports::scan_selection(self.selection))
            .transpose()
            .map_err(MendFailure::Unexpected)?;
        let pub_use_scan = mode
            .fixes
            .contains(FixKind::FixPubUse)
            .then(|| pub_use_fixes::scan_selection(self.selection, &report))
            .transpose()
            .map_err(MendFailure::Unexpected)?;

        Ok(RunPlan {
            mode,
            report,
            import_scan,
            pub_use_scan,
        })
    }

    fn execute(&self, planned: RunPlan) -> Result<ExecutionOutcome, MendFailure> {
        match planned.mode.intent {
            OperationIntent::ReadOnly => Ok(ExecutionOutcome {
                report: planned.report,
                notice: None,
            }),
            OperationIntent::DryRun => {
                let notice = Self::build_fix_notice(
                    planned.mode.intent,
                    Some(&planned.report),
                    planned.import_scan.as_ref(),
                    planned.pub_use_scan.as_ref(),
                );
                Ok(ExecutionOutcome {
                    report: planned.report,
                    notice,
                })
            },
            OperationIntent::Apply => self.apply(planned),
        }
    }

    fn apply(&self, planned: RunPlan) -> Result<ExecutionOutcome, MendFailure> {
        let fixes =
            Self::combined_fixes(planned.import_scan.as_ref(), planned.pub_use_scan.as_ref())?;
        if fixes.is_empty() {
            let notice = Self::build_fix_notice(
                planned.mode.intent,
                Some(&planned.report),
                planned.import_scan.as_ref(),
                planned.pub_use_scan.as_ref(),
            );
            return Ok(ExecutionOutcome {
                report: planned.report,
                notice,
            });
        }

        let snapshots = imports::snapshot_files(&fixes).map_err(MendFailure::Unexpected)?;
        let _applied = imports::apply_fixes(&fixes).map_err(MendFailure::Unexpected)?;
        match self.build_report(compiler::BuildOutputMode::Full) {
            Ok(report) => {
                let notice = Self::build_fix_notice(
                    planned.mode.intent,
                    Some(&report),
                    planned.import_scan.as_ref(),
                    planned.pub_use_scan.as_ref(),
                );
                Ok(ExecutionOutcome { report, notice })
            },
            Err(err) => {
                let rollback = match imports::restore_files(&snapshots) {
                    Ok(()) => RollbackStatus::Restored,
                    Err(_) => RollbackStatus::RestoreFailed,
                };
                Err(MendFailure::FixValidation(FixValidationFailure {
                    rollback,
                    cause: Self::build_report_failure_into_cause(err),
                }))
            },
        }
    }

    fn combined_fixes(
        import_scan: Option<&imports::ImportScan>,
        pub_use_scan: Option<&pub_use_fixes::PubUseFixScan>,
    ) -> Result<imports::ValidatedFixSet, MendFailure> {
        let mut fixes = Vec::new();
        if let Some(scan) = import_scan {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = pub_use_scan {
            fixes.extend(scan.fixes.iter().cloned());
        }
        imports::ValidatedFixSet::from_vec(fixes).map_err(MendFailure::Unexpected)
    }

    fn build_fix_notice(
        intent: OperationIntent,
        report: Option<&diagnostics::Report>,
        import_scan: Option<&imports::ImportScan>,
        pub_use_scan: Option<&pub_use_fixes::PubUseFixScan>,
    ) -> Option<ExecutionNotice> {
        let mut notices = Vec::new();
        if let Some(scan) = import_scan {
            notices.push(NoticeKind::ImportFixes(FixNotice::from_intent(
                intent,
                scan.fixes.len(),
            )));
        }

        if let Some(scan) = pub_use_scan {
            notices.push(NoticeKind::PubUseFixes(PubUseNotice::from_intent(
                intent,
                scan.applied_count,
                scan.skipped_count,
            )));
        }

        if matches!(intent, OperationIntent::Apply)
            && pub_use_scan.is_some_and(|scan| scan.applied_count > 0)
            && report
                .is_some_and(|report| report.facts.compiler_warnings.saw_unused_import_warnings())
        {
            notices.push(NoticeKind::ImportCleanupSuggested);
        }

        match notices.len() {
            0 => None,
            1 => notices.into_iter().next().map(ExecutionNotice::from_kind),
            _ => Some(ExecutionNotice::from_kinds(notices)),
        }
    }

    fn build_report(
        &self,
        output_mode: compiler::BuildOutputMode,
    ) -> Result<diagnostics::Report, BuildReportFailure> {
        let mut report = compiler::run_selection(self.selection, self.config, output_mode)
            .map_err(Self::mend_failure_into_build_report_failure)?;
        let import_scan =
            imports::scan_selection(self.selection).map_err(BuildReportFailure::Unexpected)?;
        report.findings.extend(import_scan.findings);
        report.findings.sort_by(|a, b| {
            (
                a.severity,
                &a.path,
                a.line,
                a.column,
                &a.code,
                &a.item,
                &a.message,
                &a.suggestion,
            )
                .cmp(&(
                    b.severity,
                    &b.path,
                    b.line,
                    b.column,
                    &b.code,
                    &b.item,
                    &b.message,
                    &b.suggestion,
                ))
        });
        report.findings.dedup_by(|a, b| {
            a.severity == b.severity
                && a.code == b.code
                && a.path == b.path
                && a.line == b.line
                && a.column == b.column
                && a.message == b.message
                && a.item == b.item
                && a.suggestion == b.suggestion
        });
        report.refresh_summary();
        Ok(report)
    }

    fn mend_failure_into_build_report_failure(error: MendFailure) -> BuildReportFailure {
        match error {
            MendFailure::Analysis(analysis) => BuildReportFailure::Analysis(analysis),
            MendFailure::Unexpected(error) => BuildReportFailure::Unexpected(error),
            MendFailure::FixValidation(_) => {
                unreachable!("build_report cannot produce fix-validation failures")
            },
        }
    }

    fn build_report_failure_into_mend_failure(error: BuildReportFailure) -> MendFailure {
        match error {
            BuildReportFailure::Analysis(analysis) => MendFailure::Analysis(analysis),
            BuildReportFailure::Unexpected(error) => MendFailure::Unexpected(error),
        }
    }

    fn build_report_failure_into_cause(error: BuildReportFailure) -> CompilerFailureCause {
        match error {
            BuildReportFailure::Analysis(analysis) => analysis.cause,
            BuildReportFailure::Unexpected(error) => CompilerFailureCause::Unexpected(error),
        }
    }
}
