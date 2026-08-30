//! The drift command: observe, classify under the ledger lock, and publish.

use std::convert::Infallible;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::PathBuf;

use super::classification;
use super::classification::PriorClassification;
use super::fingerprint;
use super::git_output::DriftFingerprintError;
use super::identity::DriftActingIdentity;
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
use crate::coordination_identity::CoordinationIdentityRejection;
use crate::coordination_identity::CoordinationIdentityValidationContext;
use crate::coordination_identity::CoordinationIdentityValidationError;
use crate::coordination_identity::RecoveryCommandLine;
use crate::coordination_identity::validate_coordination_identity;
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
    NothingToCompare { comparison: DriftComparisonChoice },
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
    request:                     DriftRequest,
    ledger:                      &'observation Ledger,
    acting_identity:             DriftActingIdentity,
    resolved_edit_authorization: ResolvedEditAuthorization,
    identity_validation:         CoordinationIdentityValidationContext,
    path_case:                   PathCase,
    observation:                 &'observation FingerprintObservation,
    prior_classification:        &'observation PriorClassification,
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
    let resolved_edit_authorization = ledger::resolve_identity(worktree_context)?;
    let identity_validation = CoordinationIdentityValidationContext::for_user_command(
        resolved_edit_authorization,
        worktree_context,
        recovery_command_line,
    );
    let Some(worktree_id) = comparable_worktree(worktree_context, &request)? else {
        return Ok(PreparedDriftExecution::NothingToCompare {
            comparison: request.comparison,
        });
    };
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
        &initial_subjects.reporting,
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
    let (
        initial_reservations,
        initial_subjects,
        cache_path,
        observation,
        acting_identity,
        resolved_edit_authorization,
        identity_validation,
    ) = match prepared {
        PreparedDriftExecution::NothingToCompare { comparison } => {
            return Ok(nothing_to_compare(comparison));
        },
        PreparedDriftExecution::Observed(observed) => {
            let ObservedDriftExecution {
                initial_reservations,
                initial_subjects,
                cache_path,
                observation,
                acting_identity,
                resolved_edit_authorization,
                identity_validation,
            } = *observed;
            (
                initial_reservations,
                initial_subjects,
                cache_path,
                observation,
                acting_identity,
                resolved_edit_authorization,
                identity_validation,
            )
        },
    };
    if !observation
        .changes
        .has_changes_for(&initial_subjects.reporting)
    {
        if !initial_subjects.reporting.is_empty() {
            fingerprint::publish_fingerprint(&cache_path, &observation.cache_value);
        }
        let report = DriftReport::unchanged(observation.comparison, &initial_subjects.reporting);
        return Ok(Enrollment::Enrolled(report));
    }
    if initial_subjects.reporting.is_empty()
        && matches!(
            initial_subjects.post_write_first_touch,
            PostWriteFirstTouchRequirement::Required
        )
    {
        let attribution = claim_post_write_paths(&observation, recovery_command_line)?;
        let report = DriftReport {
            comparison:       observation.comparison,
            path_attribution: attribution.outcome,
            results:          attribution.results,
        };
        if !report.has_blocking_effect() {
            fingerprint::publish_fingerprint(&cache_path, &observation.cache_value);
        }
        return Ok(Enrollment::Enrolled(report));
    }
    let path_case = PathCase::read(worktree_context.common_git_directory())?;
    let prior_classification = PriorClassification::build(
        &initial_reservations,
        &initial_subjects.reporting,
        &observation.changes,
        path_case,
    )?;
    let mutation_context = DriftMutationContext {
        request,
        ledger,
        acting_identity,
        resolved_edit_authorization,
        identity_validation,
        path_case,
        observation: &observation,
        prior_classification: &prior_classification,
    };
    let mut report = transact_classification(&mutation_context)?;
    provenance::name_incursion_commits(
        worktree_context.repository_root(),
        &initial_reservations,
        &observation.changes,
        repository_trunk,
        &mut report,
    )?;
    if matches!(
        initial_subjects.post_write_first_touch,
        PostWriteFirstTouchRequirement::Required
    ) {
        let attribution = claim_post_write_paths(&observation, recovery_command_line)?;
        report.path_attribution = attribution.outcome;
        report.results.extend(attribution.results);
    }
    if !report.has_blocking_effect() {
        fingerprint::publish_fingerprint(&cache_path, &observation.cache_value);
    }
    Ok(Enrollment::Enrolled(report))
}

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
    validate_coordination_identity(reservations, identity_validation).map_err(
        |error| match error {
            CoordinationIdentityValidationError::Rejected(rejection) => {
                DriftExecutionError::CoordinationIdentity(rejection)
            },
            CoordinationIdentityValidationError::InvalidCanonicalWorktreeRoot => {
                DriftExecutionError::Ledger(LedgerError::InvalidCanonicalWorktreeRoot)
            },
        },
    )?;
    Ok(DriftActingIdentity::resolve(
        resolved_edit_authorization,
        reservations,
    ))
}

/// The worktree this run reports for, or nothing when it has no comparison to make.
///
/// A post-commit run stands aside for a worktree with no recorded identity, and every run
/// stands aside while git is still replaying commits onto a moved base: git runs
/// `post-commit` for each replayed commit, and nothing re-anchors the phase until the
/// branch reference moves at the end, so a comparison taken now reads the new base's
/// commits as this phase's work and acquires the paths they touch.
fn comparable_worktree(
    worktree_context: &WorktreeContext,
    request: &DriftRequest,
) -> Result<Option<WorktreeId>, DriftExecutionError> {
    let worktree_id =
        match ledger::read_worktree_identity(worktree_context.administrative_directory()) {
            Ok(worktree_id) => worktree_id,
            Err(_)
                if matches!(
                    request.reservation,
                    DriftReservationSelection::EveryActiveForPostCommit { .. }
                ) =>
            {
                return Ok(None);
            },
            Err(error) => return Err(DriftExecutionError::Ledger(error)),
        };
    if git::rewrite_in_progress(worktree_context.administrative_directory()) {
        return Ok(None);
    }
    Ok(Some(worktree_id))
}

/// The report to give when no subject has a comparison worth making.
fn nothing_to_compare(comparison: DriftComparisonChoice) -> Enrollment<DriftReport> {
    Enrollment::Enrolled(DriftReport::unchanged(comparison.report_mode(), &[]))
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
            if let Err(error) =
                validate_coordination_identity(&reservations, &context.identity_validation)
            {
                return ReconciliationValidation::Reject(
                    DriftTransactionRejection::CoordinationIdentity(error),
                );
            }
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
                context.prior_classification,
                context.path_case,
                context.observation.comparison,
            ) {
                Ok(decision) => ReconciliationValidation::Apply {
                    operations:             decision.operations,
                    recoverable_operations: Vec::new(),
                    action:                 decision.report,
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
        )) => match error {
            CoordinationIdentityValidationError::Rejected(rejection) => {
                Err(DriftExecutionError::CoordinationIdentity(rejection))
            },
            CoordinationIdentityValidationError::InvalidCanonicalWorktreeRoot => Err(
                DriftExecutionError::Ledger(LedgerError::InvalidCanonicalWorktreeRoot),
            ),
        },
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
