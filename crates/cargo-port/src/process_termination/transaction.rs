//! Descendant observation for bounded external termination transactions.
//!
//! This module consumes only immutable [`ProcessObservationSnapshot`] evidence.
//! It identifies frozen roots that are still live, retains every previously
//! admitted descendant independently of root lifetime, and proposes new
//! descendants only when validated parent edges reach a live frozen root.
//! Build-monitor policy applies scope and executable exclusions before asking
//! the same `ProcessObserver` that made the snapshot to mint a capability.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use super::TerminationTargetId;
use crate::build_monitor::MAX_DESCENDANT_WALK_DEPTH;
use crate::process_observation::identity::ProcessIdentity;
use crate::process_observation::snapshot::AncestryLookup;
use crate::process_observation::snapshot::ParentWalkDepth;
use crate::process_observation::snapshot::ProcessObservationSnapshot;

/// One frozen external root that a transaction pass must re-observe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FrozenTerminationRootObservation {
    semantic_target_id: TerminationTargetId,
    root_identity:      ProcessIdentity,
}

impl FrozenTerminationRootObservation {
    pub(crate) const fn new(
        semantic_target_id: TerminationTargetId,
        root_identity: ProcessIdentity,
    ) -> Self {
        Self {
            semantic_target_id,
            root_identity,
        }
    }

    pub(crate) const fn semantic_target_id(&self) -> TerminationTargetId { self.semantic_target_id }

    pub(crate) const fn root_identity(&self) -> &ProcessIdentity { &self.root_identity }
}

/// One descendant admitted by an earlier pass and retained after root exit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AdmittedTerminationDescendantObservation {
    semantic_target_id: TerminationTargetId,
    process_identity:   ProcessIdentity,
}

impl AdmittedTerminationDescendantObservation {
    pub(crate) const fn new(
        semantic_target_id: TerminationTargetId,
        process_identity: ProcessIdentity,
    ) -> Self {
        Self {
            semantic_target_id,
            process_identity,
        }
    }

    pub(crate) const fn semantic_target_id(&self) -> TerminationTargetId { self.semantic_target_id }

    pub(crate) const fn process_identity(&self) -> &ProcessIdentity { &self.process_identity }
}

/// Whether a frozen root's exact process lifetime remains in this pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FrozenTerminationRootPresence {
    Live,
    Gone,
}

/// Whether an already-admitted descendant remains in this pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdmittedTerminationDescendantPresence {
    Live,
    Gone,
}

/// A process whose validated parent chain first reached a live frozen root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NewTerminationDescendantCandidate {
    root_target_id:   TerminationTargetId,
    process_identity: ProcessIdentity,
    parent_identity:  ProcessIdentity,
    depth_from_root:  usize,
}

impl NewTerminationDescendantCandidate {
    pub(crate) const fn root_target_id(&self) -> TerminationTargetId { self.root_target_id }

    pub(crate) const fn process_identity(&self) -> &ProcessIdentity { &self.process_identity }

    pub(crate) const fn parent_identity(&self) -> &ProcessIdentity { &self.parent_identity }

    pub(crate) const fn depth_from_root(&self) -> usize { self.depth_from_root }
}

/// Immutable process evidence for one transaction observation pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TerminationDescendantObservationPass {
    root_presence:       BTreeMap<TerminationTargetId, FrozenTerminationRootPresence>,
    descendant_presence: BTreeMap<TerminationTargetId, AdmittedTerminationDescendantPresence>,
    new_candidates:      Vec<NewTerminationDescendantCandidate>,
}

impl TerminationDescendantObservationPass {
    pub(crate) const fn root_presence(
        &self,
    ) -> &BTreeMap<TerminationTargetId, FrozenTerminationRootPresence> {
        &self.root_presence
    }

    pub(crate) const fn descendant_presence(
        &self,
    ) -> &BTreeMap<TerminationTargetId, AdmittedTerminationDescendantPresence> {
        &self.descendant_presence
    }

    pub(crate) fn new_candidates(&self) -> &[NewTerminationDescendantCandidate] {
        &self.new_candidates
    }
}

/// Observe exact frozen identities, retain admitted descendants, and propose
/// only processes whose validated chain reaches a root live in this snapshot.
pub(crate) fn observe_termination_descendants(
    process_observation_snapshot: &ProcessObservationSnapshot,
    frozen_roots: &[FrozenTerminationRootObservation],
    admitted_descendants: &[AdmittedTerminationDescendantObservation],
) -> TerminationDescendantObservationPass {
    let strongly_identified_processes =
        process_observation_snapshot.strongly_identified_processes();
    let root_presence: BTreeMap<_, _> = frozen_roots
        .iter()
        .map(|frozen_root| {
            let presence =
                if strongly_identified_processes.contains_key(frozen_root.root_identity()) {
                    FrozenTerminationRootPresence::Live
                } else {
                    FrozenTerminationRootPresence::Gone
                };
            (frozen_root.semantic_target_id(), presence)
        })
        .collect();
    let descendant_presence = admitted_descendants
        .iter()
        .map(|admitted_descendant| {
            let presence = if strongly_identified_processes
                .contains_key(admitted_descendant.process_identity())
            {
                AdmittedTerminationDescendantPresence::Live
            } else {
                AdmittedTerminationDescendantPresence::Gone
            };
            (admitted_descendant.semantic_target_id(), presence)
        })
        .collect();

    let frozen_root_identities: BTreeSet<&ProcessIdentity> = frozen_roots
        .iter()
        .map(FrozenTerminationRootObservation::root_identity)
        .collect();
    let admitted_identities: BTreeSet<&ProcessIdentity> = admitted_descendants
        .iter()
        .map(AdmittedTerminationDescendantObservation::process_identity)
        .collect();
    let mut new_candidates = Vec::new();

    for process_identity in strongly_identified_processes.keys() {
        if frozen_root_identities.contains(process_identity)
            || admitted_identities.contains(process_identity)
        {
            continue;
        }
        let AncestryLookup::Observed(validated_ancestry) = process_observation_snapshot
            .validated_ancestry(
                process_identity,
                ParentWalkDepth::new(MAX_DESCENDANT_WALK_DEPTH),
            )
        else {
            continue;
        };
        let Some(parent_identity) = validated_ancestry
            .edges()
            .first()
            .map(|edge| edge.parent().clone())
        else {
            continue;
        };
        let Some((root_target_id, depth_from_root)) = validated_ancestry
            .edges()
            .iter()
            .enumerate()
            .find_map(|(edge_index, edge)| {
                frozen_roots.iter().find_map(|frozen_root| {
                    (edge.parent() == frozen_root.root_identity()
                        && root_presence.get(&frozen_root.semantic_target_id())
                            == Some(&FrozenTerminationRootPresence::Live))
                    .then_some((frozen_root.semantic_target_id(), edge_index + 1))
                })
            })
        else {
            continue;
        };
        new_candidates.push(NewTerminationDescendantCandidate {
            root_target_id,
            process_identity: process_identity.clone(),
            parent_identity,
            depth_from_root,
        });
    }
    new_candidates.sort_by(|left, right| {
        left.root_target_id
            .cmp(&right.root_target_id)
            .then_with(|| right.depth_from_root.cmp(&left.depth_from_root))
            .then_with(|| left.process_identity.cmp(&right.process_identity))
    });

    TerminationDescendantObservationPass {
        root_presence,
        descendant_presence,
        new_candidates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_monitor::MAX_DESCENDANT_WALK_DEPTH;
    use crate::process_observation::snapshot_builder::ObservedProcess;
    use crate::process_observation::snapshot_builder::snapshot_of;

    fn target_id(value: u64) -> TerminationTargetId {
        let value = std::num::NonZeroU64::MIN.saturating_add(value.saturating_sub(1));
        TerminationTargetId::for_test(value)
    }

    fn process(pid: u32) -> ObservedProcess {
        ObservedProcess::new(
            pid,
            u64::from(pid),
            &format!("process-{pid}"),
            "/usr/bin/process",
            &["process"],
        )
        .with_cwd(std::path::Path::new("/workspace"))
    }

    #[test]
    fn new_descendant_requires_a_complete_validated_chain_to_a_live_root() {
        let root = process(10);
        let middle = process(11).with_validated_parent(root.identity());
        let leaf = process(12).with_validated_parent(middle.identity());
        let broken = process(13).with_unobservable_parentage();
        let snapshot = snapshot_of(&[root.clone(), middle.clone(), leaf.clone(), broken]);
        let pass = observe_termination_descendants(
            &snapshot,
            &[FrozenTerminationRootObservation::new(
                target_id(1),
                root.identity().clone(),
            )],
            &[],
        );

        assert_eq!(
            pass.new_candidates()
                .iter()
                .map(NewTerminationDescendantCandidate::process_identity)
                .collect::<Vec<_>>(),
            vec![leaf.identity(), middle.identity()]
        );
        assert_eq!(pass.new_candidates()[0].depth_from_root(), 2);
        assert_eq!(pass.new_candidates()[1].depth_from_root(), 1);
    }

    #[test]
    fn admitted_descendant_stays_tracked_after_root_exit_but_new_process_is_not_admitted() {
        let root = process(20);
        let admitted = process(21).with_validated_parent(root.identity());
        let newly_visible = process(22).with_validated_parent(admitted.identity());
        let snapshot = snapshot_of(&[admitted.clone(), newly_visible]);
        let pass = observe_termination_descendants(
            &snapshot,
            &[FrozenTerminationRootObservation::new(
                target_id(1),
                root.identity().clone(),
            )],
            &[AdmittedTerminationDescendantObservation::new(
                target_id(2),
                admitted.identity().clone(),
            )],
        );

        assert_eq!(
            pass.root_presence().get(&target_id(1)),
            Some(&FrozenTerminationRootPresence::Gone)
        );
        assert_eq!(
            pass.descendant_presence().get(&target_id(2)),
            Some(&AdmittedTerminationDescendantPresence::Live)
        );
        assert!(pass.new_candidates().is_empty());
    }

    #[test]
    fn new_descendant_admission_stops_after_the_declared_parent_walk_depth() {
        let root = process(30);
        let mut observed_processes = vec![root.clone()];
        let mut parent_identity = root.identity().clone();
        let mut next_pid = 31;

        for _ in 0..MAX_DESCENDANT_WALK_DEPTH {
            let descendant = process(next_pid).with_validated_parent(&parent_identity);
            parent_identity = descendant.identity().clone();
            observed_processes.push(descendant);
            next_pid = next_pid.saturating_add(1);
        }
        let descendant_at_depth_limit = parent_identity;
        let descendant_beyond_depth_limit =
            process(next_pid).with_validated_parent(&descendant_at_depth_limit);
        observed_processes.push(descendant_beyond_depth_limit.clone());
        let snapshot = snapshot_of(&observed_processes);

        let pass = observe_termination_descendants(
            &snapshot,
            &[FrozenTerminationRootObservation::new(
                target_id(1),
                root.identity().clone(),
            )],
            &[],
        );

        assert!(pass.new_candidates().iter().any(|candidate| {
            candidate.process_identity() == &descendant_at_depth_limit
                && candidate.depth_from_root() == MAX_DESCENDANT_WALK_DEPTH
        }));
        assert!(
            pass.new_candidates()
                .iter()
                .all(|candidate| candidate.process_identity()
                    != descendant_beyond_depth_limit.identity())
        );
    }
}
