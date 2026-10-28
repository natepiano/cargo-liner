//! Claim requests and coordination-run validation.

use std::sync::Arc;

use super::recovery_command::ClaimRecoveryCommand;
use crate::answer::OverlapAuthorizationRequest;
use crate::config::DefaultAnswer;
use crate::coordination_identity;
use crate::coordination_identity::CoordinationIdentityProvenance;
use crate::coordination_identity::CoordinationIdentityRejection;
use crate::coordination_identity::CoordinationIdentityValidationContext;
use crate::coordination_identity::CoordinationIdentityValidationError;
use crate::coordination_identity::PresentedCoordinationRun;
use crate::coordination_identity::RecoveryCommandLine;
use crate::git::RepositorySnapshot;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger::ClaimSource;
use crate::ledger::EditAuthorization;
use crate::ledger::IntegrationTarget;
use crate::ledger::ProtectedPhaseStartHead;
use crate::ledger::ReservationPurpose;
use crate::ledger::ResolvedEditAuthorization;
use crate::ledger::WorktreeContext;
use crate::reservation::RetainedReservationSet;
use crate::scope::DeclaredReservationScopeSet;
use crate::scope::PathCase;

/// A parsed claim whose provenance carries domain types rather than CLI options.
pub(crate) struct ClaimRequest {
    /// Open a reservation or continue explicitly named work.
    pub(crate) reservation_selection:      ClaimReservationSelection,
    /// An explicit local branch to integrate this reservation into.
    pub(crate) target:                     Option<String>,
    /// Lexically valid scopes before repository-case antichain reduction.
    pub(crate) declared_scopes:            DeclaredReservationScopeSet,
    /// The acquisition origin retained in the audit trail.
    pub(crate) source:                     ClaimSource,
    /// The caller-supplied explanation state.
    pub(crate) purpose:                    ReservationPurpose,
    /// How the claim selects the coordination run that will own it.
    pub(crate) coordination_run_selection: ClaimCoordinationRunSelection,
    /// The phase-start commit selection.
    pub(crate) phase_start:                PhaseStartSelection,
    /// The semantic overlap answer and its reason.
    pub(crate) overlap_authorization:      OverlapAuthorizationRequest,
    /// The parsed caller command retained for an incomplete answer retry.
    pub(crate) answer_recovery:            ClaimRecoveryCommand,
}

/// The reservation receiving a claim's scopes and overlap answer.
#[derive(Clone, Copy)]
pub(crate) enum ClaimReservationSelection {
    /// Open a separate reservation, preserving ordinary claim behavior.
    NewReservation,
    /// Continue one active reservation of the acting run and worktree.
    Existing(ReservationId),
}

/// How a claim chooses the coordination run that will own its reservation.
pub(crate) enum ClaimCoordinationRunSelection {
    /// Use the run identity supplied through `--run`.
    Specified(PresentedCoordinationRun),
    /// Continue the active run from process or worktree context, or start one.
    ContinueOrStart,
}

/// The replay validation required by the source of a resolved claim run.
#[derive(Clone, Copy)]
pub(super) enum ClaimRunValidation {
    /// An explicit argument or process environment identifies a marker-independent caller.
    IndependentWithPresentedIdentity(PresentedCoordinationRun),
    /// A marker-independent caller presented no identity, so only its actor run is issued.
    IndependentWithoutPresentedIdentity {
        /// The concrete run stamped on the new reservation and transaction.
        actor_run_id: CoordinationRunId,
    },
    /// A session mapping or marker must still agree with locked reservation state.
    ResolvedIdentityRequired(ResolvedEditAuthorization),
}

/// How a claim chooses its protected phase-start commit.
pub(crate) enum PhaseStartSelection {
    /// Use the current worktree HEAD observed during acquisition.
    CurrentHead,
    /// Retain the full object id supplied by the caller.
    Protected(ProtectedPhaseStartHead),
}

impl PhaseStartSelection {
    /// The protected phase-start commit this selection names, given the observed HEAD.
    pub(super) fn protected_head(self, current_head: &GitObjectId) -> ProtectedPhaseStartHead {
        match self {
            Self::CurrentHead => ProtectedPhaseStartHead::from(current_head.clone()),
            Self::Protected(protected_phase_start_head) => protected_phase_start_head,
        }
    }
}

/// The overlap policy selected for a claim, retaining its request until validation.
///
/// Once an answer is recorded, the caller request is consumed and only its source remains.
#[derive(Clone, Copy)]
pub(crate) enum ClaimAnswerPolicy<CallerRequest = ()> {
    /// Honor the caller's request. An answer-only result comes from explicit `--defer`.
    CallerRequest(CallerRequest),
    /// An unanswered claim uses the repository default to record its overlap answers.
    RepositoryDefault(DefaultAnswer),
}

impl ClaimAnswerPolicy<OverlapAuthorizationRequest> {
    /// Resolve the configuration boundary without pairing an explicit answer with a default.
    pub(super) fn resolve(
        request: OverlapAuthorizationRequest,
        configured_default: Option<DefaultAnswer>,
    ) -> Self {
        match request {
            OverlapAuthorizationRequest::Absent => configured_default.map_or(
                Self::CallerRequest(OverlapAuthorizationRequest::Absent),
                Self::RepositoryDefault,
            ),
            request => Self::CallerRequest(request),
        }
    }

    /// Retain the selected policy without copying the request consumed by validation.
    pub(super) const fn source(&self) -> ClaimAnswerPolicy {
        match self {
            Self::CallerRequest(_) => ClaimAnswerPolicy::CallerRequest(()),
            Self::RepositoryDefault(answer) => ClaimAnswerPolicy::RepositoryDefault(*answer),
        }
    }
}

#[derive(Clone)]
pub(super) struct ClaimValidationContext {
    pub(super) repository:             Arc<RepositorySnapshot>,
    pub(super) run_validation:         ClaimRunValidation,
    pub(super) worktree_context:       WorktreeContext,
    pub(super) recovery_command_line:  RecoveryCommandLine,
    pub(super) worktree_id:            WorktreeId,
    pub(super) path_case:              PathCase,
    pub(super) answer_policy:          ClaimAnswerPolicy<OverlapAuthorizationRequest>,
    pub(super) maximum_reservations:   u32,
    pub(super) maximum_ordering_edges: u32,
    pub(super) repository_trunk:       IntegrationTarget,
}

pub(super) enum ClaimRunValidationError {
    CoordinationIdentity(CoordinationIdentityRejection),
    InvalidCanonicalWorktreeRoot,
}

impl ClaimCoordinationRunSelection {
    pub(super) const fn resolve(
        self,
        resolved_edit_authorization: ResolvedEditAuthorization,
    ) -> ClaimRunValidation {
        match self {
            Self::Specified(acting_run) => {
                ClaimRunValidation::IndependentWithPresentedIdentity(acting_run)
            },
            Self::ContinueOrStart => match resolved_edit_authorization.edit_authorization() {
                EditAuthorization::Session { .. } | EditAuthorization::Marker { .. } => {
                    ClaimRunValidation::ResolvedIdentityRequired(resolved_edit_authorization)
                },
                // `Environment` and `Unidentified` split on exactly the question the
                // constructor answers, so it decides them rather than a second variant match
                // that could drift from it.
                authorization => {
                    match PresentedCoordinationRun::from_edit_authorization(authorization) {
                        Some(acting_run) => {
                            ClaimRunValidation::IndependentWithPresentedIdentity(acting_run)
                        },
                        None => ClaimRunValidation::IndependentWithoutPresentedIdentity {
                            actor_run_id: resolved_edit_authorization.coordination_run_id,
                        },
                    }
                },
            },
        }
    }
}

impl ClaimRunValidation {
    pub(super) const fn requires_live_head_revalidation(self) -> bool {
        matches!(self, Self::ResolvedIdentityRequired(_))
    }

    pub(super) const fn actor_run_id(self) -> CoordinationRunId {
        match self {
            Self::IndependentWithPresentedIdentity(acting_run) => acting_run.coordination_run_id(),
            Self::IndependentWithoutPresentedIdentity { actor_run_id } => actor_run_id,
            Self::ResolvedIdentityRequired(resolved_edit_authorization) => {
                resolved_edit_authorization.coordination_run_id
            },
        }
    }

    /// Record whether the caller presented the coordination identity this claim carries.
    ///
    /// Only a caller that presented nothing had its run created by this process.
    pub(super) const fn coordination_identity_provenance(self) -> CoordinationIdentityProvenance {
        match self {
            Self::IndependentWithPresentedIdentity(_) | Self::ResolvedIdentityRequired(_) => {
                CoordinationIdentityProvenance::Presented
            },
            Self::IndependentWithoutPresentedIdentity { .. } => {
                CoordinationIdentityProvenance::NotPresented
            },
        }
    }

    /// Refuse a run that cannot take work in this worktree under locked reservation state.
    ///
    /// The occupancy refusal is between two *presented* coordination runs, and only a caller
    /// that presented one can reach it. A caller that presented nothing had its run created by
    /// this process to stand in for an unknown caller, which is not a coordination run and has
    /// no second run to be: refusing it would refuse the engine's own markerless post-commit
    /// work. A caller whose identity resolved from a session mapping or a worktree marker is
    /// answered by [`coordination_identity::validate_coordination_identity`] instead, which
    /// owns the staleness questions those sources raise.
    ///
    /// The matching question about the *incumbent* is asked by
    /// [`coordination_identity::validate_worktree_occupancy`].
    pub(super) fn validate(
        self,
        reservations: &RetainedReservationSet,
        worktree_context: &WorktreeContext,
        worktree_id: WorktreeId,
        recovery_command_line: &RecoveryCommandLine,
    ) -> Result<(), ClaimRunValidationError> {
        match self {
            Self::IndependentWithPresentedIdentity(acting_run) => {
                coordination_identity::validate_worktree_occupancy(
                    reservations,
                    worktree_context,
                    worktree_id,
                    acting_run,
                )
                .map_err(ClaimRunValidationError::from)
            },
            Self::IndependentWithoutPresentedIdentity { .. } => Ok(()),
            Self::ResolvedIdentityRequired(resolved_edit_authorization) => {
                let identity_validation = CoordinationIdentityValidationContext::for_user_command(
                    resolved_edit_authorization,
                    worktree_context,
                    recovery_command_line,
                );
                coordination_identity::validate_coordination_identity(
                    reservations,
                    &identity_validation,
                )
                .map_err(ClaimRunValidationError::from)
            },
        }
    }
}

impl From<CoordinationIdentityValidationError> for ClaimRunValidationError {
    fn from(error: CoordinationIdentityValidationError) -> Self {
        match error {
            CoordinationIdentityValidationError::Rejected(rejection) => {
                Self::CoordinationIdentity(rejection)
            },
            CoordinationIdentityValidationError::InvalidCanonicalWorktreeRoot => {
                Self::InvalidCanonicalWorktreeRoot
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ClaimCoordinationRunSelection;
    use super::ClaimRunValidation;
    use crate::ids::CoordinationRunId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeId;
    use crate::ledger::EditAuthorization;
    use crate::ledger::ResolvedEditAuthorization;

    #[test]
    fn continue_or_start_uses_the_session_authorization_resolved_with_the_actor() {
        let coordination_run_id = CoordinationRunId::new();
        let reservation_id = ReservationId::new();
        let worktree_id = WorktreeId::new();
        let resolved_edit_authorization = ResolvedEditAuthorization::for_edit_authorization(
            worktree_id,
            EditAuthorization::Session {
                coordination_run_id,
                reservation_id,
                worktree_id,
            },
        );

        let run_validation =
            ClaimCoordinationRunSelection::ContinueOrStart.resolve(resolved_edit_authorization);

        assert!(matches!(
            run_validation,
            ClaimRunValidation::ResolvedIdentityRequired(actual)
                if actual == resolved_edit_authorization
        ));
    }
}
