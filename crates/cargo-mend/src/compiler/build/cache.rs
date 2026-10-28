use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

use super::stdout::AnalysisUnit;
use crate::compiler::persistence::ReportCacheState;
use crate::compiler::source_cache;
use crate::selection::Selection;

/// Cargo freshness alone does not establish that mend examined every selected unit.
pub(super) enum AnalysisCache {
    Compatible,
    RebuildRequired(Vec<PathBuf>),
    /// Cargo supplied no analyzable selected unit; removing metadata cannot repair this.
    Unrepairable,
}

impl AnalysisCache {
    pub(super) fn inspect(units: &[AnalysisUnit], selection: &Selection, config: &str) -> Self {
        let mut selected = 0;
        let mut rebuild = Vec::new();
        for unit in units.iter().filter(|unit| {
            selection.packages.iter().any(|package| {
                package.id == unit.package_id
                    && source_cache::analysis_source_root_for(&unit.crate_root, &package.root)
                        .is_some()
            })
        }) {
            selected += 1;
            match ReportCacheState::read(&unit.report_path(), config) {
                ReportCacheState::Compatible => {},
                ReportCacheState::AnotherMendBuild
                | ReportCacheState::InputsChanged
                | ReportCacheState::Missing
                | ReportCacheState::Unreadable => rebuild.push(unit.metadata_path.clone()),
            }
        }
        if selected == 0 {
            Self::Unrepairable
        } else if rebuild.is_empty() {
            Self::Compatible
        } else {
            Self::RebuildRequired(rebuild)
        }
    }
}

/// Missing compiler outputs make Cargo rebuild only the affected units and their dependents.
pub(super) fn invalidate_metadata(paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        match fs::remove_file(path) {
            Ok(()) => {},
            Err(error) if error.kind() == ErrorKind::NotFound => {},
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to invalidate {}", path.display()));
            },
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use serde_json::json;
    use tempfile::tempdir;

    use super::AnalysisCache;
    use super::invalidate_metadata;
    use crate::compiler::build::stdout::AnalysisUnit;
    use crate::compiler::constants::FINDINGS_SCHEMA_VERSION;
    use crate::compiler::settings;
    use crate::selection::PackageMetadata;
    use crate::selection::Selection;
    use crate::selection::SelectionScope;

    fn selected_package(root: &Path) -> Selection {
        Selection {
            manifest_path:    root.join("Cargo.toml"),
            manifest_dir:     root.into(),
            workspace_root:   root.into(),
            target_directory: root.join("target"),
            analysis_root:    root.into(),
            scope:            SelectionScope::SinglePackage,
            package_roots:    vec![root.into()],
            packages:         vec![PackageMetadata {
                id:            "selected".into(),
                manifest_path: root.join("Cargo.toml"),
                root:          root.into(),
                targets:       Vec::new(),
            }],
        }
    }

    #[test]
    fn cache_repair_invalidates_only_unusable_selected_units() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let root = temp.path();
        fs::create_dir(root.join("src"))?;
        let crate_root = root.join("src/lib.rs");
        fs::write(&crate_root, "pub fn examined() {}")?;
        let units: Vec<_> = ["good", "old", "missing", "dependency"]
            .into_iter()
            .map(|name| AnalysisUnit {
                package_id:    if name == "dependency" {
                    "other"
                } else {
                    "selected"
                }
                .into(),
                metadata_path: root.join(format!("{name}.rmeta")),
                crate_root:    crate_root.clone(),
            })
            .collect();
        for unit in &units {
            fs::write(&unit.metadata_path, "metadata")?;
        }
        let mut report = json!({
            "version": FINDINGS_SCHEMA_VERSION,
            "analysis_fingerprint": settings::current_analysis_fingerprint(),
            "config_fingerprint": "config", "package_root": root,
            "crate_root_file": crate_root, "findings": [],
        });
        fs::write(units[0].report_path(), serde_json::to_vec(&report)?)?;
        report["analysis_fingerprint"] = json!("previous-build");
        fs::write(units[1].report_path(), serde_json::to_vec(&report)?)?;
        let selection = selected_package(root);
        let AnalysisCache::RebuildRequired(paths) =
            AnalysisCache::inspect(&units, &selection, "config")
        else {
            anyhow::bail!("a compatible unit must not hide unusable selected units");
        };
        assert_eq!(
            paths,
            [
                units[1].metadata_path.clone(),
                units[2].metadata_path.clone()
            ]
        );
        invalidate_metadata(&paths)?;
        assert!(units[0].metadata_path.exists());
        assert!(units[3].metadata_path.exists());
        assert!(paths.iter().all(|path| !path.exists()));
        assert!(matches!(
            AnalysisCache::inspect(&units[3..], &selection, "config"),
            AnalysisCache::Unrepairable
        ));
        Ok(())
    }
}
