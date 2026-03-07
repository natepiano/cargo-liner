use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use cargo_metadata::Metadata;
use cargo_metadata::MetadataCommand;
use cargo_metadata::Package;

#[derive(Debug)]
pub(super) struct Selection {
    pub(super) manifest_dir:   PathBuf,
    pub(super) workspace_root: PathBuf,
    pub(super) analysis_root:  PathBuf,
    pub(super) packages:       Vec<Package>,
}

pub(super) fn resolve_cargo_selection(explicit_manifest_path: Option<&Path>) -> Result<Selection> {
    let manifest_path = match explicit_manifest_path {
        Some(path) => path
            .canonicalize()
            .with_context(|| format!("failed to canonicalize {}", path.display()))?,
        None => find_nearest_manifest(&std::env::current_dir()?)?,
    };

    let metadata = cargo_metadata_for(&manifest_path)?;
    let workspace_root = metadata.workspace_root.clone().into_std_path_buf();
    let manifest_dir = manifest_path
        .parent()
        .context("manifest path had no parent directory")?
        .to_path_buf();
    let workspace_manifest = workspace_root.join("Cargo.toml");

    let packages = if manifest_path == workspace_manifest {
        select_packages(&metadata, &metadata.workspace_members)?
    } else {
        let package = metadata
            .packages
            .iter()
            .find(|pkg| pkg.manifest_path.as_std_path() == manifest_path)
            .cloned()
            .with_context(|| {
                format!(
                    "manifest {} not found in cargo metadata",
                    manifest_path.display()
                )
            })?;
        vec![package]
    };

    let analysis_root = if manifest_path == workspace_manifest {
        workspace_root.clone()
    } else {
        manifest_dir.clone()
    };

    Ok(Selection {
        manifest_dir,
        workspace_root,
        analysis_root,
        packages,
    })
}

fn cargo_metadata_for(manifest_path: &Path) -> Result<Metadata> {
    let mut command = MetadataCommand::new();
    command.no_deps();
    command.manifest_path(manifest_path);
    command.exec().context("failed to run cargo metadata")
}

fn find_nearest_manifest(start: &Path) -> Result<PathBuf> {
    for dir in start.ancestors() {
        let candidate = dir.join("Cargo.toml");
        if candidate.is_file() {
            return candidate
                .canonicalize()
                .with_context(|| format!("failed to canonicalize {}", candidate.display()));
        }
    }

    bail!("could not find Cargo.toml in current directory or any parent")
}

fn select_packages(metadata: &Metadata, ids: &[cargo_metadata::PackageId]) -> Result<Vec<Package>> {
    let id_set: BTreeSet<_> = ids.iter().collect();
    let mut packages: Vec<_> = metadata
        .packages
        .iter()
        .filter(|pkg| id_set.contains(&pkg.id))
        .cloned()
        .collect();
    packages.sort_by(|a, b| a.manifest_path.cmp(&b.manifest_path));
    Ok(packages)
}
