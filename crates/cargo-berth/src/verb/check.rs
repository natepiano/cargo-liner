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
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
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
        .with_validated_reservations(Clone::clone)
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
    use crate::output::OutputEnvelope;
    use crate::reservation::MergeExtent;
    use crate::reservation::ReservationRunStatus;
    use crate::scope::DeclaredReservationScopeSet;

    const TEST_COMPACTION_THRESHOLD_BYTES: u64 = 1;
    const HELD_PATH: &str = "held.rs";
    const UNRESERVED_PATH: &str = "new.rs";

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

    struct EditCheckFixture {
        _directory: TempDir,
        root:       PathBuf,
        holder:     Value,
    }

    impl EditCheckFixture {
        fn new() -> Result<Self, Box<dyn Error>> {
            let directory = tempfile::tempdir()?;
            let root = directory.path().join("main");
            std::fs::create_dir(&root)?;
            run_git(&root, &["init", "--quiet", "-b", "main"])?;
            std::fs::write(root.join(HELD_PATH), "base\n")?;
            run_git(&root, &["add", HELD_PATH])?;
            run_git(
                &root,
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
            )?;
            Ledger::initialize(&root)?;

            let foreign = directory.path().join("holder");
            let foreign_argument = foreign.to_str().ok_or("worktree path must be UTF-8")?;
            run_git(
                &root,
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
                _directory: directory,
                root,
                holder,
            })
        }

        fn root(&self) -> &Path { &self.root }
    }

    fn edit_check(root: &Path, file: &str) -> Result<OutputEnvelope, Box<dyn Error>> {
        let recovery = RecoveryCommandLine::try_from(
            ["cargo-berth", "check", "--path", file]
                .into_iter()
                .map(Into::into)
                .collect::<Vec<_>>(),
        )?;
        Ok(execute_at(
            root,
            CheckRequest {
                declared_scopes:       DeclaredReservationScopeSet::parse(
                    vec![PathBuf::from(file)],
                    ScopeKind::File,
                )?,
                reservation_selection: CheckReservationSelection::EditHook,
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
