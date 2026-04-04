use std::path::PathBuf;

use crate::config;
use crate::config::Config;
use crate::constants::APP_NAME;
use crate::constants::CI_CACHE_DIR;
use crate::constants::LINTS_CACHE_DIR;

/// Default app-owned cache root under the platform cache directory.
pub fn default_app_cache_root() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_NAME)
}

/// Resolve the configured cache root for a given `Config`.
pub fn configured_app_cache_root_for(cfg: &Config) -> PathBuf {
    if cfg.cache.root.is_empty() {
        return default_app_cache_root();
    }

    let configured = PathBuf::from(&cfg.cache.root);
    if configured.is_absolute() {
        configured
    } else {
        default_app_cache_root().join(configured)
    }
}

/// Resolve the active app cache root from the process' last good config.
pub fn app_cache_root() -> PathBuf {
    let cfg = config::active_config();
    configured_app_cache_root_for(&cfg)
}

/// Cache root for repo-keyed CI data.
pub fn ci_cache_root() -> PathBuf { app_cache_root().join(CI_CACHE_DIR) }

/// Cache root for project-keyed lint runs under a specific config.
pub fn lint_runs_root_for(cfg: &Config) -> PathBuf {
    configured_app_cache_root_for(cfg).join(LINTS_CACHE_DIR)
}

/// Cache root for project-keyed lint runs.
pub fn lint_runs_root() -> PathBuf { app_cache_root().join(LINTS_CACHE_DIR) }

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn default_root_is_app_scoped() {
        let root = default_app_cache_root();
        assert_eq!(root.file_name().and_then(|n| n.to_str()), Some(APP_NAME));
    }

    #[test]
    fn empty_cache_root_uses_default() {
        let cfg = Config::default();
        assert_eq!(
            configured_app_cache_root_for(&cfg),
            default_app_cache_root()
        );
    }

    #[test]
    fn relative_cache_root_extends_default_root() {
        let mut cfg = Config::default();
        cfg.cache.root = "custom-cache".to_string();

        assert_eq!(
            configured_app_cache_root_for(&cfg),
            default_app_cache_root().join("custom-cache")
        );
    }

    #[test]
    fn absolute_cache_root_replaces_default_root() {
        let mut cfg = Config::default();
        cfg.cache.root = "/tmp/cargo-port-cache".to_string();

        assert_eq!(
            configured_app_cache_root_for(&cfg),
            PathBuf::from("/tmp/cargo-port-cache")
        );
    }
}
