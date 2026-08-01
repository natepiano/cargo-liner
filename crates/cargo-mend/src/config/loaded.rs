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
use super::pub_in_path::PubInPath;
use crate::constants::FINGERPRINT_HEX_WIDTH;

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default, rename = "visibility")]
    visibility_config:  ProjectVisibilityConfig,
    #[serde(default, rename = "diagnostics")]
    diagnostics_config: Option<DiagnosticsConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct ProjectVisibilityConfig {
    #[serde(default)]
    allow_pub_mod:   Vec<String>,
    #[serde(default)]
    allow_pub_items: Vec<String>,
    #[serde(default, rename = "allow_prelude_pub_mod")]
    prelude_pub_mod: Option<PreludePubMod>,
    #[serde(default)]
    pub_in_path:     Option<PubInPath>,
}

impl ProjectVisibilityConfig {
    fn resolve(self, global: &GlobalConfig) -> VisibilityConfig {
        VisibilityConfig {
            allow_pub_mod:   self.allow_pub_mod,
            allow_pub_items: self.allow_pub_items,
            prelude_pub_mod: self.prelude_pub_mod.unwrap_or(global.prelude_pub_mod),
            pub_in_path:     self.pub_in_path.unwrap_or(global.pub_in_path),
        }
    }
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub(crate) struct VisibilityConfig {
    #[serde(default)]
    pub(crate) allow_pub_mod:   Vec<String>,
    #[serde(default)]
    pub(crate) allow_pub_items: Vec<String>,
    #[serde(default, rename = "allow_prelude_pub_mod")]
    pub(crate) prelude_pub_mod: PreludePubMod,
    #[serde(default)]
    pub(crate) pub_in_path:     PubInPath,
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
            let visibility_config = config_file.visibility_config.resolve(global);
            return Ok(LoadedConfig {
                fingerprint: fingerprint_for(&root, &visibility_config)?,
                visibility_config,
                diagnostics_config,
                root,
            });
        }
    }

    let visibility_config = ProjectVisibilityConfig::default().resolve(global);
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
    Ok(format!(
        "{:0width$x}",
        hasher.finish(),
        width = FINGERPRINT_HEX_WIDTH
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;
    use tempfile::tempdir;

    use super::GlobalConfig;
    use super::PreludePubMod;
    use super::PubInPath;
    use super::VisibilityConfig;
    use super::fingerprint_for;
    use super::load_config;
    use crate::config::DiagnosticsConfig;

    fn global_config(pub_in_path: PubInPath, prelude_pub_mod: PreludePubMod) -> GlobalConfig {
        GlobalConfig {
            diagnostics: DiagnosticsConfig::default(),
            prelude_pub_mod,
            pub_in_path,
        }
    }

    #[test]
    fn project_pub_in_path_overrides_global() -> Result<()> {
        let temp = tempdir()?;
        fs::write(
            temp.path().join("mend.toml"),
            "[visibility]\npub_in_path = \"required\"\n",
        )?;
        let global = global_config(PubInPath::Forbidden, PreludePubMod::Allowed);

        let loaded_config = load_config(temp.path(), temp.path(), None, &global)?;

        assert_eq!(
            loaded_config.visibility_config.pub_in_path,
            PubInPath::Required
        );
        Ok(())
    }

    #[test]
    fn absent_project_pub_in_path_inherits_global() -> Result<()> {
        let temp = tempdir()?;
        fs::write(
            temp.path().join("mend.toml"),
            "[visibility]\nallow_pub_mod = []\n",
        )?;
        let global = global_config(PubInPath::Forbidden, PreludePubMod::Allowed);

        let loaded_config = load_config(temp.path(), temp.path(), None, &global)?;

        assert_eq!(
            loaded_config.visibility_config.pub_in_path,
            PubInPath::Forbidden
        );
        Ok(())
    }

    #[test]
    fn absent_project_and_global_pub_in_path_uses_permitted() -> Result<()> {
        let temp = tempdir()?;

        let loaded_config = load_config(temp.path(), temp.path(), None, &GlobalConfig::default())?;

        assert_eq!(
            loaded_config.visibility_config.pub_in_path,
            PubInPath::Permitted
        );
        Ok(())
    }

    #[test]
    fn project_prelude_pub_mod_overrides_global() -> Result<()> {
        let temp = tempdir()?;
        fs::write(
            temp.path().join("mend.toml"),
            "[visibility]\nallow_prelude_pub_mod = false\n",
        )?;
        let global = global_config(PubInPath::Permitted, PreludePubMod::Allowed);

        let loaded_config = load_config(temp.path(), temp.path(), None, &global)?;

        assert_eq!(
            loaded_config.visibility_config.prelude_pub_mod,
            PreludePubMod::Reviewed
        );
        Ok(())
    }

    #[test]
    fn resolved_pub_in_path_changes_fingerprint() -> Result<()> {
        let root = tempdir()?;
        let forbidden = VisibilityConfig {
            pub_in_path: PubInPath::Forbidden,
            ..VisibilityConfig::default()
        };
        let permitted = VisibilityConfig::default();

        assert_ne!(
            fingerprint_for(root.path(), &forbidden)?,
            fingerprint_for(root.path(), &permitted)?
        );
        Ok(())
    }
}
