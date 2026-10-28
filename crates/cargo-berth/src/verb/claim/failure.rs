//! Claim rejections and error output.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use super::incomplete_answer::IncompleteClaimAnswerRejection;
use super::phase_candidate::PhaseReservationCandidate;
use super::request::ClaimRunValidationError;
use crate::answer::ContradictedOrdering;
use crate::config::ConfigError;
use crate::coordination_identity::CoordinationIdentityRejection;
use crate::edge::EdgeReplayError;
use crate::edge::OrderingCycle;
use crate::git::GitError;
use crate::ids::InvalidGitObjectId;
use crate::ids::ReservationId;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::TargetRefusal;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::reconcile::ReconcileError;
use crate::reservation::ReservationConflict;
use crate::reservation::ReservationReplayError;
use crate::scope::PathCaseError;

impl From<ClaimRunValidationError> for ClaimRejection {
    fn from(error: ClaimRunValidationError) -> Self {
        match error {
            ClaimRunValidationError::CoordinationIdentity(rejection) => {
                Self::CoordinationIdentity(rejection)
            },
            ClaimRunValidationError::InvalidCanonicalWorktreeRoot => {
                Self::InvalidCanonicalWorktreeRoot
            },
        }
    }
}

pub(super) enum ClaimRejection {
    IneligibleExplicitReservation(ReservationId),
    Conflict(Vec<ReservationConflict>),
    /// The supplied answer left current holders unanswered or named stale holders.
    IncompleteAnswer(Box<IncompleteClaimAnswerRejection>),
    /// An overlap answer named these blockers while nothing overlapped the requested scopes.
    AnsweredWithoutOverlap(Vec<ReservationId>),
    /// A sequence answer without `--replace` points opposite to these live edges.
    ContradictsLiveOrdering(Vec<ContradictedOrdering>),
    OrderingCycle(OrderingCycle),
    /// A `--replace` answer to these holders contradicted no live edge.
    NothingToReplace(Vec<ReservationId>),
    Replay(ReservationReplayError),
    CoordinationIdentity(CoordinationIdentityRejection),
    InvalidCanonicalWorktreeRoot,
    ReservationLimitReached(u32),
    OrderingEdgeLimitReached(u32),
    EdgeReplay(EdgeReplayError),
}

#[derive(Debug)]
pub(crate) enum ClaimError {
    CapacityRecovery(Box<ReconcileError>),
    InvalidTarget(TargetRefusal),
    InvalidRepositoryTrunk(String),
    Io(std::io::Error),
    Git(GitError),
    Config(ConfigError),
    Ledger(LedgerError),
    PathCase(PathCaseError),
    Transaction(LedgerTransactionError),
    ReservationReplay(ReservationReplayError),
    EdgeReplay(EdgeReplayError),
    CoordinationIdentity(CoordinationIdentityRejection),
    InvalidGitObjectId(InvalidGitObjectId),
    InvalidHeadReference,
    InvalidTrunkReference,
    MissingReference(String),
    InvalidStoredReference(String),
    SymbolicReferenceDepthExceeded {
        reference: String,
        maximum:   u8,
    },
    NonUtf8WorktreeRoot,
    InvalidCanonicalWorktreeRoot,
    IneligibleExplicitReservation(ReservationId),
    AmbiguousActiveRunReservations {
        candidates: Vec<PhaseReservationCandidate>,
    },
}

impl Display for ClaimError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityRecovery(error) => Display::fmt(error, formatter),
            Self::InvalidTarget(error) => formatter.write_str(&error.message()),
            Self::InvalidRepositoryTrunk(error) => formatter.write_str(error),
            Self::Io(error) => write!(formatter, "claim I/O failed: {error}"),
            Self::Git(error) => error.fmt(formatter),
            Self::Config(error) => error.fmt(formatter),
            Self::Ledger(error) => error.fmt(formatter),
            Self::PathCase(error) => error.fmt(formatter),
            Self::Transaction(error) => error.fmt(formatter),
            Self::ReservationReplay(error) => {
                write!(formatter, "reservation replay failed: {error}")
            },
            Self::EdgeReplay(error) => write!(formatter, "ordering replay failed: {error}"),
            Self::CoordinationIdentity(rejection) => rejection.fmt(formatter),
            Self::InvalidGitObjectId(error) => error.fmt(formatter),
            Self::InvalidHeadReference => {
                formatter.write_str("git returned an invalid full HEAD reference")
            },
            Self::InvalidTrunkReference => {
                formatter.write_str("the configured trunk is not a valid full git reference")
            },
            Self::MissingReference(reference) => {
                write!(formatter, "git reference {reference} does not exist")
            },
            Self::InvalidStoredReference(reference) => {
                write!(
                    formatter,
                    "git reference {reference} does not contain a full object id"
                )
            },
            Self::SymbolicReferenceDepthExceeded { reference, maximum } => write!(
                formatter,
                "git symbolic reference resolution reached its maximum depth of {maximum} at {reference}"
            ),
            Self::NonUtf8WorktreeRoot => {
                formatter.write_str("the canonical worktree root is not UTF-8")
            },
            Self::InvalidCanonicalWorktreeRoot => {
                formatter.write_str("the worktree root is not a canonical absolute path")
            },
            Self::IneligibleExplicitReservation(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} is not active work held by the acting coordination run and worktree"
            ),
            Self::AmbiguousActiveRunReservations { candidates } => write!(
                formatter,
                "first-touch reservation selection is ambiguous among active reservations {}",
                candidates
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl std::error::Error for ClaimError {}

impl ClaimError {
    pub(crate) fn into_output(self, command_verb: CommandVerb) -> OutputEnvelope {
        match self {
            Self::CapacityRecovery(error) => error.into_output(command_verb),
            Self::InvalidTarget(error) => {
                OutputEnvelope::invalid_input(command_verb, &error.message())
            },
            Self::InvalidRepositoryTrunk(error) => {
                OutputEnvelope::invalid_input(command_verb, &error)
            },
            Self::Transaction(error) => match error {
                LedgerTransactionError::CorrectableInput(error) => {
                    OutputEnvelope::invalid_input(command_verb, &error.to_string())
                },
                LedgerTransactionError::LockContention => {
                    OutputEnvelope::contention(command_verb, &error.to_string())
                },
                LedgerTransactionError::LedgerUnreadable(error) => {
                    OutputEnvelope::ledger_error(command_verb, &error)
                },
                error @ LedgerTransactionError::DerivedRecordTooLarge { .. } => {
                    OutputEnvelope::ledger_unreadable(command_verb, &error.to_string())
                },
            },
            Self::CoordinationIdentity(rejection) => {
                OutputEnvelope::coordination_identity_rejected(command_verb, rejection)
            },
            Self::Config(error) => {
                OutputEnvelope::ledger_error(command_verb, &LedgerError::Config(error))
            },
            Self::Ledger(error) => OutputEnvelope::ledger_error(command_verb, &error),
            Self::ReservationReplay(error) => OutputEnvelope::replay_failure(command_verb, &error),
            Self::IneligibleExplicitReservation(reservation_id) => OutputEnvelope::invalid_input(
                command_verb,
                &format!(
                    "reservation {reservation_id} is not active work held by the acting coordination run and worktree"
                ),
            ),
            Self::AmbiguousActiveRunReservations { candidates } => {
                OutputEnvelope::ambiguous_active_run_reservations(command_verb, &candidates)
            },
            error => OutputEnvelope::ledger_unreadable(command_verb, &error.to_string()),
        }
    }
}

impl From<std::io::Error> for ClaimError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

impl From<GitError> for ClaimError {
    fn from(error: GitError) -> Self { Self::Git(error) }
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
