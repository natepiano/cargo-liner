//! The single failure type every git query in this module returns.
//!
//! Every git read here ends in one of two places: a `GitError`, or a completed
//! [`Output`]. `completed_git_command` is the boundary between them, so it lives
//! beside the error it produces rather than beside any one caller.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::process::Output;
use std::string::FromUtf8Error;

use super::command::GitCommandOutputAvailability;
use crate::ids::GitObjectId;
use crate::ids::InvalidGitObjectId;

/// Every way a git query in this module can fail to produce a typed observation.
///
/// The variants cover four kinds of failure. Git itself could not be started or exited
/// unsuccessfully. Its output did not parse as the UTF-8 text, object ids, reference
/// names, or record grammars the caller requires. A required revision could not be
/// resolved to exactly one commit of the expected type, or a history read finished
/// without the walk it was asked for. And an operation ended without a result of its
/// own: a branch update refused because it would not fast-forward, or a parallel reader
/// that stopped before reporting.
#[derive(Debug)]
pub(crate) enum GitError {
    /// Git could not be started or read.
    Io(std::io::Error),
    /// Git completed unsuccessfully.
    CommandFailed {
        /// The git subcommand that failed.
        command: &'static str,
        /// The diagnostic git reported.
        stderr:  String,
    },
    /// Git printed non-UTF-8 output where the operation requires UTF-8.
    InvalidOutput(FromUtf8Error),
    /// Git printed text that was not a full object id.
    InvalidObjectId(InvalidGitObjectId),
    /// A supplied or returned full reference name is invalid.
    InvalidReferenceName { reference: String },
    /// `cat-file --batch-check` did not classify every submitted object.
    InvalidBatchObjectCount { expected: usize, actual: usize },
    /// `cat-file --batch-check` printed a record without an object status or type.
    InvalidBatchObjectLine { line: String },
    /// A batched scoped-history query printed a record outside its typed grammar.
    InvalidScopedHistoryLine { line: String },
    /// The expected branch object is not an ancestor of the proposed object.
    NonFastForwardBranchUpdate {
        previous: GitObjectId,
        proposed: GitObjectId,
    },
    /// Git could not verify both objects needed for a fast-forward branch update.
    BranchUpdateObjectUnavailable {
        previous: GitObjectId,
        proposed: GitObjectId,
    },
    /// `rev-list --count` printed something that was not a commit total.
    UncountableCommitRange {
        /// The range whose total could not be read.
        range: String,
    },
    /// A path-limited first-parent walk returned a commit absent from the full walk.
    ScopedCommitMissingFromTargetWalk { commit: GitObjectId },
    /// A target-history read completed without identifying its requested tip.
    MissingTargetCommitHistory,
    /// No object resolves from a required commit expression.
    MissingCommitExpression { expression: String },
    /// More than one object matches a required commit expression.
    AmbiguousCommitExpression { expression: String },
    /// A required commit expression resolves to another object type.
    WrongCommitExpressionType {
        expression:  String,
        object_type: String,
    },
    /// One independent Git read ended before returning its typed observation.
    ConcurrentReadWorkerPanicked { activity: &'static str },
    /// One parallel scoped-proof worker ended before returning its typed observation.
    ScopedPatchWorkerPanicked { activity: &'static str },
}

impl Display for GitError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "could not run git: {error}"),
            Self::CommandFailed { command, stderr } => {
                write!(formatter, "git {command} failed: {stderr}")
            },
            Self::InvalidOutput(error) => {
                write!(formatter, "git printed non-UTF-8 output: {error}")
            },
            Self::InvalidObjectId(error) => {
                write!(formatter, "git printed an invalid object id: {error}")
            },
            Self::InvalidReferenceName { reference } => {
                write!(formatter, "invalid full git reference name: {reference:?}")
            },
            Self::InvalidBatchObjectCount { expected, actual } => write!(
                formatter,
                "git cat-file classified {actual} objects when {expected} were submitted"
            ),
            Self::InvalidBatchObjectLine { line } => {
                write!(
                    formatter,
                    "git cat-file printed an invalid object record: {line:?}"
                )
            },
            Self::InvalidScopedHistoryLine { line } => {
                write!(
                    formatter,
                    "git printed an invalid scoped-history record: {line:?}"
                )
            },
            Self::NonFastForwardBranchUpdate { previous, proposed } => write!(
                formatter,
                "refusing non-fast-forward branch update from {previous} to {proposed}"
            ),
            Self::BranchUpdateObjectUnavailable { previous, proposed } => write!(
                formatter,
                "could not verify a fast-forward branch update from {previous} to {proposed}"
            ),
            Self::UncountableCommitRange { range } => {
                write!(formatter, "git could not count the commits in {range}")
            },
            Self::ScopedCommitMissingFromTargetWalk { commit } => write!(
                formatter,
                "git returned scoped commit {commit} outside the target's first-parent walk"
            ),
            Self::MissingTargetCommitHistory => {
                formatter.write_str("git returned an empty target commit history")
            },
            Self::MissingCommitExpression { expression } => {
                write!(
                    formatter,
                    "git commit expression {expression:?} does not resolve"
                )
            },
            Self::AmbiguousCommitExpression { expression } => {
                write!(
                    formatter,
                    "git commit expression {expression:?} is ambiguous"
                )
            },
            Self::WrongCommitExpressionType {
                expression,
                object_type,
            } => write!(
                formatter,
                "git commit expression {expression:?} resolves to a {object_type} object"
            ),
            Self::ConcurrentReadWorkerPanicked { activity } => {
                write!(
                    formatter,
                    "git read worker panicked while attempting to {activity}"
                )
            },
            Self::ScopedPatchWorkerPanicked { activity } => {
                write!(
                    formatter,
                    "scoped patch worker panicked while attempting to {activity}"
                )
            },
        }
    }
}

impl std::error::Error for GitError {}

impl From<std::io::Error> for GitError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

pub(super) fn completed_git_command(
    output_availability: GitCommandOutputAvailability,
) -> Result<Output, GitError> {
    match output_availability {
        GitCommandOutputAvailability::Available(output) => Ok(output),
        GitCommandOutputAvailability::Unavailable(error) => Err(GitError::Io(error)),
    }
}
