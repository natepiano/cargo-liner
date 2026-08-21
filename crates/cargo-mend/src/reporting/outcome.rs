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

/// How many edits of each [`FixKind`] a run wrote to disk.
///
/// The tally has to travel from the applier to the notice because fixes are
/// discarded between the scan and the write: conflicting import groups are
/// dropped, byte-identical edits collapse, and a range that no longer fits its
/// file is skipped. Counting the scan's findings instead announced work that
/// never happened — the same "applied 4 import fix(es)" on every run with the
/// files untouched, which reads as a `--fix-all` that never converges.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AppliedFixCounts {
    import:           usize,
    pub_removal:      usize,
    narrowing:        usize,
    annotation:       usize,
    field_visibility: usize,
}

impl AppliedFixCounts {
    /// Records one edit written for `fix_kind`.
    pub(crate) const fn record(&mut self, fix_kind: FixKind) {
        match fix_kind {
            FixKind::Import => self.import += 1,
            FixKind::PubRemoval => self.pub_removal += 1,
            FixKind::Narrowing => self.narrowing += 1,
            FixKind::Annotation => self.annotation += 1,
            FixKind::FieldVisibility => self.field_visibility += 1,
        }
    }

    /// Edits written for `fix_kind`.
    pub(crate) const fn count(self, fix_kind: FixKind) -> usize {
        match fix_kind {
            FixKind::Import => self.import,
            FixKind::PubRemoval => self.pub_removal,
            FixKind::Narrowing => self.narrowing,
            FixKind::Annotation => self.annotation,
            FixKind::FieldVisibility => self.field_visibility,
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// No stored report reached `load_report`, so nothing was analyzed. Reported
    /// as a failure because an empty findings list would otherwise print as a
    /// clean crate.
    NoAnalysisProduced,
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
    pub applied_fixes:   AppliedFixes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppliedFixes {
    Mend,
    Compiler,
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
            CompilerFailureCause::NoAnalysisProduced => {
                write!(
                    f,
                    "no analysis was produced for this crate\n\nmend: cargo had nothing to rebuild and no compatible cached findings were available, so no code was examined; force a rebuild (touch a source file, or `cargo clean -p <package>`) and run again"
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
        let applied_fixes = match self.applied_fixes {
            AppliedFixes::Mend => "mend",
            AppliedFixes::Compiler => "compiler",
        };
        let source = match &self.cause {
            CompilerFailureCause::CargoCheck => {
                format!("compiler failed after applying {applied_fixes} fixes")
            },
            CompilerFailureCause::NoAnalysisProduced => {
                "no analysis was produced after applying mend fixes".to_string()
            },
            CompilerFailureCause::DriverSetup(error)
            | CompilerFailureCause::DriverExecution(error)
            | CompilerFailureCause::Unexpected(error) => format!("{error:#}"),
        };
        match self.rollback_status {
            RollbackStatus::Restored => write!(
                f,
                "compiler failed after applying {applied_fixes} fixes; changes were rolled back\n\n{source:#}"
            ),
            RollbackStatus::RestoreFailed => write!(
                f,
                "compiler failed after applying {applied_fixes} fixes, and rollback also failed\n\n{source:#}"
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

    pub(crate) fn merge(&mut self, additional: Self) {
        for kind in additional.kinds {
            if kind.has_applied_edits() {
                self.kinds.retain(|existing| !existing.is_empty_noop());
            }
            if kind.is_empty_noop() && self.kinds.iter().any(NoticeKind::has_applied_edits) {
                continue;
            }
            let existing = self
                .kinds
                .iter_mut()
                .find(|existing| match (&**existing, &kind) {
                    (NoticeKind::Fixes(left), NoticeKind::Fixes(right)) => {
                        left.fix_kind == right.fix_kind
                    },
                    (NoticeKind::PubUseFixes(_), NoticeKind::PubUseFixes(_)) => true,
                    _ => false,
                });
            if let Some(existing) = existing {
                existing.merge(kind);
            } else {
                self.kinds.push(kind);
            }
        }
    }
}

impl From<NoticeKind> for ExecutionNotice {
    fn from(kind: NoticeKind) -> Self { Self { kinds: vec![kind] } }
}

impl From<Vec<NoticeKind>> for ExecutionNotice {
    fn from(kinds: Vec<NoticeKind>) -> Self { Self { kinds } }
}

impl NoticeKind {
    const fn has_applied_edits(&self) -> bool {
        match self {
            Self::Fixes(FixNotice {
                outcome: FixOutcome::PreviewApplied(_) | FixOutcome::Applied(_),
                ..
            })
            | Self::PubUseFixes(
                PubUseNotice::PreviewApplied { .. } | PubUseNotice::Applied { .. },
            ) => true,
            Self::Fixes(FixNotice {
                outcome: FixOutcome::NoneAvailable,
                ..
            })
            | Self::PubUseFixes(PubUseNotice::NoneAvailable { .. }) => false,
        }
    }

    const fn is_empty_noop(&self) -> bool {
        matches!(
            self,
            Self::Fixes(FixNotice {
                outcome: FixOutcome::NoneAvailable,
                ..
            }) | Self::PubUseFixes(PubUseNotice::NoneAvailable {
                skipped_unsupported: 0,
            })
        )
    }

    fn merge(&mut self, additional: Self) {
        match (self, additional) {
            (Self::Fixes(current), Self::Fixes(additional)) => current.merge(additional),
            (Self::PubUseFixes(current), Self::PubUseFixes(additional)) => {
                current.merge(additional);
            },
            _ => {},
        }
    }

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
    fn merge(&mut self, additional: Self) {
        debug_assert_eq!(self.fix_kind, additional.fix_kind);
        self.outcome = match (&self.outcome, additional.outcome) {
            (FixOutcome::NoneAvailable, additional) => additional,
            (current, FixOutcome::NoneAvailable) => current.clone(),
            (FixOutcome::PreviewApplied(current), FixOutcome::PreviewApplied(additional)) => {
                FixOutcome::PreviewApplied(current + additional)
            },
            (FixOutcome::Applied(current), FixOutcome::Applied(additional)) => {
                FixOutcome::Applied(current + additional)
            },
            (_, additional) => additional,
        };
    }

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
    const fn merge(&mut self, additional: Self) {
        let (current_applied, current_skipped, current_intent) = self.counts();
        let (additional_applied, additional_skipped, additional_intent) = additional.counts();
        let applied = current_applied + additional_applied;
        let skipped_unsupported = current_skipped + additional_skipped;
        let merged_intent = match (current_intent, additional_intent) {
            (OperationIntent::DryRun, _) | (_, OperationIntent::DryRun) => OperationIntent::DryRun,
            _ => OperationIntent::Apply,
        };
        *self = match (applied, merged_intent) {
            (0, _) => Self::NoneAvailable {
                skipped_unsupported,
            },
            (_, OperationIntent::DryRun) => Self::PreviewApplied {
                applied,
                skipped_unsupported,
            },
            (_, OperationIntent::ReadOnly | OperationIntent::Apply) => Self::Applied {
                applied,
                skipped_unsupported,
            },
        };
    }

    const fn counts(&self) -> (usize, usize, OperationIntent) {
        match self {
            Self::NoneAvailable {
                skipped_unsupported,
            } => (0, *skipped_unsupported, OperationIntent::ReadOnly),
            Self::PreviewApplied {
                applied,
                skipped_unsupported,
            } => (*applied, *skipped_unsupported, OperationIntent::DryRun),
            Self::Applied {
                applied,
                skipped_unsupported,
            } => (*applied, *skipped_unsupported, OperationIntent::Apply),
        }
    }

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
    use super::AppliedFixes;
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
    fn no_analysis_failure_message_tells_the_user_to_force_a_rebuild() {
        let failure = AnalysisFailure {
            cause: CompilerFailureCause::NoAnalysisProduced,
        };
        let message = failure.to_string();
        assert!(message.starts_with("no analysis was produced for this crate"));
        assert!(message.contains("force a rebuild"));
    }

    #[test]
    fn fix_validation_failure_reports_rollback_success() {
        let failure = FixValidationFailure {
            rollback_status: RollbackStatus::Restored,
            cause:           CompilerFailureCause::Unexpected(anyhow!("boom")),
            applied_fixes:   AppliedFixes::Mend,
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
            applied_fixes:   AppliedFixes::Mend,
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

    #[test]
    fn merged_notice_accumulates_edits_and_discards_later_empty_passes() {
        let mut notice = ExecutionNotice::from(vec![
            NoticeKind::Fixes(FixNotice::from_intent(
                OperationIntent::Apply,
                FixKind::Annotation,
                2,
            )),
            NoticeKind::PubUseFixes(PubUseNotice::Applied {
                applied:             1,
                skipped_unsupported: 0,
            }),
        ]);
        notice.merge(ExecutionNotice::from(vec![
            NoticeKind::Fixes(FixNotice::from_intent(
                OperationIntent::Apply,
                FixKind::Annotation,
                1,
            )),
            NoticeKind::Fixes(FixNotice::from_intent(
                OperationIntent::Apply,
                FixKind::Import,
                0,
            )),
            NoticeKind::PubUseFixes(PubUseNotice::NoneAvailable {
                skipped_unsupported: 0,
            }),
        ]));

        assert_eq!(
            notice.render(),
            "mend: applied 3 annotation rewrite(s); applied 1 `pub use` fix(es)"
        );
    }
}
