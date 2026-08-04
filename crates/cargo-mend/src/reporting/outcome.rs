use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Error;

use super::constants::EXIT_CODE_WARNING;
use super::diagnostics::CompilerWarningFacts;
use super::diagnostics::Report;
use crate::config::OperationIntent;

#[derive(Debug)]
pub(crate) struct ExecutionOutcome {
    pub report:                 Report,
    pub notice:                 Option<ExecutionNotice>,
    pub check_duration:         Duration,
    pub compiler_warnings:      usize,
    pub compiler_fixable:       usize,
    /// Count of `pub use` fixes actually applied (zero in dry-run / read-only).
    pub applied_pub_use:        usize,
    /// Post-apply validation's compiler-warning summary — `UnusedImportWarnings`
    /// signals that `cargo fix` should be chained to clean up the cascade.
    pub compiler_warning_facts: CompilerWarningFacts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionNotice {
    kinds: Vec<NoticeKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NoticeKind {
    Fixes(FixNotice),
    PubUseFixes(PubUseNotice),
}

/// Which family of edits a `FixNotice` counts. Every fixer that moves no import
/// gets its own kind, because calling a `pub` removal or a `pub` →
/// `pub(in crate::…)` rewrite an "import fix" names the wrong edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixKind {
    /// `imports`, `module_imports`, `inline_types`, and `imports_at_top` — the
    /// fixers that actually move or rewrite a `use` item.
    Import,
    /// `unused_pub` — strips a `pub` that nothing outside the module uses.
    PubRemoval,
    /// `narrowed_pub` — rewrites `pub` to `pub(crate)`.
    Narrowing,
    /// `restricted_annotation` — rewrites `pub` to an exact
    /// `pub(in crate::…)` boundary.
    Annotation,
    /// `field_visibility` — rewrites a field's visibility annotation.
    FieldVisibility,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FixNotice {
    fix_kind: FixKind,
    outcome:  FixOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FixOutcome {
    NoneAvailable,
    PreviewApplied(usize),
    Applied(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PubUseNotice {
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
pub(crate) enum MendFailure {
    Analysis(AnalysisFailure),
    FixValidation(FixValidationFailure),
    Unexpected(Error),
}

#[derive(Debug)]
pub(crate) enum CompilerFailureCause {
    CargoCheck,
    DriverSetup(Error),
    DriverExecution(Error),
    Unexpected(Error),
}

#[derive(Debug)]
pub(crate) struct AnalysisFailure {
    pub cause: CompilerFailureCause,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RollbackStatus {
    Restored,
    RestoreFailed,
}

#[derive(Debug)]
pub(crate) struct FixValidationFailure {
    pub rollback_status: RollbackStatus,
    pub cause:           CompilerFailureCause,
}

impl MendFailure {
    pub(crate) fn exit_code() -> ExitCode { ExitCode::from(EXIT_CODE_WARNING) }
}

impl Display for MendFailure {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Analysis(failure) => write!(f, "{failure}"),
            Self::FixValidation(failure) => write!(f, "{failure}"),
            Self::Unexpected(error) => write!(f, "{error:#}"),
        }
    }
}

impl Display for AnalysisFailure {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.cause {
            CompilerFailureCause::CargoCheck => {
                write!(
                    f,
                    "compiler failed while validating this crate\n\nmend: did not run due to compiler errors"
                )
            },
            CompilerFailureCause::DriverSetup(error)
            | CompilerFailureCause::DriverExecution(error)
            | CompilerFailureCause::Unexpected(error) => write!(f, "{error:#}"),
        }
    }
}

impl Display for FixValidationFailure {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let source = match &self.cause {
            CompilerFailureCause::CargoCheck => {
                "compiler failed after applying mend fixes".to_string()
            },
            CompilerFailureCause::DriverSetup(error)
            | CompilerFailureCause::DriverExecution(error)
            | CompilerFailureCause::Unexpected(error) => format!("{error:#}"),
        };
        match self.rollback_status {
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
    pub(crate) fn render(&self) -> String {
        let parts = self
            .kinds
            .iter()
            .map(NoticeKind::render_part)
            .collect::<Vec<_>>();
        format!("mend: {}", parts.join("; "))
    }
}

impl From<NoticeKind> for ExecutionNotice {
    fn from(kind: NoticeKind) -> Self { Self { kinds: vec![kind] } }
}

impl From<Vec<NoticeKind>> for ExecutionNotice {
    fn from(kinds: Vec<NoticeKind>) -> Self { Self { kinds } }
}

impl NoticeKind {
    fn render_part(&self) -> String {
        match self {
            Self::Fixes(notice) => notice.render(),
            Self::PubUseFixes(notice) => notice.render(),
        }
    }
}

impl FixKind {
    /// The noun for a bare count of edits, e.g. "applied 3 import fix(es)".
    const fn counted(self) -> &'static str {
        match self {
            Self::Import => "import fix(es)",
            Self::PubRemoval => "`pub` removal(s)",
            Self::Narrowing => "visibility narrowing(s)",
            Self::Annotation => "annotation rewrite(s)",
            Self::FieldVisibility => "field visibility rewrite(s)",
        }
    }

    /// The plural noun used when nothing was available.
    const fn plural(self) -> &'static str {
        match self {
            Self::Import => "import fixes",
            Self::PubRemoval => "`pub` removals",
            Self::Narrowing => "visibility narrowings",
            Self::Annotation => "annotation rewrites",
            Self::FieldVisibility => "field visibility rewrites",
        }
    }
}

impl FixNotice {
    fn render(&self) -> String {
        match self.outcome {
            FixOutcome::NoneAvailable => format!("no {} available", self.fix_kind.plural()),
            FixOutcome::PreviewApplied(count) => {
                format!("would apply {count} {} in dry run", self.fix_kind.counted())
            },
            FixOutcome::Applied(count) => {
                format!("applied {count} {}", self.fix_kind.counted())
            },
        }
    }

    pub(crate) const fn from_intent(
        intent: OperationIntent,
        fix_kind: FixKind,
        count: usize,
    ) -> Self {
        let outcome = match intent {
            OperationIntent::ReadOnly => FixOutcome::NoneAvailable,
            OperationIntent::DryRun => {
                if count == 0 {
                    FixOutcome::NoneAvailable
                } else {
                    FixOutcome::PreviewApplied(count)
                }
            },
            OperationIntent::Apply => {
                if count == 0 {
                    FixOutcome::NoneAvailable
                } else {
                    FixOutcome::Applied(count)
                }
            },
        };
        Self { fix_kind, outcome }
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

    pub(crate) const fn from_intent(
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
    use super::CompilerFailureCause;
    use super::ExecutionNotice;
    use super::FixKind;
    use super::FixNotice;
    use super::FixValidationFailure;
    use super::NoticeKind;
    use super::PubUseNotice;
    use super::RollbackStatus;
    use crate::config::OperationIntent;

    #[test]
    fn analysis_failure_message_uses_typed_collection_wording() {
        let failure = AnalysisFailure {
            cause: CompilerFailureCause::CargoCheck,
        };
        assert_eq!(
            failure.to_string(),
            "compiler failed while validating this crate\n\nmend: did not run due to compiler errors"
        );
    }

    #[test]
    fn fix_validation_failure_reports_rollback_success() {
        let failure = FixValidationFailure {
            rollback_status: RollbackStatus::Restored,
            cause:           CompilerFailureCause::Unexpected(anyhow!("boom")),
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
            rollback_status: RollbackStatus::RestoreFailed,
            cause:           CompilerFailureCause::Unexpected(anyhow!("boom")),
        };
        assert!(
            failure
                .to_string()
                .contains("compiler failed after applying mend fixes, and rollback also failed")
        );
    }

    #[test]
    fn import_fix_notice_respects_operation_intent() {
        let preview = FixNotice::from_intent(OperationIntent::DryRun, FixKind::Import, 2);
        assert_eq!(preview.render(), "would apply 2 import fix(es) in dry run");
    }

    #[test]
    fn annotation_fix_notice_names_the_rewrite_rather_than_an_import() {
        let applied = FixNotice::from_intent(OperationIntent::Apply, FixKind::Annotation, 3);
        assert_eq!(applied.render(), "applied 3 annotation rewrite(s)");

        let none = FixNotice::from_intent(OperationIntent::Apply, FixKind::Annotation, 0);
        assert_eq!(none.render(), "no annotation rewrites available");
    }

    #[test]
    fn visibility_fix_notices_name_their_own_edit() {
        let removals = FixNotice::from_intent(OperationIntent::Apply, FixKind::PubRemoval, 2);
        assert_eq!(removals.render(), "applied 2 `pub` removal(s)");

        let narrowings = FixNotice::from_intent(OperationIntent::DryRun, FixKind::Narrowing, 1);
        assert_eq!(
            narrowings.render(),
            "would apply 1 visibility narrowing(s) in dry run"
        );

        let fields = FixNotice::from_intent(OperationIntent::Apply, FixKind::FieldVisibility, 0);
        assert_eq!(fields.render(), "no field visibility rewrites available");
    }

    #[test]
    fn combined_notice_renders_all_parts() {
        let notice = ExecutionNotice::from(vec![
            NoticeKind::Fixes(FixNotice::from_intent(
                OperationIntent::Apply,
                FixKind::Import,
                2,
            )),
            NoticeKind::Fixes(FixNotice::from_intent(
                OperationIntent::Apply,
                FixKind::Annotation,
                1,
            )),
            NoticeKind::PubUseFixes(PubUseNotice::Applied {
                applied:             1,
                skipped_unsupported: 0,
            }),
        ]);
        assert_eq!(
            notice.render(),
            "mend: applied 2 import fix(es); applied 1 annotation rewrite(s); applied 1 `pub use` fix(es)"
        );
    }
}
