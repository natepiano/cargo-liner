use std::collections::BTreeSet;

use super::cli::FixCli;
use super::cli::FixExecution;
use super::cli::FixRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FixKind {
    ShortenImport,
    PreferModuleImport,
    InlinePathQualifiedType,
    NarrowToPubCrate,
    FixPubUse,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FixSelection {
    kinds: BTreeSet<FixKind>,
}

impl FixSelection {
    pub(crate) fn from_cli(fix_cli: &FixCli) -> Self {
        match fix_cli.execution {
            FixExecution::ApplyAll | FixExecution::PreviewAll => return Self::all_fix_kinds(),
            FixExecution::ReadOnly => return Self::default(),
            FixExecution::ApplyRequested | FixExecution::PreviewRequested => {},
        }

        let mut kinds = BTreeSet::new();
        if fix_cli.includes(FixRequest::Mend) {
            kinds.insert(FixKind::ShortenImport);
            kinds.insert(FixKind::PreferModuleImport);
            kinds.insert(FixKind::InlinePathQualifiedType);
            kinds.insert(FixKind::NarrowToPubCrate);
        }
        if fix_cli.includes(FixRequest::PubUse) {
            kinds.insert(FixKind::FixPubUse);
        }
        Self { kinds }
    }

    fn all_fix_kinds() -> Self {
        let mut kinds = BTreeSet::new();
        kinds.insert(FixKind::ShortenImport);
        kinds.insert(FixKind::PreferModuleImport);
        kinds.insert(FixKind::InlinePathQualifiedType);
        kinds.insert(FixKind::NarrowToPubCrate);
        kinds.insert(FixKind::FixPubUse);
        Self { kinds }
    }

    pub(crate) fn contains(&self, kind: FixKind) -> bool { self.kinds.contains(&kind) }
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

impl OperationMode {
    pub(crate) fn from_cli(fix_cli: &FixCli) -> Self {
        let fixes = FixSelection::from_cli(fix_cli);
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
    use crate::cli::FixCli;
    use crate::cli::FixExecution;
    use crate::cli::FixRequest;

    #[test]
    fn operation_mode_is_read_only_without_fix_flags() {
        let cli = FixCli::default();
        let mode = OperationMode::from_cli(&cli);
        assert_eq!(mode.intent, OperationIntent::ReadOnly);
    }

    #[test]
    fn operation_mode_dry_run_alone_implies_all_fix_kinds() {
        let cli = FixCli {
            execution: FixExecution::PreviewAll,
            ..FixCli::default()
        };
        let mode = OperationMode::from_cli(&cli);
        assert_eq!(mode.intent, OperationIntent::DryRun);
        assert!(mode.fixes.contains(FixKind::ShortenImport));
        assert!(mode.fixes.contains(FixKind::PreferModuleImport));
        assert!(mode.fixes.contains(FixKind::InlinePathQualifiedType));
        assert!(mode.fixes.contains(FixKind::NarrowToPubCrate));
        assert!(mode.fixes.contains(FixKind::FixPubUse));
    }

    #[test]
    fn operation_mode_allows_previewing_multiple_fix_kinds() {
        let cli = FixCli {
            execution:       FixExecution::PreviewRequested,
            requested_fixes: BTreeSet::from([FixRequest::Mend, FixRequest::PubUse]),
        };
        let mode = OperationMode::from_cli(&cli);
        assert_eq!(mode.intent, OperationIntent::DryRun);
        assert!(mode.fixes.contains(FixKind::ShortenImport));
        assert!(mode.fixes.contains(FixKind::NarrowToPubCrate));
        assert!(mode.fixes.contains(FixKind::FixPubUse));
    }

    #[test]
    fn operation_mode_allows_applying_multiple_fix_kinds() {
        let cli = FixCli {
            execution:       FixExecution::ApplyRequested,
            requested_fixes: BTreeSet::from([FixRequest::Mend, FixRequest::PubUse]),
        };
        let mode = OperationMode::from_cli(&cli);
        assert_eq!(mode.intent, OperationIntent::Apply);
        assert!(mode.fixes.contains(FixKind::ShortenImport));
        assert!(mode.fixes.contains(FixKind::NarrowToPubCrate));
        assert!(mode.fixes.contains(FixKind::FixPubUse));
    }
}
