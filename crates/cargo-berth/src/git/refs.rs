//! Git references: reading them, moving them, and the reservation retention refs.
//!
//! Every read of a reference — `HEAD`, a local branch, an arbitrary full ref — and every
//! write to one lives here, including the private `refs/` namespace that retains each
//! reservation's protected commit.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Write;
use std::path::Path;

use crate::git::command::GitHookExecutionPolicy;
use crate::git::command::git_output;
use crate::git::command::git_output_dynamic;
use crate::git::command::git_output_dynamic_with_hook_execution_policy;
use crate::git::command::git_output_dynamic_with_hook_execution_policy_and_input;
use crate::git::constants::GIT_FOR_EACH_REF_COMMAND;
use crate::git::constants::GIT_FULL_REF_FORMAT_ARG;
use crate::git::constants::GIT_HEAD_REVISION;
use crate::git::constants::GIT_LOCAL_BRANCH_REF_PREFIX;
use crate::git::constants::GIT_MAX_COUNT_ONE_ARG;
use crate::git::constants::GIT_POINTS_AT_ARG_PREFIX;
use crate::git::constants::GIT_REFLOG_COMMAND;
use crate::git::constants::GIT_REFLOG_SHOW_ARG;
use crate::git::constants::GIT_REFLOG_SUBJECT_FORMAT_ARG;
use crate::git::constants::GIT_REV_PARSE_COMMAND;
use crate::git::constants::GIT_STDIN_ARG;
use crate::git::constants::GIT_SYMBOLIC_REF_COMMAND;
use crate::git::constants::GIT_UPDATE_REF_COMMAND;
use crate::git::constants::RESERVATION_RETENTION_REF_PREFIX;
use crate::git::error::GitError;
use crate::git::error::completed_git_command;
use crate::git::object::CommitAvailability;
use crate::git::object::commit_availability;
use crate::git::object::object_id;
use crate::git::reachability::Reachability;
use crate::git::reachability::ResolvedBatchCommitCandidates;
use crate::git::reachability::reachability;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ledger::FullRefName;

const GIT_DETACHED_HEAD_EXIT_CODE: i32 = 1;
const GIT_MISSING_REFERENCE_EXIT_CODE: i32 = 2;
const GIT_QUIET_ARG: &str = "--quiet";
const GIT_SHOW_REF_COMMAND: &str = "show-ref";
const GIT_SHOW_REF_EXISTS_ARG: &str = "--exists";

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

/// Ask Git for the branch reference named by `HEAD`.
pub(crate) fn symbolic_head_reference(repository_root: &Path) -> Result<FullRefName, GitError> {
    let output = completed_git_command(
        git_output(
            repository_root,
            [GIT_SYMBOLIC_REF_COMMAND, GIT_HEAD_REVISION],
        )
        .into(),
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_SYMBOLIC_REF_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let reference = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    reference
        .trim()
        .parse()
        .map_err(|_| GitError::InvalidReferenceName { reference })
}

/// Whether Git currently reports `HEAD` as attached to a branch or detached.
pub(crate) enum HeadAttachment {
    /// `HEAD` names this full branch reference.
    Branch { full_ref: FullRefName },
    /// `HEAD` names a commit directly.
    Detached,
}

/// Ask Git whether `HEAD` is attached to a branch or detached.
pub(crate) fn head_attachment(repository_root: &Path) -> Result<HeadAttachment, GitError> {
    let output = git_output(
        repository_root,
        [GIT_SYMBOLIC_REF_COMMAND, GIT_QUIET_ARG, GIT_HEAD_REVISION],
    )?;
    if output.status.success() {
        let reference = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
        return reference
            .trim()
            .parse()
            .map(|full_ref| HeadAttachment::Branch { full_ref })
            .map_err(|_| GitError::InvalidReferenceName { reference });
    }
    if output.status.code() == Some(GIT_DETACHED_HEAD_EXIT_CODE) {
        return Ok(HeadAttachment::Detached);
    }
    Err(GitError::CommandFailed {
        command: GIT_SYMBOLIC_REF_COMMAND,
        stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

/// Whether a full git reference currently resolves to an object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReferenceLookup {
    /// The reference resolves to this object id.
    Present(GitObjectId),
    /// Git reports no object under this reference name.
    Missing,
}

/// Resolve a full reference while preserving Git failures separately from a missing reference.
pub(crate) fn reference_lookup(
    repository_root: &Path,
    reference: &str,
) -> Result<ReferenceLookup, GitError> {
    reference
        .parse::<FullRefName>()
        .map_err(|_| GitError::InvalidReferenceName {
            reference: reference.to_owned(),
        })?;
    let existence_output = git_output(
        repository_root,
        [GIT_SHOW_REF_COMMAND, GIT_SHOW_REF_EXISTS_ARG, reference],
    )?;
    if !existence_output.status.success() {
        if existence_output.status.code() == Some(GIT_MISSING_REFERENCE_EXIT_CODE) {
            return Ok(ReferenceLookup::Missing);
        }
        return Err(GitError::CommandFailed {
            command: GIT_SHOW_REF_COMMAND,
            stderr:  String::from_utf8_lossy(&existence_output.stderr)
                .trim()
                .to_owned(),
        });
    }

    let output = git_output(repository_root, [GIT_REV_PARSE_COMMAND, reference])?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let object_id = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    object_id
        .trim()
        .parse()
        .map(ReferenceLookup::Present)
        .map_err(GitError::InvalidObjectId)
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

/// Whether one rename target was proven for a deleted local branch's object tip.
pub(crate) enum LocalBranchRenameTargetResolution {
    /// No local branch at the object proves a rename from the deleted branch.
    NotProven,
    /// Exactly one local branch at the object proves the rename.
    Unique(FullRefName),
    /// Several local branches prove the rename, so no single target can be chosen.
    Ambiguous,
}

/// Whether a local branch's newest reflog entry proves it replaced a deleted branch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalBranchRenameProof {
    /// The newest reflog entry records the candidate's rename from the deleted branch.
    Recorded,
    /// The candidate has no matching newest reflog entry.
    NotRecorded,
}

/// Find whether exactly one local branch at `tip` has proof it replaced the deleted branch.
pub(crate) fn local_branch_rename_target_resolution(
    repository_root: &Path,
    tip: &GitObjectId,
    deleted_reference: &FullRefName,
) -> Result<LocalBranchRenameTargetResolution, GitError> {
    let arguments = vec![
        GIT_FOR_EACH_REF_COMMAND.to_owned(),
        GIT_FULL_REF_FORMAT_ARG.to_owned(),
        format!("{GIT_POINTS_AT_ARG_PREFIX}{tip}"),
        GIT_LOCAL_BRANCH_REF_PREFIX.to_owned(),
    ];
    let output = git_output_dynamic(repository_root, &arguments)?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_FOR_EACH_REF_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let references = String::from_utf8(output.stdout)
        .map_err(GitError::InvalidOutput)?
        .lines()
        .map(|reference| {
            reference
                .parse::<FullRefName>()
                .map_err(|_| GitError::InvalidReferenceName {
                    reference: reference.to_owned(),
                })
        })
        .filter(|reference| {
            reference
                .as_ref()
                .map_or(true, |reference| reference != deleted_reference)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut proven_replacements = LocalBranchRenameTargetResolution::NotProven;
    for reference in references {
        match local_branch_rename_proof(repository_root, deleted_reference, &reference)? {
            LocalBranchRenameProof::Recorded => match proven_replacements {
                LocalBranchRenameTargetResolution::NotProven => {
                    proven_replacements = LocalBranchRenameTargetResolution::Unique(reference);
                },
                LocalBranchRenameTargetResolution::Unique(_)
                | LocalBranchRenameTargetResolution::Ambiguous => {
                    return Ok(LocalBranchRenameTargetResolution::Ambiguous);
                },
            },
            LocalBranchRenameProof::NotRecorded => {},
        }
    }
    Ok(proven_replacements)
}

/// Read whether `candidate_reference` records a rename from `deleted_reference`.
fn local_branch_rename_proof(
    repository_root: &Path,
    deleted_reference: &FullRefName,
    candidate_reference: &FullRefName,
) -> Result<LocalBranchRenameProof, GitError> {
    let candidate_reference = candidate_reference.to_string();
    let output = git_output(
        repository_root,
        [
            GIT_REFLOG_COMMAND,
            GIT_REFLOG_SHOW_ARG,
            GIT_MAX_COUNT_ONE_ARG,
            GIT_REFLOG_SUBJECT_FORMAT_ARG,
            &candidate_reference,
        ],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REFLOG_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let subject = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let expected_subject = format!("Branch: renamed {deleted_reference} to {candidate_reference}");
    Ok(match subject.lines().next() {
        Some(subject) if subject == expected_subject => LocalBranchRenameProof::Recorded,
        Some(_) | None => LocalBranchRenameProof::NotRecorded,
    })
}

/// One reservation ref that must retain its protected commit when that commit is readable.
pub(crate) struct ReservationRetentionRefRepair {
    reservation_id: ReservationId,
    protected_tip:  GitObjectId,
}

impl ReservationRetentionRefRepair {
    pub(crate) const fn new(reservation_id: ReservationId, protected_tip: GitObjectId) -> Self {
        Self {
            reservation_id,
            protected_tip,
        }
    }
}

/// The full private git ref that retains one reservation's protected tip.
struct ReservationRetentionRef(String);

impl ReservationRetentionRef {
    fn for_reservation(reservation_id: ReservationId) -> Self {
        Self(format!(
            "{RESERVATION_RETENTION_REF_PREFIX}{reservation_id}"
        ))
    }
}

impl Display for ReservationRetentionRef {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { formatter.write_str(&self.0) }
}

/// Return the full private ref used to retain one reservation's protected tip.
pub(crate) fn reservation_retention_ref_name(reservation_id: ReservationId) -> String {
    ReservationRetentionRef::for_reservation(reservation_id).to_string()
}

/// Create or update a reservation's retention ref.
pub(crate) fn write_reservation_retention_ref(
    repository_root: &Path,
    reservation_id: ReservationId,
    protected_tip: &GitObjectId,
) -> Result<(), GitError> {
    let retention_ref = reservation_retention_ref_name(reservation_id);
    let protected_tip = protected_tip.to_string();
    let arguments = [
        GIT_UPDATE_REF_COMMAND.to_owned(),
        retention_ref,
        protected_tip,
    ];
    let output = git_output_dynamic_with_hook_execution_policy(
        repository_root,
        &arguments,
        GitHookExecutionPolicy::SuppressedForRetentionRef,
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

/// Apply all retention-ref repairs and deletions in one ref-mutating transaction.
pub(crate) fn update_reservation_retention_refs(
    repository_root: &Path,
    repairs: &[ReservationRetentionRefRepair],
    deletions: &[ReservationId],
) -> Result<(), GitError> {
    if repairs.is_empty() && deletions.is_empty() {
        return Ok(());
    }
    let protected_tips = repairs
        .iter()
        .map(|repair| repair.protected_tip.clone())
        .collect::<Vec<_>>();
    let availability = if protected_tips.is_empty() {
        Vec::new()
    } else {
        commit_availability(repository_root, &protected_tips)?
    };
    let input = repairs.iter().zip(availability).fold(
        String::new(),
        |mut input, (repair, availability)| {
            if matches!(availability, CommitAvailability::Available) {
                let _ = writeln!(
                    input,
                    "update {} {}",
                    reservation_retention_ref_name(repair.reservation_id),
                    repair.protected_tip
                );
            }
            input
        },
    );
    let input = deletions.iter().fold(input, |mut input, reservation_id| {
        let _ = writeln!(
            input,
            "delete {}",
            reservation_retention_ref_name(*reservation_id)
        );
        input
    });
    if input.is_empty() {
        return Ok(());
    }
    apply_transaction(repository_root, &input)
}

/// Apply retention changes using candidate commits resolved earlier in the same locked pass.
pub(crate) fn update_reservation_retention_refs_from_resolved_batch(
    repository_root: &Path,
    repairs: &[ReservationRetentionRefRepair],
    deletions: &[ReservationId],
    resolved_candidates: &ResolvedBatchCommitCandidates,
) -> Result<(), GitError> {
    let input = repairs
        .iter()
        .filter(|repair| resolved_candidates.contains(&repair.protected_tip))
        .fold(String::new(), |mut input, repair| {
            let _ = writeln!(
                input,
                "update {} {}",
                reservation_retention_ref_name(repair.reservation_id),
                repair.protected_tip
            );
            input
        });
    let input = deletions.iter().fold(input, |mut input, reservation_id| {
        let _ = writeln!(
            input,
            "delete {}",
            reservation_retention_ref_name(*reservation_id)
        );
        input
    });
    if input.is_empty() {
        return Ok(());
    }
    apply_transaction(repository_root, &input)
}

/// Apply one transaction containing only retention-ref writes and deletions.
fn apply_transaction(repository_root: &Path, input: &str) -> Result<(), GitError> {
    let arguments = [GIT_UPDATE_REF_COMMAND.to_owned(), GIT_STDIN_ARG.to_owned()];
    let output = git_output_dynamic_with_hook_execution_policy_and_input(
        repository_root,
        &arguments,
        GitHookExecutionPolicy::SuppressedForRetentionRef,
        input.as_bytes(),
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
