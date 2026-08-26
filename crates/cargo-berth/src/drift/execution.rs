//! The drift command: observe, classify under the ledger lock, and publish.

use std::convert::Infallible;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;

use super::classification;
use super::classification::PriorClassification;
use super::fingerprint;
use super::git_output::DriftFingerprintError;
use super::identity::DriftActingIdentity;
use super::observation;
use super::observation::FingerprintObservation;
use super::observation::PostWriteClaimSubject;
use super::report::DriftPathAttributionOutcome;
use super::report::DriftReport;
use super::report::PostWriteFreePathProtection;
use super::report::ReservationDriftResult;
use super::report::UnattributedDriftPathSet;
use super::selection::DriftRequest;
use super::selection::DriftReservationSelection;
use super::selection::DriftSelectionError;
use super::selection::PostWriteFirstTouchRequirement;
use crate::config::Enrollment;
use crate::ids::WorktreeId;
use crate::ledger;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::ReconciliationValidation;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::reconcile;
use crate::reconcile::RecoveredBypassReporting;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::scope::DeclaredReservationScopeSet;
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

enum DriftTransactionRejection {
    Replay(ReservationReplayError),
    Selection(DriftSelectionError),
}

struct DriftMutationContext<'observation> {
    request:              DriftRequest,
    repository_root:      &'observation Path,
    acting_identity:      DriftActingIdentity,
    worktree_id:          WorktreeId,
    path_case:            PathCase,
    observation:          &'observation FingerprintObservation,
    prior_classification: &'observation PriorClassification,
}

/// Execute one cheap or full drift observation and reconcile any changed paths.
pub(crate) fn execute(request: DriftRequest) -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Drift, &error.to_string());
        },
    };
    let reconciliation_report =
        match reconcile::reconcile(&invocation_directory, RecoveredBypassReporting::Defer) {
            Ok(Enrollment::Enrolled(reconciliation_report)) => reconciliation_report,
            Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            }) => {
                return OutputEnvelope::unconfigured(
                    CommandVerb::Drift,
                    &expected_configuration_path,
                );
            },
            Err(error) => return error.into_output(CommandVerb::Drift),
        };
    let output_envelope = match execute_inner(request, &invocation_directory) {
        Ok(Enrollment::Enrolled(report)) => OutputEnvelope::drift(report),
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => OutputEnvelope::unconfigured(CommandVerb::Drift, &expected_configuration_path),
        Err(DriftExecutionError::Selection(error)) => {
            OutputEnvelope::invalid_input(CommandVerb::Drift, &error.to_string())
        },
        Err(DriftExecutionError::ClaimRejected(diagnostic)) => {
            OutputEnvelope::invalid_input(CommandVerb::Drift, &diagnostic)
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
        Err(error) => OutputEnvelope::ledger_unreadable(CommandVerb::Drift, &error.to_string()),
    };
    output_envelope.with_alerts(reconciliation_report.alerts)
}

fn execute_inner(
    request: DriftRequest,
    invocation_directory: &Path,
) -> Result<Enrollment<DriftReport>, DriftExecutionError> {
    let initial_snapshot = match Ledger::read_for_edit_check(invocation_directory)? {
        Enrollment::Enrolled(initial_snapshot) => initial_snapshot,
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => {
            return Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            });
        },
    };
    let worktree_context = initial_snapshot.worktree_context().clone();
    let worktree_id =
        match ledger::read_worktree_identity(worktree_context.administrative_directory()) {
            Ok(worktree_id) => worktree_id,
            Err(_)
                if matches!(
                    request.reservation,
                    DriftReservationSelection::EveryActiveForPostCommit { .. }
                ) =>
            {
                return Ok(Enrollment::Enrolled(DriftReport::unchanged(
                    request.comparison.report_mode(),
                    &[],
                )));
            },
            Err(error) => return Err(DriftExecutionError::Ledger(error)),
        };
    let initial_reservations = RetainedReservationSet::replay(initial_snapshot.events())?;
    let acting_identity =
        DriftActingIdentity::resolve(&worktree_context, worktree_id, &initial_reservations);
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
        let attribution = claim_post_write_paths(&observation)?;
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
        repository_root: worktree_context.repository_root(),
        acting_identity,
        worktree_id,
        path_case,
        observation: &observation,
        prior_classification: &prior_classification,
    };
    let mut report = transact_classification(&mutation_context)?;
    if matches!(
        initial_subjects.post_write_first_touch,
        PostWriteFirstTouchRequirement::Required
    ) {
        let attribution = claim_post_write_paths(&observation)?;
        report.path_attribution = attribution.outcome;
        report.results.extend(attribution.results);
    }
    if !report.has_blocking_effect() {
        fingerprint::publish_fingerprint(&cache_path, &observation.cache_value);
    }
    Ok(Enrollment::Enrolled(report))
}

fn claim_post_write_paths(
    observation: &FingerprintObservation,
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
        .map_err(|error| DriftExecutionError::ClaimRejected(error.to_string()))?;
    let acquisition = claim::acquire_first_touch(FirstTouchClaimRequest {
        declared_scopes,
        conflict_handling: FirstTouchConflictHandling::ProtectFreePaths,
    })?;
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
            Err(DriftExecutionError::ClaimRejected(format!(
                "repository policy permits at most {maximum} live reservations"
            )))
        },
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => Err(DriftExecutionError::ClaimRejected(format!(
            "repository enrollment disappeared while claiming; expected {}",
            expected_configuration_path.display()
        ))),
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
        DriftExecutionError::ClaimRejected("the post-write claim had no changed path".to_owned())
    })
}

fn transact_classification(
    context: &DriftMutationContext<'_>,
) -> Result<DriftReport, DriftExecutionError> {
    let ledger = Ledger::open(context.repository_root)?;
    let actor_run = context
        .acting_identity
        .run_for_mutation(context.request.reservation)?
        .into_coordination_run_id();
    let outcome = ledger.transact_reconciliation(
        context.worktree_id,
        actor_run,
        |state| {
            let reservations = match RetainedReservationSet::replay(state.events()) {
                Ok(reservations) => reservations,
                Err(error) => {
                    return ReconciliationValidation::Reject(DriftTransactionRejection::Replay(
                        error,
                    ));
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
    ClaimRejected(String),
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
            Self::ClaimRejected(diagnostic) => formatter.write_str(diagnostic),
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
