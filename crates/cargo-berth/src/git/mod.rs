//! The small git subprocess surface required by the ledger.

mod command;
mod constants;
mod refs;

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Write;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::string::FromUtf8Error;

use command::git_output;
use command::git_output_dynamic;
use command::git_output_dynamic_with_input;
use constants::GIT_ANCESTRY_PATH_ARG_PREFIX;
use constants::GIT_BATCH_CHECK_ARG;
use constants::GIT_CAT_FILE_COMMAND;
use constants::GIT_COMMIT_PEEL_SUFFIX;
use constants::GIT_COMMON_DIRECTORY_ARG;
use constants::GIT_EXCLUDE_REVISION_PREFIX;
use constants::GIT_EXISTS_ARG;
use constants::GIT_HEAD_REVISION;
use constants::GIT_HOOKS_PATH;
use constants::GIT_IS_ANCESTOR_ARG;
use constants::GIT_LOCAL_BRANCH_REF_PREFIX;
use constants::GIT_MERGE_BASE_COMMAND;
use constants::GIT_MISSING_OBJECT_SUFFIX;
use constants::GIT_NOT_ANCESTOR_EXIT_CODE;
use constants::GIT_NUL_TERMINATED_ARG;
use constants::GIT_PATH_ARG;
use constants::GIT_PATH_FORMAT_ABSOLUTE_ARG;
use constants::GIT_PORCELAIN_ARG;
use constants::GIT_REV_LIST_COMMAND;
use constants::GIT_REV_PARSE_COMMAND;
use constants::GIT_SHOW_TOPLEVEL_ARG;
use constants::GIT_UPDATE_REF_COMMAND;
use constants::GIT_WORKTREE_COMMAND;
use constants::GIT_WORKTREE_LIST_ARG;

use crate::ids::GitObjectId;
use crate::ids::InvalidGitObjectId;
use crate::ids::ReservationId;

/// Resolve the shared administrative directory for a repository worktree.
pub(crate) fn common_directory(repository_root: &Path) -> Result<PathBuf, GitError> {
    let output = git_output(
        repository_root,
        [GIT_REV_PARSE_COMMAND, GIT_COMMON_DIRECTORY_ARG],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    let git_directory = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let path = PathBuf::from(git_directory.trim());
    Ok(if path.is_absolute() {
        path
    } else {
        repository_root.join(path)
    })
}

/// Resolve the worktree root for an invocation from anywhere in the repository.
pub(crate) fn repository_root(invocation_directory: &Path) -> Result<PathBuf, GitError> {
    let output = git_output(
        invocation_directory,
        [GIT_REV_PARSE_COMMAND, GIT_SHOW_TOPLEVEL_ARG],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    let repository_root = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let path = PathBuf::from(repository_root.trim());
    Ok(if path.is_absolute() {
        path
    } else {
        invocation_directory.join(path)
    })
}

/// Resolve the hook directory Git uses after applying `core.hooksPath`.
pub(crate) fn hooks_directory(repository_root: &Path) -> Result<PathBuf, GitError> {
    let output = git_output(
        repository_root,
        [
            GIT_REV_PARSE_COMMAND,
            GIT_PATH_FORMAT_ABSOLUTE_ARG,
            GIT_PATH_ARG,
            GIT_HOOKS_PATH,
        ],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let hooks_directory = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    Ok(PathBuf::from(hooks_directory.trim()))
}

/// Read the full object id currently named by `HEAD`.
pub(crate) fn head_object_id(repository_root: &Path) -> Result<GitObjectId, GitError> {
    object_id(repository_root, GIT_HEAD_REVISION)
}

/// Read the full object id currently named by a local branch.
pub(crate) fn branch_object_id(
    repository_root: &Path,
    branch: &str,
) -> Result<GitObjectId, GitError> {
    object_id(
        repository_root,
        &format!("{GIT_LOCAL_BRANCH_REF_PREFIX}{branch}"),
    )
}

/// Return every commit that would become reachable from `proposed` but not `previous`.
pub(crate) fn newly_reachable_commits(
    repository_root: &Path,
    previous: &GitObjectId,
    proposed: &GitObjectId,
) -> Result<Vec<GitObjectId>, GitError> {
    let arguments = vec![
        GIT_REV_LIST_COMMAND.to_owned(),
        proposed.to_string(),
        format!("{GIT_EXCLUDE_REVISION_PREFIX}{previous}"),
    ];
    let output = git_output_dynamic(repository_root, &arguments)?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_LIST_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    String::from_utf8(output.stdout)
        .map_err(GitError::InvalidOutput)?
        .lines()
        .map(|line| line.parse().map_err(GitError::InvalidObjectId))
        .collect()
}

/// Return every commit reachable from a proposed initial trunk object.
pub(crate) fn reachable_commits(
    repository_root: &Path,
    proposed: &GitObjectId,
) -> Result<Vec<GitObjectId>, GitError> {
    let arguments = vec![GIT_REV_LIST_COMMAND.to_owned(), proposed.to_string()];
    let output = git_output_dynamic(repository_root, &arguments)?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_LIST_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    String::from_utf8(output.stdout)
        .map_err(GitError::InvalidOutput)?
        .lines()
        .map(|line| line.parse().map_err(GitError::InvalidObjectId))
        .collect()
}

/// Atomically move one local branch from the expected old object to a proposed object.
pub(crate) fn update_local_branch(
    repository_root: &Path,
    branch: &str,
    proposed: &GitObjectId,
    expected_previous: &GitObjectId,
) -> Result<(), GitError> {
    match reachability(repository_root, expected_previous, proposed)? {
        Reachability::Ancestor => {},
        Reachability::NotAncestor => {
            return Err(GitError::NonFastForwardBranchUpdate {
                previous: expected_previous.clone(),
                proposed: proposed.clone(),
            });
        },
        Reachability::ObjectUnknown => {
            return Err(GitError::BranchUpdateObjectUnavailable {
                previous: expected_previous.clone(),
                proposed: proposed.clone(),
            });
        },
    }
    let reference = format!("{GIT_LOCAL_BRANCH_REF_PREFIX}{branch}");
    let proposed = proposed.to_string();
    let expected_previous = expected_previous.to_string();
    let output = git_output(
        repository_root,
        [
            GIT_UPDATE_REF_COMMAND,
            &reference,
            &proposed,
            &expected_previous,
        ],
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GitError::CommandFailed {
            command: GIT_UPDATE_REF_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

/// Resolve a revision while treating an ordinary unresolved name as a typed absence.
pub(crate) fn reference_lookup(
    repository_root: &Path,
    revision: &str,
) -> Result<ReferenceLookup, GitError> {
    let output = git_output(repository_root, [GIT_REV_PARSE_COMMAND, revision])?;
    if !output.status.success() {
        return Ok(ReferenceLookup::Missing);
    }
    let object_id = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    object_id
        .trim()
        .parse()
        .map(ReferenceLookup::Present)
        .map_err(GitError::InvalidObjectId)
}

/// Return whether git can still read one commit object.
pub(crate) fn commit_is_available(
    repository_root: &Path,
    object_id: &GitObjectId,
) -> Result<bool, GitError> {
    let revision = format!("{object_id}{GIT_COMMIT_PEEL_SUFFIX}");
    let output = git_output(
        repository_root,
        [GIT_CAT_FILE_COMMAND, GIT_EXISTS_ARG, &revision],
    )?;
    Ok(output.status.success())
}

/// Read git's NUL-delimited registered-worktree representation.
pub(crate) fn worktree_list_porcelain(repository_root: &Path) -> Result<Vec<u8>, GitError> {
    let output = git_output(
        repository_root,
        [
            GIT_WORKTREE_COMMAND,
            GIT_WORKTREE_LIST_ARG,
            GIT_PORCELAIN_ARG,
            GIT_NUL_TERMINATED_ARG,
        ],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_WORKTREE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(output.stdout)
}

/// Determine whether one commit is an ancestor of another.
pub(crate) fn reachability(
    repository_root: &Path,
    ancestor: &GitObjectId,
    descendant: &GitObjectId,
) -> Result<Reachability, GitError> {
    let ancestor = ancestor.to_string();
    let descendant = descendant.to_string();
    let output = git_output(
        repository_root,
        [
            GIT_MERGE_BASE_COMMAND,
            GIT_IS_ANCESTOR_ARG,
            &ancestor,
            &descendant,
        ],
    )?;
    if output.status.success() {
        Ok(Reachability::Ancestor)
    } else if output.status.code() == Some(GIT_NOT_ANCESTOR_EXIT_CODE) {
        Ok(Reachability::NotAncestor)
    } else {
        Ok(Reachability::ObjectUnknown)
    }
}

/// Find supplied holder heads that descend from one protected predecessor tip.
pub(crate) fn descendant_commits(
    repository_root: &Path,
    ancestor: &GitObjectId,
    candidate_heads: &[GitObjectId],
) -> Result<DescendantCommitQuery, GitError> {
    if candidate_heads.is_empty() {
        return Ok(DescendantCommitQuery::Classified(Vec::new()));
    }
    let mut queried_objects = Vec::with_capacity(candidate_heads.len() + 1);
    queried_objects.push(ancestor.clone());
    queried_objects.extend(candidate_heads.iter().cloned());
    let object_availability = commit_availability(repository_root, &queried_objects)?;
    let Some((ancestor_availability, candidate_availability)) = object_availability.split_first()
    else {
        return Err(GitError::InvalidBatchObjectCount {
            expected: queried_objects.len(),
            actual:   0,
        });
    };
    if matches!(ancestor_availability, CommitAvailability::ObjectUnknown) {
        return Ok(DescendantCommitQuery::AncestorObjectUnknown);
    }
    let available_heads = candidate_heads
        .iter()
        .zip(candidate_availability)
        .filter_map(|(head, availability)| match availability {
            CommitAvailability::Available => Some(head.clone()),
            CommitAvailability::ObjectUnknown => None,
        })
        .collect::<Vec<_>>();
    if available_heads.is_empty() {
        return Ok(DescendantCommitQuery::Classified(
            candidate_heads
                .iter()
                .cloned()
                .map(CandidateHeadReachability::ObjectUnknown)
                .collect(),
        ));
    }
    let ancestor_text = ancestor.to_string();
    let mut arguments = Vec::with_capacity(available_heads.len() + 3);
    arguments.push(GIT_REV_LIST_COMMAND.to_owned());
    arguments.extend(available_heads.iter().map(ToString::to_string));
    arguments.push(format!("{GIT_ANCESTRY_PATH_ARG_PREFIX}{ancestor_text}"));
    arguments.push(format!("{GIT_EXCLUDE_REVISION_PREFIX}{ancestor_text}"));
    let output = git_output_dynamic(repository_root, &arguments)?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_LIST_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let output_text = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let mut descendants = output_text
        .lines()
        .map(str::parse)
        .collect::<Result<Vec<GitObjectId>, _>>()
        .map_err(GitError::InvalidObjectId)?;
    descendants.push(ancestor.clone());
    Ok(DescendantCommitQuery::Classified(
        candidate_heads
            .iter()
            .zip(candidate_availability)
            .map(|(head, availability)| match availability {
                CommitAvailability::Available if descendants.contains(head) => {
                    CandidateHeadReachability::Descendant(head.clone())
                },
                CommitAvailability::Available => {
                    CandidateHeadReachability::NotDescendant(head.clone())
                },
                CommitAvailability::ObjectUnknown => {
                    CandidateHeadReachability::ObjectUnknown(head.clone())
                },
            })
            .collect(),
    ))
}

fn commit_availability(
    repository_root: &Path,
    object_ids: &[GitObjectId],
) -> Result<Vec<CommitAvailability>, GitError> {
    let input = object_ids
        .iter()
        .fold(String::new(), |mut input, object_id| {
            let _ = writeln!(input, "{object_id}{GIT_COMMIT_PEEL_SUFFIX}");
            input
        });
    let arguments = [
        GIT_CAT_FILE_COMMAND.to_owned(),
        GIT_BATCH_CHECK_ARG.to_owned(),
    ];
    let output = git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes())?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_CAT_FILE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let output_text = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let availability = output_text
        .lines()
        .map(|line| {
            if line.ends_with(GIT_MISSING_OBJECT_SUFFIX) {
                CommitAvailability::ObjectUnknown
            } else {
                CommitAvailability::Available
            }
        })
        .collect::<Vec<_>>();
    if availability.len() != object_ids.len() {
        return Err(GitError::InvalidBatchObjectCount {
            expected: object_ids.len(),
            actual:   availability.len(),
        });
    }
    Ok(availability)
}

/// Create or update a reservation's retention ref.
pub(crate) fn write_reservation_retention_ref(
    repository_root: &Path,
    reservation_id: ReservationId,
    protected_tip: &GitObjectId,
) -> Result<(), GitError> {
    refs::write(repository_root, reservation_id, protected_tip)
}

/// Return the full private ref used to retain one reservation's protected tip.
pub(crate) fn reservation_retention_ref_name(reservation_id: ReservationId) -> String {
    refs::name(reservation_id)
}

/// Delete a reservation's retention ref.
pub(crate) fn delete_reservation_retention_ref(
    repository_root: &Path,
    reservation_id: ReservationId,
) -> Result<(), GitError> {
    refs::delete(repository_root, reservation_id)
}

fn object_id(repository_root: &Path, revision: &str) -> Result<GitObjectId, GitError> {
    let output = git_output(repository_root, [GIT_REV_PARSE_COMMAND, revision])?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let object_id = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    GitObjectId::from_str(object_id.trim()).map_err(GitError::InvalidObjectId)
}

/// The three outcomes of `git merge-base --is-ancestor`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Reachability {
    /// The first commit is reachable from the second.
    Ancestor,
    /// Both objects exist, but the first is not reachable from the second.
    NotAncestor,
    /// Git could not read one or both objects.
    ObjectUnknown,
}

/// One candidate head's relation to a protected predecessor tip.
pub(crate) enum CandidateHeadReachability {
    /// The candidate head contains the protected predecessor tip.
    Descendant(GitObjectId),
    /// The candidate head resolves but does not contain the predecessor tip.
    NotDescendant(GitObjectId),
    /// This candidate head does not resolve as a commit.
    ObjectUnknown(GitObjectId),
}

/// The grouped descendant result for one protected predecessor tip.
pub(crate) enum DescendantCommitQuery {
    /// Every candidate head received its own typed reachability result.
    Classified(Vec<CandidateHeadReachability>),
    /// The protected predecessor tip does not resolve as a commit.
    AncestorObjectUnknown,
}

#[derive(Clone, Copy)]
enum CommitAvailability {
    Available,
    ObjectUnknown,
}

/// Whether a full git reference currently resolves to an object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReferenceLookup {
    /// The reference resolves to this object id.
    Present(GitObjectId),
    /// Git reports no object under this reference name.
    Missing,
}

/// A failure while resolving git's shared administrative directory.
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
    /// Git printed a non-UTF-8 administrative path.
    InvalidOutput(FromUtf8Error),
    /// Git printed text that was not a full object id.
    InvalidObjectId(InvalidGitObjectId),
    /// `cat-file --batch-check` did not classify every submitted object.
    InvalidBatchObjectCount { expected: usize, actual: usize },
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
}

impl Display for GitError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "could not run git: {error}"),
            Self::CommandFailed { command, stderr } => {
                write!(formatter, "git {command} failed: {stderr}")
            },
            Self::InvalidOutput(error) => {
                write!(formatter, "git printed a non-UTF-8 path: {error}")
            },
            Self::InvalidObjectId(error) => {
                write!(formatter, "git printed an invalid object id: {error}")
            },
            Self::InvalidBatchObjectCount { expected, actual } => write!(
                formatter,
                "git cat-file classified {actual} objects when {expected} were submitted"
            ),
            Self::NonFastForwardBranchUpdate { previous, proposed } => write!(
                formatter,
                "refusing non-fast-forward branch update from {previous} to {proposed}"
            ),
            Self::BranchUpdateObjectUnavailable { previous, proposed } => write!(
                formatter,
                "could not verify a fast-forward branch update from {previous} to {proposed}"
            ),
        }
    }
}

impl std::error::Error for GitError {}

impl From<std::io::Error> for GitError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}
