//! Translate protected phase intervals through Git's commit rewrite map.

use std::collections::HashSet;

use serde::Deserialize;
use serde::Serialize;

use crate::ids::GitObjectId;

/// One destination Git recorded for an old commit; destinations need not be unique.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct RewriteMapPair {
    /// The commit before the rewrite.
    pub(crate) old: GitObjectId,
    /// The commit after the rewrite.
    pub(crate) new: GitObjectId,
}

/// Commits introduced beyond a rewrite operation's base and original histories.
#[derive(Debug)]
pub(crate) struct RewriteCreatedCommits {
    /// Reachable new commits that no excluded history already contains.
    commits: HashSet<GitObjectId>,
}

impl RewriteCreatedCommits {
    /// Record the commits selected by the rewrite's reachability exclusions.
    pub(crate) fn from_commits(commits: impl IntoIterator<Item = GitObjectId>) -> Self {
        Self {
            commits: commits.into_iter().collect(),
        }
    }

    /// Select operation pairs whose destinations this branch's rewrite introduced.
    pub(crate) fn branch_pairs(&self, pairs: &[RewriteMapPair]) -> Vec<RewriteMapPair> {
        pairs
            .iter()
            .filter(|pair| self.contains(&pair.new))
            .cloned()
            .collect()
    }

    /// Whether this commit belongs to the history introduced by the rewrite.
    fn contains(&self, commit: &GitObjectId) -> bool { self.commits.contains(commit) }
}

/// A nominated interval whose scoped contents still require replay validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MappedPhaseInterval {
    /// The parent preceding the phase destinations and their unrecorded split commits.
    pub(crate) phase_start_head: GitObjectId,
    /// The latest phase destination in the rewritten history.
    pub(crate) protected_tip:    GitObjectId,
    /// Phase destinations and unrecorded split commits in rewritten first-parent order.
    pub(crate) destinations:     Vec<GitObjectId>,
}

/// Whether the rewrite map locates a complete interval for scoped replay.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum PhaseRewriteMapping {
    /// The endpoints nominate a location, without certifying its contents.
    Mapped(MappedPhaseInterval),
    /// No phase commit was rewritten, no destination survived, or no base precedes it.
    Refused,
}

/// Locate the phase's new endpoints without assuming a one-to-one rewrite.
///
/// Both histories run oldest first. The new history includes its base commit, so a
/// destination at index zero cannot establish the phase start. An unmapped old
/// commit either survives unchanged or was skipped because the new base carried it.
/// Created commits preceding a destination belong to it until another pair's
/// destination or history already present before the rewrite establishes the base.
pub(crate) fn map_phase_interval(
    pairs: &[RewriteMapPair],
    old_phase_commits: &[GitObjectId],
    new_history: &[GitObjectId],
    created: &RewriteCreatedCommits,
) -> PhaseRewriteMapping {
    let phase_commits = old_phase_commits.iter().collect::<HashSet<_>>();
    if !pairs.iter().any(|pair| phase_commits.contains(&pair.old)) {
        return PhaseRewriteMapping::Refused;
    }
    let mut destinations = HashSet::new();
    for commit in old_phase_commits {
        let mut mapped = false;
        for pair in pairs.iter().filter(|pair| &pair.old == commit) {
            destinations.insert(&pair.new);
            mapped = true;
        }
        if !mapped {
            destinations.insert(commit);
        }
    }
    let located = new_history
        .iter()
        .enumerate()
        .filter(|(_, commit)| destinations.contains(commit))
        .collect::<Vec<_>>();
    let (Some(&(first_index, _)), Some(&(tip_index, protected_tip))) =
        (located.first(), located.last())
    else {
        return PhaseRewriteMapping::Refused;
    };
    let Some(mut base_index) = first_index.checked_sub(1) else {
        return PhaseRewriteMapping::Refused;
    };
    let pair_destinations = pairs.iter().map(|pair| &pair.new).collect::<HashSet<_>>();
    while created.contains(&new_history[base_index])
        && !pair_destinations.contains(&new_history[base_index])
    {
        let Some(previous_index) = base_index.checked_sub(1) else {
            return PhaseRewriteMapping::Refused;
        };
        base_index = previous_index;
    }
    PhaseRewriteMapping::Mapped(MappedPhaseInterval {
        phase_start_head: new_history[base_index].clone(),
        protected_tip:    protected_tip.clone(),
        destinations:     new_history[base_index + 1..=tip_index]
            .iter()
            .filter(|commit| {
                destinations.contains(commit)
                    || (created.contains(commit) && !pair_destinations.contains(commit))
            })
            .cloned()
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::MappedPhaseInterval;
    use super::PhaseRewriteMapping;
    use super::RewriteCreatedCommits;
    use super::RewriteMapPair;
    use super::map_phase_interval;
    use crate::ids::GitObjectId;

    #[test]
    fn rewrite_map_derives_phase_interval() -> Result<(), Box<dyn Error>> {
        for (name, pairs, old, new, created, expected) in [
            (
                "plain rebase",
                vec![(1, 4), (2, 5)],
                vec![1, 2],
                vec![3, 4, 5],
                vec![4, 5],
                vec![3, 4, 5],
            ),
            (
                "reorder",
                vec![(1, 5), (2, 4)],
                vec![1, 2],
                vec![3, 4, 5],
                vec![4, 5],
                vec![3, 4, 5],
            ),
            (
                "squash across checkpoint",
                vec![(1, 4), (2, 4)],
                vec![1],
                vec![3, 4],
                vec![4],
                vec![3, 4],
            ),
            (
                "fixup",
                vec![(1, 4), (2, 4)],
                vec![1, 2],
                vec![3, 4],
                vec![4],
                vec![3, 4],
            ),
            (
                "split",
                vec![(1, 5)],
                vec![1],
                vec![3, 4, 5],
                vec![4, 5],
                vec![3, 4, 5],
            ),
            (
                "unchanged commits",
                vec![(2, 4)],
                vec![1, 2],
                vec![3, 1, 4],
                vec![4],
                vec![3, 1, 4],
            ),
            (
                "skipped commit",
                vec![(2, 4)],
                vec![1, 2],
                vec![3, 4],
                vec![4],
                vec![3, 4],
            ),
            (
                "excluded phase",
                vec![(2, 4)],
                vec![1],
                vec![3, 1, 4],
                vec![4],
                vec![],
            ),
            (
                "no destination",
                vec![(1, 4)],
                vec![1],
                vec![3, 5],
                vec![5],
                vec![],
            ),
            (
                "no preceding base",
                vec![(1, 4)],
                vec![1],
                vec![4, 5],
                vec![4, 5],
                vec![],
            ),
        ] {
            assert_phase_interval(name, &pairs, &old, &new, &created, &expected)?;
        }
        Ok(())
    }

    #[test]
    fn split_destinations_respect_rewrite_boundaries() -> Result<(), Box<dyn Error>> {
        for (name, pairs, old, new, created, expected) in [
            (
                "earlier rewritten phase stops split expansion",
                vec![(2, 3), (1, 5)],
                vec![1],
                vec![6, 3, 4, 5],
                vec![3, 4, 5],
                vec![3, 4, 5],
            ),
            (
                "split commit already in trunk stops expansion",
                vec![(1, 5)],
                vec![1],
                vec![3, 4, 5],
                vec![5],
                vec![4, 5],
            ),
            (
                "unrecorded split between mapped destinations",
                vec![(1, 4), (2, 6)],
                vec![1, 2],
                vec![3, 4, 5, 6],
                vec![4, 5, 6],
                vec![3, 4, 5, 6],
            ),
            (
                "split expansion has no preceding base",
                vec![(1, 5)],
                vec![1],
                vec![4, 5],
                vec![4, 5],
                vec![],
            ),
        ] {
            assert_phase_interval(name, &pairs, &old, &new, &created, &expected)?;
        }
        Ok(())
    }

    fn assert_phase_interval(
        name: &str,
        pairs: &[(u8, u8)],
        old: &[u8],
        new: &[u8],
        created: &[u8],
        expected: &[u8],
    ) -> Result<(), Box<dyn Error>> {
        let oid = |number: u8| format!("{number:040x}").parse::<GitObjectId>();
        let commits = |numbers: &[u8]| {
            numbers
                .iter()
                .copied()
                .map(oid)
                .collect::<Result<Vec<_>, _>>()
        };
        let pairs = pairs
            .iter()
            .map(|&(old, new)| {
                Ok(RewriteMapPair {
                    old: oid(old)?,
                    new: oid(new)?,
                })
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        let actual = map_phase_interval(
            &pairs,
            &commits(old)?,
            &commits(new)?,
            &RewriteCreatedCommits::from_commits(commits(created)?),
        );
        let expected = commits(expected)?;
        let expected = match (expected.first(), expected.last()) {
            (Some(base), Some(tip)) => PhaseRewriteMapping::Mapped(MappedPhaseInterval {
                phase_start_head: base.clone(),
                protected_tip:    tip.clone(),
                destinations:     expected[1..].to_vec(),
            }),
            _ => PhaseRewriteMapping::Refused,
        };
        assert_eq!(actual, expected, "{name}");
        Ok(())
    }
}
