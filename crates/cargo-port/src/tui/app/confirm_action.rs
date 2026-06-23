use crate::project::AbsolutePath;

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
    /// Send `SIGTERM` to the running instance named by `label`. The PID
    /// is verified against `create_time` (the process's start time in
    /// epoch seconds) immediately before the signal, so a PID the OS
    /// reassigned while the dialog was open is never killed.
    KillTarget {
        label:       String,
        pid:         u32,
        create_time: u64,
    },
    /// Pause all lint operations: kill in-flight runs and hold new runs until
    /// the user toggles back. Resuming needs no confirmation.
    PauseLint,
}
