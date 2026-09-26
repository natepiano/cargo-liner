//! Foreign committed work the acting checkout's HEAD already contains.
//!
//! A worktree stacked on another branch carries that branch's commits, so their paths are
//! already part of its history rather than work it could collide with. A foreign holder
//! therefore protects only what its head would still bring to the acting HEAD, plus its
//! uncommitted paths.

use std::cell::Cell;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

use super::lifecycle::ReservationLifecycle;
use super::merge_extent::MergeExtent;
use super::merge_extent::RetainedMergeEvidence;
use super::record::RecordedTarget;
use super::record::Reservation;
use super::retention::RetainedReservationSet;
use crate::config::BerthConfig;
use crate::config::Enrollment;
use crate::git;
use crate::ids::GitObjectId;
use crate::ids::ReservationScopePath;
use crate::ids::WorktreeId;
use crate::ledger;
use crate::ledger::FullRefName;
use crate::ledger::IntegrationTarget;
use crate::ledger::ReservationScope;
use crate::ledger::ReservationScopeSet;
use crate::ledger::ResolvedEditAuthorization;
use crate::ledger::TargetSelectionRequest;
use crate::ledger::WorktreeContext;
use crate::verb::claim;

/// What each observed foreign holder head would still bring to the acting checkout's HEAD.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActingHeadContainment {
    acting_worktree: WorktreeId,
    target:          ContainmentTarget,
    holder_heads:    Vec<HolderHeadRemainder>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ContainmentTarget {
    Known {
        acting:           IntegrationTarget,
        repository_trunk: IntegrationTarget,
    },
    Unavailable,
}

/// The committed paths one holder head adds beyond its merge base with the acting HEAD.
#[derive(Clone, Debug, Eq, PartialEq)]
struct HolderHeadRemainder {
    head:            GitObjectId,
    committed_paths: HashSet<ReservationScopePath>,
    outside_target:  CrossTargetRemainder,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CrossTargetRemainder {
    NotApplicable,
    Unavailable,
    Paths(HashSet<ReservationScopePath>),
}

impl ActingHeadContainment {
    /// Select the current checkout's recorded target, or its automatic branch target.
    pub(crate) fn observe_for_actor(
        reservations: &RetainedReservationSet,
        context: &WorktreeContext,
        authorization: ResolvedEditAuthorization,
    ) -> Self {
        let acting_worktree = authorization.worktree_id;
        let Ok(Enrollment::Enrolled(config)) = BerthConfig::read(&context.configuration_lookup())
        else {
            return Self::unavailable(acting_worktree);
        };
        let Ok(repository_trunk) = config.repository_trunk() else {
            return Self::unavailable(acting_worktree);
        };
        let acting_reservation = reservations
            .iter()
            .filter(|reservation| {
                reservation.actor().worktree == acting_worktree
                    && reservation.actor().run == authorization.coordination_run_id
                    && !matches!(
                        reservation.lifecycle(),
                        ReservationLifecycle::Released { .. }
                    )
            })
            .last();
        let target = if let Some(reservation) = acting_reservation {
            match reservations.target_of(reservation.id(), &repository_trunk) {
                Ok(target) => target,
                Err(_) => return Self::unavailable(acting_worktree),
            }
        } else {
            let Ok(head) = fs::read_to_string(context.administrative_directory().join("HEAD"))
            else {
                return Self::unavailable(acting_worktree);
            };
            match automatically_selected_target(
                context.common_git_directory(),
                &head,
                &repository_trunk,
            ) {
                ContainmentTarget::Known { acting, .. } => acting,
                ContainmentTarget::Unavailable => return Self::unavailable(acting_worktree),
            }
        };
        Self::observe(
            reservations,
            context,
            acting_worktree,
            &target,
            &repository_trunk,
        )
    }

    const fn unavailable(acting_worktree: WorktreeId) -> Self {
        Self {
            acting_worktree,
            target: ContainmentTarget::Unavailable,
            holder_heads: Vec::new(),
        }
    }
    /// Read which foreign holder work the acting HEAD already contains, before any lock.
    ///
    /// Nothing is read unless a foreign holder retains keyed committed protection. A holder
    /// measured against the acting HEAD as trunk needs no read, since its extent already answers,
    /// and any failed read leaves that holder's complete protection in force.
    pub(crate) fn observe(
        reservations: &RetainedReservationSet,
        context: &WorktreeContext,
        acting_worktree: WorktreeId,
        acting_target: &IntegrationTarget,
        repository_trunk: &IntegrationTarget,
    ) -> Self {
        let mut keys = reservations
            .iter()
            .filter(|holder| holder.actor().worktree != acting_worktree)
            .filter_map(|holder| match holder.merge_extent() {
                MergeExtent::Protected { key, .. }
                | MergeExtent::Unavailable {
                    retained_evidence: RetainedMergeEvidence::Protected { key, .. },
                    ..
                } => Some(key),
                MergeExtent::NotDerived { .. }
                | MergeExtent::Empty { .. }
                | MergeExtent::Unavailable { .. } => None,
            })
            .peekable();
        let holder_heads = Vec::new();
        if keys.peek().is_none() {
            return Self {
                acting_worktree,
                target: ContainmentTarget::Known {
                    acting:           acting_target.clone(),
                    repository_trunk: repository_trunk.clone(),
                },
                holder_heads,
            };
        }
        let Ok(acting_head) = git::head_object_id(context.repository_root()) else {
            return Self {
                acting_worktree,
                target: ContainmentTarget::Known {
                    acting:           acting_target.clone(),
                    repository_trunk: repository_trunk.clone(),
                },
                holder_heads,
            };
        };
        Self::observe_at_head(
            reservations,
            context,
            acting_worktree,
            acting_target,
            repository_trunk,
            &acting_head,
        )
    }

    /// Reuse a caller's resolved HEAD when observing foreign committed protection.
    pub(crate) fn observe_at_head(
        reservations: &RetainedReservationSet,
        context: &WorktreeContext,
        acting_worktree: WorktreeId,
        acting_target: &IntegrationTarget,
        repository_trunk: &IntegrationTarget,
        acting_head: &GitObjectId,
    ) -> Self {
        let keys: Vec<_> = reservations
            .iter()
            .filter(|holder| holder.actor().worktree != acting_worktree)
            .filter_map(|holder| {
                let holder_target = holder_target(holder.target(), repository_trunk);
                match holder.merge_extent() {
                    MergeExtent::Protected { key, .. }
                    | MergeExtent::Unavailable {
                        retained_evidence: RetainedMergeEvidence::Protected { key, .. },
                        ..
                    } => Some((&key.head, holder_target)),
                    MergeExtent::NotDerived { .. }
                    | MergeExtent::Empty { .. }
                    | MergeExtent::Unavailable { .. } => None,
                }
            })
            .collect();
        let mut holder_heads: Vec<HolderHeadRemainder> = Vec::new();
        let effective_target = match claim::reference_tip_from_files(
            context.common_git_directory(),
            acting_target.reference().as_str(),
        ) {
            Ok(Some(tip)) => Some((acting_target.clone(), tip)),
            Ok(None) if acting_target != repository_trunk => {
                match claim::reference_tip_from_files(
                    context.common_git_directory(),
                    repository_trunk.reference().as_str(),
                ) {
                    Ok(tip) => tip.map(|tip| (repository_trunk.clone(), tip)),
                    Err(()) => return Self::unavailable(acting_worktree),
                }
            },
            Ok(None) => None,
            Err(()) => return Self::unavailable(acting_worktree),
        };
        for &(head, _) in &keys {
            if holder_heads.iter().any(|observed| observed.head == *head) {
                continue;
            }
            let committed_paths = if head == acting_head {
                HashSet::new()
            } else {
                match git::unmerged_branch_paths(context.repository_root(), acting_head, head) {
                    Ok(paths) => paths.into_iter().collect(),
                    Err(_) => continue,
                }
            };
            let outside_target = match effective_target.as_ref() {
                Some((target, tip))
                    if keys.iter().any(|(candidate_head, holder_target)| {
                        *candidate_head == head && *holder_target != target
                    }) =>
                {
                    if tip == head {
                        CrossTargetRemainder::Paths(HashSet::new())
                    } else {
                        git::unmerged_branch_paths(context.repository_root(), tip, head)
                            .map_or_else(
                                |_| CrossTargetRemainder::Unavailable,
                                |paths| CrossTargetRemainder::Paths(paths.into_iter().collect()),
                            )
                    }
                },
                Some(_) | None => CrossTargetRemainder::NotApplicable,
            };
            holder_heads.push(HolderHeadRemainder {
                head: head.clone(),
                committed_paths,
                outside_target,
            });
        }
        Self {
            acting_worktree,
            target: ContainmentTarget::Known {
                acting:           effective_target
                    .map_or_else(|| acting_target.clone(), |(target, _)| target),
                repository_trunk: repository_trunk.clone(),
            },
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
        if acting_worktree != self.acting_worktree {
            return scopes.as_slice().to_vec();
        }
        let key = match holder.merge_extent() {
            MergeExtent::Protected { key, .. }
            | MergeExtent::Unavailable {
                retained_evidence: RetainedMergeEvidence::Protected { key, .. },
                ..
            } => key,
            MergeExtent::NotDerived { .. }
            | MergeExtent::Empty { .. }
            | MergeExtent::Unavailable { .. } => return scopes.as_slice().to_vec(),
        };
        let Some(observed) = self
            .holder_heads
            .iter()
            .find(|observed| observed.head == key.head)
        else {
            return scopes.as_slice().to_vec();
        };
        let ContainmentTarget::Known {
            acting,
            repository_trunk,
        } = &self.target
        else {
            return scopes.as_slice().to_vec();
        };
        let holder_target = holder_target(holder.target(), repository_trunk);
        scopes
            .as_slice()
            .iter()
            .filter(|scope| {
                committed_path_remains(
                    observed,
                    &scope.path,
                    if holder_target == acting {
                        HolderTargetRelation::SameTarget
                    } else {
                        HolderTargetRelation::OtherTarget
                    },
                ) || key.working_tree.tracked_paths.contains(&scope.path)
                    || key.working_tree.untracked_paths.contains(&scope.path)
            })
            .cloned()
            .collect()
    }
}

fn automatically_selected_target(
    common_git_directory: &Path,
    head: &str,
    repository_trunk: &IntegrationTarget,
) -> ContainmentTarget {
    let attachment = head
        .trim()
        .strip_prefix("ref: ")
        .and_then(|branch| branch.parse::<FullRefName>().ok());
    let read_failed = Cell::new(false);
    let selection = ledger::resolve_claim_target(
        common_git_directory,
        attachment.as_ref(),
        TargetSelectionRequest::AutomaticAcquisition,
        repository_trunk,
        |candidate| match claim::reference_tip_from_files(
            common_git_directory,
            candidate.reference().as_str(),
        ) {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(()) => {
                read_failed.set(true);
                false
            },
        },
    );
    if read_failed.get() {
        return ContainmentTarget::Unavailable;
    }
    match selection {
        Ok(selection) => ContainmentTarget::Known {
            acting:           selection.target,
            repository_trunk: repository_trunk.clone(),
        },
        Err(_) => ContainmentTarget::Unavailable,
    }
}

const fn holder_target<'target>(
    recorded_target: &'target RecordedTarget,
    repository_trunk: &'target IntegrationTarget,
) -> &'target IntegrationTarget {
    match recorded_target {
        RecordedTarget::Recorded { target, .. } => target,
        RecordedTarget::Unrecorded => repository_trunk,
    }
}

/// Whether a holder's recorded target is the acting reservation's target.
#[derive(Clone, Copy)]
enum HolderTargetRelation {
    SameTarget,
    OtherTarget,
}

fn committed_path_remains(
    observed: &HolderHeadRemainder,
    path: &ReservationScopePath,
    holder_target_relation: HolderTargetRelation,
) -> bool {
    observed.committed_paths.contains(path)
        && (matches!(holder_target_relation, HolderTargetRelation::SameTarget)
            || match &observed.outside_target {
                CrossTargetRemainder::NotApplicable | CrossTargetRemainder::Unavailable => true,
                CrossTargetRemainder::Paths(paths) => paths.contains(path),
            })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;

    use tempfile::tempdir;

    use super::ContainmentTarget;
    use super::CrossTargetRemainder;
    use super::HolderHeadRemainder;
    use super::HolderTargetRelation;
    use super::automatically_selected_target;
    use super::committed_path_remains;
    use super::holder_target;
    use crate::ids::GitObjectId;
    use crate::ids::ReservationScopePath;
    use crate::ledger::IntegrationTarget;
    use crate::reservation::record::RecordedTarget;

    fn paths(values: &[&str]) -> Result<HashSet<ReservationScopePath>, Box<dyn std::error::Error>> {
        values
            .iter()
            .map(|path| path.parse().map_err(Into::into))
            .collect()
    }

    #[test]
    fn cross_target_committed_work_requires_a_remainder_from_the_acting_target()
    -> Result<(), Box<dyn std::error::Error>> {
        let path: ReservationScopePath = "Cargo.toml".parse()?;
        let cover = HolderHeadRemainder {
            head:            "1111111111111111111111111111111111111111".parse::<GitObjectId>()?,
            committed_paths: paths(&["Cargo.toml"])?,
            outside_target:  CrossTargetRemainder::Paths(HashSet::new()),
        };
        assert!(!committed_path_remains(
            &cover,
            &path,
            HolderTargetRelation::OtherTarget
        ));
        assert!(committed_path_remains(
            &cover,
            &path,
            HolderTargetRelation::SameTarget
        ));
        let sibling = HolderHeadRemainder {
            outside_target: CrossTargetRemainder::Paths(paths(&["Cargo.toml"])?),
            ..cover
        };
        assert!(committed_path_remains(
            &sibling,
            &path,
            HolderTargetRelation::OtherTarget
        ));
        for remainder in [
            CrossTargetRemainder::NotApplicable,
            CrossTargetRemainder::Unavailable,
        ] {
            let unfiltered = HolderHeadRemainder {
                outside_target: remainder,
                ..sibling.clone()
            };
            assert!(committed_path_remains(
                &unfiltered,
                &path,
                HolderTargetRelation::OtherTarget
            ));
        }
        Ok(())
    }

    #[test]
    fn legacy_holder_uses_repository_trunk_for_cross_target_containment()
    -> Result<(), Box<dyn std::error::Error>> {
        let repository_trunk = IntegrationTarget::from_branch_argument("main")?;
        let acting_target = IntegrationTarget::from_branch_argument("integration")?;
        let path: ReservationScopePath = "integration.txt".parse()?;
        let holder = HolderHeadRemainder {
            head:            "1111111111111111111111111111111111111111".parse()?,
            committed_paths: paths(&["integration.txt"])?,
            outside_target:  CrossTargetRemainder::Paths(HashSet::new()),
        };
        let holder_target = holder_target(&RecordedTarget::Unrecorded, &repository_trunk);
        assert_ne!(holder_target, &acting_target);
        assert!(!committed_path_remains(
            &holder,
            &path,
            if holder_target == &acting_target {
                HolderTargetRelation::SameTarget
            } else {
                HolderTargetRelation::OtherTarget
            }
        ));
        assert!(committed_path_remains(
            &holder,
            &path,
            if holder_target == &repository_trunk {
                HolderTargetRelation::SameTarget
            } else {
                HolderTargetRelation::OtherTarget
            }
        ));
        Ok(())
    }

    #[test]
    fn unreadable_target_ref_is_unavailable_but_absent_ref_uses_trunk()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        fs::write(
            directory.path().join("config"),
            "[branch \"acting\"]\n cargoBerthTarget = integration\n",
        )?;
        let repository_trunk = IntegrationTarget::from_branch_argument("main")?;
        let head = "ref: refs/heads/acting\n";
        assert_eq!(
            automatically_selected_target(directory.path(), head, &repository_trunk),
            ContainmentTarget::Known {
                acting:           repository_trunk.clone(),
                repository_trunk: repository_trunk.clone(),
            }
        );
        fs::create_dir_all(directory.path().join("refs/heads/integration"))?;
        assert_eq!(
            automatically_selected_target(directory.path(), head, &repository_trunk),
            ContainmentTarget::Unavailable
        );
        Ok(())
    }
}
