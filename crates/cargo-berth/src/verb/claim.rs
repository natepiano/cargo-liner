//! Atomic reservation acquisition.

use std::fmt;
use std::path::Path;
use std::process::Command;
use std::process::Output;

use crate::config::BerthConfig;
use crate::config::ConfigError;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::InvalidGitObjectId;
use crate::ids::ReservationId;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::ClaimHeadCommit;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimSource;
use crate::ledger::ConflictAuthorization;
use crate::ledger::EditAuthorization;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::LedgerTransactionOutcome;
use crate::ledger::ProtectedPhaseStartHead;
use crate::ledger::ReservationPurpose;
use crate::ledger::TransactionValidation;
use crate::ledger::TrunkCommitAtClaim;
use crate::ledger::WorktreeContext;
use crate::ledger::worktree_identity;
use crate::output::CommandVerb;
use crate::output::CoordinationRunMarkerPublication;
use crate::output::OutputEnvelope;
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
}

/// How a claim chooses the coordination run that will own its reservation.
pub(crate) enum ClaimCoordinationRunSelection {
    /// Use the run identity supplied through `--run`.
    Specified(CoordinationRunId),
    /// Continue the active run from process or worktree context, or start one.
    ContinueOrStart,
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

/// Acquire a reservation or return a typed conflict without appending.
pub(crate) fn execute(claim_request: ClaimRequest) -> OutputEnvelope {
    match execute_claim(claim_request) {
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
        Err(error) => OutputEnvelope::ledger_unreadable(CommandVerb::Claim, &error.to_string()),
    }
}

enum ClaimExecution {
    Claimed {
        reservation_id:      ReservationId,
        coordination_run_id: CoordinationRunId,
        scopes:              ReservationScopeSet,
        marker_publication:  CoordinationRunMarkerPublication,
    },
    Blocked(Vec<ReservationConflict>),
}

fn execute_claim(claim_request: ClaimRequest) -> Result<ClaimExecution, ClaimError> {
    let invocation_directory = std::env::current_dir()?;
    let worktree_context = WorktreeContext::discover(&invocation_directory)?;
    let coordination_run_id = claim_request
        .coordination_run_selection
        .resolve(worktree_context.administrative_directory());
    let path_case = PathCase::read(worktree_context.common_git_directory())?;
    let scopes = claim_request
        .declared_scopes
        .into_minimal_antichain(path_case);
    let claim_repository_facts = ClaimRepositoryFacts::read(&worktree_context)?;
    let phase_start_head = match claim_request.phase_start {
        PhaseStartSelection::CurrentHead => {
            ProtectedPhaseStartHead::from(claim_repository_facts.current_head.clone())
        },
        PhaseStartSelection::Protected(protected_phase_start_head) => protected_phase_start_head,
    };
    let berth_config = BerthConfig::read(worktree_context.repository_root())?;
    let trunk_at_claim =
        read_trunk_commit(worktree_context.repository_root(), &berth_config.trunk)?;
    let worktree_identity = worktree_identity(
        worktree_context.administrative_directory(),
        worktree_context.worktree_kind(),
    )?;
    let ledger = Ledger::open(worktree_context.repository_root())?;
    let reservation_id = ReservationId::new();
    let operation_scopes = scopes.clone();
    let outcome = ledger.transact(worktree_identity.id, coordination_run_id, |state| {
        let reservations = match RetainedReservationSet::replay(state.events()) {
            Ok(reservations) => reservations,
            Err(error) => return TransactionValidation::Reject(ClaimRejection::Replay(error)),
        };
        let conflicts =
            reservations.conflicts_for_claim(&operation_scopes, coordination_run_id, path_case);
        if conflicts.is_empty() {
            TransactionValidation::Append(Box::new(JournalOperation::Claim {
                reservation_id,
                scopes: operation_scopes,
                source: claim_request.source,
                purpose: claim_request.purpose,
                trunk_at_claim,
                head_snapshot: claim_repository_facts.head_snapshot,
                phase_start_head,
                worktree_root: claim_repository_facts.worktree_root,
                worktree_administrative_locator: worktree_context.administrative_locator().clone(),
                authorization: ConflictAuthorization::NoConflict,
            }))
        } else {
            TransactionValidation::Reject(ClaimRejection::Conflict(conflicts))
        }
    })?;
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
        LedgerTransactionOutcome::Rejected(ClaimRejection::Replay(error)) => {
            Err(ClaimError::ReservationReplay(error))
        },
    }
}

impl ClaimCoordinationRunSelection {
    fn resolve(self, worktree_administrative_directory: &Path) -> CoordinationRunId {
        match self {
            Self::Specified(coordination_run_id) => coordination_run_id,
            Self::ContinueOrStart => {
                match EditAuthorization::resolve(worktree_administrative_directory) {
                    EditAuthorization::Identified(coordination_run_id) => coordination_run_id,
                    EditAuthorization::Unidentified => CoordinationRunId::new(),
                }
            },
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
    Replay(ReservationReplayError),
}

#[derive(Debug)]
enum ClaimError {
    Io(std::io::Error),
    Config(ConfigError),
    Ledger(LedgerError),
    PathCase(PathCaseError),
    Transaction(LedgerTransactionError),
    ReservationReplay(ReservationReplayError),
    InvalidGitObjectId(InvalidGitObjectId),
    InvalidUtf8(std::string::FromUtf8Error),
    GitCommandFailed(String),
    InvalidHeadReference,
    NonUtf8WorktreeRoot,
    InvalidCanonicalWorktreeRoot,
}

impl fmt::Display for ClaimError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "claim I/O failed: {error}"),
            Self::Config(error) => error.fmt(formatter),
            Self::Ledger(error) => error.fmt(formatter),
            Self::PathCase(error) => error.fmt(formatter),
            Self::Transaction(error) => error.fmt(formatter),
            Self::ReservationReplay(error) => {
                write!(formatter, "reservation replay failed: {error}")
            },
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

impl From<std::string::FromUtf8Error> for ClaimError {
    fn from(error: std::string::FromUtf8Error) -> Self { Self::InvalidUtf8(error) }
}
