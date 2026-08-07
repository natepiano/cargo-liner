use crate::build_monitor::SelectedBuildTerminationAuthorization;
use crate::project::AbsolutePath;
use crate::tui::panes::SelectedBuildTerminationConfirmationDisplay;
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
    /// Terminate the selected root Cargo invocation. Display data records the
    /// column the user saw; the opaque authorization is the only signal
    /// authority and moves with this action until it is submitted or dropped.
    TerminateSelectedBuild {
        selected_build_termination_confirmation_display:
            SelectedBuildTerminationConfirmationDisplay,
        selected_build_termination_authorization:        Box<SelectedBuildTerminationAuthorization>,
    },
    /// Pause lint operations for one workspace or standalone package. A
    /// workspace member always resolves to this owning lint root.
    PauseLintProject(AbsolutePath),
    /// Pause all lint operations: kill in-flight runs and hold new runs until
    /// the user toggles back. Resuming needs no confirmation.
    PauseAllLints,
}

/// The [`crate::tui::app::App`]-owned confirmation lifecycle.
///
/// [`ConfirmationModalState::Open`] keeps its non-cloneable [`ConfirmAction`]
/// together with the [`ConfirmationReadiness`] that determines whether
/// accepting it may execute that action.
pub(crate) enum ConfirmationModalState {
    /// No confirmation is currently consuming input.
    Closed,
    /// One action is visible and owns the matching readiness state.
    Open {
        action:    ConfirmAction,
        readiness: ConfirmationReadiness,
    },
}

impl ConfirmationModalState {
    pub(crate) const fn is_open(&self) -> bool { matches!(self, Self::Open { .. }) }
}

/// Whether the action in an open [`ConfirmationModalState`] may execute.
pub(crate) enum ConfirmationReadiness {
    /// The action can execute when the user accepts it.
    Ready,
    /// A clean action must wait for the metadata refresh for this primary
    /// workspace path before it may execute.
    VerifyingCleanMetadata(AbsolutePath),
}

impl ConfirmationReadiness {
    pub(crate) const fn is_verifying(&self) -> bool {
        matches!(self, Self::VerifyingCleanMetadata(_))
    }
}

/// Result of accepting the current [`ConfirmationModalState`] with `y`.
pub(crate) enum ConfirmationAcceptance {
    /// No confirmation was open, so normal input dispatch continues.
    Closed,
    /// The open action remains blocked on clean metadata verification.
    Verifying,
    /// The ready action was removed from its modal and may execute.
    Ready(ConfirmAction),
}
