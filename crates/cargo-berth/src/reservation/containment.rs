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

use super::lifecycle::EditBlockingStatus;
use super::lifecycle::ReservationLifecycle;
use super::merge_extent::MergeExtent;
use super::merge_extent::ReservationProtection;
use super::merge_extent::RetainedMergeEvidence;
use super::record::RecordedTarget;
use super::record::Reservation;
use super::retention::RetainedReservationSet;
use crate::config::BerthConfig;
use crate::config::Enrollment;
use crate::git::RepositorySnapshot;
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
use crate::scope::PathCase;
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
        repository: &RepositorySnapshot,
        authorization: ResolvedEditAuthorization,
    ) -> Self {
        Self::observe_for_actor_holders(
            reservations,
            repository,
            authorization,
            &reservations.iter().collect::<Vec<_>>(),
        )
    }

    /// Observe only holders whose complete protection touches this edit. Containment can
    /// remove protection, never add it, so disjoint holders cannot affect the answer.
    pub(crate) fn observe_for_edit(
        reservations: &RetainedReservationSet,
        repository: &RepositorySnapshot,
        authorization: ResolvedEditAuthorization,
        requested: &ReservationScopeSet,
        path_case: PathCase,
    ) -> Self {
        let holders = requested_holders(
            reservations,
            authorization.worktree_id,
            requested,
            path_case,
        );
        Self::observe_for_actor_holders(reservations, repository, authorization, &holders)
    }

    fn observe_for_actor_holders(
        reservations: &RetainedReservationSet,
        repository: &RepositorySnapshot,
        authorization: ResolvedEditAuthorization,
        holders: &[&Reservation],
    ) -> Self {
        let context = repository.context();
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
        Self::observe_holders(
            holders,
            repository,
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
    /// Apply the same candidate-first rule during locked acquisition validation.
    pub(crate) fn observe_for_claim(
        reservations: &RetainedReservationSet,
        repository: &RepositorySnapshot,
        acting_worktree: WorktreeId,
        acting_target: &IntegrationTarget,
        repository_trunk: &IntegrationTarget,
        requested: &ReservationScopeSet,
        path_case: PathCase,
    ) -> Self {
        let holders = requested_holders(reservations, acting_worktree, requested, path_case);
        Self::observe_holders(
            &holders,
            repository,
            acting_worktree,
            acting_target,
            repository_trunk,
        )
    }

    fn observe_holders(
        holders: &[&Reservation],
        repository: &RepositorySnapshot,
        acting_worktree: WorktreeId,
        acting_target: &IntegrationTarget,
        repository_trunk: &IntegrationTarget,
    ) -> Self {
        let mut keys = holders
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
        let Ok(acting_head) = repository.head_object_id() else {
            return Self {
                acting_worktree,
                target: ContainmentTarget::Known {
                    acting:           acting_target.clone(),
                    repository_trunk: repository_trunk.clone(),
                },
                holder_heads,
            };
        };
        Self::observe_holders_at_head(
            holders,
            repository,
            repository.context(),
            acting_worktree,
            acting_target,
            repository_trunk,
            acting_head,
        )
    }

    /// Reuse a caller's resolved HEAD when observing foreign committed protection.
    pub(crate) fn observe_at_head(
        reservations: &RetainedReservationSet,
        repository: &RepositorySnapshot,
        context: &WorktreeContext,
        acting_worktree: WorktreeId,
        acting_target: &IntegrationTarget,
        repository_trunk: &IntegrationTarget,
        acting_head: &GitObjectId,
    ) -> Self {
        Self::observe_holders_at_head(
            &reservations.iter().collect::<Vec<_>>(),
            repository,
            context,
            acting_worktree,
            acting_target,
            repository_trunk,
            acting_head,
        )
    }

    fn observe_holders_at_head(
        holders: &[&Reservation],
        repository: &RepositorySnapshot,
        context: &WorktreeContext,
        acting_worktree: WorktreeId,
        acting_target: &IntegrationTarget,
        repository_trunk: &IntegrationTarget,
        acting_head: &GitObjectId,
    ) -> Self {
        let keys: Vec<_> = holders
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
            let committed_paths = match repository.unmerged_branch_paths(
                context.repository_root(),
                acting_head,
                head,
            ) {
                Ok((paths, _)) => paths.into_iter().collect(),
                Err(_) => continue,
            };
            let outside_target = match effective_target.as_ref() {
                Some((target, tip))
                    if keys.iter().any(|(candidate_head, holder_target)| {
                        *candidate_head == head && *holder_target != target
                    }) =>
                {
                    repository
                        .unmerged_branch_paths(context.repository_root(), tip, head)
                        .map_or_else(
                            |_| CrossTargetRemainder::Unavailable,
                            |(paths, _)| CrossTargetRemainder::Paths(paths.into_iter().collect()),
                        )
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

/// Query the complete unnarrowed protection of each foreign checkout. Once a checkout
/// matches, retain all its contributors: containment can change which sibling represents
/// the union in an overlap answer, even when that sibling's own paths are disjoint.
fn requested_holders<'a>(
    reservations: &'a RetainedReservationSet,
    acting_worktree: WorktreeId,
    requested: &ReservationScopeSet,
    path_case: PathCase,
) -> Vec<&'a Reservation> {
    let candidates: HashSet<_> = reservations
        .iter()
        .filter(|holder| {
            holder.actor().worktree != acting_worktree
                && holder.edit_blocking_status() == EditBlockingStatus::Blocking
                && match holder.protection_in_worktree(acting_worktree) {
                    ReservationProtection::Clear => false,
                    ReservationProtection::Protected(scopes) => {
                        scopes.as_slice().iter().any(|held| {
                            requested
                                .as_slice()
                                .iter()
                                .any(|scope| held.overlaps(scope, path_case))
                        })
                    },
                }
        })
        .map(|holder| holder.actor().worktree)
        .collect();
    reservations
        .iter()
        .filter(|holder| {
            candidates.contains(&holder.actor().worktree)
                && holder.edit_blocking_status() == EditBlockingStatus::Blocking
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;

    use cargo_berth_test_support::CommandDeadlineExt as _;
    use serde_json::Error;
    use serde_json::Value;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::ActingHeadContainment;
    use super::ContainmentTarget;
    use super::CrossTargetRemainder;
    use super::HolderHeadRemainder;
    use super::HolderTargetRelation;
    use super::automatically_selected_target;
    use super::committed_path_remains;
    use super::holder_target;
    use crate::git::RepositorySnapshot;
    use crate::ids::CoordinationRunId;
    use crate::ids::EventId;
    use crate::ids::GitObjectId;
    use crate::ids::ReservationId;
    use crate::ids::ReservationScopePath;
    use crate::ids::WorktreeId;
    use crate::ledger::EditAuthorization;
    use crate::ledger::IntegrationTarget;
    use crate::ledger::JournalEvent;
    use crate::ledger::ReservationScope;
    use crate::ledger::ReservationScopeSet;
    use crate::ledger::ResolvedEditAuthorization;
    use crate::ledger::ScopeKind;
    use crate::reservation::MergeExtent;
    use crate::reservation::RetainedReservationSet;
    use crate::reservation::record::RecordedTarget;
    use crate::scope::PathCase;

    fn paths(values: &[&str]) -> Result<HashSet<ReservationScopePath>, Box<dyn std::error::Error>> {
        values
            .iter()
            .map(|path| path.parse().map_err(Into::into))
            .collect()
    }

    #[derive(Clone, Copy)]
    enum HolderLayout {
        Independent,
        Siblings,
    }

    fn containment_fixture(
        layout: HolderLayout,
    ) -> Result<
        (TempDir, RetainedReservationSet, ResolvedEditAuthorization),
        Box<dyn std::error::Error>,
    > {
        let (seed, acting_base) = match layout {
            HolderLayout::Independent => ("requested-containment", "main"),
            HolderLayout::Siblings => ("requested-sibling-containment", "first"),
        };
        let root = cargo_berth_test_support::copy_git_repository_seed(seed, |root| {
            let git = |args: &[&str]| -> Result<(), Box<dyn std::error::Error>> {
                let output = cargo_berth_test_support::fixture_git_command()
                    .arg("-C")
                    .arg(root)
                    .args(args)
                    .bounded_output()?;
                if !output.status.success() {
                    return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
                }
                Ok(())
            };
            git(&["init", "--quiet", "--initial-branch", "main"])?;
            git(&["config", "user.name", "Test"])?;
            git(&["config", "user.email", "test@example.com"])?;
            fs::create_dir_all(root.join("src"))?;
            fs::write(root.join("src/base.rs"), "base")?;
            git(&["add", "."])?;
            git(&["commit", "--quiet", "-m", "base"])?;
            for (branch, path) in [
                ("first", "src/first.rs"),
                ("second", "src/second.rs"),
                ("acting", "src/acting.rs"),
            ] {
                let base = if branch == "acting" {
                    acting_base
                } else {
                    "main"
                };
                git(&["checkout", "--quiet", "-b", branch, base])?;
                fs::write(root.join(path), branch)?;
                git(&["add", "."])?;
                git(&["commit", "--quiet", "-m", branch])?;
            }
            fs::create_dir_all(root.join(".claude/config"))?;
            fs::write(root.join(".claude/config/berth.toml"), "trunk = \"main\"\n")?;
            Ok::<(), Box<dyn std::error::Error>>(())
        });
        let repository = RepositorySnapshot::discover(root.path())?;
        let trunk = repository.object_reader().commit("main")?;
        let mut events = Vec::new();
        let sibling_worktree = WorktreeId::new();
        for (branch, path) in [("first", "src/first.rs"), ("second", "src/second.rs")] {
            let head = repository.object_reader().commit(branch)?;
            let id = ReservationId::new();
            let actor = serde_json::json!({
                "repository": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c",
                "worktree": match layout {
                    HolderLayout::Independent => WorktreeId::new(),
                    HolderLayout::Siblings => sibling_worktree,
                },
                "run": CoordinationRunId::new(),
            });
            let event = |operation: Value| -> Result<JournalEvent, Error> {
                let mut value = serde_json::json!({
                    "schema_version": 2,
                    "event_id": EventId::new(),
                    "actor": actor,
                    "at": "2026-10-28T17:34:54.123Z",
                    "projection_generation": events.len() + 1,
                });
                if let (Some(value), Some(operation)) =
                    (value.as_object_mut(), operation.as_object())
                {
                    value.extend(operation.clone());
                }
                serde_json::from_value(value)
            };
            let claim = event(serde_json::json!({
                "op": "claim", "reservation_id": id,
                "scopes": [{"path": path, "kind": "file"}],
                "source": {"kind": "explicit"}, "purpose": {"kind": "not_provided_by_caller"},
                "trunk_at_claim": trunk,
                "head_snapshot": {"kind": "branch", "full_ref": format!("refs/heads/{branch}"), "head": head},
                "phase_start_head": trunk,
                "worktree_root": root.path(), "worktree_administrative_locator": ".",
                "authorization": {"kind": "no_conflict"}, "coordination_identity_provenance": "presented",
            }))?;
            let observed = event(serde_json::json!({
                "op": "merge_extent_observed", "reservation_id": id,
                "extent": {"status": "protected", "scopes": [{"path": path, "kind": "file"}],
                    "key": {"head": head, "trunk": trunk, "working_tree": {"tracked_paths": [], "untracked_paths": []}}},
            }))?;
            events.extend([claim, observed]);
        }
        let reservations = RetainedReservationSet::replay(&events)?;
        let authorization = ResolvedEditAuthorization::for_edit_authorization(
            WorktreeId::new(),
            EditAuthorization::Unidentified,
        );
        Ok((root, reservations, authorization))
    }

    fn requested_scopes(path: &str) -> Result<ReservationScopeSet, Box<dyn std::error::Error>> {
        Ok(ReservationScopeSet::try_from(vec![ReservationScope {
            path: path.parse()?,
            kind: ScopeKind::File,
        }])?)
    }

    #[test]
    fn disjoint_check_does_not_compute_foreign_head_containment()
    -> Result<(), Box<dyn std::error::Error>> {
        let (root, reservations, authorization) = containment_fixture(HolderLayout::Independent)?;
        let repository = RepositorySnapshot::discover(root.path())?;
        let requested = requested_scopes("src/disjoint.rs")?;
        let containment = ActingHeadContainment::observe_for_edit(
            &reservations,
            &repository,
            authorization,
            &requested,
            PathCase::Sensitive,
        );
        assert_eq!(containment.holder_heads, Vec::new());
        let narrowed = reservations
            .clone()
            .with_acting_head_containment(containment);
        assert_eq!(
            narrowed
                .conflicts_for_edit(&requested, authorization, PathCase::Sensitive)
                .len(),
            0
        );
        for holder in reservations.iter() {
            let MergeExtent::Protected { key, .. } = holder.merge_extent() else {
                return Err("protected fixture".into());
            };
            let (_, queries) = repository.unmerged_branch_paths(
                root.path(),
                repository.head_object_id()?,
                &key.head,
            )?;
            assert_eq!(
                queries, 1,
                "disjoint check must leave each merge pair unqueried"
            );
        }
        Ok(())
    }

    #[test]
    fn overlapping_check_observes_only_the_protecting_holder_and_preserves_refusal()
    -> Result<(), Box<dyn std::error::Error>> {
        let (root, reservations, authorization) = containment_fixture(HolderLayout::Independent)?;
        let repository = RepositorySnapshot::discover(root.path())?;
        let requested = requested_scopes("src/first.rs")?;
        let containment = ActingHeadContainment::observe_for_edit(
            &reservations,
            &repository,
            authorization,
            &requested,
            PathCase::Sensitive,
        );
        assert_eq!(containment.holder_heads.len(), 1);
        let narrowed = reservations
            .clone()
            .with_acting_head_containment(containment);
        let conflicts = narrowed.conflicts_for_edit(&requested, authorization, PathCase::Sensitive);
        assert_eq!(conflicts.len(), 1);
        let target = crate::ledger::IntegrationTarget::from_branch_argument("main")?;
        let full = ActingHeadContainment::observe_at_head(
            &reservations,
            &RepositorySnapshot::discover(root.path())?,
            repository.context(),
            authorization.worktree_id,
            &target,
            &target,
            repository.head_object_id()?,
        );
        let complete = reservations
            .clone()
            .with_acting_head_containment(full)
            .conflicts_for_edit(&requested, authorization, PathCase::Sensitive);
        assert_eq!(conflicts[0].reservation_id, complete[0].reservation_id);
        assert_eq!(
            conflicts[0].overlapping_scopes,
            complete[0].overlapping_scopes
        );
        for (holder, expected_queries) in reservations.iter().zip(0_u64..) {
            let MergeExtent::Protected { key, .. } = holder.merge_extent() else {
                return Err("protected fixture".into());
            };
            let (_, queries) = repository.unmerged_branch_paths(
                root.path(),
                repository.head_object_id()?,
                &key.head,
            )?;
            assert_eq!(
                queries, expected_queries,
                "only the overlapping holder's merge pair was observed"
            );
        }
        Ok(())
    }

    #[test]
    fn candidate_checkout_keeps_siblings_needed_to_select_the_overlap_representative()
    -> Result<(), Box<dyn std::error::Error>> {
        let (root, reservations, authorization) = containment_fixture(HolderLayout::Siblings)?;
        let repository = RepositorySnapshot::discover(root.path())?;
        let requested = requested_scopes("src/second.rs")?;
        let containment = ActingHeadContainment::observe_for_edit(
            &reservations,
            &repository,
            authorization,
            &requested,
            PathCase::Sensitive,
        );
        assert_eq!(containment.holder_heads.len(), 2);
        let expected = reservations.iter().last().ok_or("second sibling")?.id();
        let narrowed = reservations.with_acting_head_containment(containment);
        let conflicts = narrowed.conflicts_for_edit(&requested, authorization, PathCase::Sensitive);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(
            conflicts[0].reservation_id, expected,
            "the already-contained sibling cannot represent this overlap"
        );
        Ok(())
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
