mod cli;
mod run_mode;

use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
pub(crate) use cli::BuildInfoMode;
pub(crate) use cli::CargoCheckCli;
pub(crate) use cli::FixExecution;
pub(crate) use cli::TargetSelection;
pub(crate) use cli::WarningPolicy;
pub(crate) use cli::WorkspaceSelection;
pub(crate) use cli::parse;
pub(crate) use run_mode::FixKind;
pub(crate) use run_mode::OperationIntent;
pub(crate) use run_mode::OperationMode;
use serde::Deserialize;
use serde::Serialize;

// app/config file
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
";

// --- Diagnostic codes ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiagnosticCode {
    ForbiddenPubCrate,
    ForbiddenPubInCrate,
    ReviewPubMod,
    SuspiciousPub,
    PreferModuleImport,
    InlinePathQualifiedType,
    ShortenLocalCrateImport,
    ReplaceDeepSuperImport,
    WildcardParentPubUse,
    InternalParentPubUseFacade,
    NarrowToPubCrate,
    FieldVisibilityWiderThanType,
}

impl DiagnosticCode {
    pub(crate) const ALL: &[Self] = &[
        Self::ForbiddenPubCrate,
        Self::ForbiddenPubInCrate,
        Self::ReviewPubMod,
        Self::SuspiciousPub,
        Self::PreferModuleImport,
        Self::InlinePathQualifiedType,
        Self::ShortenLocalCrateImport,
        Self::ReplaceDeepSuperImport,
        Self::WildcardParentPubUse,
        Self::InternalParentPubUseFacade,
        Self::NarrowToPubCrate,
        Self::FieldVisibilityWiderThanType,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ForbiddenPubCrate => "forbidden_pub_crate",
            Self::ForbiddenPubInCrate => "forbidden_pub_in_crate",
            Self::ReviewPubMod => "review_pub_mod",
            Self::SuspiciousPub => "suspicious_pub",
            Self::PreferModuleImport => "prefer_module_import",
            Self::InlinePathQualifiedType => "inline_path_qualified_type",
            Self::ShortenLocalCrateImport => "shorten_local_crate_import",
            Self::ReplaceDeepSuperImport => "replace_deep_super_import",
            Self::WildcardParentPubUse => "wildcard_parent_pub_use",
            Self::InternalParentPubUseFacade => "internal_parent_pub_use_facade",
            Self::NarrowToPubCrate => "narrow_to_pub_crate",
            Self::FieldVisibilityWiderThanType => "field_visibility_wider_than_type",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiagnosticStatus {
    Enabled,
    Disabled,
}

impl DiagnosticStatus {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }
}

impl From<bool> for DiagnosticStatus {
    fn from(value: bool) -> Self { if value { Self::Enabled } else { Self::Disabled } }
}

// --- Diagnostics config ---

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct DiagnosticsConfig {
    #[serde(flatten)]
    rules: BTreeMap<DiagnosticCode, bool>,
}

impl DiagnosticsConfig {
    pub(crate) fn is_enabled(&self, code: DiagnosticCode) -> DiagnosticStatus {
        self.rules
            .get(&code)
            .copied()
            .map_or(DiagnosticStatus::Enabled, DiagnosticStatus::from)
    }

    pub(crate) fn entries(&self) -> Vec<(DiagnosticCode, DiagnosticStatus)> {
        DiagnosticCode::ALL
            .iter()
            .map(|code| (*code, self.is_enabled(*code)))
            .collect()
    }

    pub(crate) fn merge_project(&self, project: &Self) -> Self {
        let mut rules = self.rules.clone();
        for (code, enabled) in &project.rules {
            rules.insert(*code, *enabled);
        }
        Self { rules }
    }
}

// --- Global config ---

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

// --- Project config (mend.toml) ---

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default, rename = "visibility")]
    visibility_config:  VisibilityConfig,
    #[serde(default, rename = "diagnostics")]
    diagnostics_config: Option<DiagnosticsConfig>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub(crate) struct VisibilityConfig {
    #[serde(default)]
    pub allow_pub_mod:   Vec<String>,
    #[serde(default)]
    pub allow_pub_items: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct LoadedConfig {
    pub visibility_config:  VisibilityConfig,
    pub diagnostics_config: DiagnosticsConfig,
    pub root:               PathBuf,
    pub fingerprint:        String,
}

pub(crate) fn load_config(
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
            let config_file: ConfigFile = toml::from_str(&text)
                .with_context(|| format!("failed to parse config {}", path.display()))?;
            let root = path
                .parent()
                .map_or_else(|| manifest_dir.to_path_buf(), Path::to_path_buf)
                .canonicalize()
                .with_context(|| {
                    format!("failed to canonicalize config root for {}", path.display())
                })?;
            let diagnostics_config = config_file.diagnostics_config.map_or_else(
                || global_diagnostics.clone(),
                |project| global_diagnostics.merge_project(&project),
            );
            return Ok(LoadedConfig {
                fingerprint: fingerprint_for(&root, &config_file.visibility_config)?,
                visibility_config: config_file.visibility_config,
                diagnostics_config,
                root,
            });
        }
    }

    Ok(LoadedConfig {
        fingerprint:        fingerprint_for(manifest_dir, &VisibilityConfig::default())?,
        visibility_config:  VisibilityConfig::default(),
        diagnostics_config: global_diagnostics.clone(),
        root:               manifest_dir.to_path_buf(),
    })
}

fn fingerprint_for(root: &Path, config: &VisibilityConfig) -> Result<String> {
    let mut hasher = DefaultHasher::new();
    root.to_string_lossy().hash(&mut hasher);
    serde_json::to_string(config)
        .context("failed to serialize mend config for fingerprinting")?
        .hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::DEFAULT_GLOBAL_CONFIG_TOML;
    use super::DiagnosticCode;
    use super::DiagnosticStatus;
    use super::DiagnosticsConfig;
    use super::GlobalConfigFile;

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
    fn is_enabled_reflects_config_values() {
        let mut diagnostics_config = DiagnosticsConfig::default();
        assert_eq!(
            diagnostics_config.is_enabled(DiagnosticCode::PreferModuleImport),
            DiagnosticStatus::Enabled
        );
        diagnostics_config
            .rules
            .insert(DiagnosticCode::PreferModuleImport, false);
        assert_eq!(
            diagnostics_config.is_enabled(DiagnosticCode::PreferModuleImport),
            DiagnosticStatus::Disabled
        );
    }

    #[test]
    fn missing_code_defaults_to_enabled() {
        let diagnostics_config = DiagnosticsConfig::default();
        assert_eq!(
            diagnostics_config.is_enabled(DiagnosticCode::ForbiddenPubCrate),
            DiagnosticStatus::Enabled
        );
    }

    #[test]
    fn merge_project_overrides_global() {
        let mut global = DiagnosticsConfig::default();
        global
            .rules
            .insert(DiagnosticCode::PreferModuleImport, false);

        let mut project = DiagnosticsConfig::default();
        project
            .rules
            .insert(DiagnosticCode::PreferModuleImport, true);
        project.rules.insert(DiagnosticCode::SuspiciousPub, false);

        let merged = global.merge_project(&project);
        assert_eq!(
            merged.is_enabled(DiagnosticCode::PreferModuleImport),
            DiagnosticStatus::Enabled
        );
        assert_eq!(
            merged.is_enabled(DiagnosticCode::SuspiciousPub),
            DiagnosticStatus::Disabled
        );
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
