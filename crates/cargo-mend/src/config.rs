use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    visibility: VisibilityConfig,
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
    pub config: VisibilityConfig,
    pub root:   PathBuf,
}

pub fn load_config(
    manifest_dir: &Path,
    workspace_root: &Path,
    explicit: Option<&Path>,
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
                .map_or_else(|| manifest_dir.to_path_buf(), Path::to_path_buf);
            return Ok(LoadedConfig {
                config: file.visibility,
                root,
            });
        }
    }

    Ok(LoadedConfig {
        config: VisibilityConfig::default(),
        root:   manifest_dir.to_path_buf(),
    })
}
