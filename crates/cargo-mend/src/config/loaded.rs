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

use super::constants::CONFIG_FILE_NAME;
use super::diagnostics_config::DiagnosticsConfig;
use super::global::GlobalConfig;
use super::prelude_pub_mod::PreludePubMod;

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
    #[serde(default, rename = "allow_prelude_pub_mod")]
    pub(crate) prelude_pub_mod: PreludePubMod,
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
    global: &GlobalConfig,
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
                || global.diagnostics.clone(),
                |project| global.diagnostics.merge_project(&project),
            );
            // The prelude exemption is a global-only switch; stamp it onto the
            // resolved visibility config before fingerprinting so the cache
            // invalidates when the global toggle changes.
            let mut visibility_config = config_file.visibility_config;
            visibility_config.prelude_pub_mod = global.prelude_pub_mod;
            return Ok(LoadedConfig {
                fingerprint: fingerprint_for(&root, &visibility_config)?,
                visibility_config,
                diagnostics_config,
                root,
            });
        }
    }

    let visibility_config = VisibilityConfig {
        prelude_pub_mod: global.prelude_pub_mod,
        ..VisibilityConfig::default()
    };
    Ok(LoadedConfig {
        fingerprint: fingerprint_for(manifest_dir, &visibility_config)?,
        visibility_config,
        diagnostics_config: global.diagnostics.clone(),
        root: manifest_dir.to_path_buf(),
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
