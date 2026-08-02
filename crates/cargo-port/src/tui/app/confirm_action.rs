use crate::project::AbsolutePath;
use crate::tui::running_targets::RunningTargetTerminationCapability;

/// An action waiting for user confirmation (y/n).
pub(crate) enum ConfirmAction {
    /// `cargo clean` on the project at this absolute path.
    Clean(AbsolutePath),
    /// `cargo clean` fanned out across every checkout in a worktree
    /// group (primary + every linked worktree). Triggered by the
    /// Clean shortcut when a `VisibleRow::Root` over a
    /// `WorktreeGroup` is selected.
    CleanGroup {
        primary: AbsolutePath,
        linked:  Vec<AbsolutePath>,
    },
    /// Send `SIGTERM` to the running instance named by `label`. The opaque
    /// capability revalidates the strong process identity before signaling;
    /// `pid` and `create_time` are confirmation display data only.
    KillTarget {
        label:                  String,
        pid:                    u32,
        create_time:            u64,
        termination_capability: RunningTargetTerminationCapability,
    },
    /// Pause lint operations for one workspace or standalone package. A
    /// workspace member always resolves to this owning lint root.
    PauseLintProject(AbsolutePath),
    /// Pause all lint operations: kill in-flight runs and hold new runs until
    /// the user toggles back. Resuming needs no confirmation.
    PauseAllLints,
}
