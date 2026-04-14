use crate::config;
use crate::config::CargoPortConfig;
use crate::constants::APP_NAME;
use crate::constants::CI_CACHE_DIR;
use crate::constants::LINTS_CACHE_DIR;
use crate::project::AbsolutePath;

/// Default app-owned cache root under the platform cache directory.
pub(crate) fn default_app_cache_root() -> AbsolutePath {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_NAME)
        .into()
}

/// Resolve the configured cache root for a given `CargoPortConfig`.
fn configured_app_cache_root_for(cfg: &CargoPortConfig) -> AbsolutePath {
    if cfg.cache.root.is_empty() {
        return default_app_cache_root();
    }

    AbsolutePath::resolve_no_canonicalize(&cfg.cache.root, &default_app_cache_root())
}

/// Resolve the active app cache root from the process' last good config.
pub(crate) fn app_cache_root() -> AbsolutePath {
    let cfg = config::active_config();
    configured_app_cache_root_for(&cfg)
}

/// Cache root for repo-keyed CI data.
pub(crate) fn ci_cache_root() -> AbsolutePath { app_cache_root().join(CI_CACHE_DIR).into() }

/// Cache root for project-keyed lint runs under a specific config.
pub(crate) fn lint_runs_root_for(cfg: &CargoPortConfig) -> AbsolutePath {
    configured_app_cache_root_for(cfg)
        .join(LINTS_CACHE_DIR)
        .into()
}

/// Cache root for project-keyed lint runs.
pub(crate) fn lint_runs_root() -> AbsolutePath { app_cache_root().join(LINTS_CACHE_DIR).into() }

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
        let cfg = CargoPortConfig::default();
        assert_eq!(
            configured_app_cache_root_for(&cfg),
            default_app_cache_root()
        );
    }

    #[test]
    fn relative_cache_root_extends_default_root() {
        let mut cfg = CargoPortConfig::default();
        cfg.cache.root = "custom-cache".to_string();

        assert_eq!(
            configured_app_cache_root_for(&cfg),
            default_app_cache_root().join("custom-cache")
        );
    }

    #[test]
    fn absolute_cache_root_replaces_default_root() {
        let mut cfg = CargoPortConfig::default();
        cfg.cache.root = "/tmp/cargo-port-cache".to_string();

        assert_eq!(
            configured_app_cache_root_for(&cfg),
            AbsolutePath::from("/tmp/cargo-port-cache")
        );
    }
}
