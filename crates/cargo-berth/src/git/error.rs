//! The single failure type every git query in this module returns.
//!
//! Every git read here ends in one of two places: a `GitError`, or a completed
//! [`Output`]. `completed_git_command` is the boundary between them, so it lives
//! beside the error it produces rather than beside any one caller.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
#[cfg(target_os = "linux")]
use std::io::ErrorKind;
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;
use std::string::FromUtf8Error;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::SystemTime;

use super::command::GitCommandOutputAvailability;
use super::constants::GIT_LOCK_EXTENSION;
use super::constants::GIT_LOCK_REFUSAL_PREFIX;
use super::constants::GIT_LOCK_REFUSAL_SUFFIX;
use super::constants::GIT_UPDATE_REF_COMMAND;
use crate::ids::GitObjectId;
use crate::ids::InvalidGitObjectId;
use crate::ids::InvalidReservationScopePath;

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
    /// Every consumer of a snapshot sees the first unsuccessful read.
    SnapshotReadFailed(Arc<Self>),
    /// Git could not be started or read.
    Io(std::io::Error),
    /// Git completed unsuccessfully.
    CommandFailed {
        /// The git subcommand that failed.
        command: &'static str,
        /// The diagnostic git reported.
        stderr:  String,
    },
    /// A ref write was refused by an existing Git lock; the lock is never removed.
    LockRefused(Box<GitLockRefusal>),
    /// Git printed non-UTF-8 output where the operation requires UTF-8.
    InvalidOutput(FromUtf8Error),
    /// Git printed text that was not a full object id.
    InvalidObjectId(InvalidGitObjectId),
    /// Git printed a path that cannot identify a repository-relative reservation scope.
    InvalidReservationPath(InvalidReservationScopePath),
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
            Self::SnapshotReadFailed(error) => Display::fmt(error, formatter),
            Self::Io(error) => write!(formatter, "could not run git: {error}"),
            Self::CommandFailed { command, stderr } => {
                write!(formatter, "git {command} failed: {stderr}")
            },
            Self::LockRefused(lock) => Display::fmt(lock, formatter),
            Self::InvalidOutput(error) => {
                write!(formatter, "git printed non-UTF-8 output: {error}")
            },
            Self::InvalidObjectId(error) => {
                write!(formatter, "git printed an invalid object id: {error}")
            },
            Self::InvalidReservationPath(error) => {
                write!(
                    formatter,
                    "git printed an invalid reservation path: {error}"
                )
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

impl std::error::Error for GitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SnapshotReadFailed(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

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

/// A refused Git write and the lock observed at that refusal.
/// Holder discovery is delayed until reporting, after the ledger lock is released.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GitLockRefusal {
    path:       PathBuf,
    age:        GitLockAge,
    holders:    OnceLock<GitLockHolders>,
    diagnostic: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum GitLockAge {
    Observed(Duration),
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum GitLockHolders {
    OpenByProcesses(Vec<u32>),
    NoProcessHoldsIt,
    Unavailable,
}

impl GitLockRefusal {
    /// Render only a deferred refusal, never a successful maintenance result.
    pub(crate) fn maintenance_notice(&self) -> String {
        format!(
            "Retention-ref maintenance deferred: {self}. The next reconciliation will retry the maintenance."
        )
    }
}

impl Display for GitLockRefusal {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "git ref write refused by lock {} (age: ",
            self.path.display()
        )?;
        match self.age {
            GitLockAge::Observed(age) => write!(formatter, "{:.3} seconds", age.as_secs_f64())?,
            GitLockAge::Unavailable => formatter.write_str("unavailable")?,
        }
        formatter.write_str("; ")?;
        match self.holders.get_or_init(|| git_lock_holders(&self.path)) {
            GitLockHolders::OpenByProcesses(processes) => {
                write!(formatter, "held open by processes {processes:?}")?;
            },
            GitLockHolders::NoProcessHoldsIt => formatter.write_str("no process holds it")?,
            GitLockHolders::Unavailable => formatter.write_str("process holders unavailable")?,
        }
        write!(formatter, "): {}", self.diagnostic)
    }
}

/// Classify only Git's existing-lock refusal, preserving every other command failure.
pub(super) fn ref_write_failure(repository_root: &Path, stderr: &[u8]) -> GitError {
    let diagnostic = String::from_utf8_lossy(stderr).trim().to_owned();
    // Git quotes the pathname as a whole, so apostrophes within it are preserved.
    let lock_path = diagnostic
        .split_once(GIT_LOCK_REFUSAL_PREFIX)
        .and_then(|(_, refusal)| refusal.split_once(GIT_LOCK_REFUSAL_SUFFIX))
        .map(|(quoted, _)| quoted)
        .filter(|quoted| {
            Path::new(quoted)
                .extension()
                .is_some_and(|extension| extension == GIT_LOCK_EXTENSION)
        });
    if let Some(quoted) = lock_path {
        let path = repository_root.join(quoted);
        let age = std::fs::metadata(&path)
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .map_or(GitLockAge::Unavailable, GitLockAge::Observed);
        return GitError::LockRefused(Box::new(GitLockRefusal {
            path,
            age,
            holders: OnceLock::new(),
            diagnostic,
        }));
    }
    GitError::CommandFailed {
        command: GIT_UPDATE_REF_COMMAND,
        stderr:  diagnostic,
    }
}

#[cfg(target_os = "linux")]
fn git_lock_holders(path: &Path) -> GitLockHolders {
    let Ok(path) = std::fs::canonicalize(path) else {
        return GitLockHolders::Unavailable;
    };
    let Ok(metadata) = std::fs::metadata(&path) else {
        return GitLockHolders::Unavailable;
    };
    inspect_git_lock_holders(
        Path::new(super::constants::PROCESS_DIRECTORY),
        &path,
        metadata.uid(),
    )
}

/// An incomplete inspection of a lock-owner process cannot establish absence.
#[cfg(target_os = "linux")]
fn inspect_git_lock_holders(process_root: &Path, path: &Path, lock_owner: u32) -> GitLockHolders {
    use std::os::unix::fs::MetadataExt as _;

    let Ok(processes) = std::fs::read_dir(process_root) else {
        return GitLockHolders::Unavailable;
    };
    let mut holders = Vec::new();
    let mut incomplete = false;
    for process in processes {
        let Ok(process) = process else {
            incomplete = true;
            continue;
        };
        let Some(pid) = process
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let metadata = match process.metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(_) => {
                incomplete = true;
                continue;
            },
        };
        let owned = metadata.uid() == lock_owner;
        let Ok(descriptors) = std::fs::read_dir(
            process
                .path()
                .join(super::constants::PROCESS_DESCRIPTORS_DIRECTORY),
        ) else {
            incomplete |= owned && process.path().exists();
            continue;
        };
        for descriptor in descriptors {
            match descriptor.and_then(|descriptor| std::fs::read_link(descriptor.path())) {
                Ok(target) if target == path => {
                    holders.push(pid);
                    break;
                },
                Ok(_) => {},
                Err(error) if error.kind() == ErrorKind::NotFound => {},
                Err(_) => incomplete |= owned,
            }
        }
    }
    if !holders.is_empty() {
        GitLockHolders::OpenByProcesses(holders)
    } else if incomplete {
        GitLockHolders::Unavailable
    } else {
        GitLockHolders::NoProcessHoldsIt
    }
}

#[cfg(not(target_os = "linux"))]
fn git_lock_holders(path: &Path) -> GitLockHolders {
    let Ok(output) = std::process::Command::new(super::constants::LSOF_BINARY)
        .args([
            super::constants::LSOF_PROCESS_IDS_ARG,
            super::constants::GIT_PATHSPEC_SEPARATOR,
        ])
        .arg(path)
        .output()
    else {
        return GitLockHolders::Unavailable;
    };
    let holders = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|pid| pid.parse().ok())
        .collect::<Vec<_>>();
    if !holders.is_empty() {
        GitLockHolders::OpenByProcesses(holders)
    } else if output.status.code() == Some(1) && output.stderr.is_empty() {
        GitLockHolders::NoProcessHoldsIt
    } else {
        GitLockHolders::Unavailable
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use std::os::unix::fs::MetadataExt as _;

    #[cfg(target_os = "linux")]
    use super::GitLockHolders;
    #[cfg(target_os = "linux")]
    use super::inspect_git_lock_holders;
    use crate::git::RetentionMaintenance;
    use crate::output::CommandVerb;
    use crate::output::PostToolUseRendering;

    #[cfg(target_os = "linux")]
    #[test]
    fn an_uninspectable_lock_owner_process_prevents_a_no_holder_diagnosis()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = tempfile::tempdir()?;
        let lock = scratch.path().join("packed-refs.lock");
        std::fs::write(&lock, b"")?;
        let process = scratch.path().join("123");
        std::fs::create_dir(&process)?;
        // A surviving owner process whose descriptor directory cannot be read.
        std::fs::write(process.join("fd"), b"unavailable")?;
        let owner = std::fs::metadata(&lock)?.uid();
        assert!(matches!(
            inspect_git_lock_holders(scratch.path(), &lock, owner),
            GitLockHolders::Unavailable
        ));
        std::fs::remove_file(process.join("fd"))?;
        std::fs::create_dir(process.join("fd"))?;
        assert!(matches!(
            inspect_git_lock_holders(scratch.path(), &lock, owner),
            GitLockHolders::NoProcessHoldsIt
        ));
        Ok(())
    }
    #[test]
    fn deferred_feedback_is_added_once_without_deduplicating_the_primary_answer()
    -> Result<(), Box<dyn std::error::Error>> {
        let scratch = tempfile::tempdir()?;
        let lock = scratch.path().join("packed-refs.lock");
        std::fs::write(&lock, b"")?;
        let diagnostic = format!("Unable to create '{}': File exists", lock.display());
        let super::GitError::LockRefused(refusal) =
            super::ref_write_failure(scratch.path(), diagnostic.as_bytes())
        else {
            return Err("expected a typed refusal".into());
        };
        let notice = refusal.maintenance_notice();
        let maintenance = RetentionMaintenance::Deferred(refusal);
        let output = crate::output::OutputEnvelope::invalid_input(CommandVerb::Drift, &notice)
            .with_retention_maintenance(&maintenance)
            .with_retention_maintenance(&maintenance);
        let published = serde_json::to_value(&output)?;
        let message = published["message"].as_str().ok_or("message")?;
        assert_eq!(
            message
                .matches("Retention-ref maintenance deferred:")
                .count(),
            2,
            "the primary answer is independent of the single attached maintenance outcome"
        );
        let PostToolUseRendering::Feedback { detail, .. } = output.post_tool_use_rendering() else {
            return Err("expected hook feedback".into());
        };
        assert_eq!(
            detail
                .matches("Retention-ref maintenance deferred:")
                .count(),
            2,
            "rendering must not suppress maintenance because the primary answer contains the same text"
        );
        let plain = crate::output::OutputEnvelope::invalid_input(CommandVerb::Drift, "primary")
            .with_retention_maintenance(&maintenance)
            .with_retention_maintenance(&maintenance);
        assert_eq!(
            plain
                .render_text()
                .matches("Retention-ref maintenance deferred:")
                .count(),
            1
        );
        let report = serde_json::from_value(serde_json::json!({
            "comparison": "cheap_delta", "widening": {"status": "not_needed"},
            "results": [], "scope_acquisition": {"status": "permitted"}
        }))?;
        let refreshed = crate::output::OutputEnvelope::drift(report)
            .with_retention_maintenance(&maintenance)
            .with_alerts(Vec::new());
        let published = serde_json::to_value(&refreshed)?;
        assert_eq!(
            published["presentation"]["blocks"]
                .as_array()
                .ok_or("presentation blocks")?
                .len(),
            1,
            "refreshing alerts must not bake maintenance into the primary presentation"
        );
        Ok(())
    }
}
