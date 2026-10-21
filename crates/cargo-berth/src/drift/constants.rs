//! Named drift cache file spellings.

// `DRIFT_CACHE_FILE_PREFIX` and `DRIFT_CACHE_FILE_SUFFIX` delimit cache filenames.
/// The prefix every per-worktree drift fingerprint cache file carries.
pub(super) const DRIFT_CACHE_FILE_PREFIX: &str = "drift-fingerprint-";
/// The suffix every per-worktree drift fingerprint cache file carries.
pub(super) const DRIFT_CACHE_FILE_SUFFIX: &str = ".json";
