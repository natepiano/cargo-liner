//! The failure families reported by ledger reads, transactions, and committed actions.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use super::journal::JournalError;
use super::lock::MutationLockError;
use super::projection::ProjectionError;
use crate::config::ConfigError;
use crate::git::GitError;
use crate::ids::InvalidUuidV7;
use crate::session::SessionIdentityStoreError;

/// A failure that leaves ledger state unreadable or unpublished.
#[derive(Debug)]
pub(crate) enum LedgerError {
    /// The shared ledger has not been initialized for this repository.
    NotInitialized,
    /// No containing `.git` directory or file was found by filesystem traversal.
    RepositoryNotFound,
    /// A `.git` file did not contain a usable `gitdir:` locator.
    InvalidGitDirectoryFile,
    /// A linked-worktree administrative directory had no valid common-directory locator.
    InvalidCommonDirectoryFile,
    /// A linked-worktree administrative directory was outside the common git directory.
    AdministrativeDirectoryOutsideCommonGitDirectory,
    /// A discovered administrative path was not UTF-8.
    NonUtf8AdministrativePath,
    /// The derived administrative locator did not satisfy the journal contract.
    InvalidAdministrativeLocator(String),
    /// A canonical worktree root could not be reconstructed during identity validation.
    InvalidCanonicalWorktreeRoot,
    /// Git could not locate the common administrative directory.
    Git(GitError),
    /// Ordinary filesystem access failed.
    Io(std::io::Error),
    /// Repository policy could not be initialized or read.
    Config(ConfigError),
    /// The append-only journal could not be replayed safely.
    Journal(JournalError),
    /// A validated fact could not be encoded for the journal.
    JournalEncoding(serde_json::Error),
    /// The projection cache could not be validated or published.
    Projection(ProjectionError),
    /// The disposable harness-session mapping could not be read or published.
    SessionIdentityStore(SessionIdentityStoreError),
    /// The mutation lock could not be acquired.
    MutationLock(MutationLockError),
    /// The stored repository identity is not a UUID-v7 value.
    InvalidRepoInstanceId(InvalidUuidV7),
    /// The stored worktree identity is not a UUID-v7 value.
    InvalidWorktreeId(InvalidUuidV7),
    /// A registered administrative directory did not prove the recorded holder identity.
    WorktreeIdentityMismatch,
    /// A journal event names a repository identity different from its ledger.
    RepositoryIdentityMismatch,
    /// The projection counter can no longer advance.
    ProjectionGenerationExhausted,
    /// The journal size could not be represented by the public reinitialization result.
    JournalSizeUnrepresentable,
    /// Best-effort bypass auditing could not wait for or encode the journal transaction.
    BypassAuditUnavailable,
}

impl Display for LedgerError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInitialized => formatter.write_str(
                "the cargo-berth ledger is not initialized; run cargo-berth init and retry",
            ),
            Self::RepositoryNotFound => {
                formatter.write_str("no containing git worktree could be found")
            },
            Self::InvalidGitDirectoryFile => {
                formatter.write_str("the worktree .git file has no valid gitdir locator")
            },
            Self::InvalidCommonDirectoryFile => formatter
                .write_str("the linked-worktree administrative directory has no valid commondir"),
            Self::AdministrativeDirectoryOutsideCommonGitDirectory => formatter.write_str(
                "the worktree administrative directory is outside the common git directory",
            ),
            Self::NonUtf8AdministrativePath => {
                formatter.write_str("a discovered git administrative path is not UTF-8")
            },
            Self::InvalidAdministrativeLocator(locator) => {
                write!(
                    formatter,
                    "invalid worktree administrative locator: {locator}"
                )
            },
            Self::InvalidCanonicalWorktreeRoot => {
                formatter.write_str("a validated worktree root is not canonical absolute UTF-8")
            },
            Self::Git(error) => write!(formatter, "could not locate ledger: {error}"),
            Self::Io(error) => write!(formatter, "ledger I/O failed: {error}"),
            Self::Config(error) => write!(formatter, "ledger configuration failed: {error}"),
            Self::Journal(error) => write!(formatter, "journal replay failed: {error}"),
            Self::JournalEncoding(error) => {
                write!(formatter, "journal encoding failed: {error}")
            },
            Self::Projection(error) => write!(formatter, "projection validation failed: {error}"),
            Self::SessionIdentityStore(error) => error.fmt(formatter),
            Self::MutationLock(error) => write!(formatter, "ledger mutation lock failed: {error}"),
            Self::InvalidRepoInstanceId(error) => {
                write!(formatter, "invalid stored repository identity: {error}")
            },
            Self::InvalidWorktreeId(error) => {
                write!(formatter, "invalid stored worktree identity: {error}")
            },
            Self::WorktreeIdentityMismatch => formatter
                .write_str("registered worktree identity does not match the recorded holder"),
            Self::RepositoryIdentityMismatch => {
                formatter.write_str("journal belongs to a different repository instance")
            },
            Self::ProjectionGenerationExhausted => {
                formatter.write_str("projection generation counter is exhausted")
            },
            Self::JournalSizeUnrepresentable => {
                formatter.write_str("journal size cannot be represented for reinitialization")
            },
            Self::BypassAuditUnavailable => {
                formatter.write_str("the bypass audit could not be journalled immediately")
            },
        }
    }
}

impl std::error::Error for LedgerError {}

impl From<GitError> for LedgerError {
    fn from(error: GitError) -> Self { Self::Git(error) }
}

impl From<std::io::Error> for LedgerError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

impl From<ConfigError> for LedgerError {
    fn from(error: ConfigError) -> Self { Self::Config(error) }
}

impl From<JournalError> for LedgerError {
    fn from(error: JournalError) -> Self { Self::Journal(error) }
}

impl From<ProjectionError> for LedgerError {
    fn from(error: ProjectionError) -> Self { Self::Projection(error) }
}

impl From<MutationLockError> for LedgerError {
    fn from(error: MutationLockError) -> Self { Self::MutationLock(error) }
}

/// A transaction failure classified for a stateful command boundary.
#[derive(Debug)]
pub(crate) enum LedgerTransactionError {
    /// Durable state could not be read or published reliably.
    LedgerUnreadable(LedgerError),
    /// Another live mutation retained the descriptor through the bounded retry window.
    LockContention,
    /// The proposal is validly classified as caller-correctable input.
    CorrectableInput(CorrectableTransactionInput),
}

impl LedgerTransactionError {
    pub(super) fn from_ledger_error(error: LedgerError) -> Self {
        match error {
            LedgerError::MutationLock(MutationLockError::AcquisitionTimedOut) => {
                Self::LockContention
            },
            ledger_error => Self::LedgerUnreadable(ledger_error),
        }
    }
}

impl From<LedgerError> for LedgerTransactionError {
    fn from(error: LedgerError) -> Self { Self::from_ledger_error(error) }
}

impl Display for LedgerTransactionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::LedgerUnreadable(error) => error.fmt(formatter),
            Self::LockContention => formatter.write_str(
                "another cargo-berth operation is still running; wait for it to finish, then retry",
            ),
            Self::CorrectableInput(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for LedgerTransactionError {}

/// A failure before or after an append that authorizes one committed side effect.
#[derive(Debug)]
pub(crate) enum LedgerCommittedActionError<CommittedActionError> {
    /// The locked journal transaction itself failed.
    Transaction(LedgerTransactionError),
    /// The journal append committed, but its authorized side effect failed.
    Action(CommittedActionError),
}

impl<CommittedActionError: fmt::Display> Display
    for LedgerCommittedActionError<CommittedActionError>
{
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transaction(error) => error.fmt(formatter),
            Self::Action(error) => error.fmt(formatter),
        }
    }
}

impl<CommittedActionError> std::error::Error for LedgerCommittedActionError<CommittedActionError> where
    CommittedActionError: std::error::Error + 'static
{
}

/// A rejected mutation input that the caller can reduce and submit again.
#[derive(Debug)]
pub(crate) enum CorrectableTransactionInput {
    /// The encoded journal fact exceeded the bounded record format.
    RecordTooLarge {
        /// The proposed record size including its newline.
        bytes:         usize,
        /// The maximum accepted record size.
        maximum_bytes: usize,
    },
}

impl Display for CorrectableTransactionInput {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecordTooLarge {
                bytes,
                maximum_bytes,
            } => write!(
                formatter,
                "the proposed reservation record is {bytes} bytes, above the {maximum_bytes}-byte limit; reduce its scopes or shorten its provenance and purpose, then retry"
            ),
        }
    }
}

impl std::error::Error for CorrectableTransactionInput {}
