//! Which paths a batch of commits touched.
//!
//! These reads answer a different question from reachability: not whether one
//! commit contains another, but which files a range of commits changed. Both
//! members return git's output raw — the arguments supplied and the process
//! result — for `drift/git_output.rs` to parse into attribution records.

use std::collections::HashSet;
use std::fmt::Write;
use std::path::Path;

use super::command;
use super::command::GitCommandOutputAvailability;
use super::constants::GIT_DENSE_COMBINED_ARG;
use super::constants::GIT_DIFF_TREE_COMMAND;
use super::constants::GIT_EXCLUDE_REVISION_PREFIX;
use super::constants::GIT_FIRST_PARENT_MERGE_DIFF_ARG;
use super::constants::GIT_LITERAL_TOP_PATHSPEC_PREFIX;
use super::constants::GIT_LOG_COMMAND;
use super::constants::GIT_MAX_COUNT_ARG_PREFIX;
use super::constants::GIT_MERGE_BASE_COMMAND;
use super::constants::GIT_NAME_ONLY_ARG;
use super::constants::GIT_NAME_STATUS_ARG;
use super::constants::GIT_NO_COMMIT_FORMAT_ARG;
use super::constants::GIT_NO_MERGE_BASE_EXIT_CODE;
use super::constants::GIT_NO_RENAMES_ARG;
use super::constants::GIT_NUL_TERMINATED_ARG;
use super::constants::GIT_PARENTS_ARG;
use super::constants::GIT_PATHSPEC_SEPARATOR;
use super::constants::GIT_RECURSIVE_ARG;
use super::constants::GIT_REV_LIST_COMMAND;
use super::constants::GIT_ROOT_ARG;
use super::constants::GIT_STDIN_ARG;
use crate::ids::GitObjectId;
use crate::ids::ReservationScopePath;

/// Check every changed path in the candidate range, including changes a later commit reverts.
pub(crate) fn commit_range_touches_only(
    repository_root: &Path,
    start: &GitObjectId,
    end: &GitObjectId,
    owns_path: impl Fn(&[u8]) -> bool,
) -> bool {
    let range = format!("{start}..{end}");
    let arguments = [
        GIT_LOG_COMMAND,
        GIT_NO_COMMIT_FORMAT_ARG,
        GIT_NAME_ONLY_ARG,
        GIT_NUL_TERMINATED_ARG,
        GIT_NO_RENAMES_ARG,
        GIT_FIRST_PARENT_MERGE_DIFF_ARG,
        &range,
        GIT_PATHSPEC_SEPARATOR,
    ];
    command::git_output(repository_root, arguments).is_ok_and(|output| {
        output.status.success()
            && output
                .stdout
                .strip_suffix(&[0])
                .is_some_and(|paths| paths.split(|byte| *byte == 0).all(owns_path))
    })
}

/// Prove that two tips share no commits beyond the excluded histories.
pub(crate) fn commit_ranges_are_disjoint(
    repository_root: &Path,
    left: &GitObjectId,
    right: &GitObjectId,
    excluded: &[&GitObjectId],
) -> bool {
    let Ok(bases) = command::git_output(
        repository_root,
        [
            GIT_MERGE_BASE_COMMAND,
            "--all",
            &left.to_string(),
            &right.to_string(),
        ],
    ) else {
        return false;
    };
    if bases.status.code() == Some(GIT_NO_MERGE_BASE_EXIT_CODE) {
        return true;
    }
    let Ok(bases_text) = std::str::from_utf8(&bases.stdout) else {
        return false;
    };
    if !bases.status.success() || bases_text.is_empty() {
        return false;
    }
    let mut arguments = vec![
        GIT_REV_LIST_COMMAND.to_owned(),
        format!("{GIT_MAX_COUNT_ARG_PREFIX}1"),
    ];
    arguments.extend(bases_text.lines().map(str::to_owned));
    arguments.extend(
        excluded
            .iter()
            .map(|commit| format!("{GIT_EXCLUDE_REVISION_PREFIX}{commit}")),
    );
    command::git_output_dynamic(repository_root, &arguments)
        .is_ok_and(|output| output.status.success() && output.stdout.is_empty())
}

/// Prove that a foreign checkout obtained a linear candidate only through merge parents.
pub(crate) fn checkout_imported_linear_candidate(
    repository_root: &Path,
    candidate_start: &GitObjectId,
    candidate: &GitObjectId,
    foreign_tip: &GitObjectId,
    excluded: &[&GitObjectId],
) -> bool {
    let candidate_revision = candidate.to_string();
    let candidate_start_revision = candidate_start.to_string();
    let Some(candidate_history) = revision_parents(repository_root, candidate, &[candidate_start])
    else {
        return false;
    };
    let mut candidate_commits = HashSet::new();
    let mut next = candidate_revision.as_str();
    for line in candidate_history.lines() {
        let mut fields = line.split_ascii_whitespace();
        let (Some(commit), Some(parent), None) = (fields.next(), fields.next(), fields.next())
        else {
            return false;
        };
        if commit != next {
            return false;
        }
        candidate_commits.insert(commit);
        next = parent;
    }
    if next != candidate_start_revision
        || candidate_commits.contains(foreign_tip.to_string().as_str())
    {
        return false;
    }

    let mut boundaries = vec![candidate];
    boundaries.extend_from_slice(excluded);
    let Some(foreign_history) = revision_parents(repository_root, foreign_tip, &boundaries) else {
        return false;
    };
    let mut import_edges = 0;
    for line in foreign_history.lines() {
        let mut parents = line.split_ascii_whitespace().skip(1);
        if parents
            .next()
            .is_some_and(|parent| candidate_commits.contains(parent))
        {
            return false;
        }
        import_edges += parents
            .filter(|parent| candidate_commits.contains(parent))
            .count();
    }
    import_edges != 0
}

fn revision_parents(
    repository_root: &Path,
    tip: &GitObjectId,
    excluded: &[&GitObjectId],
) -> Option<String> {
    let mut arguments = vec![
        GIT_REV_LIST_COMMAND.to_owned(),
        GIT_PARENTS_ARG.to_owned(),
        tip.to_string(),
    ];
    arguments.extend(
        excluded
            .iter()
            .map(|commit| format!("{GIT_EXCLUDE_REVISION_PREFIX}{commit}")),
    );
    let output = command::git_output_dynamic(repository_root, &arguments).ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// Compare every readable phase start with one target, and read what the target itself
/// introduced, in a single git invocation.
///
/// `diff-tree --stdin` prefixes every non-empty result with the first supplied
/// object, so each pair line starts with its distinct phase-start anchor and the
/// lone target line starts with the target. Empty comparisons emit no record and
/// remain distinguishable because callers initialize every requested anchor before
/// parsing the output.
///
/// An anchor already standing at the target is dropped rather than compared with
/// itself: that comparison is empty by construction, and asking it would key two
/// different questions to one object. The record under the target is always the
/// target's own diff, so a caller that anchors there supplies the empty range itself.
pub(crate) fn phase_committed_path_diffs(
    repository_root: &Path,
    anchors: &[GitObjectId],
    target: &GitObjectId,
) -> GitCommandOutputAvailability {
    let mut input = anchors.iter().filter(|anchor| *anchor != target).fold(
        String::new(),
        |mut input, anchor| {
            let _ = writeln!(input, "{anchor} {target}");
            input
        },
    );
    let _ = writeln!(input, "{target}");
    let arguments = [
        GIT_DIFF_TREE_COMMAND.to_owned(),
        GIT_STDIN_ARG.to_owned(),
        GIT_RECURSIVE_ARG.to_owned(),
        GIT_NAME_STATUS_ARG.to_owned(),
        GIT_NUL_TERMINATED_ARG.to_owned(),
        GIT_NO_RENAMES_ARG.to_owned(),
        GIT_DENSE_COMBINED_ARG.to_owned(),
        GIT_ROOT_ARG.to_owned(),
    ];
    command::git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes()).into()
}

/// Read every selected path's commits for later per-anchor membership filtering.
///
/// Each record carries the commit, its committer time in whole seconds, and its subject,
/// then the selected paths it touched.
pub(crate) fn incursion_path_log(
    repository_root: &Path,
    target: &GitObjectId,
    reservation_scope_paths: &[ReservationScopePath],
) -> IncursionPathLogInvocation {
    let record_format =
        format!("--format=%x00{INCURSION_ATTRIBUTION_RECORD_MARKER}%x00%H%x00%ct%x00%s");
    let mut arguments = Vec::with_capacity(reservation_scope_paths.len() + 8);
    arguments.extend([
        GIT_LOG_COMMAND.to_owned(),
        GIT_NUL_TERMINATED_ARG.to_owned(),
        GIT_NAME_ONLY_ARG.to_owned(),
        GIT_NO_RENAMES_ARG.to_owned(),
        GIT_DENSE_COMBINED_ARG.to_owned(),
        record_format,
        target.to_string(),
        GIT_PATHSPEC_SEPARATOR.to_owned(),
    ]);
    arguments.extend(
        reservation_scope_paths
            .iter()
            .map(|path| format!("{GIT_LITERAL_TOP_PATHSPEC_PREFIX}{path}")),
    );
    let git_command_output_availability =
        command::git_output_dynamic(repository_root, &arguments).into();
    IncursionPathLogInvocation {
        arguments,
        output_availability: git_command_output_availability,
    }
}

/// The record boundary emitted by the batched incursion-attribution log.
pub(crate) const INCURSION_ATTRIBUTION_RECORD_MARKER: &str = "cargo-berth-incursion-commit";

/// One incursion path-log invocation and the exact arguments supplied to git.
pub(crate) struct IncursionPathLogInvocation {
    /// The arguments supplied after the git binary.
    pub(crate) arguments:           Vec<String>,
    /// Whether that invocation left a process output available.
    pub(crate) output_availability: GitCommandOutputAvailability,
}
