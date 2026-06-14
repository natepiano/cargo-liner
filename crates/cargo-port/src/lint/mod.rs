//! Reads per-project lint state from cache-rooted JSON artifacts.

mod constants;

mod cache_size_index;
mod history;
mod paths;
mod read_write;
mod reclaim;
mod run;
mod runs;
mod runtime;
mod status;
mod trigger;

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
#[cfg(test)]
pub use paths::latest_path_under;
pub use paths::project_dir;
pub(crate) fn reclaim_project_cache(project_root: &Path) {
    reclaim::reclaim_project_cache(project_root);
}

#[cfg(test)]
pub(super) fn reclaim_project_cache_under(cache_root: &Path, project_root: &Path) {
    reclaim::reclaim_project_cache_under(cache_root, project_root);
}
use std::path::Path;

#[cfg(test)]
pub use run::LintCommand;
#[cfg(test)]
pub use run::LintCommandStatus;
pub use run::LintRun;
pub use run::LintRunOrigin;
pub use run::LintRunStatus;
pub use runs::LintRuns;
pub use runtime::ProjectLanguage;
pub use runtime::RegisterProjectRequest;
pub use runtime::RuntimeHandle;
pub use runtime::project_is_eligible;
pub use runtime::spawn;
pub use status::CachedLintStatus;
pub use status::LintStatus;
pub use status::LintStatusKind;
pub(crate) use status::parse_timestamp;
pub(crate) use trigger::CargoMetadataTriggerKind;
pub(crate) use trigger::classify_cargo_metadata_basename;
pub(crate) use trigger::classify_cargo_metadata_event_path;
pub(crate) use trigger::classify_event_path;
