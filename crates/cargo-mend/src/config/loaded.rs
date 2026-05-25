use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;
use serde_json::to_string;
use toml::from_str;

use super::diagnostics_config::DiagnosticsConfig;

// config file
const CONFIG_FILE_NAME: &str = "mend.toml";

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
    pub(crate) allow_pub_mod:   Vec<String>,
    #[serde(default)]
    pub(crate) allow_pub_items: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct LoadedConfig {
    pub(crate) visibility_config:  VisibilityConfig,
    pub(crate) diagnostics_config: DiagnosticsConfig,
    pub(crate) root:               PathBuf,
    pub(crate) fingerprint:        String,
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
                result.push(root.join(CONFIG_FILE_NAME));
            }
            result
        },
        |path| vec![path.to_path_buf()],
    );

    for path in candidates {
        if path.exists() {
            let text = fs::read_to_string(&path)
                .with_context(|| format!("failed to read config {}", path.display()))?;
            let config_file: ConfigFile = from_str(&text)
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
    to_string(config)
        .context("failed to serialize mend config for fingerprinting")?
        .hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}
