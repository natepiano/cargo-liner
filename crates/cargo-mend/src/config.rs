use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;

const APP_NAME: &str = "cargo-mend";
const GLOBAL_CONFIG_FILE: &str = "config.toml";

// --- Diagnostics config ---

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiagnosticsConfig {
    #[serde(default = "default_true")]
    pub forbidden_pub_crate:            bool,
    #[serde(default = "default_true")]
    pub forbidden_pub_in_crate:         bool,
    #[serde(default = "default_true")]
    pub review_pub_mod:                 bool,
    #[serde(default = "default_true")]
    pub suspicious_pub:                 bool,
    #[serde(default = "default_true")]
    pub prefer_module_import:           bool,
    #[serde(default = "default_true")]
    pub inline_path_qualified_type:     bool,
    #[serde(default = "default_true")]
    pub shorten_local_crate_import:     bool,
    #[serde(default = "default_true")]
    pub wildcard_parent_pub_use:        bool,
    #[serde(default = "default_true")]
    pub internal_parent_pub_use_facade: bool,
}

const fn default_true() -> bool { true }

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            forbidden_pub_crate:            true,
            forbidden_pub_in_crate:         true,
            review_pub_mod:                 true,
            suspicious_pub:                 true,
            prefer_module_import:           true,
            inline_path_qualified_type:     true,
            shorten_local_crate_import:     true,
            wildcard_parent_pub_use:        true,
            internal_parent_pub_use_facade: true,
        }
    }
}

impl DiagnosticsConfig {
    pub fn is_enabled(&self, code: &str) -> bool {
        match code {
            "forbidden_pub_crate" => self.forbidden_pub_crate,
            "forbidden_pub_in_crate" => self.forbidden_pub_in_crate,
            "review_pub_mod" => self.review_pub_mod,
            "suspicious_pub" => self.suspicious_pub,
            "prefer_module_import" => self.prefer_module_import,
            "inline_path_qualified_type" => self.inline_path_qualified_type,
            "shorten_local_crate_import" => self.shorten_local_crate_import,
            "wildcard_parent_pub_use" => self.wildcard_parent_pub_use,
            "internal_parent_pub_use_facade" => self.internal_parent_pub_use_facade,
            _ => true,
        }
    }

    /// Merge a project-level override on top of this config.
    /// Only fields explicitly present in the project config override the global.
    pub fn merge_project(&self, project: &DiagnosticsConfig) -> Self { project.clone() }

    /// Returns an iterator of `(code, enabled)` pairs for display.
    pub fn entries(&self) -> Vec<(&'static str, bool)> {
        vec![
            ("forbidden_pub_crate", self.forbidden_pub_crate),
            ("forbidden_pub_in_crate", self.forbidden_pub_in_crate),
            ("review_pub_mod", self.review_pub_mod),
            ("suspicious_pub", self.suspicious_pub),
            ("prefer_module_import", self.prefer_module_import),
            (
                "inline_path_qualified_type",
                self.inline_path_qualified_type,
            ),
            (
                "shorten_local_crate_import",
                self.shorten_local_crate_import,
            ),
            ("wildcard_parent_pub_use", self.wildcard_parent_pub_use),
            (
                "internal_parent_pub_use_facade",
                self.internal_parent_pub_use_facade,
            ),
        ]
    }
}

// --- Global config ---

#[derive(Debug, Default, Deserialize)]
struct GlobalConfigFile {
    #[serde(default)]
    diagnostics: DiagnosticsConfig,
}

pub fn global_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_NAME).join(GLOBAL_CONFIG_FILE))
}

pub fn load_global_diagnostics() -> DiagnosticsConfig {
    let Some(path) = global_config_path() else {
        return DiagnosticsConfig::default();
    };

    if !path.exists() {
        let _ = create_default_global_config(&path);
        let Ok(contents) = fs::read_to_string(&path) else {
            return DiagnosticsConfig::default();
        };
        return toml::from_str::<GlobalConfigFile>(&contents)
            .map_or_else(|_| DiagnosticsConfig::default(), |f| f.diagnostics);
    }

    let Ok(contents) = fs::read_to_string(&path) else {
        return DiagnosticsConfig::default();
    };

    toml::from_str::<GlobalConfigFile>(&contents)
        .map_or_else(|_| DiagnosticsConfig::default(), |f| f.diagnostics)
}

const DEFAULT_GLOBAL_CONFIG_TOML: &str = r#"# cargo-mend global configuration
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
wildcard_parent_pub_use = true
internal_parent_pub_use_facade = true
"#;

fn create_default_global_config(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }
    fs::write(path, DEFAULT_GLOBAL_CONFIG_TOML)
        .with_context(|| format!("failed to write default config to {}", path.display()))?;
    Ok(())
}

// --- Project config (mend.toml) ---

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    visibility:  VisibilityConfig,
    #[serde(default)]
    diagnostics: Option<DiagnosticsConfig>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct VisibilityConfig {
    #[serde(default)]
    pub allow_pub_mod:   Vec<String>,
    #[serde(default)]
    pub allow_pub_items: Vec<String>,
}

#[derive(Debug)]
pub struct LoadedConfig {
    pub config:      VisibilityConfig,
    pub diagnostics: DiagnosticsConfig,
    pub root:        PathBuf,
    pub fingerprint: String,
}

pub fn load_config(
    manifest_dir: &Path,
    workspace_root: &Path,
    explicit: Option<&Path>,
    global_diagnostics: &DiagnosticsConfig,
) -> Result<LoadedConfig> {
    let candidates = explicit.map_or_else(
        || {
            let mut result = Vec::new();
            for root in [manifest_dir, workspace_root] {
                result.push(root.join("mend.toml"));
            }
            result
        },
        |path| vec![path.to_path_buf()],
    );

    for path in candidates {
        if path.exists() {
            let text = fs::read_to_string(&path)
                .with_context(|| format!("failed to read config {}", path.display()))?;
            let file: ConfigFile = toml::from_str(&text)
                .with_context(|| format!("failed to parse config {}", path.display()))?;
            let root = path
                .parent()
                .map_or_else(|| manifest_dir.to_path_buf(), Path::to_path_buf)
                .canonicalize()
                .with_context(|| {
                    format!("failed to canonicalize config root for {}", path.display())
                })?;
            let diagnostics = file.diagnostics.map_or_else(
                || global_diagnostics.clone(),
                |project| global_diagnostics.merge_project(&project),
            );
            return Ok(LoadedConfig {
                fingerprint: fingerprint_for(&root, &file.visibility)?,
                config: file.visibility,
                diagnostics,
                root,
            });
        }
    }

    Ok(LoadedConfig {
        fingerprint: fingerprint_for(manifest_dir, &VisibilityConfig::default())?,
        config:      VisibilityConfig::default(),
        diagnostics: global_diagnostics.clone(),
        root:        manifest_dir.to_path_buf(),
    })
}

fn fingerprint_for(root: &Path, config: &VisibilityConfig) -> Result<String> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    root.to_string_lossy().hash(&mut hasher);
    serde_json::to_string(config)
        .context("failed to serialize mend config for fingerprinting")?
        .hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_GLOBAL_CONFIG_TOML;
    use super::DiagnosticsConfig;
    use super::GlobalConfigFile;

    #[test]
    fn default_global_config_toml_parses_correctly() {
        let result: Result<GlobalConfigFile, _> = toml::from_str(DEFAULT_GLOBAL_CONFIG_TOML);
        assert!(result.is_ok(), "DEFAULT_GLOBAL_CONFIG_TOML should parse");
        let cfg = result.expect("just asserted ok").diagnostics;
        for (code, enabled) in cfg.entries() {
            assert!(enabled, "default config should have {code} enabled");
        }
    }

    #[test]
    fn is_enabled_reflects_field_values() {
        let mut cfg = DiagnosticsConfig::default();
        assert!(cfg.is_enabled("prefer_module_import"));
        cfg.prefer_module_import = false;
        assert!(!cfg.is_enabled("prefer_module_import"));
    }

    #[test]
    fn unknown_code_defaults_to_enabled() {
        let cfg = DiagnosticsConfig::default();
        assert!(cfg.is_enabled("some_future_diagnostic"));
    }

    #[test]
    fn merge_project_overrides_global() {
        let global = DiagnosticsConfig {
            prefer_module_import: false,
            ..DiagnosticsConfig::default()
        };
        let project = DiagnosticsConfig {
            prefer_module_import: true,
            suspicious_pub: false,
            ..DiagnosticsConfig::default()
        };
        let merged = global.merge_project(&project);
        assert!(merged.prefer_module_import);
        assert!(!merged.suspicious_pub);
    }

    #[test]
    fn partial_toml_uses_defaults_for_missing_fields() {
        let toml_str = r#"
[diagnostics]
prefer_module_import = false
"#;
        let cfg: GlobalConfigFile = toml::from_str(toml_str).expect("partial toml should parse");
        assert!(!cfg.diagnostics.prefer_module_import);
        assert!(cfg.diagnostics.forbidden_pub_crate);
        assert!(cfg.diagnostics.suspicious_pub);
    }
}
