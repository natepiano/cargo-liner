//! Reads per-project lint state from cache-rooted JSON artifacts.

use std::path::Path;

mod cache_size_index;
mod history;
mod lint_runs;
mod paths;
mod read_write;
mod runtime;
mod status;
mod trigger;
mod types;

/// Reclaim a project's lint cache directory. Best-effort: silent
/// on missing or locked paths. Called from the dismiss flow when
/// the project at `project_root` is gone from disk so a future
/// worktree/branch reusing this exact path starts clean.
pub(crate) fn reclaim_project_cache(project_root: &Path) {
    reclaim_project_cache_under(paths::cache_root().as_path(), project_root);
}

/// Cache-root-explicit variant of [`reclaim_project_cache`].
/// Matches the `_under` pattern used by the rest of the module so
/// tests can target a tempdir.
pub(super) fn reclaim_project_cache_under(cache_root: &Path, project_root: &Path) {
    let project_dir = paths::project_dir_under(cache_root, project_root);
    let bytes = history::project_dir_bytes(&project_dir);
    if std::fs::remove_dir_all(&project_dir).is_ok() && bytes > 0 {
        cache_size_index::adjust(cache_root, -i64::try_from(bytes).unwrap_or(i64::MAX));
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unreachable,
    reason = "tests should panic on unexpected values"
)]
mod tests;

pub use history::CacheUsage;
pub use history::read_history;
pub use history::retained_cache_usage;
pub use lint_runs::LintRuns;
#[cfg(test)]
pub use paths::cache_root;
#[cfg(test)]
pub use paths::latest_path_under;
pub use paths::project_dir;
pub use runtime::RegisterProjectRequest;
pub use runtime::RuntimeHandle;
pub use runtime::project_is_eligible;
pub use runtime::spawn;
pub(crate) use trigger::CargoMetadataTriggerKind;
pub(crate) use trigger::classify_cargo_metadata_basename;
pub(crate) use trigger::classify_cargo_metadata_event_path;
pub(crate) use trigger::classify_event_path;
#[cfg(test)]
pub use types::LintCommand;
#[cfg(test)]
pub use types::LintCommandStatus;
pub use types::LintRun;
pub use types::LintRunStatus;
pub use types::LintStatus;
pub use types::LintStatusKind;
