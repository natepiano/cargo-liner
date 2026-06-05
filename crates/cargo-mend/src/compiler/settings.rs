use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde_json::from_str;

use super::constants::BUILD_ID_FALLBACK;
use super::constants::CONFIG_FINGERPRINT_ENV;
use super::constants::CONFIG_JSON_ENV;
use super::constants::CONFIG_ROOT_ENV;
use super::constants::FINDINGS_DIR_ENV;
use super::constants::GIT_HASH_FALLBACK;
use super::constants::PACKAGE_ROOT_ENV;
use super::constants::SCOPE_FINGERPRINT_ENV;
use crate::config::VisibilityConfig;

pub(super) fn current_analysis_fingerprint() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let git_hash = option_env!("MEND_GIT_HASH").unwrap_or(GIT_HASH_FALLBACK);
    let build_id = option_env!("MEND_BUILD_ID").unwrap_or(BUILD_ID_FALLBACK);
    format!("{version}+{git_hash}+{build_id}")
}

#[derive(Debug, Clone)]
pub(super) struct DriverSettings {
    pub config_root:          PathBuf,
    pub visibility_config:    VisibilityConfig,
    pub config_fingerprint:   String,
    pub analysis_fingerprint: String,
    pub scope_fingerprint:    String,
    pub findings_dir:         PathBuf,
    pub package_root:         PathBuf,
}

impl DriverSettings {
    pub(super) fn from_env() -> Result<Self> {
        let config_root = PathBuf::from(
            env::var_os(CONFIG_ROOT_ENV)
                .with_context(|| format!("missing {CONFIG_ROOT_ENV} for compiler driver"))?,
        );
        let visibility_config = from_str(
            &env::var(CONFIG_JSON_ENV)
                .with_context(|| format!("missing {CONFIG_JSON_ENV} for compiler driver"))?,
        )
        .with_context(|| format!("failed to parse {CONFIG_JSON_ENV}"))?;
        let config_fingerprint = env::var(CONFIG_FINGERPRINT_ENV)
            .with_context(|| format!("missing {CONFIG_FINGERPRINT_ENV}"))?;
        let findings_dir = PathBuf::from(
            env::var_os(FINDINGS_DIR_ENV)
                .with_context(|| format!("missing {FINDINGS_DIR_ENV} for compiler driver"))?,
        );
        let scope_fingerprint = env::var(SCOPE_FINGERPRINT_ENV)
            .with_context(|| format!("missing {SCOPE_FINGERPRINT_ENV}"))?;
        let package_root = PathBuf::from(
            env::var_os(PACKAGE_ROOT_ENV)
                .with_context(|| format!("missing {PACKAGE_ROOT_ENV} for compiler driver"))?,
        );

        Ok(Self {
            config_root,
            visibility_config,
            config_fingerprint,
            analysis_fingerprint: current_analysis_fingerprint(),
            scope_fingerprint,
            findings_dir,
            package_root,
        })
    }
}

pub(super) fn config_relative_path(file_path: &Path, config_root: &Path) -> Option<String> {
    file_path
        .strip_prefix(config_root)
        .ok()
        .map(normalize_relative_path)
        .or_else(|| {
            let canonical_file = fs::canonicalize(file_path).ok()?;
            let canonical_root = fs::canonicalize(config_root).ok()?;
            canonical_file
                .strip_prefix(canonical_root)
                .ok()
                .map(normalize_relative_path)
        })
}

pub(super) fn config_relative_path_for_settings(
    file_path: &Path,
    settings: &DriverSettings,
) -> Option<String> {
    if file_path.is_relative() {
        let workspace_relative = normalize_relative_path(file_path);
        if settings.config_root.join(file_path).exists() {
            return Some(workspace_relative);
        }

        let package_relative = settings.package_root.join(file_path);
        return config_relative_path(&package_relative, &settings.config_root)
            .or(Some(workspace_relative));
    }

    config_relative_path(file_path, &settings.config_root)
}

fn normalize_relative_path(path: &Path) -> String { path.to_string_lossy().replace('\\', "/") }

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    use anyhow::Result;
    use anyhow::anyhow;
    use tempfile::tempdir;

    use crate::config::VisibilityConfig;

    #[test]
    fn config_relative_path_handles_nested_workspace_paths() -> Result<()> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let workspace_root = env::temp_dir().join(format!("mend-config-root-test-{unique}"));
        let file_path = workspace_root.join("mcp/src/brp_tools/tools/mod.rs");
        let parent = file_path
            .parent()
            .ok_or_else(|| anyhow!("test path must have a parent directory"))?;
        fs::create_dir_all(parent)?;
        fs::write(&file_path, "pub mod world_query;\n")?;

        assert_eq!(
            super::config_relative_path(&file_path, &workspace_root).as_deref(),
            Some("mcp/src/brp_tools/tools/mod.rs")
        );

        Ok(())
    }

    #[test]
    fn config_relative_path_for_settings_handles_package_relative_workspace_paths() {
        let settings = super::DriverSettings {
            config_root:          PathBuf::from("/workspace/root"),
            visibility_config:    VisibilityConfig::default(),
            config_fingerprint:   "test".to_string(),
            scope_fingerprint:    "scope".to_string(),
            findings_dir:         PathBuf::from("/workspace/root/target/mend-findings"),
            package_root:         PathBuf::from("/workspace/root/mcp"),
            analysis_fingerprint: super::current_analysis_fingerprint(),
        };
        let file_path = PathBuf::from("src/brp_tools/tools/mod.rs");

        assert_eq!(
            super::config_relative_path_for_settings(&file_path, &settings).as_deref(),
            Some("mcp/src/brp_tools/tools/mod.rs")
        );
    }

    #[test]
    fn config_relative_path_for_settings_handles_workspace_relative_paths() -> Result<()> {
        let temp = tempdir()?;
        let config_root = temp.path().join("workspace");
        let package_root = config_root.join("mcp");
        fs::create_dir_all(package_root.join("src/brp_tools/tools"))?;
        fs::write(
            package_root.join("src/brp_tools/tools/mod.rs"),
            "pub mod child;\n",
        )?;
        let settings = super::DriverSettings {
            config_root,
            visibility_config: VisibilityConfig::default(),
            config_fingerprint: "test".to_string(),
            scope_fingerprint: "scope".to_string(),
            findings_dir: temp.path().join("workspace/target/mend-findings"),
            package_root,
            analysis_fingerprint: super::current_analysis_fingerprint(),
        };
        let file_path = PathBuf::from("mcp/src/brp_tools/tools/mod.rs");

        assert_eq!(
            super::config_relative_path_for_settings(&file_path, &settings).as_deref(),
            Some("mcp/src/brp_tools/tools/mod.rs")
        );

        Ok(())
    }
}
