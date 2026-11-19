//! Configuration for source files that generators own.

use std::num::NonZeroUsize;

use serde::Deserialize;

use super::constants::DEFAULT_GENERATED_HEADER_LINES;
use super::constants::DEFAULT_GENERATED_MARKERS;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub(crate) struct GeneratedSourcesConfig {
    pub(crate) markers:      Vec<String>,
    pub(crate) header_lines: NonZeroUsize,
}

impl Default for GeneratedSourcesConfig {
    fn default() -> Self {
        Self {
            markers:      DEFAULT_GENERATED_MARKERS
                .into_iter()
                .map(str::to_string)
                .collect(),
            header_lines: DEFAULT_GENERATED_HEADER_LINES,
        }
    }
}

/// Project keys inherit their global values when the project omits them.
#[derive(Debug, Default, Deserialize)]
pub(super) struct ProjectGeneratedSources {
    markers:      Option<Vec<String>>,
    header_lines: Option<NonZeroUsize>,
}

impl ProjectGeneratedSources {
    pub(super) fn resolve(self, global: &GeneratedSourcesConfig) -> GeneratedSourcesConfig {
        GeneratedSourcesConfig {
            markers:      self.markers.unwrap_or_else(|| global.markers.clone()),
            header_lines: self.header_lines.unwrap_or(global.header_lines),
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use tempfile::tempdir;

    use super::GeneratedSourcesConfig;
    use crate::config::global::GlobalConfig;
    use crate::config::loaded;

    #[test]
    fn project_generated_keys_override_global_keys_independently() -> Result<()> {
        let directory = tempdir()?;
        let mut global = GlobalConfig::default();
        global.generated.markers = vec!["global marker".to_string()];
        global.generated.header_lines = 9.try_into()?;
        std::fs::write(directory.path().join("mend.toml"), "")?;
        let initial = loaded::load_config(directory.path(), directory.path(), None, &global)?;
        for (text, markers, lines) in [
            ("", vec!["global marker"], 9),
            (
                "[generated]\nmarkers = ['project marker']",
                vec!["project marker"],
                9,
            ),
            ("[generated]\nheader_lines = 2", vec!["global marker"], 2),
            ("[generated]\nmarkers = []\nheader_lines = 3", vec![], 3),
        ] {
            std::fs::write(directory.path().join("mend.toml"), text)?;
            let loaded = loaded::load_config(directory.path(), directory.path(), None, &global)?;
            assert_eq!(loaded.generated.markers, markers);
            assert_eq!(loaded.generated.header_lines.get(), lines);
            assert_eq!(loaded.fingerprint, initial.fingerprint);
        }
        std::fs::write(
            directory.path().join("mend.toml"),
            "[generated]\nheader_lines = 0",
        )?;
        assert!(loaded::load_config(directory.path(), directory.path(), None, &global).is_err());
        let invalid = toml::from_str::<GeneratedSourcesConfig>("header_lines = 0");
        assert!(invalid.is_err());
        Ok(())
    }
}
