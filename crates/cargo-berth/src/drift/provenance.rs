//! The commits behind an incursion's entered paths.

use std::path::Path;

use super::constants::GIT_FIELD_SEPARATOR;
use super::constants::GIT_HEAD_REVISION;
use super::constants::GIT_LOG_COMMAND;
use super::constants::GIT_LOG_FORMAT_ARGUMENT;
use super::constants::GIT_PATHSPEC_SEPARATOR;
use super::git_output;
use super::git_output::DriftFingerprintError;
use super::observation::ObservedDriftChanges;
use super::report::DriftEffect;
use super::report::DriftReport;
use super::report::IncursionCommit;
use super::report::IncursionCommitOrigin;
use super::report::ReservationDriftResult;
use crate::config::BerthConfig;
use crate::config::Enrollment;
use crate::git;
use crate::git::Reachability;
use crate::ids::GitObjectId;
use crate::ids::ReservationScopePath;
use crate::ledger::IncursionPathSet;
use crate::reservation::RetainedReservationSet;

/// Name the commits behind every entered path an incursion took from the phase range.
///
/// The message a reader acts on names paths and reservation ids only, so a path that
/// arrived on a replayed upstream commit and a path this worktree wrote read the same.
/// Only the committed component can carry a path the worktree never opened, so only
/// those paths are looked up, and a working-tree incursion is left as it was.
pub(super) fn name_incursion_commits(
    repository_root: &Path,
    reservations: &RetainedReservationSet,
    changes: &ObservedDriftChanges,
    report: &mut DriftReport,
) -> Result<(), DriftFingerprintError> {
    let trunk = trunk_object_id(repository_root);
    for result in &mut report.results {
        let ReservationDriftResult::Changed {
            reservation_id,
            effects,
        } = result
        else {
            continue;
        };
        let Ok(reservation) = reservations.reservation(*reservation_id) else {
            continue;
        };
        let phase_start = reservation.phase_start_head().as_ref().to_string();
        let committed = changes.committed_paths(*reservation_id);
        for effect in effects.as_mut_slice() {
            let DriftEffect::Incursion { paths, commits, .. } = effect else {
                continue;
            };
            *commits = commits_for_paths(
                repository_root,
                trunk.as_ref(),
                &phase_start,
                committed,
                paths,
            )?;
        }
    }
    Ok(())
}

/// The trunk tip, when the repository is configured and git can resolve it.
///
/// A missing trunk costs the origin of each commit, not the commits themselves, so
/// the lookup reports nothing rather than failing the run that found the incursion.
fn trunk_object_id(repository_root: &Path) -> Option<GitObjectId> {
    let Ok(Enrollment::Enrolled(configuration)) = BerthConfig::read(repository_root) else {
        return None;
    };
    git::branch_object_id(repository_root, &configuration.trunk).ok()
}

fn commits_for_paths(
    repository_root: &Path,
    trunk: Option<&GitObjectId>,
    phase_start: &str,
    committed: &[ReservationScopePath],
    entered: &IncursionPathSet,
) -> Result<Vec<IncursionCommit>, DriftFingerprintError> {
    let mut commits: Vec<IncursionCommit> = Vec::new();
    for path in entered.as_slice() {
        if !committed.contains(path) {
            continue;
        }
        for (commit, subject) in path_commits(repository_root, phase_start, path)? {
            if let Some(existing) = commits.iter_mut().find(|entry| entry.commit == commit) {
                existing.paths.push(path.clone());
                continue;
            }
            commits.push(IncursionCommit {
                origin: commit_origin(repository_root, trunk, &commit),
                commit,
                subject,
                paths: vec![path.clone()],
            });
        }
    }
    Ok(commits)
}

/// Whether trunk already carried a commit, so this phase received it rather than wrote it.
fn commit_origin(
    repository_root: &Path,
    trunk: Option<&GitObjectId>,
    commit: &GitObjectId,
) -> IncursionCommitOrigin {
    let Some(trunk) = trunk else {
        return IncursionCommitOrigin::Unknown;
    };
    match git::reachability(repository_root, commit, trunk) {
        Ok(Reachability::Ancestor) => IncursionCommitOrigin::AlreadyOnTrunk,
        Ok(Reachability::NotAncestor) => IncursionCommitOrigin::PhaseAuthored,
        Ok(Reachability::ObjectUnknown) | Err(_) => IncursionCommitOrigin::Unknown,
    }
}

/// Ask git which commits in the phase range touched one path.
///
/// One call per entered path rather than a single `--name-status` walk: an incursion
/// names a handful of paths, and `git log`'s interleaving of a format line with
/// NUL-terminated names has no parse that is obviously right at a glance.
fn path_commits(
    repository_root: &Path,
    phase_start: &str,
    path: &ReservationScopePath,
) -> Result<Vec<(GitObjectId, String)>, DriftFingerprintError> {
    let range = format!("{phase_start}..{GIT_HEAD_REVISION}");
    let path = path.to_string();
    let output = git_output::run_git(
        repository_root,
        &[
            GIT_LOG_COMMAND,
            GIT_LOG_FORMAT_ARGUMENT,
            &range,
            GIT_PATHSPEC_SEPARATOR,
            &path,
        ],
    )?;
    let text = String::from_utf8(output.stdout)
        .map_err(|error| DriftFingerprintError::MalformedGitOutput(error.to_string()))?;
    Ok(text
        .lines()
        .filter_map(|line| line.split_once(GIT_FIELD_SEPARATOR))
        .filter_map(|(commit, subject)| {
            commit
                .parse::<GitObjectId>()
                .ok()
                .map(|commit| (commit, subject.to_owned()))
        })
        .collect())
}
