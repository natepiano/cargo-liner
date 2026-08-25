//! Working-tree drift observation and locked reservation reconciliation.

use std::collections::HashMap;
use std::collections::HashSet;
use std::convert::Infallible;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::str::FromStr;

use serde::Deserialize;
use serde::Serialize;

use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;
use crate::ids::WorktreeId;
use crate::ledger;
use crate::ledger::CollisionPathSet;
use crate::ledger::EditAuthorization;
use crate::ledger::ForeignReservationIdSet;
use crate::ledger::IncursionIncidentId;
use crate::ledger::IncursionPathSet;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::ReconciliationValidation;
use crate::ledger::ReservationScopeAdditionSet;
use crate::ledger::WidenCause;
use crate::ledger::WorktreeContext;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::reconcile;
use crate::reservation::AuthorizedEditingIdentity;
use crate::reservation::DriftBlockingCoverage;
use crate::reservation::IncursionObservation;
use crate::reservation::Reservation;
use crate::reservation::ReservationLifecycle;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::reservation::WidenScopeBinding;
use crate::scope::PathCase;
use crate::scope::ReservationScope;
use crate::scope::ReservationScopeSet;
use crate::scope::ScopeKind;

const DRIFT_CACHE_FILE_PREFIX: &str = "drift-fingerprint-";
const DRIFT_CACHE_FILE_SUFFIX: &str = ".json";
const GIT_BINARY: &str = "git";
const GIT_CACHED_ARGUMENT: &str = "--cached";
const GIT_DIFF_COMMAND: &str = "diff";
const GIT_EXCLUDE_STANDARD_ARGUMENT: &str = "--exclude-standard";
const GIT_HEAD_REVISION: &str = "HEAD";
const GIT_LS_FILES_COMMAND: &str = "ls-files";
const GIT_NAME_STATUS_ARGUMENT: &str = "--name-status";
const GIT_NO_OPTIONAL_LOCKS_ARGUMENT: &str = "--no-optional-locks";
const GIT_NO_RENAMES_ARGUMENT: &str = "--no-renames";
const GIT_NUL_TERMINATED_ARGUMENT: &str = "-z";
const GIT_OTHERS_ARGUMENT: &str = "--others";
const GIT_PORCELAIN_ARGUMENT: &str = "--porcelain";
const GIT_STATUS_COMMAND: &str = "status";

/// Which working-tree comparison the caller requested.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DriftComparisonChoice {
    /// Compare the cheap working-tree observation with the last cache entry.
    CheapDelta,
    /// Compare all active-phase changes with the claim's protected starting commit.
    FullPhaseStart,
}

/// How a hand-run or hook-run drift command chooses its reservation subjects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DriftReservationSelection {
    /// Act on the caller-supplied reservation.
    Explicit(ReservationId),
    /// Prefer the session-mapped reservation, otherwise require one active match.
    SessionMappingOrSingleActive,
    /// Report across every local reservation while attributing widening separately.
    EveryActiveForPostCommit {
        /// How an explicit flag or implicit identity selects the widening target.
        widening: PostCommitWideningSelection,
    },
}

/// How a post-commit invocation chooses its one possible widening target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PostCommitWideningSelection {
    /// Widen only the caller-supplied active reservation.
    Explicit(ReservationId),
    /// Prefer the session mapping, then the only active local candidate.
    SessionMappingOrSingleCandidate,
}

/// A drift request after clap primitives have been converted into domain choices.
#[derive(Clone, Copy)]
pub(crate) struct DriftRequest {
    /// The comparison algorithm selected at the command boundary.
    pub(crate) comparison:  DriftComparisonChoice,
    /// The semantic reservation-selection rule.
    pub(crate) reservation: DriftReservationSelection,
}

/// The comparison algorithm that actually produced one report.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DriftComparisonMode {
    /// A valid cache enabled the two-command delta.
    CheapDelta,
    /// The caller selected the complete phase-start comparison.
    FullPhaseStart,
    /// An absent or unreadable cache required the complete comparison.
    FullPhaseStartFallback,
}

/// One complete drift report, possibly covering several commit-hook subjects.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct DriftReport {
    /// The comparison that actually ran.
    pub(crate) comparison: DriftComparisonMode,
    /// How any unclaimed paths were or must be attributed.
    pub(crate) widening:   DriftWideningOutcome,
    /// One result for every selected reservation.
    pub(crate) results:    Vec<ReservationDriftResult>,
}

/// The attribution result for paths not covered by any active reservation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum DriftWideningOutcome {
    /// This observation found no unclaimed path requiring attribution.
    NotNeeded,
    /// One reservation was selected for the widening attempt.
    Attributed {
        /// The only reservation permitted to receive unclaimed paths.
        reservation_id: ReservationId,
    },
    /// Several local reservations were candidates, so no widening was attempted.
    Ambiguous {
        /// Every active local reservation the caller may name explicitly.
        candidates: DriftAttributionCandidateSet,
        /// The exact paths left unassigned by this observation.
        paths:      UnattributedDriftPathSet,
    },
    /// No coordination run was identified, so no reservation can receive the paths.
    CoordinationRunRequired {
        /// The exact paths left unassigned by this observation.
        paths: UnattributedDriftPathSet,
    },
}

/// The drift result for one selected reservation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ReservationDriftResult {
    /// No path requires a consequence because every changed path is already
    /// covered by a reservation with this reservation's run and worktree identity.
    Unchanged {
        /// The reservation compared with the observed paths.
        reservation_id: ReservationId,
    },
    /// At least one durable or blocking consequence was found.
    Changed {
        /// The reservation receiving these consequences.
        reservation_id: ReservationId,
        /// Every distinct consequence for this reservation.
        effects:        DriftEffectSet,
    },
}

/// One non-empty consequence of classifying observed paths.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum DriftEffect {
    /// Unreserved file paths were added to the reservation.
    Widened {
        /// The exact complete scopes appended to the journal.
        added_scopes: ReservationScopeAdditionSet,
    },
    /// Writes entered paths held by foreign edit-blocking reservations.
    Incursion {
        /// The durable incident identity carried by the journal record.
        incident_id:             IncursionIncidentId,
        /// The foreign holders named by the incursion record.
        foreign_reservation_ids: ForeignReservationIdSet,
        /// The exact paths entered.
        paths:                   IncursionPathSet,
    },
    /// A path that was initially unheld gained a blocker before the widening lock.
    Collision {
        /// The reservations that prevented the locked widening.
        foreign_reservation_ids: ForeignReservationIdSet,
        /// The paths that could not be widened.
        paths:                   CollisionPathSet,
    },
}

/// A non-empty set of consequences for one reservation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct DriftEffectSet(Vec<DriftEffect>);

impl DriftEffectSet {
    /// Borrow the effects without weakening the non-empty construction boundary.
    pub(crate) fn as_slice(&self) -> &[DriftEffect] { &self.0 }
}

impl TryFrom<Vec<DriftEffect>> for DriftEffectSet {
    type Error = EmptyDriftEffectSet;

    fn try_from(effects: Vec<DriftEffect>) -> Result<Self, Self::Error> {
        if effects.is_empty() {
            Err(EmptyDriftEffectSet)
        } else {
            Ok(Self(effects))
        }
    }
}

impl<'de> Deserialize<'de> for DriftEffectSet {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        let effects = Vec::<DriftEffect>::deserialize(deserializer)?;
        Self::try_from(effects).map_err(serde::de::Error::custom)
    }
}

/// An error returned when a changed result contains no consequence.
#[derive(Debug)]
pub(crate) struct EmptyDriftEffectSet;

impl Display for EmptyDriftEffectSet {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("a changed drift result must contain at least one effect")
    }
}

impl std::error::Error for EmptyDriftEffectSet {}

macro_rules! nonempty_drift_set {
    ($name:ident, $item:ty, $error:ident, $documentation:literal, $message:literal) => {
        #[doc = $documentation]
        #[derive(Clone, Debug, Eq, PartialEq, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $name(Vec<$item>);

        impl $name {
            #[doc = concat!("Borrow the values in this `", stringify!($name), "`.")]
            pub(crate) fn as_slice(&self) -> &[$item] { &self.0 }
        }

        impl TryFrom<Vec<$item>> for $name {
            type Error = $error;

            fn try_from(values: Vec<$item>) -> Result<Self, Self::Error> {
                if values.is_empty() {
                    Err($error)
                } else {
                    Ok(Self(values))
                }
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<DeserializerType>(
                deserializer: DeserializerType,
            ) -> Result<Self, DeserializerType::Error>
            where
                DeserializerType: serde::Deserializer<'de>,
            {
                let values = Vec::<$item>::deserialize(deserializer)?;
                Self::try_from(values).map_err(serde::de::Error::custom)
            }
        }

        #[doc = concat!("An error returned when constructing an empty `", stringify!($name), "`.")]
        #[derive(Debug)]
        pub(crate) struct $error;

        impl Display for $error {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str($message)
            }
        }

        impl std::error::Error for $error {}
    };
}

nonempty_drift_set!(
    DriftAttributionCandidateSet,
    ReservationId,
    EmptyDriftAttributionCandidateSet,
    "The non-empty reservation candidates for one ambiguous widening attribution.",
    "an ambiguous widening attribution must name at least one reservation"
);
nonempty_drift_set!(
    UnattributedDriftPathSet,
    ReservationScopePath,
    EmptyUnattributedDriftPathSet,
    "The non-empty path set left unassigned by an ambiguous widening attribution.",
    "an ambiguous widening attribution must name at least one path"
);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct WorkingTreeFingerprint {
    tracked_paths:   Vec<ReservationScopePath>,
    untracked_paths: Vec<ReservationScopePath>,
}

enum StoredWorkingTreeFingerprint {
    Available(WorkingTreeFingerprint),
    Unavailable,
}

struct CheapDeltaChanges {
    tracked:   CheapTrackedChanges,
    untracked: CheapUntrackedChanges,
}

struct FullPhaseStartChanges {
    committed: HashMap<ReservationId, CommittedPhaseChanges>,
    staged:    StagedWorkingTreeChanges,
    unstaged:  UnstagedWorkingTreeChanges,
    untracked: UntrackedWorkingTreeChanges,
}

enum ObservedDriftChanges {
    Cheap(CheapDeltaChanges),
    Full(FullPhaseStartChanges),
}

macro_rules! changed_path_set {
    ($name:ident) => {
        struct $name(Vec<ReservationScopePath>);

        impl $name {
            fn as_slice(&self) -> &[ReservationScopePath] { &self.0 }
        }
    };
}

changed_path_set!(CheapTrackedChanges);
changed_path_set!(CheapUntrackedChanges);
changed_path_set!(CommittedPhaseChanges);
changed_path_set!(StagedWorkingTreeChanges);
changed_path_set!(UnstagedWorkingTreeChanges);
changed_path_set!(UntrackedWorkingTreeChanges);

struct FingerprintObservation {
    comparison:  DriftComparisonMode,
    changes:     ObservedDriftChanges,
    cache_value: WorkingTreeFingerprint,
}

#[derive(Clone, Copy)]
enum DriftActingIdentity {
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

#[derive(Clone, Copy)]
enum DriftActingRun {
    Identified(CoordinationRunId),
    Unidentified,
}

enum DriftSessionReservation {
    Mapped(ReservationId),
    Unavailable,
}

/// The run identity recorded on drift mutations from this invocation.
enum DriftMutationActorRun {
    /// The process or validated worktree marker identified the invoking run.
    Identified(CoordinationRunId),
    /// An unidentified post-commit invocation received a transaction-only run identity.
    PostCommitInvocation(CoordinationRunId),
}

struct ResolvedDriftSubjects {
    reporting: Vec<ReservationId>,
    widening:  DriftWideningSelection,
}

enum DriftWideningSelection {
    NotNeeded,
    Selected(ReservationId),
    Ambiguous(DriftAttributionCandidateSet),
    CoordinationRunRequired,
}

enum WideningAttempt {
    NotNeeded,
    Attributed,
}

struct PriorClassification {
    foreign_paths: HashSet<(ReservationId, String)>,
}

#[derive(Default)]
struct DriftEffectBuilder {
    widened_paths:          Vec<ReservationScopePath>,
    incursion_paths:        Vec<ReservationScopePath>,
    incursion_reservations: Vec<ReservationId>,
    collision_paths:        Vec<ReservationScopePath>,
    collision_reservations: Vec<ReservationId>,
}

struct DriftTransactionDecision {
    operations: Vec<JournalOperation>,
    report:     DriftReport,
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
    let reconciliation_report = match reconcile::reconcile(&invocation_directory) {
        Ok(reconciliation_report) => reconciliation_report,
        Err(error) => return error.into_output(CommandVerb::Drift),
    };
    let output_envelope = match execute_inner(request, &invocation_directory) {
        Ok(report) => OutputEnvelope::drift(report),
        Err(DriftExecutionError::Selection(error)) => {
            OutputEnvelope::invalid_input(CommandVerb::Drift, &error.to_string())
        },
        Err(DriftExecutionError::Transaction(LedgerTransactionError::LockContention)) => {
            OutputEnvelope::contention(
                CommandVerb::Drift,
                &LedgerTransactionError::LockContention.to_string(),
            )
        },
        Err(DriftExecutionError::Transaction(LedgerTransactionError::CorrectableInput(error))) => {
            OutputEnvelope::invalid_input(CommandVerb::Drift, &error.to_string())
        },
        Err(DriftExecutionError::Transaction(LedgerTransactionError::LedgerUnreadable(error))) => {
            OutputEnvelope::ledger_unreadable(CommandVerb::Drift, &error.to_string())
        },
        Err(error) => OutputEnvelope::ledger_unreadable(CommandVerb::Drift, &error.to_string()),
    };
    output_envelope.with_alerts(reconciliation_report.alerts)
}

fn execute_inner(
    request: DriftRequest,
    invocation_directory: &Path,
) -> Result<DriftReport, DriftExecutionError> {
    let initial_snapshot = Ledger::read_for_edit_check(invocation_directory)?;
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
                return Ok(DriftReport::unchanged(
                    request.comparison.report_mode(),
                    &[],
                ));
            },
            Err(error) => return Err(DriftExecutionError::Ledger(error)),
        };
    let initial_reservations = RetainedReservationSet::replay(initial_snapshot.events())?;
    let acting_identity =
        DriftActingIdentity::resolve(&worktree_context, worktree_id, &initial_reservations);
    let initial_subjects = request
        .reservation
        .resolve(&initial_reservations, acting_identity)?;
    if initial_subjects.reporting.is_empty() {
        return Ok(DriftReport::unchanged(
            request.comparison.report_mode(),
            &[],
        ));
    }
    let cache_path = fingerprint_cache_path(worktree_context.common_git_directory(), worktree_id);
    let observation = observe(
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
        publish_fingerprint(&cache_path, &observation.cache_value);
        let report = DriftReport::unchanged(observation.comparison, &initial_subjects.reporting);
        return Ok(report);
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
    let report = transact_classification(&mutation_context)?;
    if !report.has_blocking_effect() {
        publish_fingerprint(&cache_path, &observation.cache_value);
    }
    Ok(report)
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
            match classify_locked(
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

impl DriftComparisonChoice {
    const fn report_mode(self) -> DriftComparisonMode {
        match self {
            Self::CheapDelta => DriftComparisonMode::CheapDelta,
            Self::FullPhaseStart => DriftComparisonMode::FullPhaseStart,
        }
    }
}

impl DriftReport {
    fn unchanged(comparison: DriftComparisonMode, reservation_ids: &[ReservationId]) -> Self {
        Self {
            comparison,
            widening: DriftWideningOutcome::NotNeeded,
            results: reservation_ids
                .iter()
                .map(|reservation_id| ReservationDriftResult::Unchanged {
                    reservation_id: *reservation_id,
                })
                .collect(),
        }
    }

    /// Return whether a blocking effect or unresolved attribution requires a stop.
    pub(crate) fn has_blocking_effect(&self) -> bool {
        matches!(
            self.widening,
            DriftWideningOutcome::Ambiguous { .. }
                | DriftWideningOutcome::CoordinationRunRequired { .. }
        ) || self.results.iter().any(ReservationDriftResult::blocks)
    }

    /// Return whether this report has a drift effect or unresolved attribution to render.
    pub(crate) fn has_reportable_effect(&self) -> bool {
        matches!(
            self.widening,
            DriftWideningOutcome::Ambiguous { .. }
                | DriftWideningOutcome::CoordinationRunRequired { .. }
        ) || self
            .results
            .iter()
            .any(|result| matches!(result, ReservationDriftResult::Changed { .. }))
    }

    /// Return every reservation selected by this comparison.
    pub(crate) fn reservation_ids(&self) -> Vec<ReservationId> {
        self.results
            .iter()
            .map(ReservationDriftResult::reservation_id)
            .collect()
    }

    /// Return every foreign reservation that blocked classification.
    pub(crate) fn blocking_reservation_ids(&self) -> Vec<ReservationId> {
        let mut blocking = self
            .results
            .iter()
            .flat_map(ReservationDriftResult::blocking_reservation_ids)
            .collect::<Vec<_>>();
        sort_and_deduplicate_reservation_ids(&mut blocking);
        blocking
    }
}

impl ReservationDriftResult {
    const fn reservation_id(&self) -> ReservationId {
        match self {
            Self::Unchanged { reservation_id } | Self::Changed { reservation_id, .. } => {
                *reservation_id
            },
        }
    }

    fn blocks(&self) -> bool {
        match self {
            Self::Unchanged { .. } => false,
            Self::Changed { effects, .. } => effects.as_slice().iter().any(|effect| {
                matches!(
                    effect,
                    DriftEffect::Incursion { .. } | DriftEffect::Collision { .. }
                )
            }),
        }
    }

    fn blocking_reservation_ids(&self) -> Vec<ReservationId> {
        match self {
            Self::Unchanged { .. } => Vec::new(),
            Self::Changed { effects, .. } => effects
                .as_slice()
                .iter()
                .flat_map(|effect| match effect {
                    DriftEffect::Incursion {
                        foreign_reservation_ids,
                        ..
                    }
                    | DriftEffect::Collision {
                        foreign_reservation_ids,
                        ..
                    } => foreign_reservation_ids.as_slice().to_vec(),
                    DriftEffect::Widened { .. } => Vec::new(),
                })
                .collect(),
        }
    }
}

impl DriftActingIdentity {
    fn resolve(
        worktree_context: &WorktreeContext,
        current_worktree: WorktreeId,
        reservations: &RetainedReservationSet,
    ) -> Self {
        let edit_authorization = EditAuthorization::resolve(
            worktree_context.administrative_directory(),
            &worktree_context.ledger_directory(),
        );
        match reservations.resolve_editing_identity(edit_authorization) {
            AuthorizedEditingIdentity::SessionReservation {
                coordination_run_id: run,
                reservation_id: reservation,
            } => Self::Session {
                run,
                reservation,
                worktree: current_worktree,
            },
            AuthorizedEditingIdentity::Run(run) => Self::Run {
                run,
                worktree: current_worktree,
            },
            AuthorizedEditingIdentity::Unidentified => Self::Unidentified {
                worktree: current_worktree,
            },
        }
    }

    const fn worktree(self) -> WorktreeId {
        match self {
            Self::Session { worktree, .. }
            | Self::Run { worktree, .. }
            | Self::Unidentified { worktree } => worktree,
        }
    }

    const fn acting_run(self) -> DriftActingRun {
        match self {
            Self::Session { run, .. } | Self::Run { run, .. } => DriftActingRun::Identified(run),
            Self::Unidentified { .. } => DriftActingRun::Unidentified,
        }
    }

    const fn session_reservation(self) -> DriftSessionReservation {
        match self {
            Self::Session { reservation, .. } => DriftSessionReservation::Mapped(reservation),
            Self::Run { .. } | Self::Unidentified { .. } => DriftSessionReservation::Unavailable,
        }
    }

    fn run_for_mutation(
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

impl DriftMutationActorRun {
    const fn into_coordination_run_id(self) -> CoordinationRunId {
        match self {
            Self::Identified(coordination_run_id)
            | Self::PostCommitInvocation(coordination_run_id) => coordination_run_id,
        }
    }
}

impl DriftReservationSelection {
    fn resolve(
        self,
        reservations: &RetainedReservationSet,
        acting_identity: DriftActingIdentity,
    ) -> Result<ResolvedDriftSubjects, DriftSelectionError> {
        let worktree = acting_identity.worktree();
        if let Self::EveryActiveForPostCommit { widening } = self {
            return widening.resolve_post_commit(reservations, acting_identity);
        }
        let run = match acting_identity.acting_run() {
            DriftActingRun::Identified(run) => run,
            DriftActingRun::Unidentified => return Err(DriftSelectionError::UnidentifiedActingRun),
        };
        let mut candidates = reservations
            .iter()
            .filter(|reservation| {
                matches!(reservation.lifecycle(), ReservationLifecycle::Active)
                    && reservation.actor().run == run
                    && reservation.actor().worktree == worktree
            })
            .map(Reservation::id)
            .collect::<Vec<_>>();
        sort_reservation_ids(&mut candidates);
        match self {
            Self::Explicit(reservation_id) if candidates.contains(&reservation_id) => {
                Ok(ResolvedDriftSubjects {
                    reporting: vec![reservation_id],
                    widening:  DriftWideningSelection::Selected(reservation_id),
                })
            },
            Self::Explicit(reservation_id) => Err(DriftSelectionError::ExplicitNotActive {
                reservation_id,
                run,
                worktree,
            }),
            Self::SessionMappingOrSingleActive => {
                let selected = match acting_identity.session_reservation() {
                    DriftSessionReservation::Mapped(reservation_id)
                        if candidates.contains(&reservation_id) =>
                    {
                        reservation_id
                    },
                    DriftSessionReservation::Mapped(_) | DriftSessionReservation::Unavailable => {
                        match candidates.as_slice() {
                            [reservation_id] => *reservation_id,
                            [] => {
                                return Err(DriftSelectionError::NoActiveReservation {
                                    run,
                                    worktree,
                                });
                            },
                            _ => {
                                return Err(DriftSelectionError::AmbiguousActiveReservations(
                                    candidates,
                                ));
                            },
                        }
                    },
                };
                Ok(ResolvedDriftSubjects {
                    reporting: vec![selected],
                    widening:  DriftWideningSelection::Selected(selected),
                })
            },
            Self::EveryActiveForPostCommit { .. } => {
                Err(DriftSelectionError::NoPostCommitCandidate)
            },
        }
    }
}

impl PostCommitWideningSelection {
    fn resolve_post_commit(
        self,
        reservations: &RetainedReservationSet,
        acting_identity: DriftActingIdentity,
    ) -> Result<ResolvedDriftSubjects, DriftSelectionError> {
        let worktree = acting_identity.worktree();
        let mut reporting = reservations
            .iter()
            .filter(|reservation| {
                matches!(reservation.lifecycle(), ReservationLifecycle::Active)
                    && reservation.actor().worktree == worktree
            })
            .map(Reservation::id)
            .collect::<Vec<_>>();
        sort_reservation_ids(&mut reporting);
        let acting_run = acting_identity.acting_run();
        let mut candidates = match acting_run {
            DriftActingRun::Identified(run) => reservations
                .iter()
                .filter(|reservation| {
                    matches!(reservation.lifecycle(), ReservationLifecycle::Active)
                        && reservation.actor().run == run
                        && reservation.actor().worktree == worktree
                })
                .map(Reservation::id)
                .collect::<Vec<_>>(),
            DriftActingRun::Unidentified => Vec::new(),
        };
        sort_reservation_ids(&mut candidates);
        let widening = match self {
            Self::Explicit(reservation_id) => {
                let DriftActingRun::Identified(run) = acting_run else {
                    return Err(DriftSelectionError::UnidentifiedActingRun);
                };
                if !candidates.contains(&reservation_id) {
                    return Err(DriftSelectionError::ExplicitNotActive {
                        reservation_id,
                        run,
                        worktree,
                    });
                }
                DriftWideningSelection::Selected(reservation_id)
            },
            Self::SessionMappingOrSingleCandidate
                if matches!(acting_run, DriftActingRun::Unidentified) =>
            {
                if reporting.is_empty() {
                    DriftWideningSelection::NotNeeded
                } else {
                    DriftWideningSelection::CoordinationRunRequired
                }
            },
            Self::SessionMappingOrSingleCandidate => match acting_identity.session_reservation() {
                DriftSessionReservation::Mapped(reservation_id)
                    if candidates.contains(&reservation_id) =>
                {
                    DriftWideningSelection::Selected(reservation_id)
                },
                DriftSessionReservation::Mapped(_) | DriftSessionReservation::Unavailable => {
                    match candidates.as_slice() {
                        [] => DriftWideningSelection::NotNeeded,
                        [reservation_id] => DriftWideningSelection::Selected(*reservation_id),
                        _ => DriftWideningSelection::Ambiguous(
                            DriftAttributionCandidateSet::try_from(candidates)
                                .map_err(|_| DriftSelectionError::NoPostCommitCandidate)?,
                        ),
                    }
                },
            },
        };
        Ok(ResolvedDriftSubjects {
            reporting,
            widening,
        })
    }
}

impl ObservedDriftChanges {
    fn has_changes_for(&self, reservation_ids: &[ReservationId]) -> bool {
        match self {
            Self::Cheap(changes) => {
                !changes.tracked.as_slice().is_empty() || !changes.untracked.as_slice().is_empty()
            },
            Self::Full(changes) => {
                reservation_ids.iter().any(|reservation_id| {
                    changes
                        .committed
                        .get(reservation_id)
                        .is_some_and(|paths| !paths.as_slice().is_empty())
                }) || !changes.staged.as_slice().is_empty()
                    || !changes.unstaged.as_slice().is_empty()
                    || !changes.untracked.as_slice().is_empty()
            },
        }
    }

    fn visit_paths(
        &self,
        reservation_id: ReservationId,
        mut visit: impl FnMut(&ReservationScopePath),
    ) {
        match self {
            Self::Cheap(changes) => {
                for path in changes.tracked.as_slice() {
                    visit(path);
                }
                for path in changes.untracked.as_slice() {
                    visit(path);
                }
            },
            Self::Full(changes) => {
                if let Some(committed) = changes.committed.get(&reservation_id) {
                    for path in committed.as_slice() {
                        visit(path);
                    }
                }
                for path in changes.staged.as_slice() {
                    visit(path);
                }
                for path in changes.unstaged.as_slice() {
                    visit(path);
                }
                for path in changes.untracked.as_slice() {
                    visit(path);
                }
            },
        }
    }
}

impl PriorClassification {
    fn build(
        reservations: &RetainedReservationSet,
        subject_ids: &[ReservationId],
        changes: &ObservedDriftChanges,
        path_case: PathCase,
    ) -> Result<Self, DriftExecutionError> {
        let mut foreign_paths = HashSet::new();
        for reservation_id in subject_ids {
            let reservation = reservations.reservation(*reservation_id)?;
            changes.visit_paths(*reservation_id, |path| {
                if reservation_covers_path(reservation, path, path_case) {
                    return;
                }
                match blocking_coverage(reservations, reservation, path, path_case) {
                    DriftBlockingCoverage::SameIdentity | DriftBlockingCoverage::Unclaimed => {},
                    DriftBlockingCoverage::Foreign(_) => {
                        foreign_paths.insert((*reservation_id, path.to_string()));
                    },
                }
            });
        }
        Ok(Self { foreign_paths })
    }

    fn was_foreign(&self, reservation_id: ReservationId, path: &ReservationScopePath) -> bool {
        self.foreign_paths
            .contains(&(reservation_id, path.to_string()))
    }
}

fn classify_locked(
    reservations: &RetainedReservationSet,
    subjects: &ResolvedDriftSubjects,
    changes: &ObservedDriftChanges,
    prior: &PriorClassification,
    path_case: PathCase,
    comparison: DriftComparisonMode,
) -> Result<DriftTransactionDecision, ReservationReplayError> {
    let mut operations = Vec::new();
    let mut results = Vec::new();
    let mut unattributed_paths = Vec::new();
    let mut widening_attempt = WideningAttempt::NotNeeded;
    for reservation_id in &subjects.reporting {
        let reservation = reservations.reservation(*reservation_id)?;
        let mut builder = DriftEffectBuilder::default();
        changes.visit_paths(*reservation_id, |path| {
            if reservation_covers_path(reservation, path, path_case) {
                return;
            }
            match blocking_coverage(reservations, reservation, path, path_case) {
                DriftBlockingCoverage::SameIdentity => {},
                DriftBlockingCoverage::Unclaimed => match &subjects.widening {
                    DriftWideningSelection::Selected(selected) if selected == reservation_id => {
                        builder.widened_paths.push(path.clone());
                    },
                    DriftWideningSelection::Ambiguous(_)
                    | DriftWideningSelection::CoordinationRunRequired => {
                        unattributed_paths.push(path.clone());
                    },
                    DriftWideningSelection::NotNeeded | DriftWideningSelection::Selected(_) => {},
                },
                DriftBlockingCoverage::Foreign(conflicts) => {
                    let blockers = conflicts
                        .iter()
                        .map(|conflict| conflict.reservation_id)
                        .collect::<Vec<_>>();
                    if prior.was_foreign(*reservation_id, path) {
                        builder.incursion_paths.push(path.clone());
                        builder.incursion_reservations.extend(blockers);
                    } else {
                        builder.collision_paths.push(path.clone());
                        builder.collision_reservations.extend(blockers);
                    }
                },
            }
        });
        let (mut subject_operations, result, subject_widening_attempt) =
            builder.finish(reservations, reservation, path_case);
        if matches!(subject_widening_attempt, WideningAttempt::Attributed) {
            widening_attempt = WideningAttempt::Attributed;
        }
        operations.append(&mut subject_operations);
        results.push(result);
    }
    normalize_paths(&mut unattributed_paths);
    let widening = match (&subjects.widening, widening_attempt) {
        (DriftWideningSelection::Selected(reservation_id), WideningAttempt::Attributed) => {
            DriftWideningOutcome::Attributed {
                reservation_id: *reservation_id,
            }
        },
        (DriftWideningSelection::Ambiguous(candidates), _) => {
            UnattributedDriftPathSet::try_from(unattributed_paths).map_or(
                DriftWideningOutcome::NotNeeded,
                |paths| DriftWideningOutcome::Ambiguous {
                    candidates: candidates.clone(),
                    paths,
                },
            )
        },
        (DriftWideningSelection::CoordinationRunRequired, _) => {
            UnattributedDriftPathSet::try_from(unattributed_paths)
                .map_or(DriftWideningOutcome::NotNeeded, |paths| {
                    DriftWideningOutcome::CoordinationRunRequired { paths }
                })
        },
        (DriftWideningSelection::NotNeeded, _)
        | (DriftWideningSelection::Selected(_), WideningAttempt::NotNeeded) => {
            DriftWideningOutcome::NotNeeded
        },
    };
    Ok(DriftTransactionDecision {
        operations,
        report: DriftReport {
            comparison,
            widening,
            results,
        },
    })
}

impl DriftEffectBuilder {
    fn finish(
        mut self,
        reservations: &RetainedReservationSet,
        reservation: &Reservation,
        path_case: PathCase,
    ) -> (
        Vec<JournalOperation>,
        ReservationDriftResult,
        WideningAttempt,
    ) {
        let reservation_id = reservation.id();
        normalize_paths(&mut self.widened_paths);
        normalize_paths(&mut self.incursion_paths);
        normalize_paths(&mut self.collision_paths);
        sort_and_deduplicate_reservation_ids(&mut self.incursion_reservations);
        sort_and_deduplicate_reservation_ids(&mut self.collision_reservations);
        let mut operations = Vec::new();
        let mut effects = Vec::new();
        let widening_attempt = if self.widened_paths.is_empty() {
            WideningAttempt::NotNeeded
        } else {
            WideningAttempt::Attributed
        };
        if let Ok(added_scopes) = ReservationScopeAdditionSet::try_from(
            self.widened_paths
                .into_iter()
                .map(|path| ReservationScope {
                    path,
                    kind: ScopeKind::File,
                })
                .collect::<Vec<_>>(),
        ) {
            match reservations.bind_widened_scopes(reservation, &added_scopes, path_case) {
                WidenScopeBinding::Authorized(authorization) => {
                    operations.push(JournalOperation::Widen {
                        reservation_id,
                        added_scopes: added_scopes.clone(),
                        cause: WidenCause::Drift,
                        authorization,
                        edit_blocking_status: reservation.edit_blocking_status(),
                    });
                    effects.push(DriftEffect::Widened { added_scopes });
                },
                WidenScopeBinding::Blocked(conflicts) => {
                    self.collision_paths.extend(
                        added_scopes
                            .as_slice()
                            .iter()
                            .map(|scope| scope.path.clone()),
                    );
                    self.collision_reservations
                        .extend(conflicts.iter().map(|conflict| conflict.reservation_id));
                    normalize_paths(&mut self.collision_paths);
                    sort_and_deduplicate_reservation_ids(&mut self.collision_reservations);
                },
            }
        }
        if let (Ok(foreign_reservation_ids), Ok(paths)) = (
            ForeignReservationIdSet::try_from(self.incursion_reservations),
            IncursionPathSet::try_from(self.incursion_paths),
        ) {
            let incident_id = match reservations.observe_incursion(
                reservation_id,
                &foreign_reservation_ids,
                &paths,
            ) {
                IncursionObservation::AlreadyOutstanding(incident_id) => incident_id,
                IncursionObservation::NewlyObserved(incident_id) => {
                    operations.push(JournalOperation::Incursion {
                        incident_id,
                        reservation_id,
                        foreign_reservation_ids: foreign_reservation_ids.clone(),
                        paths: paths.clone(),
                    });
                    incident_id
                },
            };
            effects.push(DriftEffect::Incursion {
                incident_id,
                foreign_reservation_ids,
                paths,
            });
        }
        if let (Ok(foreign_reservation_ids), Ok(paths)) = (
            ForeignReservationIdSet::try_from(self.collision_reservations),
            CollisionPathSet::try_from(self.collision_paths),
        ) {
            effects.push(DriftEffect::Collision {
                foreign_reservation_ids,
                paths,
            });
        }
        let result = DriftEffectSet::try_from(effects).map_or(
            ReservationDriftResult::Unchanged { reservation_id },
            |effects| ReservationDriftResult::Changed {
                reservation_id,
                effects,
            },
        );
        (operations, result, widening_attempt)
    }
}

fn observe(
    choice: DriftComparisonChoice,
    repository_root: &Path,
    reservations: &RetainedReservationSet,
    reservation_ids: &[ReservationId],
    cache_path: &Path,
) -> Result<FingerprintObservation, DriftFingerprintError> {
    match choice {
        DriftComparisonChoice::CheapDelta => match read_fingerprint(cache_path) {
            StoredWorkingTreeFingerprint::Available(previous) => {
                observe_cheap(repository_root, &previous)
            },
            StoredWorkingTreeFingerprint::Unavailable => observe_full(
                repository_root,
                reservations,
                reservation_ids,
                DriftComparisonMode::FullPhaseStartFallback,
            ),
        },
        DriftComparisonChoice::FullPhaseStart => observe_full(
            repository_root,
            reservations,
            reservation_ids,
            DriftComparisonMode::FullPhaseStart,
        ),
    }
}

fn observe_cheap(
    repository_root: &Path,
    previous: &WorkingTreeFingerprint,
) -> Result<FingerprintObservation, DriftFingerprintError> {
    let status = run_git(
        repository_root,
        &[
            GIT_STATUS_COMMAND,
            GIT_PORCELAIN_ARGUMENT,
            GIT_NUL_TERMINATED_ARGUMENT,
        ],
    )?;
    let untracked = run_git(
        repository_root,
        &[
            GIT_LS_FILES_COMMAND,
            GIT_NUL_TERMINATED_ARGUMENT,
            GIT_OTHERS_ARGUMENT,
            GIT_EXCLUDE_STANDARD_ARGUMENT,
        ],
    )?;
    let current = WorkingTreeFingerprint {
        tracked_paths:   parse_status_paths(&status.stdout)?,
        untracked_paths: parse_path_list(&untracked.stdout)?,
    }
    .normalized();
    let changes = CheapDeltaChanges {
        tracked:   CheapTrackedChanges(symmetric_difference(
            &previous.tracked_paths,
            &current.tracked_paths,
        )),
        untracked: CheapUntrackedChanges(symmetric_difference(
            &previous.untracked_paths,
            &current.untracked_paths,
        )),
    };
    Ok(FingerprintObservation {
        comparison:  DriftComparisonMode::CheapDelta,
        changes:     ObservedDriftChanges::Cheap(changes),
        cache_value: current,
    })
}

fn observe_full(
    repository_root: &Path,
    reservations: &RetainedReservationSet,
    reservation_ids: &[ReservationId],
    comparison: DriftComparisonMode,
) -> Result<FingerprintObservation, DriftFingerprintError> {
    let mut committed = HashMap::new();
    for reservation_id in reservation_ids {
        let reservation = reservations
            .reservation(*reservation_id)
            .map_err(|error| DriftFingerprintError::Reservation(error.to_string()))?;
        let phase_range = format!(
            "{}..{GIT_HEAD_REVISION}",
            reservation.phase_start_head().as_ref()
        );
        let output = run_git(
            repository_root,
            &[
                GIT_DIFF_COMMAND,
                GIT_NAME_STATUS_ARGUMENT,
                GIT_NUL_TERMINATED_ARGUMENT,
                GIT_NO_RENAMES_ARGUMENT,
                &phase_range,
            ],
        )?;
        committed.insert(
            *reservation_id,
            CommittedPhaseChanges(parse_name_status_paths(&output.stdout)?),
        );
    }
    let staged = run_git(
        repository_root,
        &[
            GIT_DIFF_COMMAND,
            GIT_CACHED_ARGUMENT,
            GIT_NAME_STATUS_ARGUMENT,
            GIT_NUL_TERMINATED_ARGUMENT,
            GIT_NO_RENAMES_ARGUMENT,
            GIT_HEAD_REVISION,
        ],
    )?;
    let unstaged = run_git(
        repository_root,
        &[
            GIT_DIFF_COMMAND,
            GIT_NAME_STATUS_ARGUMENT,
            GIT_NUL_TERMINATED_ARGUMENT,
            GIT_NO_RENAMES_ARGUMENT,
        ],
    )?;
    let untracked = run_git(
        repository_root,
        &[
            GIT_LS_FILES_COMMAND,
            GIT_NUL_TERMINATED_ARGUMENT,
            GIT_OTHERS_ARGUMENT,
            GIT_EXCLUDE_STANDARD_ARGUMENT,
        ],
    )?;
    let staged_paths = parse_name_status_paths(&staged.stdout)?;
    let unstaged_paths = parse_name_status_paths(&unstaged.stdout)?;
    let untracked_paths = parse_path_list(&untracked.stdout)?;
    let mut tracked_cache_paths = staged_paths.clone();
    tracked_cache_paths.extend(unstaged_paths.iter().cloned());
    normalize_paths(&mut tracked_cache_paths);
    let cache_value = WorkingTreeFingerprint {
        tracked_paths:   tracked_cache_paths,
        untracked_paths: untracked_paths.clone(),
    }
    .normalized();
    Ok(FingerprintObservation {
        comparison,
        changes: ObservedDriftChanges::Full(FullPhaseStartChanges {
            committed,
            staged: StagedWorkingTreeChanges(staged_paths),
            unstaged: UnstagedWorkingTreeChanges(unstaged_paths),
            untracked: UntrackedWorkingTreeChanges(untracked_paths),
        }),
        cache_value,
    })
}

impl WorkingTreeFingerprint {
    fn normalized(mut self) -> Self {
        normalize_paths(&mut self.tracked_paths);
        normalize_paths(&mut self.untracked_paths);
        self
    }
}

fn read_fingerprint(path: &Path) -> StoredWorkingTreeFingerprint {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .map_or(
            StoredWorkingTreeFingerprint::Unavailable,
            StoredWorkingTreeFingerprint::Available,
        )
}

fn publish_fingerprint(path: &Path, fingerprint: &WorkingTreeFingerprint) {
    if let Ok(serialized) = serde_json::to_vec(fingerprint) {
        std::mem::drop(fs::write(path, serialized));
    }
}

fn fingerprint_cache_path(common_git_directory: &Path, worktree_id: WorktreeId) -> PathBuf {
    common_git_directory.join("cargo-berth").join(format!(
        "{DRIFT_CACHE_FILE_PREFIX}{worktree_id}{DRIFT_CACHE_FILE_SUFFIX}"
    ))
}

fn run_git(repository_root: &Path, arguments: &[&str]) -> Result<Output, DriftFingerprintError> {
    let output = Command::new(GIT_BINARY)
        .arg(GIT_NO_OPTIONAL_LOCKS_ARGUMENT)
        .args(arguments)
        .current_dir(repository_root)
        .output()?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(DriftFingerprintError::CommandFailed {
            command: arguments.join(" "),
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

fn parse_name_status_paths(
    bytes: &[u8],
) -> Result<Vec<ReservationScopePath>, DriftFingerprintError> {
    let fields = nul_fields(bytes);
    let mut paths = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let status_field = fields[index];
        index += 1;
        let tab_position = status_field.iter().position(|byte| *byte == b'\t');
        let (status, first_path) = tab_position.map_or((status_field, None), |position| {
            (
                &status_field[..position],
                Some(&status_field[position + 1..]),
            )
        });
        let path = if let Some(path) = first_path {
            path
        } else {
            let Some(path) = fields.get(index) else {
                return Err(DriftFingerprintError::MalformedGitOutput(
                    "name-status output ended before its path".to_owned(),
                ));
            };
            index += 1;
            path
        };
        paths.push(parse_path(path)?);
        if matches!(status.first(), Some(b'R' | b'C')) {
            let Some(second_path) = fields.get(index) else {
                return Err(DriftFingerprintError::MalformedGitOutput(
                    "rename or copy status ended before its second path".to_owned(),
                ));
            };
            index += 1;
            paths.push(parse_path(second_path)?);
        }
    }
    normalize_paths(&mut paths);
    Ok(paths)
}

fn parse_status_paths(bytes: &[u8]) -> Result<Vec<ReservationScopePath>, DriftFingerprintError> {
    let fields = nul_fields(bytes);
    let mut paths = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let record = fields[index];
        index += 1;
        if record.len() < 4 || record[2] != b' ' {
            return Err(DriftFingerprintError::MalformedGitOutput(
                "porcelain status record did not contain XY and a path".to_owned(),
            ));
        }
        let status = &record[..2];
        if status != b"??" && status != b"!!" {
            paths.push(parse_path(&record[3..])?);
        }
        if status.iter().any(|column| matches!(column, b'R' | b'C')) {
            let Some(second_path) = fields.get(index) else {
                return Err(DriftFingerprintError::MalformedGitOutput(
                    "porcelain rename or copy ended before its second path".to_owned(),
                ));
            };
            index += 1;
            paths.push(parse_path(second_path)?);
        }
    }
    normalize_paths(&mut paths);
    Ok(paths)
}

fn parse_path_list(bytes: &[u8]) -> Result<Vec<ReservationScopePath>, DriftFingerprintError> {
    let mut paths = nul_fields(bytes)
        .into_iter()
        .map(parse_path)
        .collect::<Result<Vec<_>, _>>()?;
    normalize_paths(&mut paths);
    Ok(paths)
}

fn nul_fields(bytes: &[u8]) -> Vec<&[u8]> {
    bytes
        .split(|byte| *byte == b'\0')
        .filter(|field| !field.is_empty())
        .collect()
}

fn parse_path(bytes: &[u8]) -> Result<ReservationScopePath, DriftFingerprintError> {
    let path = std::str::from_utf8(bytes)
        .map_err(|error| DriftFingerprintError::NonUtf8Path(error.to_string()))?;
    ReservationScopePath::from_str(path)
        .map_err(|error| DriftFingerprintError::InvalidPath(error.to_string()))
}

fn symmetric_difference(
    previous: &[ReservationScopePath],
    current: &[ReservationScopePath],
) -> Vec<ReservationScopePath> {
    let previous_names = previous
        .iter()
        .map(ToString::to_string)
        .collect::<HashSet<_>>();
    let current_names = current
        .iter()
        .map(ToString::to_string)
        .collect::<HashSet<_>>();
    let mut paths = previous
        .iter()
        .filter(|path| !current_names.contains(&path.to_string()))
        .chain(
            current
                .iter()
                .filter(|path| !previous_names.contains(&path.to_string())),
        )
        .cloned()
        .collect::<Vec<_>>();
    normalize_paths(&mut paths);
    paths
}

fn reservation_covers_path(
    reservation: &Reservation,
    path: &ReservationScopePath,
    path_case: PathCase,
) -> bool {
    let candidate = ReservationScope {
        path: path.clone(),
        kind: ScopeKind::File,
    };
    reservation
        .scopes()
        .as_slice()
        .iter()
        .any(|scope| scope.contains(&candidate, path_case))
}

fn blocking_coverage(
    reservations: &RetainedReservationSet,
    subject: &Reservation,
    path: &ReservationScopePath,
    path_case: PathCase,
) -> DriftBlockingCoverage {
    let Ok(candidate) = ReservationScopeSet::try_from(vec![ReservationScope {
        path: path.clone(),
        kind: ScopeKind::File,
    }]) else {
        return DriftBlockingCoverage::Unclaimed;
    };
    reservations.blocking_coverage_for_drift(
        &candidate,
        subject.actor().run,
        subject.actor().worktree,
        path_case,
    )
}

fn normalize_paths(paths: &mut Vec<ReservationScopePath>) {
    paths.sort_by_key(ToString::to_string);
    paths.dedup_by(|left, right| left.to_string() == right.to_string());
}

fn sort_reservation_ids(reservation_ids: &mut [ReservationId]) {
    reservation_ids.sort_by_key(ToString::to_string);
}

fn sort_and_deduplicate_reservation_ids(reservation_ids: &mut Vec<ReservationId>) {
    sort_reservation_ids(reservation_ids);
    reservation_ids.dedup();
}

/// A caller identity could not choose a safe drift subject.
#[derive(Debug)]
enum DriftSelectionError {
    UnidentifiedActingRun,
    NoPostCommitCandidate,
    NoActiveReservation {
        run:      CoordinationRunId,
        worktree: WorktreeId,
    },
    AmbiguousActiveReservations(Vec<ReservationId>),
    ExplicitNotActive {
        reservation_id: ReservationId,
        run:            CoordinationRunId,
        worktree:       WorktreeId,
    },
}

impl Display for DriftSelectionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnidentifiedActingRun => formatter
                .write_str("drift requires a live session mapping, active coordination-run marker, or CARGO_BERTH_RUN"),
            Self::NoPostCommitCandidate => formatter
                .write_str("post-commit drift found no active reservation candidate"),
            Self::NoActiveReservation { run, worktree } => write!(
                formatter,
                "coordination run {run} has no active reservation in worktree {worktree}"
            ),
            Self::AmbiguousActiveReservations(candidates) => write!(
                formatter,
                "drift is ambiguous; choose one active reservation with --reservation: {}",
                candidates
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::ExplicitNotActive {
                reservation_id,
                run,
                worktree,
            } => write!(
                formatter,
                "reservation {reservation_id} is not active for coordination run {run} in worktree {worktree}"
            ),
        }
    }
}

impl std::error::Error for DriftSelectionError {}

/// A git fingerprint could not be computed or interpreted.
#[derive(Debug)]
enum DriftFingerprintError {
    Io(std::io::Error),
    CommandFailed { command: String, stderr: String },
    MalformedGitOutput(String),
    NonUtf8Path(String),
    InvalidPath(String),
    Reservation(String),
}

impl Display for DriftFingerprintError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "could not run drift fingerprint: {error}"),
            Self::CommandFailed { command, stderr } => write!(
                formatter,
                "git {command} failed while computing drift: {stderr}"
            ),
            Self::MalformedGitOutput(diagnostic) => {
                write!(
                    formatter,
                    "git returned malformed drift output: {diagnostic}"
                )
            },
            Self::NonUtf8Path(diagnostic) => {
                write!(
                    formatter,
                    "git reported a non-UTF-8 drift path: {diagnostic}"
                )
            },
            Self::InvalidPath(diagnostic) => {
                write!(
                    formatter,
                    "git reported an invalid drift path: {diagnostic}"
                )
            },
            Self::Reservation(diagnostic) => formatter.write_str(diagnostic),
        }
    }
}

impl std::error::Error for DriftFingerprintError {}

impl From<std::io::Error> for DriftFingerprintError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

/// A drift command failed before it could publish a coherent result.
#[derive(Debug)]
enum DriftExecutionError {
    Io(std::io::Error),
    Ledger(LedgerError),
    Replay(ReservationReplayError),
    Selection(DriftSelectionError),
    Fingerprint(DriftFingerprintError),
    PathCase(crate::scope::PathCaseError),
    Transaction(LedgerTransactionError),
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

impl From<crate::scope::PathCaseError> for DriftExecutionError {
    fn from(error: crate::scope::PathCaseError) -> Self { Self::PathCase(error) }
}

#[cfg(test)]
mod tests {
    use super::parse_status_paths;

    #[test]
    fn porcelain_parser_consumes_second_path_for_combined_rename_and_copy_statuses()
    -> Result<(), Box<dyn std::error::Error>> {
        let paths =
            parse_status_paths(b"RM renamed.txt\0original.txt\0CM copied.txt\0source.txt\0")?;
        let path_names = paths.iter().map(ToString::to_string).collect::<Vec<_>>();

        assert_eq!(
            path_names,
            vec!["copied.txt", "original.txt", "renamed.txt", "source.txt"]
        );
        Ok(())
    }
}
