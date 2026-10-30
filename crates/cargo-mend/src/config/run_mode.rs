use std::collections::BTreeSet;

use super::cli::FixCli;
use super::cli::FixExecution;
use super::cli::FixRequest;
use super::cli::FixRequestSelection;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FixKind {
    ShortenImport,
    PreferModuleImport,
    InlinePathQualifiedType,
    UnusedPub,
    NarrowToPubCrate,
    RestrictedAnnotation,
    FieldVisibility,
    ImportsAtTop,
    PubUseOutsideSubtree,
    PubUse,
}

/// Selection means the operation requests this fix kind. Diagnostic policy may still exclude
/// it from a pass; set membership is converted here so callers retain the named decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixKindSelection {
    Selected,
    NotSelected,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FixSelection {
    fix_kinds: BTreeSet<FixKind>,
}

impl FixSelection {
    fn all_fix_kinds() -> Self {
        let mut fix_kinds = BTreeSet::new();
        fix_kinds.insert(FixKind::RestrictedAnnotation);
        fix_kinds.insert(FixKind::PubUse);
        fix_kinds.insert(FixKind::UnusedPub);
        fix_kinds.insert(FixKind::PreferModuleImport);
        fix_kinds.insert(FixKind::InlinePathQualifiedType);
        fix_kinds.insert(FixKind::ShortenImport);
        fix_kinds.insert(FixKind::NarrowToPubCrate);
        fix_kinds.insert(FixKind::FieldVisibility);
        fix_kinds.insert(FixKind::ImportsAtTop);
        fix_kinds.insert(FixKind::PubUseOutsideSubtree);
        Self { fix_kinds }
    }

    pub(crate) fn selection(&self, kind: FixKind) -> FixKindSelection {
        if self.fix_kinds.contains(&kind) {
            FixKindSelection::Selected
        } else {
            FixKindSelection::NotSelected
        }
    }
}

impl From<&FixCli> for FixSelection {
    fn from(fix_cli: &FixCli) -> Self {
        match fix_cli.execution {
            FixExecution::ApplyAll | FixExecution::PreviewAll => Self::all_fix_kinds(),
            FixExecution::ReadOnly => Self::default(),
            FixExecution::ApplyRequested | FixExecution::PreviewRequested => {
                let mut fix_kinds = BTreeSet::new();
                if fix_cli.includes(FixRequest::Mend) == FixRequestSelection::Requested {
                    fix_kinds.insert(FixKind::RestrictedAnnotation);
                }
                if fix_cli.includes(FixRequest::PubUse) == FixRequestSelection::Requested {
                    fix_kinds.insert(FixKind::PubUse);
                }
                if fix_cli.includes(FixRequest::Mend) == FixRequestSelection::Requested {
                    fix_kinds.insert(FixKind::UnusedPub);
                    fix_kinds.insert(FixKind::PreferModuleImport);
                    fix_kinds.insert(FixKind::InlinePathQualifiedType);
                    fix_kinds.insert(FixKind::ShortenImport);
                    fix_kinds.insert(FixKind::NarrowToPubCrate);
                    fix_kinds.insert(FixKind::FieldVisibility);
                    fix_kinds.insert(FixKind::ImportsAtTop);
                    fix_kinds.insert(FixKind::PubUseOutsideSubtree);
                }
                Self { fix_kinds }
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationIntent {
    ReadOnly,
    DryRun,
    Apply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OperationMode {
    pub intent: OperationIntent,
    pub fixes:  FixSelection,
}

impl From<&FixCli> for OperationMode {
    fn from(fix_cli: &FixCli) -> Self {
        let fixes = FixSelection::from(fix_cli);
        match fix_cli.execution {
            FixExecution::PreviewRequested | FixExecution::PreviewAll => Self {
                intent: OperationIntent::DryRun,
                fixes,
            },
            FixExecution::ReadOnly => Self {
                intent: OperationIntent::ReadOnly,
                fixes,
            },
            FixExecution::ApplyRequested | FixExecution::ApplyAll => Self {
                intent: OperationIntent::Apply,
                fixes,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::FixKind;
    use super::FixKindSelection;
    use super::FixSelection;
    use super::OperationIntent;
    use super::OperationMode;
    use crate::config::cli::FixCli;
    use crate::config::cli::FixExecution;
    use crate::config::cli::FixRequest;

    #[test]
    fn fix_kind_selection_distinguishes_empty_partial_and_complete_requests() {
        let empty = FixSelection::default();
        let partial = FixSelection::from(&FixCli {
            execution:       FixExecution::ApplyRequested,
            requested_fixes: BTreeSet::from([FixRequest::PubUse]),
        });
        let complete = FixSelection::all_fix_kinds();
        for kind in [
            FixKind::ShortenImport,
            FixKind::PreferModuleImport,
            FixKind::InlinePathQualifiedType,
            FixKind::UnusedPub,
            FixKind::NarrowToPubCrate,
            FixKind::RestrictedAnnotation,
            FixKind::FieldVisibility,
            FixKind::ImportsAtTop,
            FixKind::PubUseOutsideSubtree,
            FixKind::PubUse,
        ] {
            assert_eq!(empty.selection(kind), FixKindSelection::NotSelected);
            assert_eq!(complete.selection(kind), FixKindSelection::Selected);
            let expected = match kind {
                FixKind::PubUse => FixKindSelection::Selected,
                FixKind::ShortenImport
                | FixKind::PreferModuleImport
                | FixKind::InlinePathQualifiedType
                | FixKind::UnusedPub
                | FixKind::NarrowToPubCrate
                | FixKind::RestrictedAnnotation
                | FixKind::FieldVisibility
                | FixKind::ImportsAtTop
                | FixKind::PubUseOutsideSubtree => FixKindSelection::NotSelected,
            };
            assert_eq!(partial.selection(kind), expected);
        }
    }

    #[test]
    fn operation_mode_is_read_only_without_fix_flags() {
        let cli = FixCli::default();
        let operation_mode = OperationMode::from(&cli);
        assert_eq!(operation_mode.intent, OperationIntent::ReadOnly);
    }

    #[test]
    fn operation_mode_dry_run_alone_implies_all_fix_kinds() {
        let cli = FixCli {
            execution: FixExecution::PreviewAll,
            ..FixCli::default()
        };
        let operation_mode = OperationMode::from(&cli);
        assert_eq!(operation_mode.intent, OperationIntent::DryRun);
        assert_eq!(
            operation_mode.fixes.selection(FixKind::ShortenImport),
            FixKindSelection::Selected
        );
        assert_eq!(
            operation_mode.fixes.selection(FixKind::PreferModuleImport),
            FixKindSelection::Selected
        );
        assert_eq!(
            operation_mode
                .fixes
                .selection(FixKind::InlinePathQualifiedType),
            FixKindSelection::Selected
        );
        assert_eq!(
            operation_mode.fixes.selection(FixKind::NarrowToPubCrate),
            FixKindSelection::Selected
        );
        assert_eq!(
            operation_mode.fixes.selection(FixKind::UnusedPub),
            FixKindSelection::Selected
        );
        assert_eq!(
            operation_mode.fixes.selection(FixKind::PubUse),
            FixKindSelection::Selected
        );
    }

    #[test]
    fn operation_mode_allows_previewing_multiple_fix_kinds() {
        let cli = FixCli {
            execution:       FixExecution::PreviewRequested,
            requested_fixes: BTreeSet::from([FixRequest::Mend, FixRequest::PubUse]),
        };
        let operation_mode = OperationMode::from(&cli);
        assert_eq!(operation_mode.intent, OperationIntent::DryRun);
        assert_eq!(
            operation_mode.fixes.selection(FixKind::ShortenImport),
            FixKindSelection::Selected
        );
        assert_eq!(
            operation_mode.fixes.selection(FixKind::NarrowToPubCrate),
            FixKindSelection::Selected
        );
        assert_eq!(
            operation_mode.fixes.selection(FixKind::UnusedPub),
            FixKindSelection::Selected
        );
        assert_eq!(
            operation_mode.fixes.selection(FixKind::PubUse),
            FixKindSelection::Selected
        );
    }

    #[test]
    fn operation_mode_allows_applying_multiple_fix_kinds() {
        let cli = FixCli {
            execution:       FixExecution::ApplyRequested,
            requested_fixes: BTreeSet::from([FixRequest::Mend, FixRequest::PubUse]),
        };
        let operation_mode = OperationMode::from(&cli);
        assert_eq!(operation_mode.intent, OperationIntent::Apply);
        assert_eq!(
            operation_mode.fixes.selection(FixKind::ShortenImport),
            FixKindSelection::Selected
        );
        assert_eq!(
            operation_mode.fixes.selection(FixKind::NarrowToPubCrate),
            FixKindSelection::Selected
        );
        assert_eq!(
            operation_mode.fixes.selection(FixKind::UnusedPub),
            FixKindSelection::Selected
        );
        assert_eq!(
            operation_mode.fixes.selection(FixKind::PubUse),
            FixKindSelection::Selected
        );
    }
}
