use std::fmt;
use std::process::ExitCode;

use anyhow::Error;

use crate::diagnostics::Report;
use crate::run_mode::OperationIntent;

#[derive(Debug)]
pub struct ExecutionOutcome {
    pub report: Report,
    pub notice: Option<ExecutionNotice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionNotice {
    ImportFixes(FixNotice),
    PubUseFixes(PubUseNotice),
    ImportCleanupSuggested,
    Combined(Vec<Self>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixNotice {
    NoneAvailable,
    PreviewApplied(usize),
    Applied(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PubUseNotice {
    NoneAvailable {
        skipped_unsupported: usize,
    },
    PreviewApplied {
        applied:             usize,
        skipped_unsupported: usize,
    },
    Applied {
        applied:             usize,
        skipped_unsupported: usize,
    },
}

#[derive(Debug)]
pub enum MendFailure {
    Analysis(AnalysisFailure),
    FixValidation(FixValidationFailure),
    Unexpected(Error),
}

#[derive(Debug)]
pub enum AnalysisFailure {
    CargoCheck,
    CargoRustcRefresh { package: String },
    DriverSetup(Error),
    DriverExecution(Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RollbackStatus {
    Restored,
    RestoreFailed,
}

#[derive(Debug)]
pub struct FixValidationFailure {
    pub rollback: RollbackStatus,
    pub source:   FixValidationSource,
}

#[derive(Debug)]
pub enum FixValidationSource {
    CargoCheck,
    CargoRustcRefresh { package: String },
    Unexpected(Error),
}

impl MendFailure {
    pub fn exit_code() -> ExitCode { ExitCode::from(2) }
}

impl fmt::Display for MendFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Analysis(failure) => write!(f, "{failure}"),
            Self::FixValidation(failure) => write!(f, "{failure}"),
            Self::Unexpected(error) => write!(f, "{error:#}"),
        }
    }
}

impl fmt::Display for AnalysisFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CargoCheck => write!(f, "compiler failed while validating this crate"),
            Self::CargoRustcRefresh { package } => {
                write!(
                    f,
                    "compiler refresh failed while validating package `{package}`"
                )
            },
            Self::DriverSetup(error) | Self::DriverExecution(error) => write!(f, "{error:#}"),
        }
    }
}

impl fmt::Display for FixValidationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let source = match &self.source {
            FixValidationSource::CargoCheck => {
                "compiler failed after applying mend fixes".to_string()
            },
            FixValidationSource::CargoRustcRefresh { package } => {
                format!("compiler refresh failed after applying mend fixes for package `{package}`")
            },
            FixValidationSource::Unexpected(error) => format!("{error:#}"),
        };
        match self.rollback {
            RollbackStatus::Restored => write!(
                f,
                "compiler failed after applying mend fixes; changes were rolled back\n\n{source:#}"
            ),
            RollbackStatus::RestoreFailed => write!(
                f,
                "compiler failed after applying mend fixes, and rollback also failed\n\n{source:#}"
            ),
        }
    }
}

impl From<Error> for MendFailure {
    fn from(value: Error) -> Self { Self::Unexpected(value) }
}

impl ExecutionNotice {
    pub fn render(&self) -> String {
        match self {
            Self::ImportFixes(notice) => format!("mend: {}", notice.render()),
            Self::PubUseFixes(notice) => format!("mend: {}", notice.render()),
            Self::ImportCleanupSuggested => format!("mend: {}", self.render_part()),
            Self::Combined(notices) => {
                let parts = notices.iter().map(Self::render_part).collect::<Vec<_>>();
                format!("mend: {}", parts.join("; "))
            },
        }
    }

    fn render_part(&self) -> String {
        match self {
            Self::ImportFixes(notice) => notice.render(),
            Self::PubUseFixes(notice) => notice.render(),
            Self::ImportCleanupSuggested => {
                "some imports may now be unused; consider running cargo fix or cleaning them up manually"
                    .to_string()
            },
            Self::Combined(notices) => notices
                .iter()
                .map(Self::render_part)
                .collect::<Vec<_>>()
                .join("; "),
        }
    }
}

impl FixNotice {
    fn render(&self) -> String {
        match self {
            Self::NoneAvailable => "no import fixes available".to_string(),
            Self::PreviewApplied(count) => format!("would apply {count} import fix(es) in dry run"),
            Self::Applied(count) => format!("applied {count} import fix(es)"),
        }
    }

    pub const fn from_intent(intent: OperationIntent, count: usize) -> Self {
        match intent {
            OperationIntent::ReadOnly => Self::NoneAvailable,
            OperationIntent::DryRun => {
                if count == 0 {
                    Self::NoneAvailable
                } else {
                    Self::PreviewApplied(count)
                }
            },
            OperationIntent::Apply => {
                if count == 0 {
                    Self::NoneAvailable
                } else {
                    Self::Applied(count)
                }
            },
        }
    }
}

impl PubUseNotice {
    fn render(&self) -> String {
        match self {
            Self::NoneAvailable {
                skipped_unsupported: 0,
            } => "no `pub use` fixes available".to_string(),
            Self::NoneAvailable {
                skipped_unsupported,
            } => format!(
                "no `pub use` fixes available; skipped {skipped_unsupported} unsupported `pub use` candidate(s)"
            ),
            Self::PreviewApplied {
                applied,
                skipped_unsupported: 0,
            } => format!("would apply {applied} `pub use` fix(es) in dry run"),
            Self::PreviewApplied {
                applied,
                skipped_unsupported,
            } => format!(
                "would apply {applied} `pub use` fix(es) in dry run; skipped {skipped_unsupported} unsupported `pub use` candidate(s)"
            ),
            Self::Applied {
                applied,
                skipped_unsupported: 0,
            } => format!("applied {applied} `pub use` fix(es)"),
            Self::Applied {
                applied,
                skipped_unsupported,
            } => format!(
                "applied {applied} `pub use` fix(es); skipped {skipped_unsupported} unsupported `pub use` candidate(s)"
            ),
        }
    }

    pub const fn from_intent(
        intent: OperationIntent,
        applied: usize,
        skipped_unsupported: usize,
    ) -> Self {
        match intent {
            OperationIntent::ReadOnly => Self::NoneAvailable {
                skipped_unsupported,
            },
            OperationIntent::DryRun => {
                if applied == 0 {
                    Self::NoneAvailable {
                        skipped_unsupported,
                    }
                } else {
                    Self::PreviewApplied {
                        applied,
                        skipped_unsupported,
                    }
                }
            },
            OperationIntent::Apply => {
                if applied == 0 {
                    Self::NoneAvailable {
                        skipped_unsupported,
                    }
                } else {
                    Self::Applied {
                        applied,
                        skipped_unsupported,
                    }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::AnalysisFailure;
    use super::ExecutionNotice;
    use super::FixNotice;
    use super::FixValidationFailure;
    use super::FixValidationSource;
    use super::PubUseNotice;
    use super::RollbackStatus;
    use crate::run_mode::OperationIntent;

    #[test]
    fn analysis_failure_message_uses_typed_collection_wording() {
        let failure = AnalysisFailure::CargoCheck;
        assert_eq!(
            failure.to_string(),
            "compiler failed while validating this crate"
        );
    }

    #[test]
    fn fix_validation_failure_reports_rollback_success() {
        let failure = FixValidationFailure {
            rollback: RollbackStatus::Restored,
            source:   FixValidationSource::Unexpected(anyhow!("boom")),
        };
        assert!(
            failure
                .to_string()
                .contains("compiler failed after applying mend fixes; changes were rolled back")
        );
    }

    #[test]
    fn fix_validation_failure_reports_rollback_failure() {
        let failure = FixValidationFailure {
            rollback: RollbackStatus::RestoreFailed,
            source:   FixValidationSource::Unexpected(anyhow!("boom")),
        };
        assert!(
            failure
                .to_string()
                .contains("compiler failed after applying mend fixes, and rollback also failed")
        );
    }

    #[test]
    fn import_fix_notice_respects_operation_intent() {
        let preview = FixNotice::from_intent(OperationIntent::DryRun, 2);
        assert_eq!(preview.render(), "would apply 2 import fix(es) in dry run");
    }

    #[test]
    fn combined_notice_renders_all_parts() {
        let notice = ExecutionNotice::Combined(vec![
            ExecutionNotice::ImportFixes(FixNotice::Applied(2)),
            ExecutionNotice::PubUseFixes(PubUseNotice::Applied {
                applied:             1,
                skipped_unsupported: 0,
            }),
        ]);
        assert_eq!(
            notice.render(),
            "mend: applied 2 import fix(es); applied 1 `pub use` fix(es)"
        );
    }
}
