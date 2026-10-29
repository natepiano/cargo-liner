//! Atomic first-touch edit claims with one blocked-path reconciliation retry.

use std::path::Path;

use super::claim;
use super::claim::CheckAcquisitionDecision;
use super::claim::CheckReservationSelection;
use super::claim::ClaimCapacityRecovery;
use super::claim::ClaimError;
use super::claim::FirstTouchClaimExecution;
use super::claim::FirstTouchClaimRequest;
use super::claim::FirstTouchConflictHandling;
use super::claim::FirstTouchConflictOutcome;
use super::refusal;
use crate::answer::OverlapApprover;
use crate::config::DefaultAnswer;
use crate::config::Enrollment;
use crate::coordination_identity;
use crate::coordination_identity::CoordinationIdentityRejection;
use crate::coordination_identity::CoordinationIdentityValidationContext;
use crate::coordination_identity::CoordinationIdentityValidationError;
use crate::coordination_identity::PresentedCoordinationRun;
use crate::coordination_identity::RecoveryCommandLine;
use crate::git::RepositorySnapshot;
use crate::ledger;
use crate::ledger::Ledger;
use crate::ledger::LedgerError;
use crate::ledger::RefusedVerb;
use crate::ledger::ReservationScopeSet;
use crate::ledger::ResolvedEditAuthorization;
use crate::ledger::ResolvedJournalMutationActor;
use crate::ledger::WorktreeContext;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::reconcile;
use crate::reservation::ActingHeadContainment;
use crate::reservation::ReservationConflict;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::scope::DeclaredReservationScopeSet;
use crate::scope::PathCase;
use crate::scope::PathCaseError;

/// A parsed edit check with lexically valid requested paths.
pub(crate) struct CheckRequest {
    /// The exact paths the edit operation proposes to modify.
    pub(crate) declared_scopes:       DeclaredReservationScopeSet,
    /// How locked first-touch validation selects the reservation to continue.
    pub(crate) reservation_selection: CheckReservationSelection,
}

struct CheckDecision {
    requester:         ResolvedJournalMutationActor,
    worktree_context:  WorktreeContext,
    acquisition:       CheckAcquisitionDecision,
    scopes:            ReservationScopeSet,
    conflicts:         Vec<ReservationConflict>,
    merge_observation: MergeObservationRequirement,
    approver:          Option<OverlapApprover>,
    /// The answer that lets an overlapping edit proceed instead of stopping, when configured.
    default_answer:    Option<DefaultAnswer>,
}

/// Whether a cached overlap answer can grant an edit without observing other branches.
enum MergeObservationRequirement {
    /// At least one foreign branch can have acquired new committed or dirty paths.
    Required,
    /// Only this checkout or explicitly released reservations remain.
    NotNeeded,
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

/// Permit an already-protected edit immediately; acquire every other edit under the lock.
pub(crate) fn execute(
    check_request: CheckRequest,
    recovery_command_line: &RecoveryCommandLine,
) -> OutputEnvelope {
    let mut capacity_recovery = claim::ClaimCapacityRecovery::default();
    let output = execute_with_capacity_recovery(
        check_request,
        recovery_command_line,
        &mut capacity_recovery,
    );
    capacity_recovery.into_output(output)
}

fn execute_with_capacity_recovery(
    check_request: CheckRequest,
    recovery_command_line: &RecoveryCommandLine,
    capacity_recovery: &mut ClaimCapacityRecovery,
) -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Check, &error.to_string());
        },
    };
    let repository = match crate::git::RepositorySnapshot::discover(&invocation_directory) {
        Ok(repository) => repository,
        Err(error) => return OutputEnvelope::ledger_error(CommandVerb::Check, &error),
    };
    let first_decision = match decide(
        &invocation_directory,
        &repository,
        check_request.declared_scopes.clone(),
        check_request.reservation_selection,
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
    if let CheckAcquisitionDecision::HeldBySelectedReservation {
        scopes,
        acquisition,
    } = first_decision.acquisition
    {
        return OutputEnvelope::clear_check(scopes, acquisition, None);
    }
    if matches!(
        first_decision.merge_observation,
        MergeObservationRequirement::Required
    ) {
        return reconcile_and_retry(
            &invocation_directory,
            &repository,
            check_request.declared_scopes,
            check_request.reservation_selection,
            recovery_command_line,
            capacity_recovery,
        );
    }
    if first_decision.conflicts.is_empty() {
        return match acquire_first_touch(
            &repository,
            check_request.declared_scopes.clone(),
            check_request.reservation_selection,
            recovery_command_line,
            capacity_recovery,
        ) {
            Ok(Enrollment::Enrolled(FirstTouchClaimExecution::Blocked { .. })) => {
                reconcile_and_retry(
                    &invocation_directory,
                    &repository,
                    check_request.declared_scopes,
                    check_request.reservation_selection,
                    recovery_command_line,
                    capacity_recovery,
                )
            },
            acquisition => render_acquisition(
                acquisition,
                first_decision.approver,
                first_decision.requester,
                &first_decision.worktree_context,
            ),
        };
    }
    reconcile_and_retry(
        &invocation_directory,
        &repository,
        check_request.declared_scopes,
        check_request.reservation_selection,
        recovery_command_line,
        capacity_recovery,
    )
}

/// Reconcile, then decide again; report reconciliation failures directly.
fn reconcile_and_retry(
    invocation_directory: &Path,
    repository: &RepositorySnapshot,
    declared_scopes: DeclaredReservationScopeSet,
    reservation_selection: CheckReservationSelection,
    recovery_command_line: &RecoveryCommandLine,
    capacity_recovery: &mut ClaimCapacityRecovery,
) -> OutputEnvelope {
    let reconciliation_report = match reconcile::reconcile_for_check(repository) {
        Ok(Enrollment::Enrolled(reconciliation_report)) => reconciliation_report,
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => {
            return OutputEnvelope::unconfigured(CommandVerb::Check, &expected_configuration_path);
        },
        Err(error) => return error.into_output(CommandVerb::Check),
    };
    let retried_decision = match decide(
        invocation_directory,
        repository,
        declared_scopes.clone(),
        reservation_selection,
        recovery_command_line,
    ) {
        Ok(Enrollment::Enrolled(check_decision)) => check_decision,
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => {
            return OutputEnvelope::unconfigured(CommandVerb::Check, &expected_configuration_path)
                .with_alerts(reconciliation_report.alerts)
                .with_retention_maintenance(&reconciliation_report.retention_maintenance)
                .with_automatic_releases(&reconciliation_report.settlements);
        },
        Err(error) => {
            return error
                .into_output()
                .with_alerts(reconciliation_report.alerts)
                .with_retention_maintenance(&reconciliation_report.retention_maintenance)
                .with_automatic_releases(&reconciliation_report.settlements);
        },
    };
    // A configured default answer records itself under the lock before the edit is protected.
    if retried_decision.conflicts.is_empty() || retried_decision.default_answer.is_some() {
        render_acquisition(
            acquire_first_touch(
                repository,
                declared_scopes,
                reservation_selection,
                recovery_command_line,
                capacity_recovery,
            ),
            retried_decision.approver,
            retried_decision.requester,
            &retried_decision.worktree_context,
        )
        .with_alerts(reconciliation_report.alerts)
        .with_retention_maintenance(&reconciliation_report.retention_maintenance)
        .with_automatic_releases(&reconciliation_report.settlements)
    } else {
        refusal::record(
            &retried_decision.worktree_context,
            retried_decision.requester,
            RefusedVerb::Check,
            &retried_decision.scopes,
            &retried_decision.conflicts,
        );
        OutputEnvelope::blocked_check(
            retried_decision.scopes,
            retried_decision.conflicts,
            retried_decision.approver,
        )
        .with_alerts(reconciliation_report.alerts)
        .with_retention_maintenance(&reconciliation_report.retention_maintenance)
        .with_automatic_releases(&reconciliation_report.settlements)
    }
}

fn acquire_first_touch(
    repository: &RepositorySnapshot,
    declared_scopes: DeclaredReservationScopeSet,
    reservation_selection: CheckReservationSelection,
    recovery_command_line: &RecoveryCommandLine,
    capacity_recovery: &mut ClaimCapacityRecovery,
) -> Result<Enrollment<FirstTouchClaimExecution>, ClaimError> {
    claim::acquire_first_touch_for_check(
        FirstTouchClaimRequest {
            declared_scopes,
            conflict_handling: FirstTouchConflictHandling::RefuseRequest,
        },
        reservation_selection,
        repository,
        recovery_command_line,
        capacity_recovery,
    )
}

fn render_acquisition(
    acquisition: Result<Enrollment<FirstTouchClaimExecution>, ClaimError>,
    approver: Option<OverlapApprover>,
    requester: ResolvedJournalMutationActor,
    worktree_context: &WorktreeContext,
) -> OutputEnvelope {
    match acquisition {
        Ok(Enrollment::Enrolled(FirstTouchClaimExecution::Acquired {
            acquisition,
            scopes,
            conflicts: FirstTouchConflictOutcome::None,
        })) => OutputEnvelope::clear_check(scopes, acquisition, None),
        Ok(Enrollment::Enrolled(FirstTouchClaimExecution::Acquired {
            acquisition,
            scopes,
            conflicts: FirstTouchConflictOutcome::AnsweredByDefault { answered },
        })) => OutputEnvelope::clear_check(scopes, acquisition, Some(answered)),
        Ok(Enrollment::Enrolled(FirstTouchClaimExecution::Acquired {
            scopes,
            conflicts: FirstTouchConflictOutcome::PostWriteIncursion { conflicts, .. },
            ..
        })) => OutputEnvelope::blocked_check(scopes, conflicts, approver),
        Ok(Enrollment::Enrolled(FirstTouchClaimExecution::Blocked { scopes, conflicts })) => {
            refusal::record(
                worktree_context,
                requester,
                RefusedVerb::Check,
                &scopes,
                &conflicts,
            );
            OutputEnvelope::blocked_check(scopes, conflicts, approver)
        },
        Ok(Enrollment::Enrolled(FirstTouchClaimExecution::ReservationLimitReached(maximum))) => {
            OutputEnvelope::check_reservation_limit_reached(maximum)
        },
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => OutputEnvelope::unconfigured(CommandVerb::Check, &expected_configuration_path),
        Err(error) => error.into_output(CommandVerb::Check),
    }
}

fn decide(
    invocation_directory: &Path,
    repository: &RepositorySnapshot,
    declared_scopes: DeclaredReservationScopeSet,
    reservation_selection: CheckReservationSelection,
    recovery_command_line: &RecoveryCommandLine,
) -> Result<Enrollment<CheckDecision>, CheckDecisionError> {
    let (reservations, worktree_context, configuration) =
        match Ledger::read_for_edit_check(invocation_directory)
            .map_err(CheckDecisionError::Ledger)?
        {
            Enrollment::Enrolled(snapshot) => snapshot.into_parts(),
            Enrollment::Unconfigured {
                expected_configuration_path,
            } => {
                return Ok(Enrollment::Unconfigured {
                    expected_configuration_path,
                });
            },
        };
    let path_case = PathCase::read(worktree_context.common_git_directory())
        .map_err(CheckDecisionError::PathCase)?;
    // Refusal follows the full declared edit, including every child of a tree scope.
    // First-touch acquisition separately retains its exact-file scope contract.
    let acquisition_scopes = declared_scopes.clone().into_exact_file_antichain(path_case);
    let scopes = declared_scopes.into_minimal_antichain(path_case);
    let reservations = reservations.map_err(CheckDecisionError::ReservationReplay)?;
    let resolved_edit_authorization =
        ledger::resolve_identity(&worktree_context).map_err(CheckDecisionError::Ledger)?;
    let repository = check_repository_observation(repository);
    let acting_head_containment = ActingHeadContainment::observe_for_actor(
        &reservations,
        &repository,
        resolved_edit_authorization,
    );
    let reservations = reservations.with_acting_head_containment(acting_head_containment);
    let identity_validation = CoordinationIdentityValidationContext::for_user_command(
        resolved_edit_authorization,
        &worktree_context,
        recovery_command_line,
    );
    coordination_identity::validate_coordination_identity(&reservations, &identity_validation)
        .map_err(CheckDecisionError::from)?;
    validate_edit_worktree_occupancy(
        &reservations,
        &worktree_context,
        resolved_edit_authorization,
    )
    .map_err(CheckDecisionError::from)?;
    let conflicts =
        reservations.conflicts_for_edit(&scopes, resolved_edit_authorization, path_case);
    let acquisition = if conflicts.is_empty()
        && let Ok(repository_trunk) = configuration.repository_trunk()
    {
        claim::selected_reservation_check_acquisition(
            &reservations,
            &repository_trunk,
            acquisition_scopes,
            reservation_selection,
            resolved_edit_authorization,
            path_case,
        )
    } else {
        CheckAcquisitionDecision::NeedsLockedDecision
    };
    let merge_observation = if reservations.iter().any(|holder| {
        holder.actor().worktree != resolved_edit_authorization.worktree_id
            && !matches!(
                holder.lifecycle(),
                crate::reservation::ReservationLifecycle::Released { .. }
            )
    }) {
        MergeObservationRequirement::Required
    } else {
        MergeObservationRequirement::NotNeeded
    };
    Ok(Enrollment::Enrolled(CheckDecision {
        acquisition,
        scopes,
        conflicts,
        merge_observation,
        approver: configuration.overlap_approver(worktree_context.repository_root()),
        default_answer: configuration.default_answer,
        requester: resolved_edit_authorization
            .journal_mutation_actor_for(resolved_edit_authorization.coordination_run_id),
        worktree_context,
    }))
}

/// Refuse a pre-edit check whose run is not the one occupying the issuing worktree.
///
/// The occupancy rule holds between two coordination runs that both presented an identity, so
/// only [`crate::ledger::EditAuthorization::Environment`] can reach it here --- which is what
/// [`PresentedCoordinationRun::from_edit_authorization`] answers, rather than a variant match
/// repeated at this site. A session mapping and a worktree marker each require an active
/// reservation of their own run in this worktree, which this same rule stops a second run from
/// ever acquiring, and an unidentified caller presented nothing.
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
    let Some(acting_run) = PresentedCoordinationRun::from_edit_authorization(
        resolved_edit_authorization.edit_authorization(),
    ) else {
        return Ok(());
    };
    coordination_identity::validate_worktree_occupancy(
        reservations,
        worktree_context,
        resolved_edit_authorization.worktree_id,
        acting_run,
    )
}

/// Refresh mutable checkout facts while retaining this check's immutable evidence.
fn check_repository_observation(repository: &RepositorySnapshot) -> RepositorySnapshot {
    repository.fresh_observation()
}

#[cfg(test)]
mod cost_tests {
    use cargo_berth_test_support::CommandDeadlineExt as _;

    use super::check_repository_observation;
    use crate::git::RepositorySnapshot;

    #[test]
    fn check_reuses_merge_pairs_across_decision_passes() -> Result<(), Box<dyn std::error::Error>> {
        let root = cargo_berth_test_support::copy_git_repository_seed("check-pair-cache", |root| {
            for args in [
                vec!["init", "--quiet"],
                vec![
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.com",
                    "commit",
                    "--quiet",
                    "--allow-empty",
                    "-m",
                    "base",
                ],
            ] {
                let status = std::process::Command::new("git")
                    .arg("-C")
                    .arg(root)
                    .args(args)
                    .bounded_status()?;
                assert!(status.success());
            }
            Ok::<(), Box<dyn std::error::Error>>(())
        });
        let repository = RepositorySnapshot::discover(root.path())?;
        let trunk = repository.head_object_id()?.clone();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(root.path())
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "head",
            ])
            .bounded_status()?;
        assert!(status.success());
        let head = repository.fresh_observation().head_object_id()?.clone();
        let (expected, queries) = repository.unmerged_branch_paths(root.path(), &trunk, &head)?;
        assert_eq!(queries, 1);
        let next_pass = check_repository_observation(&repository);
        let (observed, queries) = next_pass.unmerged_branch_paths(root.path(), &trunk, &head)?;
        assert_eq!(observed, expected);
        assert_eq!(queries, 0, "one merge-tree per pair for the entire check");
        Ok(())
    }
}
