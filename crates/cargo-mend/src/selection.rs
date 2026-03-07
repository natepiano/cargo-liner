use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use cargo_metadata::Metadata;
use cargo_metadata::MetadataCommand;

#[derive(Debug)]
pub(super) struct Selection {
    pub(super) manifest_path:          PathBuf,
    pub(super) manifest_dir:           PathBuf,
    pub(super) workspace_root:         PathBuf,
    pub(super) analysis_root:          PathBuf,
    pub(super) is_workspace_selection: bool,
    pub(super) package_roots:          Vec<PathBuf>,
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
    let is_workspace_selection =
        manifest_path == workspace_manifest && metadata.workspace_members.len() > 1;

    let package_roots = if is_workspace_selection {
        metadata
            .workspace_members
            .iter()
            .filter_map(|id| metadata.packages.iter().find(|pkg| &pkg.id == id))
            .filter_map(|pkg| {
                pkg.manifest_path
                    .as_std_path()
                    .parent()
                    .map(Path::to_path_buf)
            })
            .collect()
    } else {
        let package = metadata
            .packages
            .iter()
            .find(|pkg| pkg.manifest_path.as_std_path() == manifest_path)
            .with_context(|| {
                format!(
                    "manifest {} not found in cargo metadata",
                    manifest_path.display()
                )
            })?;
        vec![
            package
                .manifest_path
                .as_std_path()
                .parent()
                .context("package manifest path had no parent directory")?
                .to_path_buf(),
        ]
    };

    let analysis_root = if is_workspace_selection {
        workspace_root.clone()
    } else {
        manifest_dir.clone()
    };

    Ok(Selection {
        manifest_path,
        manifest_dir,
        workspace_root,
        analysis_root,
        is_workspace_selection,
        package_roots,
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
