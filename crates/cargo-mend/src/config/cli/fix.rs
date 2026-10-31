use std::collections::BTreeSet;

use clap::Args;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FixCli {
    pub execution:       FixExecution,
    pub requested_fixes: BTreeSet<FixRequest>,
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

/// Whether a particular fix request was explicitly selected at the CLI boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::config) enum FixRequestSelection {
    Requested,
    NotRequested,
}

/// Whether this run writes fixes to disk rather than reporting or previewing them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixApplication {
    Apply,
    DoNotApply,
}

/// Whether to run compiler fixes; applying all fixes always includes them, while previews never do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompilerFixExecution {
    Run,
    Skip,
}

impl FixCli {
    pub(in crate::config) fn includes(&self, requested_fix: FixRequest) -> FixRequestSelection {
        if self.requested_fixes.contains(&requested_fix) {
            FixRequestSelection::Requested
        } else {
            FixRequestSelection::NotRequested
        }
    }

    /// Whether this run writes fixes to disk, as opposed to reporting or
    /// previewing them.
    pub(crate) const fn applies_fixes(&self) -> FixApplication {
        match self.execution {
            FixExecution::ApplyRequested | FixExecution::ApplyAll => FixApplication::Apply,
            FixExecution::ReadOnly | FixExecution::PreviewAll | FixExecution::PreviewRequested => {
                FixApplication::DoNotApply
            },
        }
    }

    pub(crate) fn runs_compiler_fix(&self) -> CompilerFixExecution {
        match self.execution {
            FixExecution::ApplyAll => CompilerFixExecution::Run,
            FixExecution::ApplyRequested => match self.includes(FixRequest::Compiler) {
                FixRequestSelection::Requested => CompilerFixExecution::Run,
                FixRequestSelection::NotRequested => CompilerFixExecution::Skip,
            },
            FixExecution::ReadOnly | FixExecution::PreviewAll | FixExecution::PreviewRequested => {
                CompilerFixExecution::Skip
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

    use super::CompilerFixExecution;
    use super::FixApplication;
    use super::FixCli;
    use super::FixExecution;
    use super::FixRequest;
    use super::FixRequestSelection;

    #[test]
    fn runs_compiler_fix_false_for_read_only() {
        let fix = FixCli::default();
        assert_eq!(fix.runs_compiler_fix(), CompilerFixExecution::Skip);
    }

    #[test]
    fn runs_compiler_fix_false_for_preview_all() {
        let fix = FixCli {
            execution: FixExecution::PreviewAll,
            ..FixCli::default()
        };
        assert_eq!(fix.runs_compiler_fix(), CompilerFixExecution::Skip);
    }

    #[test]
    fn runs_compiler_fix_false_for_preview_requested_with_compiler() {
        let fix = FixCli {
            execution:       FixExecution::PreviewRequested,
            requested_fixes: BTreeSet::from([FixRequest::Compiler]),
        };
        assert_eq!(fix.runs_compiler_fix(), CompilerFixExecution::Skip);
    }

    #[test]
    fn runs_compiler_fix_true_for_apply_all() {
        let fix = FixCli {
            execution: FixExecution::ApplyAll,
            ..FixCli::default()
        };
        assert_eq!(fix.runs_compiler_fix(), CompilerFixExecution::Run);
    }

    #[test]
    fn runs_compiler_fix_true_for_apply_requested_with_compiler() {
        let fix = FixCli {
            execution:       FixExecution::ApplyRequested,
            requested_fixes: BTreeSet::from([FixRequest::Compiler]),
        };
        assert_eq!(fix.runs_compiler_fix(), CompilerFixExecution::Run);
    }

    #[test]
    fn runs_compiler_fix_false_for_apply_requested_without_compiler() {
        let fix = FixCli {
            execution:       FixExecution::ApplyRequested,
            requested_fixes: BTreeSet::from([FixRequest::Mend]),
        };
        assert_eq!(fix.runs_compiler_fix(), CompilerFixExecution::Skip);
    }
    #[test]
    fn named_fix_policies_cover_every_execution_and_request_set() {
        let requests = [FixRequest::Compiler, FixRequest::Mend, FixRequest::PubUse];
        for execution in [
            FixExecution::ReadOnly,
            FixExecution::ApplyRequested,
            FixExecution::ApplyAll,
            FixExecution::PreviewRequested,
            FixExecution::PreviewAll,
        ] {
            for mask in 0..8 {
                let requested_fixes = requests
                    .iter()
                    .enumerate()
                    .filter_map(|(bit, request)| (mask & (1 << bit) != 0).then_some(*request))
                    .collect();
                let fix = FixCli {
                    execution,
                    requested_fixes,
                };
                for (bit, request) in requests.iter().enumerate() {
                    let expected = if mask & (1 << bit) != 0 {
                        FixRequestSelection::Requested
                    } else {
                        FixRequestSelection::NotRequested
                    };
                    assert_eq!(fix.includes(*request), expected);
                }
                let (application, compiler) = match execution {
                    FixExecution::ApplyAll => (FixApplication::Apply, CompilerFixExecution::Run),
                    FixExecution::ApplyRequested => {
                        let compiler = if mask & 1 != 0 {
                            CompilerFixExecution::Run
                        } else {
                            CompilerFixExecution::Skip
                        };
                        (FixApplication::Apply, compiler)
                    },
                    FixExecution::ReadOnly
                    | FixExecution::PreviewRequested
                    | FixExecution::PreviewAll => {
                        (FixApplication::DoNotApply, CompilerFixExecution::Skip)
                    },
                };
                assert_eq!(fix.applies_fixes(), application);
                assert_eq!(fix.runs_compiler_fix(), compiler);
            }
        }
    }
}
