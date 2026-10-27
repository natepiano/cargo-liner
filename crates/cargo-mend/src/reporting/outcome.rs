use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::num::NonZeroUsize;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Error;

use super::constants::EXIT_CODE_WARNING;
use super::diagnostics::CompilerWarningFacts;
use super::diagnostics::Report;
use crate::config::OperationIntent;

/// Whether a `--fix` pass wrote any mend edit to disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PassEdits {
    Written,
    Unchanged,
}

#[derive(Debug)]
pub(crate) struct ExecutionOutcome {
    pub report:                   Report,
    pub notice:                   Option<ExecutionNotice>,
    pub check_duration:           Duration,
    pub compiler_warnings:        usize,
    pub compiler_fixable:         usize,
    /// Count of `pub use` fixes actually applied (zero in dry-run / read-only).
    pub applied_pub_use:          usize,
    /// Whether this pass wrote `pub_use_outside_subtree` edits. Removing one
    /// link of a re-export chain turns the next link into a new finding, so a
    /// pass can make progress while the fixable count stays level.
    pub applied_subtree_reexport: bool,
    /// Whether this pass wrote mend edits; a pass that wrote none leaves the
    /// tree the next pass would plan against unchanged.
    pub pass_edits:               PassEdits,
    /// Post-apply validation's compiler-warning summary — `UnusedImportWarnings`
    /// signals that `cargo fix` should be chained to clean up the cascade.
    pub compiler_warning_facts:   CompilerWarningFacts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionNotice {
    kinds: Vec<NoticeKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NoticeKind {
    Fixes(FixNotice),
    PubUseFixes(PubUseNotice),
    /// Fixable `pub_use_outside_subtree` findings the pass left unedited. A
    /// later convergence pass replaces the count, since each pass re-scans the
    /// findings still standing; zero renders nothing.
    SubtreeReexportSkipped(usize),
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

/// Applied states always carry at least one edit; zero belongs to `NoneAvailable`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FixOutcome {
    NoneAvailable,
    PreviewApplied(NonZeroUsize),
    Applied(NonZeroUsize),
}

enum NoticeEffect {
    Applied,
    EmptyNoop,
    RetainedWithoutEdits,
}

impl From<&NoticeKind> for NoticeEffect {
    fn from(kind: &NoticeKind) -> Self {
        match kind {
            NoticeKind::Fixes(FixNotice {
                outcome: FixOutcome::PreviewApplied(_) | FixOutcome::Applied(_),
                ..
            })
            | NoticeKind::PubUseFixes(
                PubUseNotice::PreviewApplied { .. } | PubUseNotice::Applied { .. },
            ) => Self::Applied,
            NoticeKind::Fixes(FixNotice {
                outcome: FixOutcome::NoneAvailable,
                ..
            })
            | NoticeKind::PubUseFixes(PubUseNotice::NoneAvailable {
                skipped_unsupported: 0,
            }) => Self::EmptyNoop,
            NoticeKind::PubUseFixes(PubUseNotice::NoneAvailable { .. })
            | NoticeKind::SubtreeReexportSkipped(_) => Self::RetainedWithoutEdits,
        }
    }
}

/// Applied pub-use states require a positive count; skipped work can stand alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PubUseNotice {
    NoneAvailable {
        skipped_unsupported: usize,
    },
    PreviewApplied {
        applied:             NonZeroUsize,
        skipped_unsupported: usize,
    },
    Applied {
        applied:             NonZeroUsize,
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
            CompilerFailureCause::DriverSetup(error) | CompilerFailureCause::Unexpected(error) => {
                write!(f, "{error:#}")
            },
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
            CompilerFailureCause::DriverSetup(error) | CompilerFailureCause::Unexpected(error) => {
                format!("{error:#}")
            },
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
            .filter_map(NoticeKind::render_part)
            .collect::<Vec<_>>();
        format!("mend: {}", parts.join("; "))
    }

    pub(crate) fn merge(&mut self, additional: Self) {
        for kind in additional.kinds {
            match NoticeEffect::from(&kind) {
                NoticeEffect::Applied => self.kinds.retain(|existing| {
                    !matches!(NoticeEffect::from(existing), NoticeEffect::EmptyNoop)
                }),
                NoticeEffect::EmptyNoop => {
                    if self.kinds.iter().any(|existing| {
                        matches!(NoticeEffect::from(existing), NoticeEffect::Applied)
                    }) {
                        continue;
                    }
                },
                NoticeEffect::RetainedWithoutEdits => {},
            }
            self.merge_kind(kind);
        }
    }

    /// Select the family and fix key before merging its payload. A mismatched
    /// family cannot silently discard a notice, nor can different fixes combine.
    fn merge_kind(&mut self, kind: NoticeKind) {
        match kind {
            NoticeKind::Fixes(FixNotice { fix_kind, outcome }) => {
                let current = self.kinds.iter_mut().find_map(|entry| match entry {
                    NoticeKind::Fixes(notice) => {
                        (notice.fix_kind == fix_kind).then_some(&mut notice.outcome)
                    },
                    NoticeKind::PubUseFixes(_) | NoticeKind::SubtreeReexportSkipped(_) => None,
                });
                if let Some(current) = current {
                    current.merge(outcome);
                } else {
                    self.kinds
                        .push(NoticeKind::Fixes(FixNotice { fix_kind, outcome }));
                }
            },
            NoticeKind::PubUseFixes(additional) => {
                let current = self.kinds.iter_mut().find_map(|entry| match entry {
                    NoticeKind::PubUseFixes(notice) => Some(notice),
                    NoticeKind::Fixes(_) | NoticeKind::SubtreeReexportSkipped(_) => None,
                });
                if let Some(current) = current {
                    current.merge(additional);
                } else {
                    self.kinds.push(NoticeKind::PubUseFixes(additional));
                }
            },
            NoticeKind::SubtreeReexportSkipped(additional) => {
                let current = self.kinds.iter_mut().find_map(|entry| match entry {
                    NoticeKind::SubtreeReexportSkipped(skipped) => Some(skipped),
                    NoticeKind::Fixes(_) | NoticeKind::PubUseFixes(_) => None,
                });
                if let Some(current) = current {
                    *current = additional;
                } else {
                    self.kinds
                        .push(NoticeKind::SubtreeReexportSkipped(additional));
                }
            },
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
    fn render_part(&self) -> Option<String> {
        match self {
            Self::Fixes(notice) => Some(notice.render()),
            Self::PubUseFixes(notice) => Some(notice.render()),
            Self::SubtreeReexportSkipped(0) => None,
            Self::SubtreeReexportSkipped(skipped) => Some(format!(
                "skipped {skipped} `pub_use_outside_subtree` finding(s) `--fix` could not rewrite"
            )),
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

impl FixOutcome {
    fn merge(&mut self, additional: Self) {
        *self = match (&*self, additional) {
            (
                Self::NoneAvailable,
                additional @ (Self::NoneAvailable | Self::PreviewApplied(_) | Self::Applied(_)),
            )
            | (Self::PreviewApplied(_), additional @ Self::Applied(_))
            | (Self::Applied(_), additional @ Self::PreviewApplied(_)) => additional,
            (current @ (Self::PreviewApplied(_) | Self::Applied(_)), Self::NoneAvailable) => {
                current.clone()
            },
            (Self::PreviewApplied(current), Self::PreviewApplied(additional)) => {
                Self::PreviewApplied(current.saturating_add(additional.get()))
            },
            (Self::Applied(current), Self::Applied(additional)) => {
                Self::Applied(current.saturating_add(additional.get()))
            },
        };
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
        let outcome = match (intent, NonZeroUsize::new(count)) {
            (OperationIntent::ReadOnly, None | Some(_))
            | (OperationIntent::DryRun | OperationIntent::Apply, None) => FixOutcome::NoneAvailable,
            (OperationIntent::DryRun, Some(count)) => FixOutcome::PreviewApplied(count),
            (OperationIntent::Apply, Some(count)) => FixOutcome::Applied(count),
        };
        Self { fix_kind, outcome }
    }
}

impl PubUseNotice {
    const fn merge(&mut self, additional: Self) {
        let (current, current_skipped) = self.parts();
        let (additional, additional_skipped) = additional.parts();
        // Preview dominates applied for pub-use notices, including mixed passes.
        let current = match (current, additional) {
            (
                FixOutcome::NoneAvailable,
                additional @ (FixOutcome::NoneAvailable
                | FixOutcome::PreviewApplied(_)
                | FixOutcome::Applied(_)),
            ) => additional,
            (
                current @ (FixOutcome::PreviewApplied(_) | FixOutcome::Applied(_)),
                FixOutcome::NoneAvailable,
            ) => current,
            (
                FixOutcome::PreviewApplied(left),
                FixOutcome::PreviewApplied(right) | FixOutcome::Applied(right),
            )
            | (FixOutcome::Applied(left), FixOutcome::PreviewApplied(right)) => {
                FixOutcome::PreviewApplied(left.saturating_add(right.get()))
            },
            (FixOutcome::Applied(left), FixOutcome::Applied(right)) => {
                FixOutcome::Applied(left.saturating_add(right.get()))
            },
        };
        let skipped_unsupported = current_skipped + additional_skipped;
        *self = match current {
            FixOutcome::NoneAvailable => Self::NoneAvailable {
                skipped_unsupported,
            },
            FixOutcome::PreviewApplied(applied) => Self::PreviewApplied {
                applied,
                skipped_unsupported,
            },
            FixOutcome::Applied(applied) => Self::Applied {
                applied,
                skipped_unsupported,
            },
        };
    }

    const fn parts(self) -> (FixOutcome, usize) {
        match self {
            Self::NoneAvailable {
                skipped_unsupported,
            } => (FixOutcome::NoneAvailable, skipped_unsupported),
            Self::PreviewApplied {
                applied,
                skipped_unsupported,
            } => (FixOutcome::PreviewApplied(applied), skipped_unsupported),
            Self::Applied {
                applied,
                skipped_unsupported,
            } => (FixOutcome::Applied(applied), skipped_unsupported),
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
        match (intent, NonZeroUsize::new(applied)) {
            (OperationIntent::ReadOnly, None | Some(_))
            | (OperationIntent::DryRun | OperationIntent::Apply, None) => Self::NoneAvailable {
                skipped_unsupported,
            },
            (OperationIntent::DryRun, Some(applied)) => Self::PreviewApplied {
                applied,
                skipped_unsupported,
            },
            (OperationIntent::Apply, Some(applied)) => Self::Applied {
                applied,
                skipped_unsupported,
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
    fn applied_states_retain_positive_counts_and_zero_has_its_own_state() {
        for intent in [
            OperationIntent::ReadOnly,
            OperationIntent::DryRun,
            OperationIntent::Apply,
        ] {
            for count in [0, 2] {
                let fix = FixNotice::from_intent(intent, FixKind::Import, count);
                let pub_use = PubUseNotice::from_intent(intent, count, 3);
                let expected = match intent {
                    OperationIntent::ReadOnly => 0,
                    OperationIntent::DryRun | OperationIntent::Apply => count,
                };
                let fix_count = match fix.outcome {
                    super::FixOutcome::NoneAvailable => 0,
                    super::FixOutcome::PreviewApplied(count)
                    | super::FixOutcome::Applied(count) => count.get(),
                };
                let pub_use_count = match pub_use {
                    PubUseNotice::NoneAvailable { .. } => 0,
                    PubUseNotice::PreviewApplied { applied, .. }
                    | PubUseNotice::Applied { applied, .. } => applied.get(),
                };
                assert_eq!((fix_count, pub_use_count), (expected, expected));
            }
        }
    }

    #[test]
    fn merge_payloads_preserve_counts_and_each_familys_mode_policy() {
        let cases = [
            (
                OperationIntent::ReadOnly,
                OperationIntent::ReadOnly,
                0,
                false,
            ),
            (OperationIntent::ReadOnly, OperationIntent::DryRun, 3, true),
            (OperationIntent::ReadOnly, OperationIntent::Apply, 3, false),
            (OperationIntent::DryRun, OperationIntent::ReadOnly, 2, true),
            (OperationIntent::DryRun, OperationIntent::DryRun, 5, true),
            (OperationIntent::DryRun, OperationIntent::Apply, 3, false),
            (OperationIntent::Apply, OperationIntent::ReadOnly, 2, false),
            (OperationIntent::Apply, OperationIntent::DryRun, 3, true),
            (OperationIntent::Apply, OperationIntent::Apply, 5, false),
        ];
        for (left, right, count, preview) in cases {
            let mut fix = FixNotice::from_intent(left, FixKind::Import, 2);
            fix.outcome
                .merge(FixNotice::from_intent(right, FixKind::Import, 3).outcome);
            let intent = if preview {
                OperationIntent::DryRun
            } else {
                OperationIntent::Apply
            };
            assert_eq!(fix, FixNotice::from_intent(intent, FixKind::Import, count));
            let mut pub_use = PubUseNotice::from_intent(left, 2, 1);
            pub_use.merge(PubUseNotice::from_intent(right, 3, 2));
            let total = usize::from(left != OperationIntent::ReadOnly) * 2
                + usize::from(right != OperationIntent::ReadOnly) * 3;
            let intent = if left == OperationIntent::DryRun || right == OperationIntent::DryRun {
                OperationIntent::DryRun
            } else {
                OperationIntent::Apply
            };
            assert_eq!(pub_use, PubUseNotice::from_intent(intent, total, 3));
        }
    }

    #[test]
    fn keyed_merge_keeps_all_families_and_fix_kinds_in_first_seen_order() {
        let kinds = [
            FixKind::Import,
            FixKind::PubRemoval,
            FixKind::Narrowing,
            FixKind::Annotation,
            FixKind::FieldVisibility,
        ];
        let mut notice = ExecutionNotice::from(Vec::new());
        for count in [2, 3] {
            for kind in kinds {
                notice.merge_kind(NoticeKind::Fixes(FixNotice::from_intent(
                    OperationIntent::Apply,
                    kind,
                    count,
                )));
            }
            notice.merge_kind(NoticeKind::PubUseFixes(PubUseNotice::from_intent(
                OperationIntent::Apply,
                count,
                1,
            )));
            notice.merge_kind(NoticeKind::SubtreeReexportSkipped(count));
        }
        let mut expected = kinds
            .into_iter()
            .map(|kind| NoticeKind::Fixes(FixNotice::from_intent(OperationIntent::Apply, kind, 5)))
            .collect::<Vec<_>>();
        expected.push(NoticeKind::PubUseFixes(PubUseNotice::from_intent(
            OperationIntent::Apply,
            5,
            2,
        )));
        expected.push(NoticeKind::SubtreeReexportSkipped(3));
        assert_eq!(notice.kinds, expected);
    }

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
    fn notice_counts_preserve_read_only_zero_preview_and_applied_outcomes() {
        for (intent, count, expected_fix, expected_pub_use) in [
            (
                OperationIntent::ReadOnly,
                0,
                "no import fixes available",
                "no `pub use` fixes available",
            ),
            (
                OperationIntent::ReadOnly,
                2,
                "no import fixes available",
                "no `pub use` fixes available",
            ),
            (
                OperationIntent::DryRun,
                0,
                "no import fixes available",
                "no `pub use` fixes available",
            ),
            (
                OperationIntent::Apply,
                0,
                "no import fixes available",
                "no `pub use` fixes available",
            ),
            (
                OperationIntent::DryRun,
                2,
                "would apply 2 import fix(es) in dry run",
                "would apply 2 `pub use` fix(es) in dry run",
            ),
            (
                OperationIntent::Apply,
                2,
                "applied 2 import fix(es)",
                "applied 2 `pub use` fix(es)",
            ),
        ] {
            assert_eq!(
                FixNotice::from_intent(intent, FixKind::Import, count).render(),
                expected_fix
            );
            assert_eq!(
                PubUseNotice::from_intent(intent, count, 0).render(),
                expected_pub_use
            );
            assert_eq!(
                PubUseNotice::from_intent(intent, count, 3).render(),
                format!("{expected_pub_use}; skipped 3 unsupported `pub use` candidate(s)")
            );
        }
    }

    #[test]
    fn merging_applied_notices_prunes_noops_but_keeps_skipped_work() {
        let mut notice = ExecutionNotice::from(vec![
            NoticeKind::Fixes(FixNotice::from_intent(
                OperationIntent::Apply,
                FixKind::Import,
                0,
            )),
            NoticeKind::PubUseFixes(PubUseNotice::NoneAvailable {
                skipped_unsupported: 3,
            }),
        ]);
        notice.merge(ExecutionNotice::from(NoticeKind::Fixes(
            FixNotice::from_intent(OperationIntent::DryRun, FixKind::Annotation, 2),
        )));
        assert_eq!(
            notice.render(),
            concat!(
                "mend: no `pub use` fixes available; ",
                "skipped 3 unsupported `pub use` candidate(s); ",
                "would apply 2 annotation rewrite(s) in dry run",
            )
        );
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
            NoticeKind::PubUseFixes(PubUseNotice::from_intent(OperationIntent::Apply, 1, 0)),
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
            NoticeKind::PubUseFixes(PubUseNotice::from_intent(OperationIntent::Apply, 1, 0)),
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

    #[test]
    fn subtree_reexport_skips_keep_the_latest_pass_count() {
        let mut notice = ExecutionNotice::from(vec![
            NoticeKind::Fixes(FixNotice::from_intent(
                OperationIntent::Apply,
                FixKind::Import,
                3,
            )),
            NoticeKind::SubtreeReexportSkipped(2),
        ]);
        notice.merge(ExecutionNotice::from(vec![
            NoticeKind::Fixes(FixNotice::from_intent(
                OperationIntent::Apply,
                FixKind::Import,
                0,
            )),
            NoticeKind::SubtreeReexportSkipped(1),
        ]));
        assert_eq!(
            notice.render(),
            "mend: applied 3 import fix(es); skipped 1 `pub_use_outside_subtree` finding(s) \
             `--fix` could not rewrite"
        );

        notice.merge(ExecutionNotice::from(vec![
            NoticeKind::SubtreeReexportSkipped(0),
        ]));
        assert_eq!(notice.render(), "mend: applied 3 import fix(es)");
    }
}
