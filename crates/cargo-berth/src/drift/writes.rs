//! Select unit-authored committed paths before any post-write consequence.

use std::collections::HashMap;
use std::collections::HashSet;

use super::git_output;
use super::git_output::DriftFingerprintError;
use super::git_output::IncursionAttributionAnchorState;
use super::git_output::IncursionPathCommit;
use super::observation::FullPhaseHistoryObservation;
use super::ordering;
use super::selection::CommittedWriteSelection;
use crate::edge::RepositorySnapshot;
use crate::git;
use crate::git::HeadAttachment;
use crate::git::LocalTargetUpstream;
use crate::git::PhaseMergeHistory;
use crate::git::PhaseStartTargetFirstParentHistories;
use crate::git::ScopedPatchTargetHistory;
use crate::ids::GitObjectId;
use crate::ids::ReservationScopePath;

/// A commit's contribution to one unit phase, separate from integration-target membership.
pub(super) enum PhaseCommitOrigin {
    /// A direct commit or work introduced by the merge being judged now.
    UnitWritten,
    /// The commit entered through an earlier merge, rather than this invocation.
    ReceivedFromMergeBranch,
    /// No retained ancestry proves which side supplied the commit.
    Unclassified,
}

/// One owner for side-history attribution, shared by write selection and incursion evidence.
pub(super) fn phase_commit_origin(
    ancestry: &PhaseStartTargetFirstParentHistories,
    phase_start: &GitObjectId,
    commit: &GitObjectId,
) -> PhaseCommitOrigin {
    match ancestry.after_phase_start(phase_start) {
        ScopedPatchTargetHistory::ProvenFirstParentInterval { commits } => {
            if commits.contains(commit) || ancestry.entered_with_target_merge(commit) {
                PhaseCommitOrigin::UnitWritten
            } else {
                PhaseCommitOrigin::ReceivedFromMergeBranch
            }
        },
        ScopedPatchTargetHistory::NeedsGitQueries
        | ScopedPatchTargetHistory::MappedDestinations { .. } => PhaseCommitOrigin::Unclassified,
    }
}

impl CommittedWriteSelection {
    pub(super) fn observe(
        repository: &git::RepositorySnapshot,
        snapshot: &RepositorySnapshot,
        history: &FullPhaseHistoryObservation,
        paths: &[ReservationScopePath],
    ) -> Result<Self, DriftFingerprintError> {
        let FullPhaseHistoryObservation::Anchored {
            target,
            anchor_states,
            ancestry,
        } = history
        else {
            return Ok(Self::Unclassified);
        };
        if paths.is_empty() {
            return Ok(Self::Unclassified);
        }
        let tips = integration_authorship_tips(repository, snapshot);
        if tips.is_empty() {
            return Ok(Self::Unclassified);
        }
        let no_new_target_history = ancestry.integration_tips_precede_phases(
            &tips,
            &anchor_states.keys().cloned().collect::<Vec<_>>(),
        );
        if no_new_target_history
            && anchor_states.keys().all(|anchor| {
                matches!(
                    ancestry.phase_merge_history(anchor),
                    PhaseMergeHistory::NoEarlierMerge
                )
            })
        {
            // Linear intervals need no path log. A merge at HEAD is judged at its
            // own post-commit boundary; only earlier merges are received history.
            return Ok(Self::Unclassified);
        }
        let invocation = git::incursion_path_log(repository.repository_root(), target, paths);
        let output = git_output::completed_git_output(
            invocation.output_availability,
            &["log", "unit-authored paths"],
        )?;
        // Dense combined merge diffs name only paths changed relative to every parent:
        // the resolution is the unit's write, while a path copied from a parent is not.
        let commits = git_output::parse_incursion_path_log(&output.stdout)?;
        if no_new_target_history {
            return Ok(Self::writes_from_first_parent_intervals(
                ancestry,
                anchor_states,
                &commits,
                target,
                paths,
            ));
        }
        Self::writes_outside_targets(repository, ancestry, anchor_states, &tips, &commits, target)
    }

    fn writes_outside_targets(
        repository: &git::RepositorySnapshot,
        ancestry: &PhaseStartTargetFirstParentHistories,
        anchor_states: &HashMap<GitObjectId, IncursionAttributionAnchorState>,
        tips: &[GitObjectId],
        commits: &[IncursionPathCommit],
        target: &GitObjectId,
    ) -> Result<Self, DriftFingerprintError> {
        let mut anchors = tips.to_vec();
        anchors.extend(anchor_states.iter().filter_map(|(anchor, state)| {
            (*state != IncursionAttributionAnchorState::ObjectUnknown).then_some(anchor.clone())
        }));
        let ranges = git::incursion_range_commits(repository.repository_root(), &anchors, target)?;
        // Intersecting HEAD-minus-each-target excludes the union of integrated history.
        let mut authored = ranges[0].clone();
        for range in &ranges[1..tips.len()] {
            authored.retain(|commit| range.contains(commit));
        }
        if authored.is_empty() {
            return Ok(Self::UnitWrites {
                by_phase_start: anchors[tips.len()..]
                    .iter()
                    .cloned()
                    .map(|anchor| (anchor, Vec::new()))
                    .collect(),
                head_paths:     Vec::new(),
            });
        }
        let by_phase_start = anchors
            .into_iter()
            .zip(ranges)
            .skip(tips.len())
            .map(|(anchor, range)| {
                let mut selected = commits
                    .iter()
                    .filter(|commit| {
                        authored.contains(&commit.commit)
                            && range.contains(&commit.commit)
                            && match phase_commit_origin(ancestry, &anchor, &commit.commit) {
                                PhaseCommitOrigin::UnitWritten
                                | PhaseCommitOrigin::Unclassified => true,
                                PhaseCommitOrigin::ReceivedFromMergeBranch => false,
                            }
                    })
                    .flat_map(|commit| commit.paths.iter().cloned())
                    .collect::<Vec<_>>();
                ordering::normalize_paths(&mut selected);
                (anchor, selected)
            })
            .collect::<HashMap<_, _>>();
        let head_paths = commits
            .iter()
            .filter(|commit| {
                (&commit.commit == target || ancestry.entered_with_target_merge(&commit.commit))
                    && authored.contains(&commit.commit)
            })
            .flat_map(|commit| commit.paths.iter().cloned())
            .collect();
        Ok(Self::UnitWrites {
            by_phase_start,
            head_paths,
        })
    }

    fn writes_from_first_parent_intervals(
        ancestry: &PhaseStartTargetFirstParentHistories,
        anchor_states: &HashMap<GitObjectId, IncursionAttributionAnchorState>,
        commits: &[IncursionPathCommit],
        target: &GitObjectId,
        paths: &[ReservationScopePath],
    ) -> Self {
        // The recorded targets contribute no new commits, but an unrecorded merge
        // branch can. Reuse the observed first-parent intervals instead of reading
        // another complete revision graph just to exclude its side history.
        let by_phase_start = anchor_states
            .iter()
            .filter_map(|(anchor, state)| {
                if *state == IncursionAttributionAnchorState::ObjectUnknown {
                    return None;
                }
                let selected = match ancestry.after_phase_start(anchor) {
                    ScopedPatchTargetHistory::ProvenFirstParentInterval { .. } => commits
                        .iter()
                        .filter(|commit| {
                            match phase_commit_origin(ancestry, anchor, &commit.commit) {
                                PhaseCommitOrigin::UnitWritten => true,
                                PhaseCommitOrigin::ReceivedFromMergeBranch
                                | PhaseCommitOrigin::Unclassified => false,
                            }
                        })
                        .flat_map(|commit| commit.paths.iter().cloned())
                        .collect(),
                    ScopedPatchTargetHistory::NeedsGitQueries
                    | ScopedPatchTargetHistory::MappedDestinations { .. } => paths.to_vec(),
                };
                Some((anchor.clone(), selected))
            })
            .collect();
        let head_paths = commits
            .iter()
            .filter(|commit| {
                &commit.commit == target || ancestry.entered_with_target_merge(&commit.commit)
            })
            .flat_map(|commit| commit.paths.iter().cloned())
            .collect();
        Self::UnitWrites {
            by_phase_start,
            head_paths,
        }
    }
}

/// Authorship uses local integration branches and available upstreams. Integration
/// checkouts retain the existing per-holder cover checks: a cover cannot exempt
/// a contribution that entered an unrelated holder's scope.
fn integration_authorship_tips(
    repository: &git::RepositorySnapshot,
    snapshot: &RepositorySnapshot,
) -> Vec<GitObjectId> {
    let objects = repository.object_reader();
    let Ok(attachment) = repository.head_attachment() else {
        return Vec::new();
    };
    if snapshot.targets().keys().any(|target| {
        matches!(&attachment, HeadAttachment::Branch { full_ref } if full_ref == target.reference())
    }) {
        return Vec::new();
    }
    let mut tips = HashSet::new();
    for target in snapshot.targets().keys() {
        if let Ok(tip) = objects.commit(target.reference().as_str()) {
            tips.insert(tip);
        }
        if let Ok(LocalTargetUpstream::Available(tip)) = target.local_upstream(&objects) {
            tips.insert(tip);
        }
    }
    tips.into_iter().collect()
}
