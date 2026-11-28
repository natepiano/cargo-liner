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
use super::claim::FirstTouchObservation;
use super::claim::PhaseReservationCandidate;
use super::refusal;
use crate::answer::OverlapApprover;
use crate::config::BerthConfig;
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
use crate::reservation::ReservationFacts;
use crate::reservation::ReservationReplayError;
use crate::scope::DeclaredReservationScopeSet;
use crate::scope::PathCase;
use crate::scope::PathCaseError;
use crate::session;

/// A parsed edit check with lexically valid requested paths.
pub(crate) struct CheckRequest {
    /// The exact paths the edit operation proposes to modify.
    pub(crate) declared_scopes:       DeclaredReservationScopeSet,
    /// How locked first-touch validation selects the reservation to continue.
    pub(crate) reservation_selection: CheckReservationSelection,
}

struct CheckDecision {
    requester:         ResolvedJournalMutationActor,
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
    /// More than one reservation declares the edit and the caller has not selected an owner.
    AmbiguousEditHookOwners(Vec<PhaseReservationCandidate>),
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
            Self::AmbiguousEditHookOwners(candidates) => {
                OutputEnvelope::ambiguous_active_run_reservations(CommandVerb::Check, &candidates)
            },
        }
    }
}

/// Permit an already-protected edit immediately; acquire every other edit under the lock.
pub(crate) fn execute(
    check_request: CheckRequest,
    recovery_command_line: &RecoveryCommandLine,
) -> OutputEnvelope {
    let invocation_directory = match session::command_directory() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Check, &error.to_string());
        },
    };
    execute_at(&invocation_directory, check_request, recovery_command_line)
}

fn execute_at(
    invocation_directory: &Path,
    check_request: CheckRequest,
    recovery_command_line: &RecoveryCommandLine,
) -> OutputEnvelope {
    let repository = match RepositorySnapshot::discover(invocation_directory) {
        Ok(repository) => repository,
        Err(error) => return OutputEnvelope::ledger_error(CommandVerb::Check, &error),
    };
    let ledger = if matches!(
        check_request.reservation_selection,
        CheckReservationSelection::EditHook
    ) {
        Ledger::for_edit_check(repository.context())
    } else {
        Ledger::for_command_check(repository.context())
    };
    let mut capacity_recovery = ClaimCapacityRecovery::default();
    let output = execute_with_capacity_recovery(
        &ledger,
        &repository,
        check_request,
        recovery_command_line,
        &mut capacity_recovery,
    );
    capacity_recovery.into_output(output)
}

fn execute_with_capacity_recovery(
    ledger: &Ledger,
    repository: &RepositorySnapshot,
    check_request: CheckRequest,
    recovery_command_line: &RecoveryCommandLine,
    capacity_recovery: &mut ClaimCapacityRecovery,
) -> OutputEnvelope {
    let first_decision = match decide(
        ledger,
        repository,
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
    if let CheckAcquisitionDecision::HeldBySelectedReservation(held) = first_decision.acquisition {
        let (acquisition, scopes) = held.publish(repository.context());
        return OutputEnvelope::clear_check(scopes, acquisition, None);
    }
    if matches!(
        first_decision.merge_observation,
        MergeObservationRequirement::Required
    ) {
        if matches!(
            check_request.reservation_selection,
            CheckReservationSelection::EditHook
        ) && first_decision.conflicts.is_empty()
        {
            let acquisition = acquire_first_touch(
                ledger,
                repository,
                check_request.declared_scopes.clone(),
                check_request.reservation_selection,
                FirstTouchObservation::EditedPath,
                recovery_command_line,
                capacity_recovery,
            );
            if !matches!(acquisition, Err(ClaimError::FullReconciliationRequired)) {
                return render_acquisition(
                    acquisition,
                    first_decision.approver,
                    first_decision.requester,
                    ledger,
                );
            }
        }
        return reconcile_and_retry(
            ledger,
            repository,
            check_request.declared_scopes,
            check_request.reservation_selection,
            recovery_command_line,
            capacity_recovery,
        );
    }
    if first_decision.conflicts.is_empty() {
        return match acquire_first_touch(
            ledger,
            repository,
            check_request.declared_scopes.clone(),
            check_request.reservation_selection,
            FirstTouchObservation::Retained,
            recovery_command_line,
            capacity_recovery,
        ) {
            Ok(Enrollment::Enrolled(FirstTouchClaimExecution::Blocked { .. })) => {
                reconcile_and_retry(
                    ledger,
                    repository,
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
                ledger,
            ),
        };
    }
    reconcile_and_retry(
        ledger,
        repository,
        check_request.declared_scopes,
        check_request.reservation_selection,
        recovery_command_line,
        capacity_recovery,
    )
}

/// Reconcile, then decide again; report reconciliation failures directly.
fn reconcile_and_retry(
    ledger: &Ledger,
    repository: &RepositorySnapshot,
    declared_scopes: DeclaredReservationScopeSet,
    reservation_selection: CheckReservationSelection,
    recovery_command_line: &RecoveryCommandLine,
    capacity_recovery: &mut ClaimCapacityRecovery,
) -> OutputEnvelope {
    let reconciliation_report = match reconcile::reconcile_for_check(repository, ledger) {
        Ok(Enrollment::Enrolled(reconciliation_report)) => reconciliation_report,
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => {
            return OutputEnvelope::unconfigured(CommandVerb::Check, &expected_configuration_path);
        },
        Err(error) => return error.into_output(CommandVerb::Check),
    };
    let retried_decision = match decide(
        ledger,
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
                ledger,
                repository,
                declared_scopes,
                reservation_selection,
                FirstTouchObservation::Retained,
                recovery_command_line,
                capacity_recovery,
            ),
            retried_decision.approver,
            retried_decision.requester,
            ledger,
        )
        .with_alerts(reconciliation_report.alerts)
        .with_retention_maintenance(&reconciliation_report.retention_maintenance)
        .with_automatic_releases(&reconciliation_report.settlements)
    } else {
        refusal::record(
            ledger,
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
    ledger: &Ledger,
    repository: &RepositorySnapshot,
    declared_scopes: DeclaredReservationScopeSet,
    reservation_selection: CheckReservationSelection,
    observation: FirstTouchObservation,
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
        ledger,
        observation,
        recovery_command_line,
        capacity_recovery,
    )
}

fn render_acquisition(
    acquisition: Result<Enrollment<FirstTouchClaimExecution>, ClaimError>,
    approver: Option<OverlapApprover>,
    requester: ResolvedJournalMutationActor,
    ledger: &Ledger,
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
            refusal::record(ledger, requester, RefusedVerb::Check, &scopes, &conflicts);
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
    ledger: &Ledger,
    repository: &RepositorySnapshot,
    declared_scopes: DeclaredReservationScopeSet,
    reservation_selection: CheckReservationSelection,
    recovery_command_line: &RecoveryCommandLine,
) -> Result<Enrollment<CheckDecision>, CheckDecisionError> {
    let worktree_context = repository.context();
    let configuration = match BerthConfig::read(&worktree_context.configuration_lookup())
        .map_err(LedgerError::from)
        .map_err(CheckDecisionError::Ledger)?
    {
        Enrollment::Enrolled(configuration) => configuration,
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => {
            return Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            });
        },
    };
    let reservations = ledger
        .with_validated_reservation_facts(|facts, _| facts.cloned().map_err(Clone::clone))
        .map_err(CheckDecisionError::Ledger)?;
    let path_case = PathCase::read(worktree_context.common_git_directory())
        .map_err(CheckDecisionError::PathCase)?;
    // Refusal follows the full declared edit, including every child of a tree scope.
    // First-touch acquisition separately retains its exact-file scope contract.
    let acquisition_scopes = declared_scopes.clone().into_exact_file_antichain(path_case);
    let scopes = declared_scopes.into_minimal_antichain(path_case);
    let reservations = reservations.map_err(CheckDecisionError::ReservationReplay)?;
    let caller_identity =
        ledger::resolve_identity(worktree_context).map_err(CheckDecisionError::Ledger)?;
    let edit_authorization = if matches!(reservation_selection, CheckReservationSelection::EditHook)
    {
        claim::edit_hook_authorization(
            &reservations,
            caller_identity,
            &acquisition_scopes,
            path_case,
            &worktree_context.ledger_directory(),
        )
        .map_err(CheckDecisionError::AmbiguousEditHookOwners)
    } else {
        Ok(caller_identity)
    };
    let repository = check_repository_observation(repository);
    let acting_head_containment = ActingHeadContainment::observe_for_edit(
        &reservations,
        &repository,
        *edit_authorization.as_ref().unwrap_or(&caller_identity),
        &scopes,
        path_case,
    );
    let reservations = reservations.with_acting_head_containment(acting_head_containment);
    let identity_validation = CoordinationIdentityValidationContext::for_user_command(
        caller_identity,
        worktree_context,
        recovery_command_line,
    );
    coordination_identity::validate_coordination_identity(&reservations, &identity_validation)
        .map_err(CheckDecisionError::from)?;
    validate_edit_worktree_occupancy(&reservations, worktree_context, caller_identity)
        .map_err(CheckDecisionError::from)?;
    let resolved_edit_authorization = edit_authorization?;
    let conflicts =
        reservations.conflicts_for_edit(&scopes, resolved_edit_authorization, path_case);
    let acquisition = if conflicts.is_empty()
        && resolved_edit_authorization == caller_identity
        && !session::command_selection_needs_publication(&worktree_context.ledger_directory())
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
    reservations: &ReservationFacts,
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
    use std::error::Error;
    use std::fs;
    use std::os::unix::fs::MetadataExt as _;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::ExitCode;
    use std::process::Stdio;

    use cargo_berth_test_support::CommandDeadlineExt as _;
    use cargo_berth_test_support::TestProcessGroup;
    use serde_json::Value;
    use tempfile::TempDir;

    use super::CheckRequest;
    use super::CheckReservationSelection;
    use super::check_repository_observation;
    use super::execute_at;
    use crate::config::Enrollment;
    use crate::coordination_identity::RecoveryCommandLine;
    use crate::exit::BerthExit;
    use crate::git::RepositorySnapshot;
    use crate::hook::pre_tool_use;
    use crate::ids::CoordinationRunId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeId;
    use crate::ledger;
    use crate::ledger::HARNESS_SESSION_ENVIRONMENT;
    use crate::ledger::JournalOperation;
    use crate::ledger::Ledger;
    use crate::ledger::LedgerTransactionOutcome;
    use crate::ledger::ScopeKind;
    use crate::ledger::TransactionValidation;
    use crate::ledger::publication::sync_tests;
    use crate::ledger::publication::sync_tests::SyncStep;
    use crate::output::OutputEnvelope;
    use crate::reconcile::RecoveredBypassReporting;
    use crate::reservation::MergeExtent;
    use crate::reservation::ReservationRunStatus;
    use crate::scope::DeclaredReservationScopeSet;
    use crate::session;
    use crate::test_work::WorkCounter;

    const TEST_COMPACTION_THRESHOLD_BYTES: u64 = 1;
    const HELD_PATH: &str = "held.rs";
    const UNRESERVED_PATH: &str = "new.rs";

    #[test]
    fn held_check_reports_marker_publication_after_both_syncs() -> Result<(), Box<dyn Error>> {
        assert_held_check_publication("marker_publication", |repository| {
            repository
                .context()
                .administrative_directory()
                .to_path_buf()
        })
    }

    #[test]
    fn held_check_reports_mapping_publication_after_both_syncs() -> Result<(), Box<dyn Error>> {
        assert_held_check_publication("session_mapping_publication", |repository| {
            repository.context().ledger_directory()
        })
    }

    fn assert_held_check_publication(
        field: &str,
        publication_directory: impl FnOnce(&RepositorySnapshot) -> PathBuf,
    ) -> Result<(), Box<dyn Error>> {
        let directory = initialized_check_repository()?;
        let root = directory.path();
        let repository = RepositorySnapshot::discover(root)?;
        let publication_directory = publication_directory(&repository);
        session::with_test_harness_session(Some("publication-test".parse()?), || {
            let check = || {
                check_with_selection(
                    root,
                    HELD_PATH,
                    CheckReservationSelection::SessionMappingOrNew,
                )
            };
            let first = check()?;
            assert_eq!(first.exit_code(), BerthExit::Clear, "{first:?}");
            let counter = WorkCounter::start();
            for (fail_at, status, expected_steps) in [
                (Some(SyncStep::File), "unavailable", &[SyncStep::File][..]),
                (
                    Some(SyncStep::Directory),
                    "unavailable",
                    &[SyncStep::File, SyncStep::Directory],
                ),
                (None, "published", &[SyncStep::File, SyncStep::Directory]),
            ] {
                let (result, steps) = sync_tests::record(&publication_directory, fail_at, check);
                let result = result?;
                assert_eq!(result.exit_code(), BerthExit::Clear, "{result:?}");
                let envelope = serde_json::to_value(result)?;
                let acquisition = &envelope["payload"]["data"]["acquisition"];
                assert_eq!(acquisition["kind"], "already_held", "{envelope}");
                let publication = &acquisition[field];
                assert_eq!(publication["status"], status, "{envelope}");
                if let Some(step) = fail_at {
                    let diagnostic = format!("injected {step:?} sync failure");
                    assert!(
                        publication["diagnostic"]
                            .as_str()
                            .is_some_and(|value| value.contains(&diagnostic)),
                        "{publication}"
                    );
                }
                assert_eq!(steps, expected_steps);
            }
            assert_eq!(counter.finish().mutation_lock_attempts, 0);
            Ok(())
        })
    }

    #[test]
    fn checkpoint_gap_hold_reports_whether_a_mapping_is_required() -> Result<(), Box<dyn Error>> {
        let directory = initialized_check_repository()?;
        let root = directory.path();
        let trunk_snapshot = RepositorySnapshot::discover(root)?
            .head_object_id()?
            .clone();
        run_git(root, &["switch", "--quiet", "-c", "phase"])?;
        let harness_session_id: session::HarnessSessionId = "plan-owner".parse()?;
        let claimed = session::with_test_harness_session(Some(harness_session_id.clone()), || {
            edit_check(root, HELD_PATH)
        })?;
        assert_eq!(claimed.exit_code(), BerthExit::Clear, "{claimed:?}");
        let ledger = Ledger::open(root)?;
        let events = ledger.journal_events_for_test()?;
        let (actor, reservation_id) = events
            .iter()
            .find_map(|event| match &event.operation {
                JournalOperation::Claim {
                    reservation_id,
                    source: ledger::ClaimSource::FirstTouch,
                    ..
                } => Some((&event.actor, *reservation_id)),
                _ => None,
            })
            .ok_or("expected the first-touch reservation")?;
        fs::write(root.join(HELD_PATH), "checkpointed work\n")?;
        run_git(
            root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--quiet",
                "-am",
                "checkpoint work",
            ],
        )?;
        let protected_tip = RepositorySnapshot::discover(root)?
            .head_object_id()?
            .clone();
        let checkpoint = ledger.transact(actor.worktree, actor.run, |_| {
            TransactionValidation::<()>::Append(Box::new(JournalOperation::Checkpoint {
                reservation_id,
                protected_tip: protected_tip.clone().into(),
                trunk_snapshot: trunk_snapshot.clone(),
            }))
        })?;
        assert!(matches!(
            checkpoint,
            LedgerTransactionOutcome::Appended { .. }
        ));

        let repository = RepositorySnapshot::discover(root)?;
        let removed = repository
            .context()
            .remove_coordination_run_marker(actor.run)?;
        assert_eq!(removed, ledger::CoordinationRunMarkerRemoval::Removed);
        for (path, harness_session_id, expected) in [
            (UNRESERVED_PATH, Some(harness_session_id), "left_unmapped"),
            ("another.rs", None, "published"),
        ] {
            session::with_test_harness_session(harness_session_id, || {
                let result = edit_check(root, path)?;
                assert_eq!(result.exit_code(), BerthExit::Clear, "{result:?}");
                let envelope = serde_json::to_value(result)?;
                let acquisition = &envelope["payload"]["data"]["acquisition"];
                assert_eq!(acquisition["kind"], "appended", "{envelope}");
                assert!(matches!(
                    session::resolve(&repository.context().ledger_directory()),
                    session::SessionIdentityLookup::Unavailable
                ));
                let events = ledger.journal_events_for_test()?;
                assert!(
                    events.last().is_some_and(|event| {
                        matches!(&event.operation, JournalOperation::Claim {
                        source: ledger::ClaimSource::PlanShrink, scopes, ..
                    } if scopes.covers_path(path.as_bytes()))
                    }),
                    "{envelope}"
                );
                assert_eq!(
                    acquisition["session_mapping_publication"]["status"], expected,
                    "{envelope}"
                );
                Ok::<_, Box<dyn Error>>(())
            })?;
        }
        Ok(())
    }

    #[test]
    fn edit_hook_first_touch_decodes_checkpoint_once() -> Result<(), Box<dyn Error>> {
        const CHILD_MODE: &str = "CARGO_BERTH_TEST_COUNT_PRE_TOOL_USE_DECODES";
        const TEST: &str = "verb::check::cost_tests::edit_hook_first_touch_decodes_checkpoint_once";
        if std::env::var_os(CHILD_MODE).is_some() {
            let (exit_code, decodes) =
                Ledger::with_compaction_threshold_for_test(TEST_COMPACTION_THRESHOLD_BYTES, || {
                    ledger::count_checkpoint_decodes(pre_tool_use::execute)
                });
            assert_eq!(exit_code, ExitCode::SUCCESS);
            assert_eq!(decodes, 1);
            return Ok(());
        }

        let fixture = EditCheckFixture::new()?;
        let ledger = Ledger::open(fixture.root())?;
        let events = ledger.journal_events_for_test()?;
        let (actor, reservation_id, scopes) = events
            .iter()
            .find_map(|event| match &event.operation {
                JournalOperation::Claim {
                    reservation_id,
                    scopes,
                    ..
                } => Some((&event.actor, *reservation_id, scopes)),
                _ => None,
            })
            .ok_or("expected the foreign claim")?;
        let observation = JournalOperation::MergeExtentObserved {
            reservation_id,
            extent: MergeExtent::NotDerived {
                protection: scopes.clone(),
            },
            run_status: ReservationRunStatus::default(),
        };
        ledger.append_legacy_journal_operation(actor.clone(), observation.clone())?;
        ledger.append_legacy_journal_operation(actor.clone(), observation)?;
        Ledger::repair_projection(fixture.root())?;
        ledger.write_replay_checkpoint_for_test()?;
        let before_hook = ledger.journal_events_for_test()?;
        let journal = fixture.root().join(".git/cargo-berth/journal.ndjson");
        let original_bytes = fs::read(&journal)?;
        let original_inode = fs::metadata(&journal)?.ino();
        let payload = serde_json::to_vec(&serde_json::json!({
            "tool_name": "Write",
            "cwd": fixture.root(),
            "tool_input": {"file_path": fixture.root().join(UNRESERVED_PATH)},
        }))?;
        let mut command = cargo_berth_test_support::berth_command(std::env::current_exe()?);
        let child = TestProcessGroup::spawn(
            command
                .args(["--exact", TEST, "--nocapture"])
                .env(CHILD_MODE, "1")
                .env_remove("CARGO_BERTH_BYPASS")
                .env_remove("CARGO_BERTH_RUN")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )?;
        let output = child.output_with_input(&payload)?;
        assert!(output.status.success(), "{output:?}");

        assert!(
            fs::read(&journal)?.starts_with(&original_bytes),
            "the edit hook must preserve the journal byte prefix"
        );
        assert_eq!(
            fs::metadata(&journal)?.ino(),
            original_inode,
            "the edit hook must keep the journal inode"
        );
        let events = ledger.journal_events_for_test()?;
        assert!(
            events.starts_with(&before_hook),
            "the edit hook must preserve due history"
        );
        let appended = &events[before_hook.len()..];
        assert!(appended.iter().any(|event| {
            matches!(
                &event.operation,
                JournalOperation::HolderMergeExtentObserved { .. }
            )
        }));
        assert!(appended.iter().any(|event| {
            matches!(
                &event.operation,
                JournalOperation::Claim { scopes, .. } if scopes.covers_path(UNRESERVED_PATH.as_bytes())
            )
        }));
        assert!(
            ledger::superseded_bytes(&ledger)? >= TEST_COMPACTION_THRESHOLD_BYTES,
            "compaction must still be due after the edit hook"
        );
        assert_ordinary_renewal_compacts_journal(
            fixture.root(),
            &ledger,
            reservation_id,
            events.len(),
        )
    }

    fn assert_ordinary_renewal_compacts_journal(
        root: &Path,
        ledger: &Ledger,
        reservation_id: ReservationId,
        records_before_renewal: usize,
    ) -> Result<(), Box<dyn Error>> {
        let outcome =
            Ledger::with_compaction_threshold_for_test(TEST_COMPACTION_THRESHOLD_BYTES, || {
                Ledger::open(root)?.transact(WorktreeId::new(), CoordinationRunId::new(), |_| {
                    TransactionValidation::<()>::Append(Box::new(JournalOperation::Renew {
                        reservation_id,
                    }))
                })
            })?;
        assert!(matches!(outcome, LedgerTransactionOutcome::Appended { .. }));
        let compacted = ledger.journal_events_for_test()?;
        assert!(
            compacted.len() < records_before_renewal + 1,
            "the ordinary mutation must compact"
        );
        assert!(compacted.last().is_some_and(|event| {
            matches!(event.operation, JournalOperation::Renew { reservation_id: renewed } if renewed == reservation_id)
        }));
        assert_eq!(
            ledger::superseded_bytes(ledger)?,
            0,
            "the ordinary mutation must remove all superseded bytes"
        );
        ledger.read_validated_journal()?;
        Ok(())
    }

    #[test]
    fn edit_hook_keeps_first_touch_held_and_refused_answers() -> Result<(), Box<dyn Error>> {
        const CHILD_MODE: &str = "CARGO_BERTH_TEST_CHECK_EDIT_HOOK_ANSWERS";
        const TEST: &str =
            "verb::check::cost_tests::edit_hook_keeps_first_touch_held_and_refused_answers";
        if std::env::var_os(CHILD_MODE).is_none() {
            // `edit_check` calls `execute_at` directly. The fixture needs a process without
            // a harness session or run identity from the test launcher.
            let output = cargo_berth_test_support::berth_command(std::env::current_exe()?)
                .args(["--exact", TEST, "--nocapture"])
                .env(CHILD_MODE, "1")
                .env_remove(HARNESS_SESSION_ENVIRONMENT)
                .env_remove("CARGO_BERTH_RUN")
                .bounded_output()?;
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return Ok(());
        }

        let fixture = EditCheckFixture::new()?;
        let ledger = Ledger::open(fixture.root())?;
        ledger.write_replay_checkpoint_for_test()?;

        let first = serde_json::to_value(edit_check(fixture.root(), UNRESERVED_PATH)?)?;
        assert_eq!(first["status"], "clear", "{first}");
        assert_eq!(
            first["message"],
            "No foreign reservation overlaps the requested paths; a first-touch reservation was acquired."
        );
        let before_held = ledger.journal_events_for_test()?.len();
        let held = serde_json::to_value(edit_check(fixture.root(), UNRESERVED_PATH)?)?;
        assert_eq!(held["status"], "clear", "{held}");
        assert_eq!(held["reservations"], first["reservations"]);
        assert_eq!(
            held["message"],
            "No foreign reservation overlaps the requested paths; the acting run already holds them."
        );
        let events = ledger.journal_events_for_test()?;
        let appended = &events[before_held..];
        assert!(
            appended.iter().all(|event| {
                !matches!(
                    event.operation,
                    JournalOperation::Claim { .. } | JournalOperation::Widen { .. }
                )
            }),
            "an already-held edit acquired scope again: {appended:?}"
        );

        let (refusal, decodes) =
            ledger::count_checkpoint_decodes(|| edit_check(fixture.root(), HELD_PATH));
        let refusal = serde_json::to_value(refusal?)?;
        assert_eq!(refusal["status"], "blocked_by_overlap", "{refusal}");
        assert_eq!(refusal["blocked_by"], fixture.holder);
        assert_eq!(decodes, 1);
        let events = ledger.journal_events_for_test()?;
        assert!(events.last().is_some_and(|event| {
            matches!(event.operation, JournalOperation::OverlapRefused { .. })
        }));
        Ok(())
    }

    #[test]
    fn edit_hook_observes_a_foreign_change_outside_the_foreign_claim() -> Result<(), Box<dyn Error>>
    {
        run_edit_observation_test(
            "verb::check::cost_tests::edit_hook_observes_a_foreign_change_outside_the_foreign_claim",
            || {
                let fixture = EditCheckFixture::new()?;
                assert_eq!(
                    board_settlements(fixture.root())?,
                    Vec::<ReservationId>::new()
                );
                fs::write(fixture.foreign.join(UNRESERVED_PATH), "foreign edit\n")?;

                let refusal = serde_json::to_value(edit_check(fixture.root(), UNRESERVED_PATH)?)?;
                assert_eq!(refusal["status"], "blocked_by_overlap", "{refusal}");
                assert_eq!(refusal["blocked_by"], fixture.holder);
                Ok(())
            },
        )
    }

    #[test]
    fn edit_hook_observes_an_ascii_case_variant_in_a_foreign_checkout() -> Result<(), Box<dyn Error>>
    {
        run_edit_observation_test(
            "verb::check::cost_tests::edit_hook_observes_an_ascii_case_variant_in_a_foreign_checkout",
            || {
                let fixture = EditCheckFixture::new()?;
                run_git(fixture.root(), &["config", "core.ignorecase", "true"])?;
                assert_eq!(
                    board_settlements(fixture.root())?,
                    Vec::<ReservationId>::new()
                );
                fs::write(fixture.foreign.join("NEW.rs"), "foreign edit\n")?;

                let path = UNRESERVED_PATH.parse()?;
                assert!(matches!(
                    RepositorySnapshot::edited_path_status_at(
                        &fixture.foreign,
                        &path,
                        crate::scope::PathCase::Insensitive,
                    )?,
                    crate::git::EditedPathStatus::Changed
                ));
                let refusal = serde_json::to_value(edit_check(fixture.root(), UNRESERVED_PATH)?)?;
                assert_eq!(refusal["status"], "blocked_by_overlap", "{refusal}");
                assert_eq!(refusal["blocked_by"], fixture.holder);
                Ok(())
            },
        )
    }

    #[test]
    fn edit_hook_fully_observes_non_ascii_edits_in_a_case_insensitive_checkout()
    -> Result<(), Box<dyn Error>> {
        run_edit_observation_test(
            "verb::check::cost_tests::edit_hook_fully_observes_non_ascii_edits_in_a_case_insensitive_checkout",
            || {
                let fixture = EditCheckFixture::new()?;
                run_git(fixture.root(), &["config", "core.ignorecase", "true"])?;
                fixture.integrate_holder()?;

                let answer = edit_check(fixture.root(), "é.rs")?;
                assert_eq!(answer.exit_code(), BerthExit::Clear, "{answer:?}");
                let holder = fixture.holder_reservation_id()?;
                let events = Ledger::open(fixture.root())?.journal_events_for_test()?;
                assert!(events.iter().any(|event| {
                    matches!(event.operation, JournalOperation::Release { reservation_id, .. } if reservation_id == holder)
                }));
                Ok(())
            },
        )
    }

    #[test]
    fn edit_hook_keeps_an_integrated_holder_with_an_untracked_path() -> Result<(), Box<dyn Error>> {
        run_edit_observation_test(
            "verb::check::cost_tests::edit_hook_keeps_an_integrated_holder_with_an_untracked_path",
            || {
                let fixture = EditCheckFixture::new()?;
                fixture.integrate_holder()?;
                fs::write(fixture.foreign.join("untracked.rs"), "uncommitted work\n")?;

                let answer = edit_check(fixture.root(), UNRESERVED_PATH)?;
                assert_eq!(answer.exit_code(), BerthExit::Clear, "{answer:?}");
                fixture.assert_holder_not_released()?;
                Ok(())
            },
        )
    }

    #[test]
    fn edit_hook_leaves_clean_holder_settlement_to_the_board() -> Result<(), Box<dyn Error>> {
        run_edit_observation_test(
            "verb::check::cost_tests::edit_hook_leaves_clean_holder_settlement_to_the_board",
            || {
                let fixture = EditCheckFixture::new()?;
                fixture.integrate_holder()?;

                Ledger::open(fixture.root())?.write_replay_checkpoint_for_test()?;
                let (answer, parts) = ledger::count_checkpoint_part_decodes(|| {
                    edit_check(fixture.root(), UNRESERVED_PATH)
                });
                let answer = answer?;
                assert_eq!(
                    parts, 3,
                    "read the core, reservations, and ordering records"
                );

                assert_eq!(answer.exit_code(), BerthExit::Clear, "{answer:?}");
                fixture.assert_holder_not_released()?;

                assert_eq!(
                    board_settlements(fixture.root())?,
                    [fixture.holder_reservation_id()?]
                );
                assert_eq!(
                    board_settlements(fixture.root())?,
                    Vec::<ReservationId>::new()
                );
                Ok(())
            },
        )
    }

    #[test]
    fn edit_hook_reconciles_an_integrated_holder_at_capacity() -> Result<(), Box<dyn Error>> {
        run_edit_observation_test(
            "verb::check::cost_tests::edit_hook_reconciles_an_integrated_holder_at_capacity",
            || {
                let fixture = EditCheckFixture::new()?;
                fixture.integrate_holder()?;
                fixture.set_reservation_limit_to_one()?;

                let answer = edit_check(fixture.root(), UNRESERVED_PATH)?;
                assert_eq!(answer.exit_code(), BerthExit::Clear, "{answer:?}");
                assert_eq!(
                    board_settlements(fixture.root())?,
                    Vec::<ReservationId>::new()
                );
                let holder = fixture.holder_reservation_id()?;
                let journal = Ledger::open(fixture.root())?.journal_events_for_test()?;
                assert!(journal.iter().any(|event| {
                    matches!(event.operation, JournalOperation::Release { reservation_id, .. } if reservation_id == holder)
                }));
                Ok(())
            },
        )
    }

    #[test]
    fn edit_hook_counts_a_checkpointed_holder_with_new_work_at_capacity()
    -> Result<(), Box<dyn Error>> {
        run_edit_observation_test(
            "verb::check::cost_tests::edit_hook_counts_a_checkpointed_holder_with_new_work_at_capacity",
            || {
                let fixture = EditCheckFixture::new()?;
                fixture.set_reservation_limit_to_one()?;
                fixture.checkpoint_holder()?;
                run_git(&fixture.foreign, &["switch", "--quiet", "--detach", "main"])?;
                assert_eq!(
                    board_settlements(fixture.root())?,
                    Vec::<ReservationId>::new()
                );
                let ledger = Ledger::open(fixture.root())?;
                let holder = fixture.holder_reservation_id()?;
                let reservations = ledger.read_validated_live_reservations()??;
                let reservation = reservations
                    .iter()
                    .find(|reservation| reservation.id() == holder)
                    .ok_or("expected the checkpointed holder")?;
                assert_eq!(reservation.run_status(), ReservationRunStatus::Ended);
                assert!(matches!(
                    reservation.merge_extent(),
                    MergeExtent::Empty { .. }
                ));
                fs::write(fixture.foreign.join("z.rs"), "uncommitted work\n")?;

                let edit = serde_json::to_value(edit_check(fixture.root(), UNRESERVED_PATH)?)?;
                let full = serde_json::to_value(check_with_selection(
                    fixture.root(),
                    UNRESERVED_PATH,
                    CheckReservationSelection::SessionMappingOrSingleActive,
                )?)?;
                let expected =
                    serde_json::to_value(OutputEnvelope::check_reservation_limit_reached(1))?;
                assert_eq!(edit["status"], expected["status"], "{edit}");
                assert_eq!(
                    edit["payload"]["data"], expected["payload"]["data"],
                    "{edit}"
                );
                assert_eq!(edit["status"], full["status"], "{full}");
                assert_eq!(edit["payload"]["data"], full["payload"]["data"], "{full}");
                fixture.assert_holder_not_released()
            },
        )
    }

    #[test]
    fn edit_hook_rechecks_exact_overlap_revision_when_other_paths_change()
    -> Result<(), Box<dyn Error>> {
        run_edit_observation_test(
            "verb::check::cost_tests::edit_hook_rechecks_exact_overlap_revision_when_other_paths_change",
            || {
                let fixture = EditCheckFixture::new()?;
                let ledger = Ledger::open(fixture.root())?;
                assert_eq!(
                    board_settlements(fixture.root())?,
                    Vec::<ReservationId>::new()
                );
                record_exact_overlap_answer(&fixture, &ledger)?;
                fs::write(
                    fixture.foreign.join(UNRESERVED_PATH),
                    "change full revision\n",
                )?;

                let refusal = serde_json::to_value(edit_check(fixture.root(), HELD_PATH)?)?;
                assert_eq!(refusal["status"], "blocked_by_overlap", "{refusal}");
                assert_eq!(refusal["blocked_by"], fixture.holder);
                let before_full_observation = ledger.journal_events_for_test()?.len();
                board_settlements(fixture.root())?;
                let after_full = serde_json::to_value(edit_check(fixture.root(), HELD_PATH)?)?;
                assert_eq!(after_full["status"], refusal["status"]);
                assert_eq!(after_full["blocked_by"], refusal["blocked_by"]);
                assert!(ledger.journal_events_for_test()?.len() > before_full_observation);
                Ok(())
            },
        )
    }

    fn record_exact_overlap_answer(
        fixture: &EditCheckFixture,
        ledger: &Ledger,
    ) -> Result<(), Box<dyn Error>> {
        let scopes =
            DeclaredReservationScopeSet::parse(vec![PathBuf::from(HELD_PATH)], ScopeKind::File)?
                .into_minimal_antichain(crate::scope::PathCase::Sensitive);
        let overlaps = crate::answer::AuthorizedOverlapSet::try_from(vec![
            crate::answer::AuthorizedOverlap {
                reservation_id: fixture.holder_reservation_id()?,
                scope_revision: crate::answer::OverlapScopeRevision::from(&scopes),
                scopes:         scopes.clone().into(),
            },
        ])?;
        let answer = JournalOperation::Answer {
            reservation_id: None,
            scopes:         scopes.clone(),
            authorizations: vec![
                crate::answer::ConflictAuthorization::ExistingAnswersCoverEveryOverlap { overlaps },
            ],
        };
        let context = ledger::WorktreeContext::discover(fixture.root())?;
        let identity = ledger::resolve_identity(&context)?;
        let run = CoordinationRunId::new();
        let conflicts = || {
            ledger.with_validated_reservation_facts(|facts, _| {
                facts.map_err(Clone::clone).map(|facts| {
                    facts.conflicts_for_first_touch(
                        &scopes,
                        run,
                        identity.worktree_id,
                        crate::scope::PathCase::Sensitive,
                    )
                })
            })
        };
        let before = conflicts()??;
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].reservation_id, fixture.holder_reservation_id()?);
        let outcome = ledger.transact(identity.worktree_id, run, |_| {
            TransactionValidation::<()>::Append(Box::new(answer.clone()))
        })?;
        assert!(matches!(outcome, LedgerTransactionOutcome::Appended { .. }));
        assert!(
            conflicts()??.is_empty(),
            "the answer must cover the original extent"
        );
        Ok(())
    }

    fn board_settlements(root: &Path) -> Result<Vec<ReservationId>, Box<dyn Error>> {
        let Enrollment::Enrolled(report) =
            crate::reconcile::reconcile_for_board(root, RecoveredBypassReporting::Report)?
        else {
            return Err("expected an enrolled board".into());
        };
        Ok(report
            .settlements
            .into_iter()
            .map(|settlement| settlement.reservation_id)
            .collect())
    }

    fn run_edit_observation_test(
        name: &str,
        exercise: impl FnOnce() -> Result<(), Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        const CHILD_MODE: &str = "CARGO_BERTH_TEST_EDIT_OBSERVATION";
        if std::env::var_os(CHILD_MODE).is_some() {
            return exercise();
        }
        let output = cargo_berth_test_support::berth_command(std::env::current_exe()?)
            .args(["--exact", name, "--nocapture"])
            .env(CHILD_MODE, "1")
            .env_remove(HARNESS_SESSION_ENVIRONMENT)
            .env_remove("CARGO_BERTH_RUN")
            .bounded_output()?;
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    struct EditCheckFixture {
        directory:          TempDir,
        _foreign_directory: TempDir,
        foreign:            PathBuf,
        holder:             Value,
    }

    impl EditCheckFixture {
        fn new() -> Result<Self, Box<dyn Error>> {
            let directory = initialized_check_repository()?;
            let root = directory.path();

            let foreign_directory = tempfile::tempdir()?;
            let foreign = foreign_directory.path().join("holder");
            let foreign_argument = foreign.to_str().ok_or("worktree path must be UTF-8")?;
            run_git(
                root,
                &[
                    "worktree",
                    "add",
                    "--quiet",
                    "-b",
                    "holder",
                    foreign_argument,
                ],
            )?;
            let answer = edit_check(&foreign, HELD_PATH)?;
            assert_eq!(answer.exit_code(), BerthExit::Clear, "{answer:?}");
            std::fs::write(foreign.join(HELD_PATH), "holder edit\n")?;
            let holder = serde_json::to_value(answer)?["reservations"].clone();
            Ok(Self {
                directory,
                _foreign_directory: foreign_directory,
                foreign,
                holder,
            })
        }

        fn root(&self) -> &Path { self.directory.path() }

        fn integrate_holder(&self) -> Result<(), Box<dyn Error>> {
            assert_eq!(board_settlements(self.root())?, Vec::<ReservationId>::new());
            self.commit_holder()?;
            run_git(self.root(), &["merge", "--quiet", "--ff-only", "holder"])
        }

        fn set_reservation_limit_to_one(&self) -> Result<(), std::io::Error> {
            fs::write(
                self.root()
                    .join(crate::config::BerthConfig::relative_path()),
                "trunk = \"main\"\nmaximum_reservations = 1\n",
            )
        }

        fn checkpoint_holder(&self) -> Result<(), Box<dyn Error>> {
            self.commit_holder()?;
            let trunk_snapshot = RepositorySnapshot::discover(self.root())?
                .head_object_id()?
                .clone();
            let protected_tip = RepositorySnapshot::discover(&self.foreign)?
                .head_object_id()?
                .clone();
            let ledger = Ledger::open(self.root())?;
            let reservation_id = self.holder_reservation_id()?;
            let holder = ledger
                .read_validated_live_reservations()??
                .into_iter()
                .find(|reservation| reservation.id() == reservation_id)
                .ok_or("expected holder")?;
            let actor = holder.actor();
            let outcome = ledger.transact(actor.worktree, actor.run, |_| {
                TransactionValidation::<()>::Append(Box::new(JournalOperation::Checkpoint {
                    reservation_id,
                    protected_tip: protected_tip.clone().into(),
                    trunk_snapshot: trunk_snapshot.clone(),
                }))
            })?;
            assert!(matches!(outcome, LedgerTransactionOutcome::Appended { .. }));
            Ok(())
        }

        fn commit_holder(&self) -> Result<(), Box<dyn Error>> {
            run_git(
                &self.foreign,
                &[
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.com",
                    "commit",
                    "--quiet",
                    "-am",
                    "land holder work",
                ],
            )
        }

        fn holder_reservation_id(&self) -> Result<ReservationId, Box<dyn Error>> {
            Ok(self.holder[0]
                .as_str()
                .ok_or("expected holder ID")?
                .parse()?)
        }

        fn assert_holder_not_released(&self) -> Result<(), Box<dyn Error>> {
            let holder = self.holder_reservation_id()?;
            let events = Ledger::open(self.root())?.journal_events_for_test()?;
            assert!(
                events.iter().all(|event| {
                    !matches!(event.operation, JournalOperation::Release { reservation_id, .. } if reservation_id == holder)
                }),
                "the edit hook must keep the foreign holder"
            );
            Ok(())
        }
    }

    fn initialized_check_repository() -> Result<TempDir, Box<dyn Error>> {
        let directory =
            cargo_berth_test_support::copy_git_repository_seed("edit-check-base", |root| {
                run_git(root, &["init", "--quiet", "-b", "main"])?;
                run_git(root, &["config", "core.ignorecase", "false"])?;
                fs::write(root.join(HELD_PATH), "base\n")?;
                run_git(root, &["add", HELD_PATH])?;
                run_git(
                    root,
                    &[
                        "-c",
                        "user.name=Test",
                        "-c",
                        "user.email=test@example.com",
                        "commit",
                        "--quiet",
                        "-m",
                        "base",
                    ],
                )
            });
        Ledger::initialize(directory.path())?;
        Ok(directory)
    }

    fn edit_check(root: &Path, file: &str) -> Result<OutputEnvelope, Box<dyn Error>> {
        check_with_selection(root, file, CheckReservationSelection::EditHook)
    }

    fn check_with_selection(
        root: &Path,
        file: &str,
        reservation_selection: CheckReservationSelection,
    ) -> Result<OutputEnvelope, Box<dyn Error>> {
        let recovery = RecoveryCommandLine::try_from(
            ["cargo-berth", "check", "--path", file]
                .into_iter()
                .map(Into::into)
                .collect::<Vec<_>>(),
        )?;
        Ok(execute_at(
            root,
            CheckRequest {
                declared_scopes: DeclaredReservationScopeSet::parse(
                    vec![PathBuf::from(file)],
                    ScopeKind::File,
                )?,
                reservation_selection,
            },
            &recovery,
        ))
    }

    fn run_git(root: &Path, arguments: &[&str]) -> Result<(), Box<dyn Error>> {
        let status = cargo_berth_test_support::fixture_git_command()
            .args(arguments)
            .current_dir(root)
            .bounded_status()?;
        assert!(status.success(), "git {arguments:?} failed");
        Ok(())
    }

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
