use std::collections::BTreeSet;

use super::cli::FixCli;
use super::cli::FixExecution;
use super::cli::FixRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FixKind {
    ShortenImport,
    PreferModuleImport,
    InlinePathQualifiedType,
    UnusedPub,
    NarrowToPubCrate,
    FieldVisibility,
    ImportsAtTop,
    PubUse,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FixSelection {
    fix_kinds: BTreeSet<FixKind>,
}

impl From<&FixCli> for FixSelection {
    fn from(fix_cli: &FixCli) -> Self {
        match fix_cli.execution {
            FixExecution::ApplyAll | FixExecution::PreviewAll => Self::all_fix_kinds(),
            FixExecution::ReadOnly => Self::default(),
            FixExecution::ApplyRequested | FixExecution::PreviewRequested => {
                let mut fix_kinds = BTreeSet::new();
                if fix_cli.includes(FixRequest::Mend) {
                    fix_kinds.insert(FixKind::ShortenImport);
                    fix_kinds.insert(FixKind::PreferModuleImport);
                    fix_kinds.insert(FixKind::InlinePathQualifiedType);
                    fix_kinds.insert(FixKind::UnusedPub);
                    fix_kinds.insert(FixKind::NarrowToPubCrate);
                    fix_kinds.insert(FixKind::FieldVisibility);
                    fix_kinds.insert(FixKind::ImportsAtTop);
                }
                if fix_cli.includes(FixRequest::PubUse) {
                    fix_kinds.insert(FixKind::PubUse);
                }
                Self { fix_kinds }
            },
        }
    }
}

impl FixSelection {
    fn all_fix_kinds() -> Self {
        let mut fix_kinds = BTreeSet::new();
        fix_kinds.insert(FixKind::ShortenImport);
        fix_kinds.insert(FixKind::PreferModuleImport);
        fix_kinds.insert(FixKind::InlinePathQualifiedType);
        fix_kinds.insert(FixKind::UnusedPub);
        fix_kinds.insert(FixKind::NarrowToPubCrate);
        fix_kinds.insert(FixKind::FieldVisibility);
        fix_kinds.insert(FixKind::ImportsAtTop);
        fix_kinds.insert(FixKind::PubUse);
        Self { fix_kinds }
    }

    pub(crate) fn contains(&self, kind: FixKind) -> bool { self.fix_kinds.contains(&kind) }
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
    use super::OperationIntent;
    use super::OperationMode;
    use crate::config::cli::FixCli;
    use crate::config::cli::FixExecution;
    use crate::config::cli::FixRequest;

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
        assert!(operation_mode.fixes.contains(FixKind::ShortenImport));
        assert!(operation_mode.fixes.contains(FixKind::PreferModuleImport));
        assert!(
            operation_mode
                .fixes
                .contains(FixKind::InlinePathQualifiedType)
        );
        assert!(operation_mode.fixes.contains(FixKind::NarrowToPubCrate));
        assert!(operation_mode.fixes.contains(FixKind::UnusedPub));
        assert!(operation_mode.fixes.contains(FixKind::PubUse));
    }

    #[test]
    fn operation_mode_allows_previewing_multiple_fix_kinds() {
        let cli = FixCli {
            execution:       FixExecution::PreviewRequested,
            requested_fixes: BTreeSet::from([FixRequest::Mend, FixRequest::PubUse]),
        };
        let operation_mode = OperationMode::from(&cli);
        assert_eq!(operation_mode.intent, OperationIntent::DryRun);
        assert!(operation_mode.fixes.contains(FixKind::ShortenImport));
        assert!(operation_mode.fixes.contains(FixKind::NarrowToPubCrate));
        assert!(operation_mode.fixes.contains(FixKind::UnusedPub));
        assert!(operation_mode.fixes.contains(FixKind::PubUse));
    }

    #[test]
    fn operation_mode_allows_applying_multiple_fix_kinds() {
        let cli = FixCli {
            execution:       FixExecution::ApplyRequested,
            requested_fixes: BTreeSet::from([FixRequest::Mend, FixRequest::PubUse]),
        };
        let operation_mode = OperationMode::from(&cli);
        assert_eq!(operation_mode.intent, OperationIntent::Apply);
        assert!(operation_mode.fixes.contains(FixKind::ShortenImport));
        assert!(operation_mode.fixes.contains(FixKind::NarrowToPubCrate));
        assert!(operation_mode.fixes.contains(FixKind::UnusedPub));
        assert!(operation_mode.fixes.contains(FixKind::PubUse));
    }
}
