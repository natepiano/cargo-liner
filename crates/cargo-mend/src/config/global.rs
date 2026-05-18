use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;

use super::diagnostics_config::DiagnosticsConfig;

pub(crate) const APP_NAME: &str = "cargo-mend";
pub(crate) const GLOBAL_CONFIG_FILE: &str = "config.toml";
pub(crate) const DEFAULT_GLOBAL_CONFIG_TOML: &str = r"# cargo-mend global configuration
# See https://github.com/natepiano/cargo-mend#diagnostics for details on each rule.
# Per-project overrides go in mend.toml at your project or workspace root.

[diagnostics]
forbidden_pub_crate = true
forbidden_pub_in_crate = true
review_pub_mod = true
suspicious_pub = true
prefer_module_import = true
inline_path_qualified_type = true
shorten_local_crate_import = true
replace_deep_super_import = true
wildcard_parent_pub_use = true
internal_parent_pub_use_facade = true
narrow_to_pub_crate = true
field_visibility_wider_than_type = true
imports_at_top = true
";

#[derive(Debug, Default, Deserialize)]
struct GlobalConfigFile {
    #[serde(default, rename = "diagnostics")]
    diagnostics_config: DiagnosticsConfig,
}

pub(crate) fn global_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME).join(GLOBAL_CONFIG_FILE))
}

pub(crate) fn load_global_diagnostics() -> DiagnosticsConfig {
    let Some(path) = global_config_path() else {
        return DiagnosticsConfig::default();
    };

    if !path.exists() {
        let _ = create_default_global_config(&path);
        let Ok(contents) = fs::read_to_string(&path) else {
            return DiagnosticsConfig::default();
        };
        return toml::from_str::<GlobalConfigFile>(&contents).map_or_else(
            |_| DiagnosticsConfig::default(),
            |file| file.diagnostics_config,
        );
    }

    let Ok(contents) = fs::read_to_string(&path) else {
        return DiagnosticsConfig::default();
    };

    toml::from_str::<GlobalConfigFile>(&contents).map_or_else(
        |_| DiagnosticsConfig::default(),
        |file| file.diagnostics_config,
    )
}

fn create_default_global_config(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }
    fs::write(path, DEFAULT_GLOBAL_CONFIG_TOML)
        .with_context(|| format!("failed to write default config to {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::DEFAULT_GLOBAL_CONFIG_TOML;
    use super::GlobalConfigFile;
    use crate::config::DiagnosticCode;
    use crate::config::DiagnosticStatus;

    #[test]
    fn default_global_config_toml_parses_correctly() {
        let result: Result<GlobalConfigFile, _> = toml::from_str(DEFAULT_GLOBAL_CONFIG_TOML);
        assert!(result.is_ok(), "DEFAULT_GLOBAL_CONFIG_TOML should parse");
        let global_config_file = result.unwrap();
        for (code, enabled) in global_config_file.diagnostics_config.entries() {
            assert!(
                matches!(enabled, DiagnosticStatus::Enabled),
                "default config should have {} enabled",
                code.as_str()
            );
        }
    }

    #[test]
    fn partial_toml_uses_defaults_for_missing_fields() {
        let toml_str = r"
[diagnostics]
prefer_module_import = false
";
        let result: Result<GlobalConfigFile, _> = toml::from_str(toml_str);
        assert!(result.is_ok(), "partial toml should parse");
        let global_config_file = result.unwrap();
        assert!(matches!(
            global_config_file
                .diagnostics_config
                .is_enabled(DiagnosticCode::PreferModuleImport),
            DiagnosticStatus::Disabled
        ));
        assert!(matches!(
            global_config_file
                .diagnostics_config
                .is_enabled(DiagnosticCode::ForbiddenPubCrate),
            DiagnosticStatus::Enabled
        ));
        assert!(matches!(
            global_config_file
                .diagnostics_config
                .is_enabled(DiagnosticCode::SuspiciousPub),
            DiagnosticStatus::Enabled
        ));
    }
}
