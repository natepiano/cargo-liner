//! Repository facts used to validate claims.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;

use super::constants::HEADS_REF_PREFIX;
use super::failure::ClaimError;
use super::request::ClaimRunValidation;
use crate::config::BerthConfig;
use crate::git;
use crate::git::HeadAttachment;
use crate::git::ReferenceLookup;
use crate::git::RepositorySnapshot;
use crate::ids::GitObjectId;
use crate::ledger;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::ClaimHeadCommit;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimTarget;
use crate::ledger::FullRefName;
use crate::ledger::TargetSelectionRequest;
use crate::ledger::TrunkObservationAtClaim;
use crate::ledger::WorktreeContext;

pub(super) struct ClaimRepositoryFacts {
    pub(super) repository:    Arc<RepositorySnapshot>,
    pub(super) head_snapshot: ClaimHeadSnapshot,
    pub(super) current_head:  GitObjectId,
    pub(super) worktree_root: CanonicalWorktreeRoot,
}

#[derive(Clone, Copy)]
struct SymbolicReferenceDepth(u8);

impl SymbolicReferenceDepth {
    const MAXIMUM: u8 = 32;
    const ROOT: Self = Self(0);

    fn descend(self, reference: &str) -> Result<Self, ClaimError> {
        if self.0 >= Self::MAXIMUM {
            return Err(ClaimError::SymbolicReferenceDepthExceeded {
                reference: reference.to_owned(),
                maximum:   Self::MAXIMUM,
            });
        }
        Ok(Self(self.0 + 1))
    }
}

enum FilesystemReferenceResolution {
    Resolved(GitObjectId),
    RequiresGitResolution {
        rejection_if_git_reports_missing: ClaimError,
    },
}

impl ClaimRepositoryFacts {
    pub(super) fn read(
        worktree_context: &WorktreeContext,
        run_validation: ClaimRunValidation,
    ) -> Result<Self, ClaimError> {
        let repository = Arc::new(git::RepositorySnapshot::new(worktree_context.clone()));
        let (current_head, head_snapshot) = if run_validation.requires_live_head_revalidation() {
            read_live_head_snapshot(&repository)?
        } else {
            read_head_snapshot_from_files(&repository)?
        };
        let worktree_root = worktree_context
            .repository_root()
            .to_str()
            .ok_or(ClaimError::NonUtf8WorktreeRoot)?
            .parse()
            .map_err(|_| ClaimError::InvalidCanonicalWorktreeRoot)?;
        Ok(Self {
            repository,
            head_snapshot,
            current_head,
            worktree_root,
        })
    }
}

fn read_live_head_snapshot(
    repository: &RepositorySnapshot,
) -> Result<(GitObjectId, ClaimHeadSnapshot), ClaimError> {
    let current_head = repository.head_object_id()?.clone();
    let head_snapshot = match repository.head_attachment()?.clone() {
        HeadAttachment::Branch { full_ref } => ClaimHeadSnapshot::Branch {
            full_ref,
            head: ClaimHeadCommit::from(current_head.clone()),
        },
        HeadAttachment::Detached => ClaimHeadSnapshot::Detached {
            head: ClaimHeadCommit::from(current_head.clone()),
        },
    };
    Ok((current_head, head_snapshot))
}

fn read_trunk_commit(
    worktree_context: &WorktreeContext,
    trunk: &str,
) -> Result<TrunkObservationAtClaim, ClaimError> {
    let trunk_ref = format!("{HEADS_REF_PREFIX}{trunk}");
    match read_reference(worktree_context, &trunk_ref) {
        Ok(trunk_commit) => Ok(TrunkObservationAtClaim::from(trunk_commit)),
        Err(ClaimError::MissingReference(reference)) => reference
            .parse::<FullRefName>()
            .map(TrunkObservationAtClaim::from)
            .map_err(|_| ClaimError::InvalidTrunkReference),
        Err(error) => Err(error),
    }
}

pub(super) fn selected_target_and_tip(
    worktree_context: &WorktreeContext,
    head_snapshot: &ClaimHeadSnapshot,
    request: TargetSelectionRequest<'_>,
    berth_config: &BerthConfig,
) -> Result<(ClaimTarget, TrunkObservationAtClaim), ClaimError> {
    let target = select_target(worktree_context, head_snapshot, request, berth_config)?;
    let tip = read_trunk_commit(worktree_context, target.target.short_name())?;
    Ok((target, tip))
}

fn select_target(
    worktree_context: &WorktreeContext,
    head_snapshot: &ClaimHeadSnapshot,
    request: TargetSelectionRequest<'_>,
    berth_config: &BerthConfig,
) -> Result<ClaimTarget, ClaimError> {
    let claimant_branch = match head_snapshot {
        ClaimHeadSnapshot::Branch { full_ref, .. } => Some(full_ref),
        ClaimHeadSnapshot::Detached { .. } => None,
    };
    let repository_trunk = berth_config
        .repository_trunk()
        .map_err(ClaimError::InvalidRepositoryTrunk)?;
    ledger::resolve_claim_target(
        worktree_context.common_git_directory(),
        claimant_branch,
        request,
        &repository_trunk,
        |target| read_reference(worktree_context, target.reference().as_str()).is_ok(),
    )
    .map_err(ClaimError::InvalidTarget)
}

fn read_head_snapshot_from_files(
    repository: &RepositorySnapshot,
) -> Result<(GitObjectId, ClaimHeadSnapshot), ClaimError> {
    let worktree_context = repository.context();
    let head = fs::read_to_string(worktree_context.administrative_directory().join("HEAD"))?;
    let head = head.trim();
    if let Some(reference) = head.strip_prefix("ref: ") {
        let full_ref = reference
            .parse()
            .or_else(|_| match repository.head_attachment() {
                Ok(HeadAttachment::Branch { full_ref }) => Ok(full_ref.clone()),
                Ok(HeadAttachment::Detached) | Err(_) => Err(ClaimError::InvalidHeadReference),
            })?;
        let current_head = read_reference(worktree_context, &full_ref.to_string())?;
        return Ok((
            current_head.clone(),
            ClaimHeadSnapshot::Branch {
                full_ref,
                head: ClaimHeadCommit::from(current_head),
            },
        ));
    }
    let current_head: GitObjectId = head.parse().map_err(ClaimError::InvalidGitObjectId)?;
    Ok((
        current_head.clone(),
        ClaimHeadSnapshot::Detached {
            head: ClaimHeadCommit::from(current_head),
        },
    ))
}

pub(in crate::verb) fn read_reference(
    worktree_context: &WorktreeContext,
    reference: &str,
) -> Result<GitObjectId, ClaimError> {
    match read_reference_from_files(worktree_context.common_git_directory(), reference) {
        FilesystemReferenceResolution::Resolved(object_id) => Ok(object_id),
        FilesystemReferenceResolution::RequiresGitResolution {
            rejection_if_git_reports_missing,
        } => resolve_reference_through_git(
            worktree_context,
            reference,
            rejection_if_git_reports_missing,
        ),
    }
}

fn resolve_reference_through_git(
    worktree_context: &WorktreeContext,
    reference: &str,
    rejection_if_git_reports_missing: ClaimError,
) -> Result<GitObjectId, ClaimError> {
    match git::reference_lookup(worktree_context.repository_root(), reference)? {
        ReferenceLookup::Present(object_id) => Ok(object_id),
        ReferenceLookup::Missing => Err(rejection_if_git_reports_missing),
    }
}

fn read_reference_from_files(
    common_git_directory: &Path,
    reference: &str,
) -> FilesystemReferenceResolution {
    read_reference_from_files_at_depth(
        common_git_directory,
        reference,
        SymbolicReferenceDepth::ROOT,
    )
}

/// Resolve a local branch from loose or packed refs without starting Git.
pub(crate) fn reference_tip_from_files(
    common_git_directory: &Path,
    reference: &str,
) -> Result<Option<GitObjectId>, ()> {
    match read_reference_from_files(common_git_directory, reference) {
        FilesystemReferenceResolution::Resolved(commit) => Ok(Some(commit)),
        FilesystemReferenceResolution::RequiresGitResolution {
            rejection_if_git_reports_missing: ClaimError::MissingReference(_),
        } => Ok(None),
        FilesystemReferenceResolution::RequiresGitResolution { .. } => Err(()),
    }
}

fn read_reference_from_files_at_depth(
    common_git_directory: &Path,
    reference: &str,
    depth: SymbolicReferenceDepth,
) -> FilesystemReferenceResolution {
    match fs::read_to_string(common_git_directory.join(reference)) {
        Ok(value) => parse_reference_value(common_git_directory, reference, &value, depth),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            match read_packed_reference(common_git_directory, reference) {
                Ok(object_id) => FilesystemReferenceResolution::Resolved(object_id),
                Err(ClaimError::MissingReference(_)) => {
                    FilesystemReferenceResolution::RequiresGitResolution {
                        rejection_if_git_reports_missing: ClaimError::MissingReference(
                            reference.to_owned(),
                        ),
                    }
                },
                Err(error) => FilesystemReferenceResolution::RequiresGitResolution {
                    rejection_if_git_reports_missing: error,
                },
            }
        },
        Err(error) if error.kind() == ErrorKind::NotADirectory => {
            FilesystemReferenceResolution::RequiresGitResolution {
                rejection_if_git_reports_missing: ClaimError::MissingReference(
                    reference.to_owned(),
                ),
            }
        },
        Err(error) => FilesystemReferenceResolution::RequiresGitResolution {
            rejection_if_git_reports_missing: ClaimError::Io(error),
        },
    }
}

fn parse_reference_value(
    common_git_directory: &Path,
    reference: &str,
    value: &str,
    depth: SymbolicReferenceDepth,
) -> FilesystemReferenceResolution {
    let value = value.trim();
    if let Some(target) = value.strip_prefix("ref: ") {
        return match depth.descend(reference) {
            Ok(depth) => read_reference_from_files_at_depth(common_git_directory, target, depth),
            Err(error) => FilesystemReferenceResolution::RequiresGitResolution {
                rejection_if_git_reports_missing: error,
            },
        };
    }
    value.parse().map_or_else(
        |_| FilesystemReferenceResolution::RequiresGitResolution {
            rejection_if_git_reports_missing: ClaimError::InvalidStoredReference(
                reference.to_owned(),
            ),
        },
        FilesystemReferenceResolution::Resolved,
    )
}

fn read_packed_reference(
    common_git_directory: &Path,
    reference: &str,
) -> Result<GitObjectId, ClaimError> {
    let packed_references = match fs::read_to_string(common_git_directory.join("packed-refs")) {
        Ok(packed_references) => packed_references,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(ClaimError::MissingReference(reference.to_owned()));
        },
        Err(error) => return Err(ClaimError::Io(error)),
    };
    let object_id = packed_references.lines().find_map(|line| {
        let (object_id, name) = line.split_once(' ')?;
        (name == reference).then_some(object_id)
    });
    object_id.map_or_else(
        || Err(ClaimError::MissingReference(reference.to_owned())),
        |object_id| {
            object_id
                .parse()
                .map_err(|_| ClaimError::InvalidStoredReference(reference.to_owned()))
        },
    )
}
