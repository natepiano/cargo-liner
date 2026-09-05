//! The drift command: observe, classify under the ledger lock, and publish.

use std::convert::Infallible;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::io::ErrorKind;
use std::path::PathBuf;

use super::classification;
use super::classification::PreLockForeignPathClassification;
use super::classification::RefusedRunPathEntry;
use super::fingerprint;
use super::git_output::DriftFingerprintError;
use super::identity::DriftActingIdentity;
use super::identity::DriftRunValidation;
use super::identity::DriftScopeAcquisition;
use super::observation;
use super::observation::FingerprintObservation;
use super::observation::PostWriteClaimSubject;
use super::provenance;
use super::report::DriftPathAttributionOutcome;
use super::report::DriftReport;
use super::report::PostWriteFreePathProtection;
use super::report::ReservationDriftResult;
use super::report::UnattributedDriftPathSet;
use super::selection::DriftComparisonChoice;
use super::selection::DriftRequest;
use super::selection::DriftReservationSelection;
use super::selection::DriftSelectionError;
use super::selection::PostWriteFirstTouchRequirement;
use super::selection::ResolvedDriftSubjects;
use crate::config::Enrollment;
use crate::coordination_identity;
use crate::coordination_identity::CoordinationIdentityRejection;
use crate::coordination_identity::CoordinationIdentityValidationContext;
use crate::coordination_identity::CoordinationIdentityValidationError;
use crate::coordination_identity::IssuingWorktreeRun;
use crate::coordination_identity::RecoveryCommandLine;
use crate::edge::RepositoryTrunk;
use crate::git;
use crate::ids::WorktreeId;
use crate::ledger;
use crate::ledger::JournalEvent;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::ReconciliationValidation;
use crate::ledger::ResolvedEditAuthorization;
use crate::ledger::WorktreeContext;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::reconcile;
use crate::reconcile::ReconciledDriftPreflight;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::scope::DeclaredReservationScopeSet;
use crate::scope::DeclaredReservationScopeSetError;
use crate::scope::PathCase;
use crate::scope::PathCaseError;
use crate::scope::ReservationScopeSet;
use crate::verb::claim;
use crate::verb::claim::ClaimError;
use crate::verb::claim::FirstTouchClaimExecution;
use crate::verb::claim::FirstTouchClaimRequest;
use crate::verb::claim::FirstTouchConflictHandling;
use crate::verb::claim::FirstTouchConflictOutcome;

struct PostWritePathAttribution {
    outcome: DriftPathAttributionOutcome,
    results: Vec<ReservationDriftResult>,
}

enum PreparedDriftExecution {
    CompletedUnchanged(Box<DriftReport>),
    Observed(Box<ObservedDriftExecution>),
}

struct ObservedDriftExecution {
    initial_reservations:        RetainedReservationSet,
    initial_subjects:            ResolvedDriftSubjects,
    cache_path:                  PathBuf,
    observation:                 FingerprintObservation,
    acting_identity:             DriftActingIdentity,
    resolved_edit_authorization: ResolvedEditAuthorization,
    identity_validation:         CoordinationIdentityValidationContext,
}

enum DriftTransactionRejection {
    CoordinationIdentity(CoordinationIdentityValidationError),
    Replay(ReservationReplayError),
    Selection(DriftSelectionError),
}

struct DriftMutationContext<'observation> {
    request:                              DriftRequest,
    ledger:                               &'observation Ledger,
    worktree_context:                     &'observation WorktreeContext,
    acting_identity:                      DriftActingIdentity,
    resolved_edit_authorization:          ResolvedEditAuthorization,
    identity_validation:                  CoordinationIdentityValidationContext,
    path_case:                            PathCase,
    observation:                          &'observation FingerprintObservation,
    pre_lock_foreign_path_classification: &'observation PreLockForeignPathClassification,
}

/// Execute one cheap or full drift observation and reconcile any changed paths.
pub(crate) fn execute(
    request: DriftRequest,
    recovery_command_line: &RecoveryCommandLine,
) -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Drift, &error.to_string());
        },
    };
    let reconciled_drift_preflight = match reconcile::reconcile_for_drift(
        &invocation_directory,
        |worktree_context, _, events| {
            prepare_drift_execution(request, worktree_context, events, recovery_command_line)
        },
    ) {
        Ok(Enrollment::Enrolled(reconciled_drift_preflight)) => reconciled_drift_preflight,
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => {
            return OutputEnvelope::unconfigured(CommandVerb::Drift, &expected_configuration_path);
        },
        Err(error) => return error.into_output(CommandVerb::Drift),
    };
    let ReconciledDriftPreflight {
        report: reconciliation_report,
        worktree_context,
        ledger,
        observation,
    } = reconciled_drift_preflight;
    let repository_trunk = reconciliation_report.repository_trunk().clone();
    let output_envelope = match observation.and_then(|prepared| {
        execute_inner(
            request,
            &worktree_context,
            &ledger,
            prepared,
            &repository_trunk,
            recovery_command_line,
        )
    }) {
        Ok(Enrollment::Enrolled(report)) => OutputEnvelope::drift(report),
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => OutputEnvelope::unconfigured(CommandVerb::Drift, &expected_configuration_path),
        Err(DriftExecutionError::Selection(error)) => {
            OutputEnvelope::invalid_input(CommandVerb::Drift, &error.to_string())
        },
        Err(DriftExecutionError::PostWriteClaimRejected(rejection)) => {
            OutputEnvelope::invalid_input(CommandVerb::Drift, &rejection.to_string())
        },
        Err(DriftExecutionError::CoordinationIdentity(rejection)) => {
            OutputEnvelope::coordination_identity_rejected(CommandVerb::Drift, rejection)
        },
        Err(DriftExecutionError::Claim(error)) => error.into_output(CommandVerb::Drift),
        Err(DriftExecutionError::Transaction(LedgerTransactionError::LockContention)) => {
            OutputEnvelope::contention(
                CommandVerb::Drift,
                &LedgerTransactionError::LockContention.to_string(),
            )
        },
        Err(DriftExecutionError::Transaction(LedgerTransactionError::CorrectableInput(error))) => {
            OutputEnvelope::invalid_input(CommandVerb::Drift, &error.to_string())
        },
        Err(
            DriftExecutionError::Ledger(error)
            | DriftExecutionError::Transaction(LedgerTransactionError::LedgerUnreadable(error)),
        ) => OutputEnvelope::ledger_error(CommandVerb::Drift, &error),
        Err(DriftExecutionError::Replay(error)) => {
            OutputEnvelope::replay_failure(CommandVerb::Drift, &error)
        },
        Err(error) => OutputEnvelope::ledger_unreadable(CommandVerb::Drift, &error.to_string()),
    };
    output_envelope.with_alerts(reconciliation_report.alerts)
}

fn prepare_drift_execution(
    request: DriftRequest,
    worktree_context: &WorktreeContext,
    events: &[JournalEvent],
    recovery_command_line: &RecoveryCommandLine,
) -> Result<PreparedDriftExecution, DriftExecutionError> {
    let (resolved_edit_authorization, worktree_id) = match request.reservation {
        DriftReservationSelection::EveryActiveForPostCommit { .. } => {
            let worktree_id = match prepare_worktree_comparison(
                comparable_worktree(worktree_context, &request)?,
                request.comparison,
            ) {
                WorktreeComparisonReadiness::Ready(worktree_id) => worktree_id,
                WorktreeComparisonReadiness::CompletedUnchanged(report) => {
                    return Ok(PreparedDriftExecution::CompletedUnchanged(report));
                },
            };
            (ledger::resolve_identity(worktree_context)?, worktree_id)
        },
        DriftReservationSelection::Explicit(_)
        | DriftReservationSelection::SessionMappingOrSingleActive => {
            let resolved_edit_authorization = ledger::resolve_identity(worktree_context)?;
            let worktree_id = match prepare_worktree_comparison(
                comparable_worktree(worktree_context, &request)?,
                request.comparison,
            ) {
                WorktreeComparisonReadiness::Ready(worktree_id) => worktree_id,
                WorktreeComparisonReadiness::CompletedUnchanged(report) => {
                    return Ok(PreparedDriftExecution::CompletedUnchanged(report));
                },
            };
            (resolved_edit_authorization, worktree_id)
        },
    };
    let identity_validation = CoordinationIdentityValidationContext::for_user_command(
        resolved_edit_authorization,
        worktree_context,
        recovery_command_line,
    );
    let initial_reservations = RetainedReservationSet::replay(events)?;
    let acting_identity =
        validated_drift_identity(worktree_id, &initial_reservations, &identity_validation)?;
    let initial_subjects = request
        .reservation
        .resolve(&initial_reservations, acting_identity)?;
    let cache_path =
        fingerprint::fingerprint_cache_path(worktree_context.common_git_directory(), worktree_id);
    let observation = observation::observe(
        request.comparison,
        worktree_context.repository_root(),
        &initial_reservations,
        initial_subjects.reporting.as_slice(),
        &cache_path,
    )?;
    Ok(PreparedDriftExecution::Observed(Box::new(
        ObservedDriftExecution {
            initial_reservations,
            initial_subjects,
            cache_path,
            observation,
            acting_identity,
            resolved_edit_authorization,
            identity_validation,
        },
    )))
}

fn execute_inner(
    request: DriftRequest,
    worktree_context: &WorktreeContext,
    ledger: &Ledger,
    prepared: PreparedDriftExecution,
    repository_trunk: &RepositoryTrunk,
    recovery_command_line: &RecoveryCommandLine,
) -> Result<Enrollment<DriftReport>, DriftExecutionError> {
    let ObservedDriftExecution {
        initial_reservations,
        initial_subjects,
        cache_path,
        observation,
        acting_identity,
        resolved_edit_authorization,
        identity_validation,
    } = match prepared {
        PreparedDriftExecution::CompletedUnchanged(report) => {
            return Ok(Enrollment::Enrolled(*report));
        },
        PreparedDriftExecution::Observed(observed) => *observed,
    };
    if !observation
        .changes
        .has_changes_for(initial_subjects.reporting.as_slice())
    {
        if !initial_subjects.reporting.is_empty() {
            fingerprint::publish_fingerprint(&cache_path, &observation.cache_value);
        }
        let report = DriftReport::unchanged(
            observation.comparison,
            initial_subjects.reporting.as_slice(),
        );
        return Ok(Enrollment::Enrolled(report));
    }
    // This branch returns before the lock that decides refusal, and states `Permitted` without
    // asking. Two separate things make that right, and both are needed.
    //
    // The acquisition it performs is not unguarded: `claim_post_write_paths` goes through
    // `claim::acquire_first_touch`, which asks the same occupancy question through the same
    // `coordination_identity::validate_worktree_occupancy` from inside its own ledger
    // transaction. A second presented run reaching here is refused by the claim, not waved
    // through.
    //
    // The reported `Permitted` is a statement about this worktree, and it holds because
    // `reporting` is empty. Post-commit subject selection matches every `Active` reservation
    // whose actor worktree is this one, with the acting run deliberately dropped from the
    // filter (`PostCommitWideningSelection::resolve_post_commit`), so an empty reporting set
    // means no `Active` reservation exists here at all. Occupancy requires an `Active` holder,
    // so there is no incumbent for the question to find. Widening the reporting filter to ask
    // about the run would break that implication and this branch with it.
    if initial_subjects.reporting.is_empty()
        && matches!(
            initial_subjects.post_write_first_touch,
            PostWriteFirstTouchRequirement::Required
        )
    {
        let attribution = claim_post_write_paths(&observation, recovery_command_line)?;
        let report = DriftReport {
            comparison:        observation.comparison,
            path_attribution:  attribution.outcome,
            results:           attribution.results,
            scope_acquisition: DriftScopeAcquisition::Permitted,
        };
        if !report.has_blocking_effect() {
            fingerprint::publish_fingerprint(&cache_path, &observation.cache_value);
        }
        return Ok(Enrollment::Enrolled(report));
    }
    let path_case = PathCase::read(worktree_context.common_git_directory())?;
    let pre_lock_foreign_path_classification = PreLockForeignPathClassification::build(
        &initial_reservations,
        initial_subjects.reporting.as_slice(),
        &observation.changes,
        path_case,
    )?;
    let committed_history = provenance::read_committed_foreign_paths(
        worktree_context.repository_root(),
        &initial_reservations,
        &observation.changes,
        repository_trunk,
        &pre_lock_foreign_path_classification.committed_foreign_paths(),
    )?;
    let pre_lock_foreign_path_classification =
        pre_lock_foreign_path_classification.with_committed_history(committed_history);
    let mutation_context = DriftMutationContext {
        request,
        ledger,
        worktree_context,
        acting_identity,
        resolved_edit_authorization,
        identity_validation,
        path_case,
        observation: &observation,
        pre_lock_foreign_path_classification: &pre_lock_foreign_path_classification,
    };
    let mut report = transact_classification(&mutation_context)?;
    provenance::name_incursion_commits(
        &initial_reservations,
        &observation.changes,
        pre_lock_foreign_path_classification.committed_history(),
        &mut report,
    );
    // From here the refusal withholds every acquisition this invocation could still make: the
    // post-write first touch below, and the fingerprint publication that would move the
    // worktree's shared comparison baseline under the run that does occupy it. Nothing else
    // above is withheld, so the refused run's report states what its commit did before it
    // states that it may take nothing here.
    //
    // The unchanged early return sits outside that account entirely. An observation that finds
    // no change for any reporting subject publishes the fingerprint and returns above, before
    // the lock that decides refusal, so such a run is never refused and never told it was.
    let acquisition_permitted =
        matches!(report.scope_acquisition, DriftScopeAcquisition::Permitted);
    if acquisition_permitted
        && matches!(
            initial_subjects.post_write_first_touch,
            PostWriteFirstTouchRequirement::Required
        )
    {
        let attribution = claim_post_write_paths(&observation, recovery_command_line)?;
        report.path_attribution = attribution.outcome;
        report.results.extend(attribution.results);
    }
    if acquisition_permitted && !report.has_blocking_effect() {
        fingerprint::publish_fingerprint(&cache_path, &observation.cache_value);
    }
    Ok(Enrollment::Enrolled(report))
}

/// Refuse a drift invocation whose resolved identity is stale, then name what it acts as.
///
/// Only the staleness questions a session mapping or a worktree marker raises are answered
/// here. The same-worktree occupancy question belongs to the acquisition step and is asked
/// there, under the ledger lock, by [`DriftRunValidation::authorize_scope_acquisition`]:
/// answering it here aborts the invocation before [`observation::observe`] runs, which would
/// leave a second presented run's commits with no incursion record against any foreign holder
/// in the repository.
fn validated_drift_identity(
    worktree_id: WorktreeId,
    reservations: &RetainedReservationSet,
    identity_validation: &CoordinationIdentityValidationContext,
) -> Result<DriftActingIdentity, DriftExecutionError> {
    let resolved_edit_authorization = identity_validation.resolved_edit_authorization();
    if resolved_edit_authorization.worktree_id != worktree_id {
        return Err(DriftExecutionError::Ledger(
            LedgerError::WorktreeIdentityMismatch,
        ));
    }
    coordination_identity::validate_coordination_identity(reservations, identity_validation)?;
    Ok(DriftActingIdentity::resolve(
        resolved_edit_authorization,
        reservations,
    ))
}

/// Whether this run can compare the current worktree now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorktreeComparability {
    /// The worktree has an identity and git is not rewriting it.
    Comparable(WorktreeId),
    /// No identity has been recorded yet for the worktree.
    IdentityNotRecorded,
    /// Git is still replaying commits, so comparison waits for the final reference move.
    DeferredPendingRewrite,
}

/// Whether drift should observe the worktree or return its completed no-change report.
enum WorktreeComparisonReadiness {
    /// The worktree can proceed to fingerprint observation.
    Ready(WorktreeId),
    /// No worktree comparison ran and the empty unchanged report is complete.
    CompletedUnchanged(Box<DriftReport>),
}

/// Determine whether this run can compare the current worktree now.
///
/// A post-commit run stands aside for a worktree with no recorded identity, and every run
/// stands aside while git is still replaying commits onto a moved base: git runs
/// `post-commit` for each replayed commit, and nothing re-anchors the phase until the
/// branch reference moves at the end, so a comparison taken now reads the new base's
/// commits as this phase's work and acquires the paths they touch.
fn comparable_worktree(
    worktree_context: &WorktreeContext,
    request: &DriftRequest,
) -> Result<WorktreeComparability, DriftExecutionError> {
    let worktree_id =
        match ledger::read_worktree_identity(worktree_context.administrative_directory()) {
            Ok(worktree_id) => worktree_id,
            Err(LedgerError::Io(error))
                if matches!(
                    request.reservation,
                    DriftReservationSelection::EveryActiveForPostCommit { .. }
                ) && error.kind() == ErrorKind::NotFound =>
            {
                return Ok(WorktreeComparability::IdentityNotRecorded);
            },
            Err(error) => return Err(error.into()),
        };
    if git::rewrite_in_progress(worktree_context.administrative_directory()) {
        return Ok(WorktreeComparability::DeferredPendingRewrite);
    }
    Ok(WorktreeComparability::Comparable(worktree_id))
}

/// Convert worktree comparability into either observation readiness or a final report.
fn prepare_worktree_comparison(
    comparability: WorktreeComparability,
    comparison: DriftComparisonChoice,
) -> WorktreeComparisonReadiness {
    match comparability {
        WorktreeComparability::Comparable(worktree_id) => {
            WorktreeComparisonReadiness::Ready(worktree_id)
        },
        WorktreeComparability::IdentityNotRecorded
        | WorktreeComparability::DeferredPendingRewrite => {
            WorktreeComparisonReadiness::CompletedUnchanged(Box::new(DriftReport::unchanged(
                comparison.report_mode(),
                &[],
            )))
        },
    }
}

fn claim_post_write_paths(
    observation: &FingerprintObservation,
    recovery_command_line: &RecoveryCommandLine,
) -> Result<PostWritePathAttribution, DriftExecutionError> {
    let paths = match observation.post_write_claim_subject() {
        PostWriteClaimSubject::NoModifiedPath => {
            return Ok(PostWritePathAttribution {
                outcome: DriftPathAttributionOutcome::NotNeeded,
                results: Vec::new(),
            });
        },
        PostWriteClaimSubject::ModifiedPaths(paths) => paths,
    };
    let declared_scopes = DeclaredReservationScopeSet::from_file_paths(paths)
        .map_err(PostWriteClaimRejection::InvalidDeclaredScopes)
        .map_err(DriftExecutionError::PostWriteClaimRejected)?;
    let acquisition = claim::acquire_first_touch(
        FirstTouchClaimRequest {
            declared_scopes,
            conflict_handling: FirstTouchConflictHandling::ProtectFreePaths,
        },
        recovery_command_line,
    )?;
    match acquisition {
        Enrollment::Enrolled(FirstTouchClaimExecution::Acquired {
            acquisition,
            scopes,
            conflicts: FirstTouchConflictOutcome::None,
        }) => {
            let reservation_id = acquisition.reservation_id;
            Ok(PostWritePathAttribution {
                outcome: DriftPathAttributionOutcome::FirstTouchReserved {
                    acquisition,
                    scopes,
                },
                results: vec![ReservationDriftResult::Unchanged { reservation_id }],
            })
        },
        Enrollment::Enrolled(FirstTouchClaimExecution::Acquired {
            acquisition,
            scopes,
            conflicts:
                FirstTouchConflictOutcome::PostWriteIncursion {
                    scopes: conflicting_scopes,
                    conflicts,
                },
        }) => {
            let reservation_id = acquisition.reservation_id;
            Ok(PostWritePathAttribution {
                outcome: DriftPathAttributionOutcome::IncursionDetected {
                    paths: paths_from_scopes(&conflicting_scopes)?,
                    conflicts,
                    protection: PostWriteFreePathProtection::Acquired {
                        acquisition,
                        scopes,
                    },
                },
                results: vec![ReservationDriftResult::Unchanged { reservation_id }],
            })
        },
        Enrollment::Enrolled(FirstTouchClaimExecution::Blocked { scopes, conflicts }) => {
            Ok(PostWritePathAttribution {
                outcome: DriftPathAttributionOutcome::IncursionDetected {
                    paths: paths_from_scopes(&scopes)?,
                    conflicts,
                    protection: PostWriteFreePathProtection::NotAcquired,
                },
                results: Vec::new(),
            })
        },
        Enrollment::Enrolled(FirstTouchClaimExecution::ReservationLimitReached(maximum)) => {
            Err(DriftExecutionError::PostWriteClaimRejected(
                PostWriteClaimRejection::ReservationLimitReached(maximum),
            ))
        },
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => Err(DriftExecutionError::PostWriteClaimRejected(
            PostWriteClaimRejection::EnrollmentDisappeared(expected_configuration_path),
        )),
    }
}

fn paths_from_scopes(
    scopes: &ReservationScopeSet,
) -> Result<UnattributedDriftPathSet, DriftExecutionError> {
    UnattributedDriftPathSet::try_from(
        scopes
            .as_slice()
            .iter()
            .map(|scope| scope.path.clone())
            .collect::<Vec<_>>(),
    )
    .map_err(|_| {
        DriftExecutionError::PostWriteClaimRejected(PostWriteClaimRejection::MissingChangedPath)
    })
}

/// Classify the observation under the ledger lock, refusing acquisition but not observation.
fn transact_classification(
    context: &DriftMutationContext<'_>,
) -> Result<DriftReport, DriftExecutionError> {
    let actor_run = context
        .acting_identity
        .run_for_mutation(context.request.reservation)?
        .into_coordination_run_id();
    let journal_mutation_actor = context
        .resolved_edit_authorization
        .journal_mutation_actor_for(actor_run);
    let outcome = context.ledger.transact_reconciliation(
        journal_mutation_actor.worktree_id,
        journal_mutation_actor.coordination_run_id,
        |state| {
            let reservations = match RetainedReservationSet::replay(state.events()) {
                Ok(reservations) => reservations,
                Err(error) => {
                    return ReconciliationValidation::Reject(DriftTransactionRejection::Replay(
                        error,
                    ));
                },
            };
            let acquisition = match DriftRunValidation::resolve(context.resolved_edit_authorization)
                .authorize_scope_acquisition(
                    &reservations,
                    context.worktree_context,
                    &context.identity_validation,
                ) {
                Ok(acquisition) => acquisition,
                Err(error) => {
                    return ReconciliationValidation::Reject(
                        DriftTransactionRejection::CoordinationIdentity(error),
                    );
                },
            };
            let subjects = match context
                .request
                .reservation
                .resolve(&reservations, context.acting_identity)
            {
                Ok(subject_ids) => subject_ids,
                Err(error) => {
                    return ReconciliationValidation::Reject(DriftTransactionRejection::Selection(
                        error,
                    ));
                },
            };
            match classification::classify_locked(
                &reservations,
                &subjects,
                &context.observation.changes,
                context.pre_lock_foreign_path_classification,
                context.path_case,
                context.observation.comparison,
                acquisition.widening_authorization(),
            ) {
                Ok(decision) => {
                    let mut report = decision.report;
                    // A refused run holds nothing here, so no subject can carry its entry into
                    // the incumbent's own scopes: the incumbent is the subject, and a
                    // reservation never enters its own scopes. The same locked reservation set
                    // that answered the occupancy question answers this one.
                    if let DriftScopeAcquisition::RefusedToSecondRun { .. } = acquisition
                        && let RefusedRunPathEntry::HoldersEntered(attribution) =
                            classification::attribute_refused_run_entry(
                                &reservations,
                                &context.observation.changes,
                                IssuingWorktreeRun {
                                    coordination_run_id: actor_run,
                                    worktree_id:         journal_mutation_actor.worktree_id,
                                },
                                context.path_case,
                            )
                    {
                        report.path_attribution = attribution;
                    }
                    report.scope_acquisition = acquisition;
                    ReconciliationValidation::Apply {
                        operations:             decision.operations,
                        recoverable_operations: Vec::new(),
                        action:                 report,
                    }
                },
                Err(error) => {
                    ReconciliationValidation::Reject(DriftTransactionRejection::Replay(error))
                },
            }
        },
        |report, _, _| Ok::<DriftReport, Infallible>(report),
    );
    match outcome {
        Ok(LedgerCommittedActionOutcome::Appended { output: report, .. }) => Ok(report),
        Ok(LedgerCommittedActionOutcome::Rejected(
            DriftTransactionRejection::CoordinationIdentity(error),
        )) => Err(error.into()),
        Ok(LedgerCommittedActionOutcome::Rejected(DriftTransactionRejection::Replay(error))) => {
            Err(DriftExecutionError::Replay(error))
        },
        Ok(LedgerCommittedActionOutcome::Rejected(DriftTransactionRejection::Selection(error))) => {
            Err(DriftExecutionError::Selection(error))
        },
        Err(LedgerCommittedActionError::Transaction(error)) => {
            Err(DriftExecutionError::Transaction(error))
        },
        Err(LedgerCommittedActionError::Action(error)) => match error {},
    }
}

/// A drift command failed before it could publish a coherent result.
#[derive(Debug)]
enum DriftExecutionError {
    Io(std::io::Error),
    Ledger(LedgerError),
    Replay(ReservationReplayError),
    Selection(DriftSelectionError),
    Fingerprint(DriftFingerprintError),
    PathCase(PathCaseError),
    Transaction(LedgerTransactionError),
    Claim(ClaimError),
    CoordinationIdentity(CoordinationIdentityRejection),
    PostWriteClaimRejected(PostWriteClaimRejection),
}

#[derive(Debug)]
enum PostWriteClaimRejection {
    InvalidDeclaredScopes(DeclaredReservationScopeSetError),
    ReservationLimitReached(u32),
    EnrollmentDisappeared(PathBuf),
    MissingChangedPath,
}

impl Display for DriftExecutionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Ledger(error) => error.fmt(formatter),
            Self::Replay(error) => error.fmt(formatter),
            Self::Selection(error) => error.fmt(formatter),
            Self::Fingerprint(error) => error.fmt(formatter),
            Self::PathCase(error) => error.fmt(formatter),
            Self::Transaction(error) => error.fmt(formatter),
            Self::Claim(error) => error.fmt(formatter),
            Self::CoordinationIdentity(rejection) => rejection.fmt(formatter),
            Self::PostWriteClaimRejected(rejection) => rejection.fmt(formatter),
        }
    }
}

impl Display for PostWriteClaimRejection {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDeclaredScopes(error) => error.fmt(formatter),
            Self::ReservationLimitReached(maximum) => write!(
                formatter,
                "repository policy permits at most {maximum} live reservations"
            ),
            Self::EnrollmentDisappeared(expected_configuration_path) => write!(
                formatter,
                "repository enrollment disappeared while claiming; expected {}",
                expected_configuration_path.display()
            ),
            Self::MissingChangedPath => {
                formatter.write_str("the post-write claim had no changed path")
            },
        }
    }
}

impl std::error::Error for DriftExecutionError {}

impl From<std::io::Error> for DriftExecutionError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

impl From<LedgerError> for DriftExecutionError {
    fn from(error: LedgerError) -> Self { Self::Ledger(error) }
}

impl From<ReservationReplayError> for DriftExecutionError {
    fn from(error: ReservationReplayError) -> Self { Self::Replay(error) }
}

impl From<DriftSelectionError> for DriftExecutionError {
    fn from(error: DriftSelectionError) -> Self { Self::Selection(error) }
}

impl From<DriftFingerprintError> for DriftExecutionError {
    fn from(error: DriftFingerprintError) -> Self { Self::Fingerprint(error) }
}

impl From<PathCaseError> for DriftExecutionError {
    fn from(error: PathCaseError) -> Self { Self::PathCase(error) }
}

impl From<ClaimError> for DriftExecutionError {
    fn from(error: ClaimError) -> Self { Self::Claim(error) }
}

impl From<CoordinationIdentityValidationError> for DriftExecutionError {
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

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected fixture and preparation states"
)]
mod tests {
    use std::fs;
    use std::io::ErrorKind;
    use std::path::PathBuf;

    use serde_json::Value;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::PreparedDriftExecution;
    use super::prepare_drift_execution;
    use crate::coordination_identity::RecoveryCommandLine;
    use crate::drift::constants::DRIFT_CACHE_FILE_PREFIX;
    use crate::drift::identity::DriftScopeAcquisition;
    use crate::drift::report::DriftComparisonMode;
    use crate::drift::report::DriftPathAttributionOutcome;
    use crate::drift::report::DriftReport;
    use crate::drift::selection::DriftComparisonChoice;
    use crate::drift::selection::DriftRequest;
    use crate::drift::selection::DriftReservationSelection;
    use crate::drift::selection::PostCommitWideningSelection;
    use crate::ledger;
    use crate::ledger::LedgerError;
    use crate::ledger::WorktreeContext;
    use crate::output::OutputEnvelope;

    struct WorktreeComparisonFixture {
        repository:       TempDir,
        worktree_context: WorktreeContext,
    }

    impl WorktreeComparisonFixture {
        fn new() -> Self {
            let repository = tempdir().expect("temporary repository should exist");
            fs::create_dir(repository.path().join(".git"))
                .expect("git administrative directory should exist");
            let worktree_context = WorktreeContext::discover(repository.path())
                .expect("worktree should be discovered");
            Self {
                repository,
                worktree_context,
            }
        }

        fn fingerprint_cache_paths(&self) -> Vec<PathBuf> {
            let cache_directory = self.repository.path().join(".git/cargo-berth");
            fs::read_dir(cache_directory).map_or_else(
                |_| Vec::new(),
                |entries| {
                    entries
                        .filter_map(Result::ok)
                        .filter(|entry| {
                            entry
                                .file_name()
                                .to_str()
                                .is_some_and(|name| name.starts_with(DRIFT_CACHE_FILE_PREFIX))
                        })
                        .map(|entry| entry.path())
                        .collect()
                },
            )
        }

        fn assert_worktree_identity_absent(&self) {
            let error =
                ledger::read_worktree_identity(self.worktree_context.administrative_directory())
                    .expect_err("worktree identity file should remain absent");
            let LedgerError::Io(error) = error else {
                panic!("missing worktree identity should report a filesystem error");
            };
            assert_eq!(error.kind(), ErrorKind::NotFound);
        }
    }

    #[test]
    fn drift_reports_no_change_when_worktree_identity_is_not_recorded() {
        let fixture = WorktreeComparisonFixture::new();
        let request = post_commit_request();

        fixture.assert_worktree_identity_absent();
        let prepared = prepare_drift_execution(
            request,
            &fixture.worktree_context,
            &[],
            &RecoveryCommandLine::current_process(),
        )
        .expect("missing post-commit identity should be accepted");

        fixture.assert_worktree_identity_absent();
        assert!(fixture.fingerprint_cache_paths().is_empty());
        assert_unchanged_empty_report(prepared);
        assert!(fixture.fingerprint_cache_paths().is_empty());
    }

    #[test]
    fn drift_reports_no_change_while_a_rewrite_is_pending() {
        let fixture = WorktreeComparisonFixture::new();
        ledger::resolve_identity(&fixture.worktree_context)
            .expect("worktree identity should be available");
        fs::create_dir(
            fixture
                .worktree_context
                .administrative_directory()
                .join("rebase-merge"),
        )
        .expect("rewrite marker should exist");
        let request = post_commit_request();

        let prepared = prepare_drift_execution(
            request,
            &fixture.worktree_context,
            &[],
            &RecoveryCommandLine::current_process(),
        )
        .expect("pending rewrite should defer comparison");

        assert!(fixture.fingerprint_cache_paths().is_empty());
        assert_unchanged_empty_report(prepared);
        assert!(fixture.fingerprint_cache_paths().is_empty());
    }

    fn post_commit_request() -> DriftRequest {
        DriftRequest {
            comparison:  DriftComparisonChoice::FullPhaseStart,
            reservation: DriftReservationSelection::EveryActiveForPostCommit {
                widening: PostCommitWideningSelection::SessionMappingOrSingleCandidate,
            },
        }
    }

    fn assert_unchanged_empty_report(prepared: PreparedDriftExecution) {
        let PreparedDriftExecution::CompletedUnchanged(report) = prepared else {
            panic!("non-comparable worktree should complete without observation");
        };
        let report = *report;
        assert_eq!(
            report,
            DriftReport {
                comparison:        DriftComparisonMode::FullPhaseStart,
                path_attribution:  DriftPathAttributionOutcome::NotNeeded,
                results:           Vec::new(),
                scope_acquisition: DriftScopeAcquisition::Permitted,
            }
        );
        assert!(!report.has_reportable_effect());
        assert!(!report.has_blocking_effect());

        let output_envelope = serde_json::to_value(OutputEnvelope::drift(report))
            .expect("drift output should serialize");
        assert_eq!(output_envelope["status"], "clear");
        assert_eq!(output_envelope["exit_code"], 0);
        assert_eq!(output_envelope["reservations"], Value::Array(Vec::new()));
        assert_eq!(output_envelope["blocked_by"], Value::Array(Vec::new()));
        assert_eq!(output_envelope["payload"]["kind"], "drift");
        assert_eq!(
            output_envelope["payload"]["data"]["widening"]["status"],
            "not_needed"
        );
        assert_eq!(
            output_envelope["payload"]["data"]["results"],
            Value::Array(Vec::new())
        );
    }
}
