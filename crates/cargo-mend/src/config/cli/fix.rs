use std::collections::BTreeSet;

use clap::Args;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FixCli {
    pub(crate) execution:       FixExecution,
    pub(crate) requested_fixes: BTreeSet<FixRequest>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum FixExecution {
    #[default]
    ReadOnly,
    ApplyRequested,
    ApplyAll,
    PreviewRequested,
    PreviewAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum FixRequest {
    Compiler,
    Mend,
    PubUse,
}

impl FixCli {
    pub fn includes(&self, requested_fix: FixRequest) -> bool {
        self.requested_fixes.contains(&requested_fix)
    }

    pub fn runs_compiler_fix(&self) -> bool {
        match self.execution {
            FixExecution::ApplyAll => true,
            FixExecution::ApplyRequested => self.includes(FixRequest::Compiler),
            FixExecution::ReadOnly | FixExecution::PreviewAll | FixExecution::PreviewRequested => {
                false
            },
        }
    }
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
#[command(next_help_heading = "Mend Actions")]
pub(super) struct RawFixCli {
    #[command(flatten)]
    auto_fix: RawAutoFixCli,

    #[command(flatten)]
    execution: RawFixExecutionCli,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
struct RawAutoFixCli {
    /// Auto-fix mend import and visibility findings
    #[arg(long = "fix")]
    mend: bool,

    /// Auto-fix stale `pub use` re-exports
    #[arg(long = "fix-pub-use")]
    pub_use: bool,

    /// Run `cargo fix` for compiler-fixable warnings
    #[arg(long = "fix-compiler")]
    compiler: bool,
}

#[derive(Args, Debug, Clone, Default, PartialEq, Eq)]
struct RawFixExecutionCli {
    /// Apply all fixes (--fix + --fix-pub-use + --fix-compiler)
    #[arg(long)]
    fix_all: bool,

    /// Preview fixes without applying them
    #[arg(long)]
    dry_run: bool,
}

impl From<RawFixCli> for FixCli {
    fn from(raw: RawFixCli) -> Self {
        let mut requested_fixes = BTreeSet::new();
        if raw.auto_fix.mend {
            requested_fixes.insert(FixRequest::Mend);
        }
        if raw.auto_fix.pub_use {
            requested_fixes.insert(FixRequest::PubUse);
        }
        if raw.auto_fix.compiler {
            requested_fixes.insert(FixRequest::Compiler);
        }

        let execution = match (
            raw.execution.fix_all,
            raw.execution.dry_run,
            requested_fixes.is_empty(),
        ) {
            (true, true, _) | (false, true, true) => FixExecution::PreviewAll,
            (true, false, _) => FixExecution::ApplyAll,
            (false, true, false) => FixExecution::PreviewRequested,
            (false, false, true) => FixExecution::ReadOnly,
            (false, false, false) => FixExecution::ApplyRequested,
        };

        Self {
            execution,
            requested_fixes,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::FixCli;
    use super::FixExecution;
    use super::FixRequest;

    #[test]
    fn runs_compiler_fix_false_for_read_only() {
        let fix = FixCli::default();
        assert!(!fix.runs_compiler_fix());
    }

    #[test]
    fn runs_compiler_fix_false_for_preview_all() {
        let fix = FixCli {
            execution: FixExecution::PreviewAll,
            ..FixCli::default()
        };
        assert!(!fix.runs_compiler_fix());
    }

    #[test]
    fn runs_compiler_fix_false_for_preview_requested_with_compiler() {
        let fix = FixCli {
            execution:       FixExecution::PreviewRequested,
            requested_fixes: BTreeSet::from([FixRequest::Compiler]),
        };
        assert!(!fix.runs_compiler_fix());
    }

    #[test]
    fn runs_compiler_fix_true_for_apply_all() {
        let fix = FixCli {
            execution: FixExecution::ApplyAll,
            ..FixCli::default()
        };
        assert!(fix.runs_compiler_fix());
    }

    #[test]
    fn runs_compiler_fix_true_for_apply_requested_with_compiler() {
        let fix = FixCli {
            execution:       FixExecution::ApplyRequested,
            requested_fixes: BTreeSet::from([FixRequest::Compiler]),
        };
        assert!(fix.runs_compiler_fix());
    }

    #[test]
    fn runs_compiler_fix_false_for_apply_requested_without_compiler() {
        let fix = FixCli {
            execution:       FixExecution::ApplyRequested,
            requested_fixes: BTreeSet::from([FixRequest::Mend]),
        };
        assert!(!fix.runs_compiler_fix());
    }
}
