//! Atomic reservation acquisition.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;
use std::process::Command;
use std::process::Output;
use std::string::FromUtf8Error;

use crate::answer::ConflictAuthorization;
use crate::answer::OverlapAuthorizationRequest;
use crate::answer::OverlapEscalationPayload;
use crate::answer::OverlapProposal;
use crate::answer::OverlapProposalSubmission;
use crate::answer::OverlapRequester;
use crate::answer::PermissiveOverlapAnswer;
use crate::answer::PermissiveOverlapAuthorizationRequest;
use crate::answer::RequesterCoordinationIdentity;
use crate::config::BerthConfig;
use crate::config::ConfigError;
use crate::edge::EdgeReplayError;
use crate::edge::OrderingGraph;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::InvalidGitObjectId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::ClaimHeadCommit;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimSource;
use crate::ledger::EditAuthorization;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::LedgerTransactionOutcome;
use crate::ledger::ProtectedPhaseStartHead;
use crate::ledger::ReplayedLedgerState;
use crate::ledger::ReservationPurpose;
use crate::ledger::TransactionValidation;
use crate::ledger::TrunkCommitAtClaim;
use crate::ledger::WorktreeAdministrativeLocator;
use crate::ledger::WorktreeContext;
use crate::output::CommandVerb;
use crate::output::CoordinationRunMarkerPublication;
use crate::output::OutputEnvelope;
use crate::reconcile;
use crate::reservation::ReservationConflict;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::scope::DeclaredReservationScopeSet;
use crate::scope::PathCase;
use crate::scope::PathCaseError;
use crate::scope::ReservationScopeSet;

const GIT_BINARY: &str = "git";
const GIT_FULL_REF_NAME_ARG: &str = "--quiet";
const GIT_HEAD_REVISION: &str = "HEAD";
const GIT_NO_OPTIONAL_LOCKS_ARG: &str = "--no-optional-locks";
const GIT_REV_PARSE_COMMAND: &str = "rev-parse";
const GIT_SYMBOLIC_REF_COMMAND: &str = "symbolic-ref";
const HEADS_REF_PREFIX: &str = "refs/heads/";

/// A parsed claim whose provenance carries domain types rather than CLI options.
pub(crate) struct ClaimRequest {
    /// Lexically valid scopes before repository-case antichain reduction.
    pub(crate) declared_scopes:            DeclaredReservationScopeSet,
    /// Work-plan or explicit provenance.
    pub(crate) source:                     ClaimSource,
    /// The caller-supplied explanation state.
    pub(crate) purpose:                    ReservationPurpose,
    /// How the claim selects the coordination run that will own it.
    pub(crate) coordination_run_selection: ClaimCoordinationRunSelection,
    /// The phase-start commit selection.
    pub(crate) phase_start:                PhaseStartSelection,
    /// The semantic overlap answer, its reason, and proposal state.
    pub(crate) overlap_authorization:      OverlapAuthorizationRequest,
}

/// How a claim chooses the coordination run that will own its reservation.
pub(crate) enum ClaimCoordinationRunSelection {
    /// Use the run identity supplied through `--run`.
    Specified(CoordinationRunId),
    /// Continue the active run from process or worktree context, or start one.
    ContinueOrStart,
}

/// The replay validation required by the source of a resolved claim run.
#[derive(Clone, Copy)]
enum ClaimRunValidation {
    /// An explicit argument or process environment identifies a marker-independent caller.
    IndependentWithPresentedIdentity(CoordinationRunId),
    /// A marker-independent caller presented no identity, so only its actor run is minted.
    IndependentWithoutPresentedIdentity {
        /// The concrete run stamped on the new reservation and transaction.
        actor_run_id: CoordinationRunId,
    },
    /// A marker remains valid only while this worktree and run retain active work.
    ActiveMarkerRequired {
        coordination_run_id: CoordinationRunId,
        worktree_id:         WorktreeId,
    },
}

/// How a claim chooses its protected phase-start commit.
pub(crate) enum PhaseStartSelection {
    /// Use the current worktree HEAD observed during acquisition.
    CurrentHead,
    /// Retain the full object id supplied by the caller.
    Protected(ProtectedPhaseStartHead),
}

struct ClaimRepositoryFacts {
    head_snapshot: ClaimHeadSnapshot,
    current_head:  GitObjectId,
    worktree_root: CanonicalWorktreeRoot,
}

struct PreparedClaim {
    reservation_id:                  ReservationId,
    scopes:                          ReservationScopeSet,
    source:                          ClaimSource,
    purpose:                         ReservationPurpose,
    trunk_at_claim:                  TrunkCommitAtClaim,
    head_snapshot:                   ClaimHeadSnapshot,
    phase_start_head:                ProtectedPhaseStartHead,
    worktree_root:                   CanonicalWorktreeRoot,
    worktree_administrative_locator: WorktreeAdministrativeLocator,
}

struct ClaimValidationContext {
    run_validation:         ClaimRunValidation,
    coordination_run_id:    CoordinationRunId,
    path_case:              PathCase,
    requester:              OverlapRequester,
    overlap_authorization:  OverlapAuthorizationRequest,
    maximum_reservations:   u32,
    maximum_ordering_edges: u32,
}

/// Acquire a reservation or return a typed conflict without appending.
pub(crate) fn execute(claim_request: ClaimRequest) -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Claim, &error.to_string());
        },
    };
    let reconciliation_report = match reconcile::reconcile(&invocation_directory) {
        Ok(reconciliation_report) => reconciliation_report,
        Err(error) => return error.into_output(CommandVerb::Claim),
    };
    let output_envelope = match execute_claim(claim_request) {
        Ok(ClaimExecution::Claimed {
            reservation_id,
            coordination_run_id,
            scopes,
            marker_publication,
        }) => OutputEnvelope::claimed(
            reservation_id,
            coordination_run_id,
            scopes,
            marker_publication,
        ),
        Ok(ClaimExecution::Blocked(conflicts)) => OutputEnvelope::blocked_claim(conflicts),
        Ok(ClaimExecution::AuthorizationRequired(escalation)) => {
            OutputEnvelope::claim_authorization_required(*escalation)
        },
        Ok(ClaimExecution::ReservationLimitReached(maximum)) => {
            OutputEnvelope::reservation_limit_reached(maximum)
        },
        Ok(ClaimExecution::OrderingEdgeLimitReached(maximum)) => {
            OutputEnvelope::claim_ordering_edge_limit_reached(maximum)
        },
        Err(ClaimError::Transaction(error)) => match error {
            LedgerTransactionError::CorrectableInput(error) => {
                OutputEnvelope::invalid_input(CommandVerb::Claim, &error.to_string())
            },
            LedgerTransactionError::LockContention => {
                OutputEnvelope::contention(CommandVerb::Claim, &error.to_string())
            },
            LedgerTransactionError::LedgerUnreadable(error) => {
                OutputEnvelope::ledger_unreadable(CommandVerb::Claim, &error.to_string())
            },
        },
        Err(ClaimError::InactiveMarkerRun(coordination_run_id)) => OutputEnvelope::invalid_input(
            CommandVerb::Claim,
            &format!(
                "coordination-run marker {coordination_run_id} no longer has an active reservation; retry the claim"
            ),
        ),
        Err(error) => OutputEnvelope::ledger_unreadable(CommandVerb::Claim, &error.to_string()),
    };
    output_envelope.with_alerts(reconciliation_report.alerts)
}

enum ClaimExecution {
    Claimed {
        reservation_id:      ReservationId,
        coordination_run_id: CoordinationRunId,
        scopes:              ReservationScopeSet,
        marker_publication:  CoordinationRunMarkerPublication,
    },
    Blocked(Vec<ReservationConflict>),
    AuthorizationRequired(Box<OverlapEscalationPayload>),
    ReservationLimitReached(u32),
    OrderingEdgeLimitReached(u32),
}

fn execute_claim(claim_request: ClaimRequest) -> Result<ClaimExecution, ClaimError> {
    let ClaimRequest {
        declared_scopes,
        source,
        purpose,
        coordination_run_selection,
        phase_start,
        overlap_authorization,
    } = claim_request;
    let invocation_directory = std::env::current_dir()?;
    let worktree_context = WorktreeContext::discover(&invocation_directory)?;
    let claim_run_validation =
        coordination_run_selection.resolve(worktree_context.administrative_directory());
    let actor_run_id = claim_run_validation.actor_run_id();
    let path_case = PathCase::read(worktree_context.common_git_directory())?;
    let scopes = declared_scopes.into_minimal_antichain(path_case);
    let claim_repository_facts = ClaimRepositoryFacts::read(&worktree_context)?;
    let phase_start_head = match phase_start {
        PhaseStartSelection::CurrentHead => {
            ProtectedPhaseStartHead::from(claim_repository_facts.current_head.clone())
        },
        PhaseStartSelection::Protected(protected_phase_start_head) => protected_phase_start_head,
    };
    let berth_config = BerthConfig::read(worktree_context.repository_root())?;
    let trunk_at_claim =
        read_trunk_commit(worktree_context.repository_root(), &berth_config.trunk)?;
    let worktree_identity = ledger::worktree_identity(
        worktree_context.administrative_directory(),
        worktree_context.worktree_kind(),
    )?;
    let ledger = Ledger::open(worktree_context.repository_root())?;
    let reservation_id = ReservationId::new();
    let prepared_claim = PreparedClaim {
        reservation_id,
        scopes: scopes.clone(),
        source,
        purpose,
        trunk_at_claim,
        head_snapshot: claim_repository_facts.head_snapshot,
        phase_start_head,
        worktree_root: claim_repository_facts.worktree_root,
        worktree_administrative_locator: worktree_context.administrative_locator().clone(),
    };
    let requester = OverlapRequester::new(
        claim_run_validation.presented_coordination_identity(),
        worktree_identity.id,
        prepared_claim.source.clone(),
        prepared_claim.purpose.clone(),
    );
    let outcome = ledger.transact(worktree_identity.id, actor_run_id, |state| {
        validate_claim_transaction(
            &state,
            prepared_claim,
            ClaimValidationContext {
                run_validation: claim_run_validation,
                coordination_run_id: actor_run_id,
                path_case,
                requester,
                overlap_authorization,
                maximum_reservations: berth_config.maximum_reservations,
                maximum_ordering_edges: berth_config.maximum_ordering_edges,
            },
        )
    })?;
    claim_execution_from_outcome(outcome, reservation_id, scopes, &worktree_context)
}

fn claim_execution_from_outcome(
    outcome: LedgerTransactionOutcome<ClaimRejection>,
    reservation_id: ReservationId,
    scopes: ReservationScopeSet,
    worktree_context: &WorktreeContext,
) -> Result<ClaimExecution, ClaimError> {
    match outcome {
        LedgerTransactionOutcome::Appended(event) => {
            let coordination_run_id = event.actor.run;
            let marker_publication = worktree_context
                .publish_coordination_run_marker(coordination_run_id)
                .map_or_else(
                    |error| CoordinationRunMarkerPublication::Unavailable {
                        diagnostic: error.to_string(),
                    },
                    |()| CoordinationRunMarkerPublication::Published,
                );
            Ok(ClaimExecution::Claimed {
                reservation_id,
                coordination_run_id,
                scopes,
                marker_publication,
            })
        },
        LedgerTransactionOutcome::Rejected(ClaimRejection::Conflict(conflicts)) => {
            Ok(ClaimExecution::Blocked(conflicts))
        },
        LedgerTransactionOutcome::Rejected(ClaimRejection::AuthorizationRequired(escalation)) => {
            Ok(ClaimExecution::AuthorizationRequired(escalation))
        },
        LedgerTransactionOutcome::Rejected(ClaimRejection::Replay(error)) => {
            Err(ClaimError::ReservationReplay(error))
        },
        LedgerTransactionOutcome::Rejected(ClaimRejection::EdgeReplay(error)) => {
            Err(ClaimError::EdgeReplay(error))
        },
        LedgerTransactionOutcome::Rejected(ClaimRejection::InactiveMarkerRun(
            coordination_run_id,
        )) => Err(ClaimError::InactiveMarkerRun(coordination_run_id)),
        LedgerTransactionOutcome::Rejected(ClaimRejection::ReservationLimitReached(maximum)) => {
            Ok(ClaimExecution::ReservationLimitReached(maximum))
        },
        LedgerTransactionOutcome::Rejected(ClaimRejection::OrderingEdgeLimitReached(maximum)) => {
            Ok(ClaimExecution::OrderingEdgeLimitReached(maximum))
        },
    }
}

fn validate_claim_transaction(
    state: &ReplayedLedgerState<'_>,
    prepared_claim: PreparedClaim,
    context: ClaimValidationContext,
) -> TransactionValidation<ClaimRejection> {
    let ClaimValidationContext {
        run_validation,
        coordination_run_id,
        path_case,
        requester,
        overlap_authorization,
        maximum_reservations,
        maximum_ordering_edges,
    } = context;
    let reservations = match RetainedReservationSet::replay(state.events()) {
        Ok(reservations) => reservations,
        Err(error) => return TransactionValidation::Reject(ClaimRejection::Replay(error)),
    };
    if let Err(rejection) = run_validation.validate(&reservations) {
        return TransactionValidation::Reject(rejection);
    }
    if count_reaches_limit(reservations.nonterminal_count(), maximum_reservations) {
        return TransactionValidation::Reject(ClaimRejection::ReservationLimitReached(
            maximum_reservations,
        ));
    }
    let ordering_graph = match OrderingGraph::replay(state.events()) {
        Ok(ordering_graph) => ordering_graph,
        Err(error) => {
            return TransactionValidation::Reject(ClaimRejection::EdgeReplay(error));
        },
    };
    let conflicts =
        reservations.conflicts_for_claim(&prepared_claim.scopes, coordination_run_id, path_case);
    match overlap_authorization {
        OverlapAuthorizationRequest::Absent if conflicts.is_empty() => {
            TransactionValidation::Append(Box::new(
                prepared_claim.into_operation(ConflictAuthorization::NoConflict),
            ))
        },
        OverlapAuthorizationRequest::Absent => {
            TransactionValidation::Reject(ClaimRejection::Conflict(conflicts))
        },
        OverlapAuthorizationRequest::Permissive(request) => validate_authorization(
            *request,
            conflicts,
            requester,
            prepared_claim,
            &ordering_graph,
            maximum_ordering_edges,
        ),
    }
}

fn validate_authorization(
    request: PermissiveOverlapAuthorizationRequest,
    conflicts: Vec<ReservationConflict>,
    requester: OverlapRequester,
    prepared_claim: PreparedClaim,
    ordering_graph: &OrderingGraph,
    maximum_ordering_edges: u32,
) -> TransactionValidation<ClaimRejection> {
    let PermissiveOverlapAuthorizationRequest {
        answer,
        reason,
        proposal_submission,
    } = request;
    let [conflict] = conflicts.as_slice() else {
        return TransactionValidation::Reject(ClaimRejection::Conflict(conflicts));
    };
    if conflict.reservation_id != answer.blocker() {
        return TransactionValidation::Reject(ClaimRejection::Conflict(conflicts));
    }
    let edge_effect = match &answer {
        PermissiveOverlapAnswer::Sequence { .. } => ClaimEdgeEffect::Adds,
        PermissiveOverlapAnswer::Defer { .. } | PermissiveOverlapAnswer::Override { .. } => {
            ClaimEdgeEffect::Unchanged
        },
    };
    let proposal =
        OverlapProposal::recompute(requester, reason, &prepared_claim.scopes, answer, conflict);
    match proposal_submission {
        OverlapProposalSubmission::Apply(proposal_token) if proposal_token.matches(&proposal) => {
            if matches!(edge_effect, ClaimEdgeEffect::Adds)
                && count_reaches_limit(ordering_graph.edge_count(), maximum_ordering_edges)
            {
                return TransactionValidation::Reject(ClaimRejection::OrderingEdgeLimitReached(
                    maximum_ordering_edges,
                ));
            }
            let authorization = ConflictAuthorization::from_approved_proposal(proposal);
            TransactionValidation::Append(Box::new(prepared_claim.into_operation(authorization)))
        },
        OverlapProposalSubmission::Mint | OverlapProposalSubmission::Apply(_) => {
            TransactionValidation::Reject(ClaimRejection::AuthorizationRequired(Box::new(
                proposal.escalation(conflicts),
            )))
        },
    }
}

#[derive(Clone, Copy)]
enum ClaimEdgeEffect {
    Unchanged,
    Adds,
}

fn count_reaches_limit(count: usize, maximum: u32) -> bool {
    u64::try_from(count).map_or(true, |count| count >= u64::from(maximum))
}

impl PreparedClaim {
    fn into_operation(self, authorization: ConflictAuthorization) -> JournalOperation {
        JournalOperation::Claim {
            reservation_id: self.reservation_id,
            scopes: self.scopes,
            source: self.source,
            purpose: self.purpose,
            trunk_at_claim: self.trunk_at_claim,
            head_snapshot: self.head_snapshot,
            phase_start_head: self.phase_start_head,
            worktree_root: self.worktree_root,
            worktree_administrative_locator: self.worktree_administrative_locator,
            authorization,
        }
    }
}

impl ClaimCoordinationRunSelection {
    fn resolve(self, worktree_administrative_directory: &Path) -> ClaimRunValidation {
        match self {
            Self::Specified(coordination_run_id) => {
                ClaimRunValidation::IndependentWithPresentedIdentity(coordination_run_id)
            },
            Self::ContinueOrStart => {
                match EditAuthorization::resolve(worktree_administrative_directory) {
                    EditAuthorization::Environment(coordination_run_id) => {
                        ClaimRunValidation::IndependentWithPresentedIdentity(coordination_run_id)
                    },
                    EditAuthorization::Marker {
                        coordination_run_id,
                        worktree_id,
                    } => ClaimRunValidation::ActiveMarkerRequired {
                        coordination_run_id,
                        worktree_id,
                    },
                    EditAuthorization::Unidentified => {
                        ClaimRunValidation::IndependentWithoutPresentedIdentity {
                            actor_run_id: CoordinationRunId::new(),
                        }
                    },
                }
            },
        }
    }
}

impl ClaimRunValidation {
    const fn actor_run_id(self) -> CoordinationRunId {
        match self {
            Self::IndependentWithPresentedIdentity(actor_run_id)
            | Self::IndependentWithoutPresentedIdentity { actor_run_id }
            | Self::ActiveMarkerRequired {
                coordination_run_id: actor_run_id,
                ..
            } => actor_run_id,
        }
    }

    const fn presented_coordination_identity(self) -> RequesterCoordinationIdentity {
        match self {
            Self::IndependentWithPresentedIdentity(coordination_run_id) => {
                RequesterCoordinationIdentity::Presented(coordination_run_id)
            },
            Self::IndependentWithoutPresentedIdentity { .. } => {
                RequesterCoordinationIdentity::NotPresented
            },
            Self::ActiveMarkerRequired {
                coordination_run_id,
                ..
            } => RequesterCoordinationIdentity::Presented(coordination_run_id),
        }
    }

    fn validate(self, reservations: &RetainedReservationSet) -> Result<(), ClaimRejection> {
        let Self::ActiveMarkerRequired {
            coordination_run_id,
            worktree_id,
        } = self
        else {
            return Ok(());
        };
        if reservations.iter().any(|reservation| {
            reservation.actor().run == coordination_run_id
                && reservation.actor().worktree == worktree_id
                && matches!(
                    reservation.lifecycle(),
                    crate::reservation::ReservationLifecycle::Active
                )
        }) {
            Ok(())
        } else {
            Err(ClaimRejection::InactiveMarkerRun(coordination_run_id))
        }
    }
}

impl ClaimRepositoryFacts {
    fn read(worktree_context: &WorktreeContext) -> Result<Self, ClaimError> {
        let repository_root = worktree_context.repository_root();
        let current_head =
            read_git_object_id(repository_root, &[GIT_REV_PARSE_COMMAND, GIT_HEAD_REVISION])?;
        let head_snapshot = read_head_snapshot(repository_root, current_head.clone())?;
        let worktree_root = worktree_context
            .repository_root()
            .to_str()
            .ok_or(ClaimError::NonUtf8WorktreeRoot)?
            .parse()
            .map_err(|_| ClaimError::InvalidCanonicalWorktreeRoot)?;
        Ok(Self {
            head_snapshot,
            current_head,
            worktree_root,
        })
    }
}

fn read_trunk_commit(
    repository_root: &Path,
    trunk: &str,
) -> Result<TrunkCommitAtClaim, ClaimError> {
    let trunk_ref = format!("{HEADS_REF_PREFIX}{trunk}");
    read_git_object_id(repository_root, &[GIT_REV_PARSE_COMMAND, &trunk_ref])
        .map(TrunkCommitAtClaim::from)
}

fn read_head_snapshot(
    repository_root: &Path,
    current_head: GitObjectId,
) -> Result<ClaimHeadSnapshot, ClaimError> {
    let output = git_output(
        repository_root,
        &[
            GIT_SYMBOLIC_REF_COMMAND,
            GIT_FULL_REF_NAME_ARG,
            GIT_HEAD_REVISION,
        ],
    )?;
    if output.status.success() {
        let full_ref = String::from_utf8(output.stdout)?
            .trim()
            .parse()
            .map_err(|_| ClaimError::InvalidHeadReference)?;
        Ok(ClaimHeadSnapshot::Branch {
            full_ref,
            head: ClaimHeadCommit::from(current_head),
        })
    } else if output.status.code() == Some(1) {
        Ok(ClaimHeadSnapshot::Detached {
            head: ClaimHeadCommit::from(current_head),
        })
    } else {
        Err(ClaimError::GitCommandFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

fn read_git_object_id(
    repository_root: &Path,
    arguments: &[&str],
) -> Result<GitObjectId, ClaimError> {
    let output = git_output(repository_root, arguments)?;
    if !output.status.success() {
        return Err(ClaimError::GitCommandFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    String::from_utf8(output.stdout)?
        .trim()
        .parse()
        .map_err(ClaimError::InvalidGitObjectId)
}

fn git_output(repository_root: &Path, arguments: &[&str]) -> Result<Output, std::io::Error> {
    Command::new(GIT_BINARY)
        .arg(GIT_NO_OPTIONAL_LOCKS_ARG)
        .args(arguments)
        .current_dir(repository_root)
        .output()
}

enum ClaimRejection {
    Conflict(Vec<ReservationConflict>),
    AuthorizationRequired(Box<OverlapEscalationPayload>),
    Replay(ReservationReplayError),
    InactiveMarkerRun(CoordinationRunId),
    ReservationLimitReached(u32),
    OrderingEdgeLimitReached(u32),
    EdgeReplay(EdgeReplayError),
}

#[derive(Debug)]
enum ClaimError {
    Io(std::io::Error),
    Config(ConfigError),
    Ledger(LedgerError),
    PathCase(PathCaseError),
    Transaction(LedgerTransactionError),
    ReservationReplay(ReservationReplayError),
    EdgeReplay(EdgeReplayError),
    InactiveMarkerRun(CoordinationRunId),
    InvalidGitObjectId(InvalidGitObjectId),
    InvalidUtf8(FromUtf8Error),
    GitCommandFailed(String),
    InvalidHeadReference,
    NonUtf8WorktreeRoot,
    InvalidCanonicalWorktreeRoot,
}

impl Display for ClaimError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "claim I/O failed: {error}"),
            Self::Config(error) => error.fmt(formatter),
            Self::Ledger(error) => error.fmt(formatter),
            Self::PathCase(error) => error.fmt(formatter),
            Self::Transaction(error) => error.fmt(formatter),
            Self::ReservationReplay(error) => {
                write!(formatter, "reservation replay failed: {error}")
            },
            Self::EdgeReplay(error) => write!(formatter, "ordering replay failed: {error}"),
            Self::InactiveMarkerRun(coordination_run_id) => write!(
                formatter,
                "coordination-run marker {coordination_run_id} no longer has an active reservation"
            ),
            Self::InvalidGitObjectId(error) => error.fmt(formatter),
            Self::InvalidUtf8(error) => write!(formatter, "git output was not UTF-8: {error}"),
            Self::GitCommandFailed(stderr) => write!(formatter, "git command failed: {stderr}"),
            Self::InvalidHeadReference => {
                formatter.write_str("git returned an invalid full HEAD reference")
            },
            Self::NonUtf8WorktreeRoot => {
                formatter.write_str("the canonical worktree root is not UTF-8")
            },
            Self::InvalidCanonicalWorktreeRoot => {
                formatter.write_str("the worktree root is not a canonical absolute path")
            },
        }
    }
}

impl std::error::Error for ClaimError {}

impl From<std::io::Error> for ClaimError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

impl From<ConfigError> for ClaimError {
    fn from(error: ConfigError) -> Self { Self::Config(error) }
}

impl From<LedgerError> for ClaimError {
    fn from(error: LedgerError) -> Self { Self::Ledger(error) }
}

impl From<PathCaseError> for ClaimError {
    fn from(error: PathCaseError) -> Self { Self::PathCase(error) }
}

impl From<LedgerTransactionError> for ClaimError {
    fn from(error: LedgerTransactionError) -> Self { Self::Transaction(error) }
}

impl From<FromUtf8Error> for ClaimError {
    fn from(error: FromUtf8Error) -> Self { Self::InvalidUtf8(error) }
}
