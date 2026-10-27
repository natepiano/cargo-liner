//! Evidence that a holder has no committed or dirty change in a reservation's scopes.

use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use crate::drift;
use crate::git;
use crate::git::GitCommandOutputAvailability;
use crate::git::RepositorySnapshot;
use crate::ids::GitObjectId;
use crate::ids::InvalidReservationScopePath;
use crate::ids::ReservationScopePath;
use crate::ledger::ReservationScope;
use crate::ledger::ReservationScopeSet;
use crate::ledger::ScopeKind;
use crate::scope::PathCase;

/// A successful observation against this exact target found no differing scope path.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct UnchangedScopesEvidence {
    /// The target commit at which the reservation's scopes were empty.
    #[schemars(with = "String")]
    #[schemars(length(min = 1))]
    target_commit: GitObjectId,
}

impl UnchangedScopesEvidence {
    pub(crate) const fn target_commit(&self) -> &GitObjectId { &self.target_commit }
}

/// Complete changed paths shared by every stale reservation in one holder and target.
pub(crate) struct HolderScopeChanges {
    target_commit: GitObjectId,
    paths:         Vec<ReservationScopePath>,
}

/// Whether a successful holder observation proves these declared scopes empty.
pub(crate) enum ScopeChangeEvidence {
    Changed,
    Unchanged(UnchangedScopesEvidence),
}

impl HolderScopeChanges {
    /// Read commits since the merge base, staged, unstaged and untracked paths.
    /// Failed Git reads never construct evidence of emptiness.
    pub(crate) fn observe(
        repository: &RepositorySnapshot,
        root: &Path,
        target: &GitObjectId,
        head: &GitObjectId,
    ) -> Result<Self, String> {
        let range = format!("{target}...{head}");
        let mut paths = Self::git_paths(
            root,
            &[
                "diff",
                "--no-ext-diff",
                "--name-only",
                "--no-renames",
                "-z",
                &range,
                "--",
            ],
        )?;
        // Cached porcelain is shared with merge-extent observation. Its path parser includes
        // both sides of renames and keeps staged and unstaged changes independently.
        let status = repository
            .working_tree_status_at(root)
            .map_err(|error| error.to_string())?;
        paths.extend(drift::working_tree_changed_paths(&status.porcelain)?);
        Ok(Self {
            target_commit: target.clone(),
            paths,
        })
    }

    fn git_paths(root: &Path, arguments: &[&str]) -> Result<Vec<ReservationScopePath>, String> {
        let GitCommandOutputAvailability::Available(output) =
            git::execute_read_only_git(root, arguments)
        else {
            return Err("could not read scope changes".to_owned());
        };
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        String::from_utf8(output.stdout)
            .map_err(|error| error.to_string())?
            .split_terminator('\0')
            .map(|path| {
                path.parse()
                    .map_err(|error: InvalidReservationScopePath| error.to_string())
            })
            .collect()
    }

    pub(crate) fn for_scopes(
        &self,
        scopes: &ReservationScopeSet,
        path_case: PathCase,
    ) -> ScopeChangeEvidence {
        if self.paths.iter().any(|path| {
            let changed = ReservationScope {
                path: path.clone(),
                kind: ScopeKind::File,
            };
            scopes
                .as_slice()
                .iter()
                .any(|scope| scope.overlaps(&changed, path_case))
        }) {
            ScopeChangeEvidence::Changed
        } else {
            ScopeChangeEvidence::Unchanged(UnchangedScopesEvidence {
                target_commit: self.target_commit.clone(),
            })
        }
    }
}
