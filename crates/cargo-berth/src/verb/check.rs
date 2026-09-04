//! Atomic first-touch edit claims with one blocked-path reconciliation retry.

use std::path::Path;

use super::claim;
use super::claim::CheckReservationSelection;
use super::claim::ClaimError;
use super::claim::FirstTouchClaimExecution;
use super::claim::FirstTouchClaimRequest;
use super::claim::FirstTouchConflictHandling;
use super::claim::FirstTouchConflictOutcome;
use crate::config::Enrollment;
use crate::coordination_identity;
use crate::coordination_identity::CoordinationIdentityRejection;
use crate::coordination_identity::CoordinationIdentityValidationContext;
use crate::coordination_identity::CoordinationIdentityValidationError;
use crate::coordination_identity::RecoveryCommandLine;
use crate::ledger;
use crate::ledger::EditAuthorization;
use crate::ledger::Ledger;
use crate::ledger::LedgerError;
use crate::ledger::ResolvedEditAuthorization;
use crate::ledger::WorktreeContext;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::reconcile;
use crate::reconcile::RecoveredBypassReporting;
use crate::reservation::ReservationConflict;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::scope::DeclaredReservationScopeSet;
use crate::scope::PathCase;
use crate::scope::PathCaseError;
use crate::scope::ReservationScopeSet;

/// A parsed edit check with lexically valid requested paths.
pub(crate) struct CheckRequest {
    /// The exact paths the edit operation proposes to modify.
    pub(crate) declared_scopes:       DeclaredReservationScopeSet,
    /// How locked first-touch validation selects the reservation to continue.
    pub(crate) reservation_selection: CheckReservationSelection,
}

struct CheckDecision {
    scopes:    ReservationScopeSet,
    conflicts: Vec<ReservationConflict>,
}

/// A prerequisite that failed before an overlap decision could be reached.
enum CheckDecisionError {
    /// The ledger snapshot could not be read.
    Ledger(LedgerError),
    /// The repository's path-case rule could not be determined.
    PathCase(PathCaseError),
    /// The retained reservation set could not be replayed.
    ReservationReplay(ReservationReplayError),
    /// The process identity is stale or belongs to another worktree.
    CoordinationIdentity(CoordinationIdentityRejection),
}

impl From<CoordinationIdentityValidationError> for CheckDecisionError {
    fn from(error: CoordinationIdentityValidationError) -> Self {
        match error {
            CoordinationIdentityValidationError::Rejected(rejection) => {
                Self::CoordinationIdentity(rejection)
            },
            CoordinationIdentityValidationError::InvalidCanonicalWorktreeRoot => {
                Self::Ledger(LedgerError::InvalidCanonicalWorktreeRoot)
            },
        }
    }
}

impl CheckDecisionError {
    fn into_output(self) -> OutputEnvelope {
        match self {
            Self::Ledger(error) => OutputEnvelope::ledger_error(CommandVerb::Check, &error),
            Self::PathCase(error) => {
                OutputEnvelope::ledger_unreadable(CommandVerb::Check, &error.to_string())
            },
            Self::ReservationReplay(error) => {
                OutputEnvelope::replay_failure(CommandVerb::Check, &error)
            },
            Self::CoordinationIdentity(rejection) => {
                OutputEnvelope::coordination_identity_rejected(CommandVerb::Check, rejection)
            },
        }
    }
}

/// Evaluate tier-one overlap and permit only after a locked acquisition decision.
pub(crate) fn execute(
    check_request: CheckRequest,
    recovery_command_line: &RecoveryCommandLine,
) -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Check, &error.to_string());
        },
    };
    let first_decision = match decide(
        &invocation_directory,
        check_request.declared_scopes.clone(),
        recovery_command_line,
    ) {
        Ok(Enrollment::Enrolled(check_decision)) => check_decision,
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => {
            return OutputEnvelope::unconfigured(CommandVerb::Check, &expected_configuration_path);
        },
        Err(error) => return error.into_output(),
    };
    if first_decision.conflicts.is_empty() {
        return match acquire_first_touch(
            check_request.declared_scopes.clone(),
            check_request.reservation_selection,
            recovery_command_line,
        ) {
            Ok(Enrollment::Enrolled(FirstTouchClaimExecution::Blocked { conflicts, .. })) => {
                reconcile_and_retry(
                    &invocation_directory,
                    check_request.declared_scopes,
                    first_decision.scopes,
                    conflicts,
                    check_request.reservation_selection,
                    recovery_command_line,
                )
            },
            acquisition => render_acquisition(acquisition),
        };
    }
    reconcile_and_retry(
        &invocation_directory,
        check_request.declared_scopes,
        first_decision.scopes,
        first_decision.conflicts,
        check_request.reservation_selection,
        recovery_command_line,
    )
}

fn reconcile_and_retry(
    invocation_directory: &Path,
    declared_scopes: DeclaredReservationScopeSet,
    fallback_scopes: ReservationScopeSet,
    fallback_conflicts: Vec<ReservationConflict>,
    reservation_selection: CheckReservationSelection,
    recovery_command_line: &RecoveryCommandLine,
) -> OutputEnvelope {
    let reconciliation_report =
        match reconcile::reconcile(invocation_directory, RecoveredBypassReporting::Defer) {
            Ok(Enrollment::Enrolled(reconciliation_report)) => reconciliation_report,
            Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            }) => {
                return OutputEnvelope::unconfigured(
                    CommandVerb::Check,
                    &expected_configuration_path,
                );
            },
            Err(_) => {
                return OutputEnvelope::blocked_check(fallback_scopes, fallback_conflicts);
            },
        };
    let retried_decision = match decide(
        invocation_directory,
        declared_scopes.clone(),
        recovery_command_line,
    ) {
        Ok(Enrollment::Enrolled(check_decision)) => check_decision,
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => {
            return OutputEnvelope::unconfigured(CommandVerb::Check, &expected_configuration_path)
                .with_alerts(reconciliation_report.alerts);
        },
        Err(error) => {
            return error
                .into_output()
                .with_alerts(reconciliation_report.alerts);
        },
    };
    if retried_decision.conflicts.is_empty() {
        render_acquisition(acquire_first_touch(
            declared_scopes,
            reservation_selection,
            recovery_command_line,
        ))
        .with_alerts(reconciliation_report.alerts)
    } else {
        OutputEnvelope::blocked_check(retried_decision.scopes, retried_decision.conflicts)
            .with_alerts(reconciliation_report.alerts)
    }
}

fn acquire_first_touch(
    declared_scopes: DeclaredReservationScopeSet,
    reservation_selection: CheckReservationSelection,
    recovery_command_line: &RecoveryCommandLine,
) -> Result<Enrollment<FirstTouchClaimExecution>, ClaimError> {
    claim::acquire_first_touch_for_check(
        FirstTouchClaimRequest {
            declared_scopes,
            conflict_handling: FirstTouchConflictHandling::RefuseRequest,
        },
        reservation_selection,
        recovery_command_line,
    )
}

fn render_acquisition(
    acquisition: Result<Enrollment<FirstTouchClaimExecution>, ClaimError>,
) -> OutputEnvelope {
    match acquisition {
        Ok(Enrollment::Enrolled(FirstTouchClaimExecution::Acquired {
            acquisition,
            scopes,
            conflicts: FirstTouchConflictOutcome::None,
        })) => OutputEnvelope::clear_check(scopes, acquisition),
        Ok(Enrollment::Enrolled(
            FirstTouchClaimExecution::Acquired {
                scopes,
                conflicts: FirstTouchConflictOutcome::PostWriteIncursion { conflicts, .. },
                ..
            }
            | FirstTouchClaimExecution::Blocked { scopes, conflicts },
        )) => OutputEnvelope::blocked_check(scopes, conflicts),
        Ok(Enrollment::Enrolled(FirstTouchClaimExecution::ReservationLimitReached(maximum))) => {
            OutputEnvelope::invalid_input(
                CommandVerb::Check,
                &format!("repository policy permits at most {maximum} live reservations"),
            )
        },
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => OutputEnvelope::unconfigured(CommandVerb::Check, &expected_configuration_path),
        Err(error) => error.into_output(CommandVerb::Check),
    }
}

fn decide(
    invocation_directory: &Path,
    declared_scopes: DeclaredReservationScopeSet,
    recovery_command_line: &RecoveryCommandLine,
) -> Result<Enrollment<CheckDecision>, CheckDecisionError> {
    let snapshot = match Ledger::read_for_edit_check(invocation_directory)
        .map_err(CheckDecisionError::Ledger)?
    {
        Enrollment::Enrolled(snapshot) => snapshot,
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => {
            return Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            });
        },
    };
    let path_case = PathCase::read(snapshot.worktree_context().common_git_directory())
        .map_err(CheckDecisionError::PathCase)?;
    let scopes = declared_scopes.into_exact_file_antichain(path_case);
    let reservations = RetainedReservationSet::replay(snapshot.events())
        .map_err(CheckDecisionError::ReservationReplay)?;
    let resolved_edit_authorization = ledger::resolve_identity(snapshot.worktree_context())
        .map_err(CheckDecisionError::Ledger)?;
    let identity_validation = CoordinationIdentityValidationContext::for_user_command(
        resolved_edit_authorization,
        snapshot.worktree_context(),
        recovery_command_line,
    );
    coordination_identity::validate_coordination_identity(&reservations, &identity_validation)
        .map_err(CheckDecisionError::from)?;
    validate_edit_worktree_occupancy(
        &reservations,
        snapshot.worktree_context(),
        resolved_edit_authorization,
    )
    .map_err(CheckDecisionError::from)?;
    let conflicts = reservations.conflicts_for_edit(
        &scopes,
        resolved_edit_authorization.edit_authorization(),
        path_case,
    );
    Ok(Enrollment::Enrolled(CheckDecision { scopes, conflicts }))
}

/// Refuse a pre-edit check whose run is not the one occupying the issuing worktree.
///
/// The occupancy rule holds between two coordination runs that both presented an identity, so
/// only [`EditAuthorization::Environment`] can reach it here. A session mapping and a worktree
/// marker each require an active reservation of their own run in this worktree, which this same
/// rule stops a second run from ever acquiring, and an unidentified caller presented nothing.
///
/// Deciding this before overlap is what makes the refusal repairable. Left to the overlap pass,
/// a second run in an occupied worktree was refused with the generic overlap answer, whose
/// stated remedy is to record one answer for the named holder --- a
/// `claim --run <second> --override <holder>` that this same rule refuses before any conflict
/// is evaluated. The occupancy refusal names the repair that works instead.
fn validate_edit_worktree_occupancy(
    reservations: &RetainedReservationSet,
    worktree_context: &WorktreeContext,
    resolved_edit_authorization: ResolvedEditAuthorization,
) -> Result<(), CoordinationIdentityValidationError> {
    match resolved_edit_authorization.edit_authorization() {
        EditAuthorization::Environment {
            coordination_run_id,
            ..
        } => coordination_identity::validate_worktree_occupancy(
            reservations,
            worktree_context,
            resolved_edit_authorization.worktree_id,
            coordination_run_id,
        ),
        EditAuthorization::Session { .. }
        | EditAuthorization::Marker { .. }
        | EditAuthorization::Unidentified => Ok(()),
    }
}
