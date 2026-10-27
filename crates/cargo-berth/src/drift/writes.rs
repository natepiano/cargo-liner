//! Select unit-authored committed paths before any post-write consequence.

use std::collections::HashMap;
use std::collections::HashSet;

use super::git_output;
use super::git_output::DriftFingerprintError;
use super::git_output::IncursionAttributionAnchorState;
use super::observation::FullPhaseHistoryObservation;
use super::ordering;
use super::selection::CommittedWriteSelection;
use crate::edge::RepositorySnapshot;
use crate::git;
use crate::git::HeadAttachment;
use crate::git::LocalTargetUpstream;
use crate::ids::GitObjectId;
use crate::ids::ReservationScopePath;

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
        if ancestry.integration_tips_precede_phases(
            &tips,
            &anchor_states.keys().cloned().collect::<Vec<_>>(),
        ) {
            return Ok(Self::NoReceivedCommits);
        }
        let mut anchors = tips.clone();
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
        let invocation = git::incursion_path_log(repository.repository_root(), target, paths);
        let output = git_output::completed_git_output(
            invocation.output_availability,
            &["log", "unit-authored paths"],
        )?;
        let commits = git_output::parse_incursion_path_log(&output.stdout)?;
        let by_phase_start = anchors
            .into_iter()
            .zip(ranges)
            .skip(tips.len())
            .map(|(anchor, range)| {
                let mut selected = commits
                    .iter()
                    .filter(|commit| {
                        authored.contains(&commit.commit) && range.contains(&commit.commit)
                    })
                    .flat_map(|commit| commit.paths.iter().cloned())
                    .collect::<Vec<_>>();
                ordering::normalize_paths(&mut selected);
                (anchor, selected)
            })
            .collect::<HashMap<_, _>>();
        let head_paths = commits
            .iter()
            .filter(|commit| &commit.commit == target && authored.contains(target))
            .flat_map(|commit| commit.paths.iter().cloned())
            .collect();
        Ok(Self::UnitWrites {
            by_phase_start,
            head_paths,
        })
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
