use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use cargo_metadata::Metadata;
use cargo_metadata::MetadataCommand;
use cargo_metadata::Package;
use cargo_metadata::Target;
use cargo_metadata::TargetKind;

#[derive(Debug)]
pub(super) struct Selection {
    pub(super) manifest_path:          PathBuf,
    pub(super) manifest_dir:           PathBuf,
    pub(super) workspace_root:         PathBuf,
    pub(super) target_directory:       PathBuf,
    pub(super) analysis_root:          PathBuf,
    pub(super) is_workspace_selection: bool,
    pub(super) package_roots:          Vec<PathBuf>,
    pub(super) packages:               Vec<SelectedPackage>,
}

#[derive(Debug, Clone)]
pub(super) struct SelectedPackage {
    pub(super) name:          String,
    pub(super) manifest_path: PathBuf,
    pub(super) root:          PathBuf,
    pub(super) target:        TargetSelector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TargetSelector {
    Implicit,
    Lib,
    Bin(String),
    Example(String),
    Test(String),
    Bench(String),
}

impl TargetSelector {
    pub(super) fn cargo_args(&self) -> Vec<String> {
        match self {
            Self::Implicit => Vec::new(),
            Self::Lib => vec!["--lib".to_string()],
            Self::Bin(name) => vec!["--bin".to_string(), name.clone()],
            Self::Example(name) => vec!["--example".to_string(), name.clone()],
            Self::Test(name) => vec!["--test".to_string(), name.clone()],
            Self::Bench(name) => vec!["--bench".to_string(), name.clone()],
        }
    }
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
    let target_directory = metadata.target_directory.clone().into_std_path_buf();
    let manifest_dir = manifest_path
        .parent()
        .context("manifest path had no parent directory")?
        .to_path_buf();
    let workspace_manifest = workspace_root.join("Cargo.toml");
    let is_workspace_selection =
        manifest_path == workspace_manifest && metadata.workspace_members.len() > 1;

    let packages: Vec<SelectedPackage> = if is_workspace_selection {
        metadata
            .workspace_members
            .iter()
            .filter_map(|id| metadata.packages.iter().find(|pkg| &pkg.id == id))
            .map(selected_package_from_metadata)
            .collect::<Result<Vec<_>>>()?
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
        vec![selected_package_from_metadata(package)?]
    };
    let package_roots = packages.iter().map(|package| package.root.clone()).collect();

    let analysis_root = if is_workspace_selection {
        workspace_root.clone()
    } else {
        manifest_dir.clone()
    };

    Ok(Selection {
        manifest_path,
        manifest_dir,
        workspace_root,
        target_directory,
        analysis_root,
        is_workspace_selection,
        package_roots,
        packages,
    })
}

fn selected_package_from_metadata(package: &Package) -> Result<SelectedPackage> {
    let manifest_path = package.manifest_path.as_std_path().to_path_buf();
    let root = manifest_path
        .parent()
        .context("package manifest path had no parent directory")?
        .to_path_buf();
    Ok(SelectedPackage {
        name:          package.name.to_string(),
        manifest_path,
        root,
        target:        select_primary_target(package.targets.as_slice()),
    })
}

fn select_primary_target(targets: &[Target]) -> TargetSelector {
    if targets.len() == 1 {
        return TargetSelector::Implicit;
    }

    let select_named = |kind: TargetKind, ctor: fn(String) -> TargetSelector| {
        targets
            .iter()
            .find(|target| target.kind.iter().any(|item| *item == kind))
            .map(|target| ctor(target.name.clone()))
    };

    if targets
        .iter()
        .any(|target| {
            target
                .kind
                .iter()
                .any(|kind| *kind == TargetKind::Lib || *kind == TargetKind::ProcMacro)
        })
    {
        return TargetSelector::Lib;
    }

    select_named(TargetKind::Bin, TargetSelector::Bin)
        .or_else(|| select_named(TargetKind::Example, TargetSelector::Example))
        .or_else(|| select_named(TargetKind::Test, TargetSelector::Test))
        .or_else(|| select_named(TargetKind::Bench, TargetSelector::Bench))
        .unwrap_or(TargetSelector::Implicit)
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

#[cfg(test)]
mod tests {
    use super::TargetSelector;

    #[test]
    fn target_selector_cargo_args_for_lib() {
        assert_eq!(TargetSelector::Lib.cargo_args(), vec!["--lib"]);
    }

    #[test]
    fn target_selector_cargo_args_for_bin() {
        assert_eq!(
            TargetSelector::Bin("demo".to_string()).cargo_args(),
            vec!["--bin", "demo"]
        );
    }

    #[test]
    fn target_selector_cargo_args_for_implicit_target() {
        assert!(TargetSelector::Implicit.cargo_args().is_empty());
    }
}
