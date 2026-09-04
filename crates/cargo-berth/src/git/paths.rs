//! Which paths a batch of commits touched.
//!
//! These reads answer a different question from reachability: not whether one
//! commit contains another, but which files a range of commits changed. Both
//! members return git's output raw — the arguments supplied and the process
//! result — for `drift/git_output.rs` to parse into attribution records.

use std::fmt::Write;
use std::path::Path;

use super::command;
use super::command::GitCommandOutputAvailability;
use super::constants::GIT_DENSE_COMBINED_ARG;
use super::constants::GIT_DIFF_TREE_COMMAND;
use super::constants::GIT_LITERAL_TOP_PATHSPEC_PREFIX;
use super::constants::GIT_LOG_COMMAND;
use super::constants::GIT_NAME_ONLY_ARG;
use super::constants::GIT_NAME_STATUS_ARG;
use super::constants::GIT_NO_RENAMES_ARG;
use super::constants::GIT_NUL_TERMINATED_ARG;
use super::constants::GIT_PATHSPEC_SEPARATOR;
use super::constants::GIT_RECURSIVE_ARG;
use super::constants::GIT_ROOT_ARG;
use super::constants::GIT_STDIN_ARG;
use crate::ids::GitObjectId;
use crate::ids::ReservationScopePath;

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
pub(crate) fn incursion_path_log(
    repository_root: &Path,
    target: &GitObjectId,
    paths: &[ReservationScopePath],
) -> IncursionPathLogInvocation {
    let record_format = format!("--format=%x00{INCURSION_ATTRIBUTION_RECORD_MARKER}%x00%H%x00%s");
    let mut arguments = Vec::with_capacity(paths.len() + 8);
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
        paths
            .iter()
            .map(|path| format!("{GIT_LITERAL_TOP_PATHSPEC_PREFIX}{path}")),
    );
    let output_availability = command::git_output_dynamic(repository_root, &arguments).into();
    IncursionPathLogInvocation {
        arguments,
        output_availability,
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
