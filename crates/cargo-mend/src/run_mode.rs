use std::collections::BTreeSet;

use anyhow::Result;

use super::cli::FixCli;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FixKind {
    ShortenImport,
    FixPubUse,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FixSelection {
    kinds: BTreeSet<FixKind>,
}

impl FixSelection {
    pub fn from_cli(fix_cli: &FixCli) -> Self {
        let mut kinds = BTreeSet::new();
        if fix_cli.fix {
            kinds.insert(FixKind::ShortenImport);
        }
        if fix_cli.fix_pub_use {
            kinds.insert(FixKind::FixPubUse);
        }
        Self { kinds }
    }

    pub fn contains(&self, kind: FixKind) -> bool { self.kinds.contains(&kind) }

    pub fn is_empty(&self) -> bool { self.kinds.is_empty() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationIntent {
    ReadOnly,
    DryRun,
    Apply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationMode {
    pub intent: OperationIntent,
    pub fixes:  FixSelection,
}

impl OperationMode {
    pub fn from_cli(fix_cli: &FixCli) -> Result<Self> {
        let fixes = FixSelection::from_cli(fix_cli);
        if fix_cli.dry_run {
            if fixes.is_empty() {
                anyhow::bail!("`--dry-run` requires `--fix` or `--fix-pub-use`");
            }
            return Ok(Self {
                intent: OperationIntent::DryRun,
                fixes,
            });
        }

        if fixes.is_empty() {
            Ok(Self {
                intent: OperationIntent::ReadOnly,
                fixes,
            })
        } else {
            Ok(Self {
                intent: OperationIntent::Apply,
                fixes,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FixKind;
    use super::OperationIntent;
    use super::OperationMode;
    use crate::cli::FixCli;

    #[test]
    fn operation_mode_is_read_only_without_fix_flags() {
        let cli = FixCli {
            fix:         false,
            fix_pub_use: false,
            dry_run:     false,
        };
        let Ok(mode) = OperationMode::from_cli(&cli) else {
            unreachable!("read-only mode should parse");
        };
        assert_eq!(mode.intent, OperationIntent::ReadOnly);
    }

    #[test]
    fn operation_mode_rejects_dry_run_without_fix_flags() {
        let cli = FixCli {
            fix:         false,
            fix_pub_use: false,
            dry_run:     true,
        };
        let mode = OperationMode::from_cli(&cli);
        let Err(err) = mode else {
            unreachable!("dry run without fix flags should be rejected");
        };
        assert!(
            err.to_string()
                .contains("`--dry-run` requires `--fix` or `--fix-pub-use`")
        );
    }

    #[test]
    fn operation_mode_allows_previewing_multiple_fix_kinds() {
        let cli = FixCli {
            fix:         true,
            fix_pub_use: true,
            dry_run:     true,
        };
        let Ok(mode) = OperationMode::from_cli(&cli) else {
            unreachable!("preview mode should parse");
        };
        assert_eq!(mode.intent, OperationIntent::DryRun);
        assert!(mode.fixes.contains(FixKind::ShortenImport));
        assert!(mode.fixes.contains(FixKind::FixPubUse));
    }

    #[test]
    fn operation_mode_allows_applying_multiple_fix_kinds() {
        let cli = FixCli {
            fix:         true,
            fix_pub_use: true,
            dry_run:     false,
        };
        let Ok(mode) = OperationMode::from_cli(&cli) else {
            unreachable!("apply mode should parse");
        };
        assert_eq!(mode.intent, OperationIntent::Apply);
        assert!(mode.fixes.contains(FixKind::ShortenImport));
        assert!(mode.fixes.contains(FixKind::FixPubUse));
    }
}
