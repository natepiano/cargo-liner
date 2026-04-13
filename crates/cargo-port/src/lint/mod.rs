//! Reads per-project lint state from cache-rooted JSON artifacts.

mod history;
mod lint_runs;
mod paths;
mod read_write;
mod runtime;
mod status;
mod types;

#[cfg(test)]
#[allow(
    clippy::expect_used,
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
#[cfg(test)]
pub use types::LintCommand;
pub use types::LintCommandStatus;
pub use types::LintRun;
pub use types::LintRunStatus;
pub use types::LintStatus;
