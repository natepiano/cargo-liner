//! The coordination-run, reservation, and worktree identity behind one drift invocation.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::selection::DriftReservationSelection;
use super::selection::DriftSelectionError;
use crate::coordination_identity;
use crate::coordination_identity::CoordinationIdentityRejection;
use crate::coordination_identity::CoordinationIdentityValidationContext;
use crate::coordination_identity::CoordinationIdentityValidationError;
use crate::coordination_identity::PresentedCoordinationRun;
use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger::EditAuthorization;
use crate::ledger::ResolvedEditAuthorization;
use crate::ledger::WorktreeContext;
use crate::reservation::AuthorizedEditingIdentity;
use crate::reservation::RetainedReservationSet;

/// Which validation one drift invocation's coordination identity must pass.
///
/// This mirrors `verb::claim`'s `ClaimRunValidation` for the drift path. The same three
/// identity sources raise the same three questions, so a run the occupancy rule refuses a
/// claim is refused the same scopes after a commit. Drift asked only the resolved-identity
/// question until this existed, which left a presented second run free to widen inside a
/// worktree another presented run occupies.
#[derive(Clone, Copy)]
pub(super) enum DriftRunValidation {
    /// `CARGO_BERTH_RUN` named the run, so the same-worktree occupancy rule answers it.
    IndependentWithPresentedIdentity(PresentedCoordinationRun),
    /// Nothing identified the caller, so this process issued the run it acts under. That is
    /// not a coordination run and has no second run to be: refusing it would refuse the
    /// engine's own markerless post-commit work.
    IndependentWithoutPresentedIdentity,
    /// A session mapping or a worktree marker supplied the run, and
    /// [`crate::coordination_identity::validate_coordination_identity`] owns the staleness
    /// questions those sources raise.
    ResolvedIdentityRequired,
}

impl DriftRunValidation {
    pub(super) const fn resolve(resolved_edit_authorization: ResolvedEditAuthorization) -> Self {
        match resolved_edit_authorization.edit_authorization() {
            EditAuthorization::Session { .. } | EditAuthorization::Marker { .. } => {
                Self::ResolvedIdentityRequired
            },
            // `Environment` and `Unidentified` split on exactly the question the constructor
            // answers, so it decides them rather than a second variant match beside it.
            authorization => match PresentedCoordinationRun::from_edit_authorization(authorization)
            {
                Some(acting_run) => Self::IndependentWithPresentedIdentity(acting_run),
                None => Self::IndependentWithoutPresentedIdentity,
            },
        }
    }

    /// Answer whether this invocation may take or widen reservation scopes in this worktree.
    ///
    /// Drift asks this from inside the ledger transaction, at the acquisition step, for two
    /// reasons that are the same reason. Asking it earlier aborts the invocation before
    /// `observation::observe` runs, so a second presented run's commits leave no incursion
    /// record against any foreign holder in the repository; asking it only outside the lock
    /// lets a run that raced the incumbent's claim pass the question and then acquire under
    /// it. One question, one place, and the answer withholds the acquisition rather than the
    /// observation.
    pub(super) fn authorize_scope_acquisition(
        self,
        reservations: &RetainedReservationSet,
        worktree_context: &WorktreeContext,
        identity_validation: &CoordinationIdentityValidationContext,
    ) -> Result<DriftScopeAcquisition, CoordinationIdentityValidationError> {
        match self {
            Self::IndependentWithPresentedIdentity(acting_run) => {
                match coordination_identity::validate_worktree_occupancy(
                    reservations,
                    worktree_context,
                    identity_validation
                        .resolved_edit_authorization()
                        .worktree_id,
                    acting_run,
                ) {
                    Ok(()) => Ok(DriftScopeAcquisition::Permitted),
                    Err(CoordinationIdentityValidationError::Rejected(rejection)) => {
                        Ok(DriftScopeAcquisition::RefusedToSecondRun { rejection })
                    },
                    Err(
                        error @ CoordinationIdentityValidationError::InvalidCanonicalWorktreeRoot,
                    ) => Err(error),
                }
            },
            Self::IndependentWithoutPresentedIdentity => Ok(DriftScopeAcquisition::Permitted),
            Self::ResolvedIdentityRequired => {
                coordination_identity::validate_coordination_identity(
                    reservations,
                    identity_validation,
                )
                .map(|()| DriftScopeAcquisition::Permitted)
            },
        }
    }
}

/// Whether this drift invocation may take or widen reservation scopes in this worktree.
///
/// This travels on [`super::report::DriftReport`] rather than replacing it: a refusal that
/// displaced the report would downgrade an incursion the caller must act on into a rejection
/// that reads as "nothing happened". One envelope states both, so the refusal is carried as a
/// value the report owns.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "drift_scope_acquisition")]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum DriftScopeAcquisition {
    /// No other presented coordination run occupies this worktree, so widening and the
    /// post-write first touch may proceed.
    Permitted,
    /// A second presented coordination run may not take scopes in a worktree another
    /// presented run already occupies, and a separate checkout is the remedy. Observation and
    /// classification still ran, so every incursion this commit made into a foreign holder is
    /// recorded; only the acquisition is withheld.
    RefusedToSecondRun {
        /// The refusal, named for the caller in the words the identity rule chose.
        rejection: CoordinationIdentityRejection,
    },
}

impl DriftScopeAcquisition {
    /// Project this answer onto the one thing it withholds from classification.
    pub(super) const fn widening_authorization(&self) -> DriftWideningAuthorization {
        match self {
            Self::Permitted => DriftWideningAuthorization::Permitted,
            Self::RefusedToSecondRun { .. } => DriftWideningAuthorization::WithheldFromRefusedRun,
        }
    }
}

/// Whether this invocation may still add observed paths to its widening target.
///
/// Withholding the widening is the whole of what a refusal does to classification. Answering
/// the acquisition question by rewriting `ResolvedDriftSubjects::widening` did more than that:
/// [`super::classification`] reads the same field as its blocker filter, so the rewrite also
/// changed which holder a subject could be told it entered — an effect no refusal intends.
/// The acquisition answer travels on its own instead, and the widening selection keeps
/// answering only the question it names.
#[derive(Clone, Copy)]
pub(super) enum DriftWideningAuthorization {
    /// Unclaimed paths that carry work may still be attributed to the widening target.
    Permitted,
    /// A refused second run acquires nothing here, so no observed path is widened into it.
    WithheldFromRefusedRun,
}

#[derive(Clone, Copy)]
pub(super) enum DriftActingIdentity {
    Session {
        run:         CoordinationRunId,
        reservation: ReservationId,
        worktree:    WorktreeId,
    },
    Run {
        run:      CoordinationRunId,
        worktree: WorktreeId,
    },
    Unidentified {
        worktree: WorktreeId,
    },
}

impl DriftActingIdentity {
    pub(super) fn resolve(
        resolved_edit_authorization: ResolvedEditAuthorization,
        reservations: &RetainedReservationSet,
    ) -> Self {
        match reservations
            .resolve_editing_identity(resolved_edit_authorization.edit_authorization())
        {
            AuthorizedEditingIdentity::SessionReservation {
                coordination_run_id: run,
                reservation_id: reservation,
                worktree_id: worktree,
            } => Self::Session {
                run,
                reservation,
                worktree,
            },
            AuthorizedEditingIdentity::Run {
                coordination_run_id: run,
                worktree_id: worktree,
            } => Self::Run { run, worktree },
            AuthorizedEditingIdentity::Unidentified => Self::Unidentified {
                worktree: resolved_edit_authorization.worktree_id,
            },
        }
    }

    pub(super) const fn worktree(self) -> WorktreeId {
        match self {
            Self::Session { worktree, .. }
            | Self::Run { worktree, .. }
            | Self::Unidentified { worktree } => worktree,
        }
    }

    pub(super) const fn acting_run(self) -> DriftActingRun {
        match self {
            Self::Session { run, .. } | Self::Run { run, .. } => DriftActingRun::Identified(run),
            Self::Unidentified { .. } => DriftActingRun::Unidentified,
        }
    }

    pub(super) const fn session_reservation(self) -> DriftSessionReservation {
        match self {
            Self::Session { reservation, .. } => DriftSessionReservation::Mapped(reservation),
            Self::Run { .. } | Self::Unidentified { .. } => DriftSessionReservation::Unavailable,
        }
    }

    pub(super) fn run_for_mutation(
        self,
        reservation_selection: DriftReservationSelection,
    ) -> Result<DriftMutationActorRun, DriftSelectionError> {
        match self.acting_run() {
            DriftActingRun::Identified(run) => Ok(DriftMutationActorRun::Identified(run)),
            DriftActingRun::Unidentified
                if matches!(
                    reservation_selection,
                    DriftReservationSelection::EveryActiveForPostCommit { .. }
                ) =>
            {
                Ok(DriftMutationActorRun::PostCommitInvocation(
                    CoordinationRunId::new(),
                ))
            },
            DriftActingRun::Unidentified => Err(DriftSelectionError::UnidentifiedActingRun),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum DriftActingRun {
    Identified(CoordinationRunId),
    Unidentified,
}

pub(super) enum DriftSessionReservation {
    Mapped(ReservationId),
    Unavailable,
}

/// The run identity recorded on drift mutations from this invocation.
pub(super) enum DriftMutationActorRun {
    /// The process or validated worktree marker identified the invoking run.
    Identified(CoordinationRunId),
    /// An unidentified post-commit invocation received a transaction-only run identity.
    PostCommitInvocation(CoordinationRunId),
}

impl DriftMutationActorRun {
    pub(super) const fn into_coordination_run_id(self) -> CoordinationRunId {
        match self {
            Self::Identified(coordination_run_id)
            | Self::PostCommitInvocation(coordination_run_id) => coordination_run_id,
        }
    }
}
