//! Foreign committed work the acting checkout's HEAD already contains.
//!
//! A worktree stacked on another branch carries that branch's commits, so their paths are
//! already part of its history rather than work it could collide with. A foreign holder
//! therefore protects only what its head would still bring to the acting HEAD, plus its
//! uncommitted paths.

use std::collections::HashSet;
use std::path::Path;

use super::merge_extent::MergeExtentKey;
use super::record::Reservation;
use super::retention::RetainedReservationSet;
use crate::git;
use crate::ids::GitObjectId;
use crate::ids::ReservationScopePath;
use crate::ids::WorktreeId;
use crate::scope::ReservationScope;
use crate::scope::ReservationScopeSet;

/// What each observed foreign holder head would still bring to the acting checkout's HEAD.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActingHeadContainment {
    acting_worktree: WorktreeId,
    holder_heads:    Vec<HolderHeadRemainder>,
}

/// The committed paths one holder head adds beyond its merge base with the acting HEAD.
#[derive(Clone, Debug, Eq, PartialEq)]
struct HolderHeadRemainder {
    head:            GitObjectId,
    committed_paths: HashSet<ReservationScopePath>,
}

impl ActingHeadContainment {
    /// Read which foreign holder work the acting HEAD already contains, before any lock.
    ///
    /// Nothing is read unless a foreign holder retains keyed committed protection. A holder
    /// measured against the acting HEAD as trunk needs no read, since its extent already answers,
    /// and any failed read leaves that holder's complete protection in force.
    pub(crate) fn observe(
        reservations: &RetainedReservationSet,
        repository_root: &Path,
        acting_worktree: WorktreeId,
    ) -> Self {
        let mut keys = reservations
            .iter()
            .filter(|holder| holder.actor().worktree != acting_worktree)
            .filter_map(|holder| holder.merge_extent().protected_key())
            .peekable();
        let mut holder_heads: Vec<HolderHeadRemainder> = Vec::new();
        if keys.peek().is_none() {
            return Self {
                acting_worktree,
                holder_heads,
            };
        }
        let Ok(acting_head) = git::head_object_id(repository_root) else {
            return Self {
                acting_worktree,
                holder_heads,
            };
        };
        for MergeExtentKey { trunk, head, .. } in keys {
            if *trunk == acting_head || holder_heads.iter().any(|observed| observed.head == *head) {
                continue;
            }
            let committed_paths = if *head == acting_head {
                HashSet::new()
            } else {
                match git::unmerged_branch_paths(repository_root, &acting_head, head) {
                    Ok(paths) => paths.into_iter().collect(),
                    Err(_) => continue,
                }
            };
            holder_heads.push(HolderHeadRemainder {
                head: head.clone(),
                committed_paths,
            });
        }
        Self {
            acting_worktree,
            holder_heads,
        }
    }

    /// Narrow a foreign holder's extent to paths the acting HEAD does not already contain.
    ///
    /// A holder whose head was not observed, or whose extent has no key, keeps every scope.
    pub(super) fn remaining_protection(
        &self,
        holder: &Reservation,
        acting_worktree: WorktreeId,
        scopes: &ReservationScopeSet,
    ) -> Vec<ReservationScope> {
        let remainder = (acting_worktree == self.acting_worktree)
            .then(|| holder.merge_extent().protected_key())
            .flatten()
            .and_then(|key| {
                self.holder_heads
                    .iter()
                    .find(|observed| observed.head == key.head)
                    .map(|observed| (key, observed))
            });
        let Some((key, observed)) = remainder else {
            return scopes.as_slice().to_vec();
        };
        scopes
            .as_slice()
            .iter()
            .filter(|scope| {
                observed.committed_paths.contains(&scope.path)
                    || key.working_tree.tracked_paths.contains(&scope.path)
                    || key.working_tree.untracked_paths.contains(&scope.path)
            })
            .cloned()
            .collect()
    }
}
