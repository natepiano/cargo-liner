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
pub struct Selection {
    pub manifest_path:      PathBuf,
    pub manifest_dir:       PathBuf,
    pub workspace_root:     PathBuf,
    pub target_directory:   PathBuf,
    pub analysis_root:      PathBuf,
    pub workspace_selected: bool,
    pub package_roots:      Vec<PathBuf>,
    pub packages:           Vec<SelectedPackage>,
}

#[derive(Debug, Clone)]
pub struct SelectedPackage {
    pub name:          String,
    pub manifest_path: PathBuf,
    pub root:          PathBuf,
    pub target:        TargetSelector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetSelector {
    Implicit,
    Lib,
    Bin(String),
    Example(String),
    Test(String),
    Bench(String),
}

impl TargetSelector {
    pub fn cargo_args(&self) -> Vec<String> {
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

pub fn resolve_cargo_selection(explicit_manifest_path: Option<&Path>) -> Result<Selection> {
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
    let workspace_selected =
        manifest_path == workspace_manifest && metadata.workspace_members.len() > 1;

    let packages: Vec<SelectedPackage> = if workspace_selected {
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
    let package_roots = packages
        .iter()
        .map(|package| package.root.clone())
        .collect();

    let analysis_root = if workspace_selected {
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
        workspace_selected,
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
        name: package.name.to_string(),
        manifest_path,
        root,
        target: select_primary_target(package.targets.as_slice()),
    })
}

fn select_primary_target(targets: &[Target]) -> TargetSelector {
    if targets.len() == 1 {
        let target = &targets[0];
        if target
            .kind
            .iter()
            .any(|kind| *kind == TargetKind::Lib || *kind == TargetKind::ProcMacro)
        {
            return TargetSelector::Lib;
        }
        if target.kind.contains(&TargetKind::Bin) {
            return TargetSelector::Bin(target.name.clone());
        }
        if target.kind.contains(&TargetKind::Example) {
            return TargetSelector::Example(target.name.clone());
        }
        if target.kind.contains(&TargetKind::Test) {
            return TargetSelector::Test(target.name.clone());
        }
        if target.kind.contains(&TargetKind::Bench) {
            return TargetSelector::Bench(target.name.clone());
        }

        return TargetSelector::Implicit;
    }

    let select_named = |kind: TargetKind, ctor: fn(String) -> TargetSelector| {
        targets
            .iter()
            .find(|target| target.kind.contains(&kind))
            .map(|target| ctor(target.name.clone()))
    };

    if targets.iter().any(|target| {
        target
            .kind
            .iter()
            .any(|kind| *kind == TargetKind::Lib || *kind == TargetKind::ProcMacro)
    }) {
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
#[allow(clippy::panic)]
mod tests {
    use std::fs;

    use super::TargetSelector;
    use super::resolve_cargo_selection;

    fn write_fixture_manifest(dir: &std::path::Path, body: &str) {
        fs::write(dir.join("Cargo.toml"), body)
            .unwrap_or_else(|error| panic!("write fixture manifest: {error}"));
    }

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

    #[test]
    fn select_primary_target_uses_named_target_for_single_example() {
        let temp =
            tempfile::tempdir().unwrap_or_else(|error| panic!("create temp fixture dir: {error}"));
        fs::create_dir_all(temp.path().join("examples"))
            .unwrap_or_else(|error| panic!("create examples dir: {error}"));
        write_fixture_manifest(
            temp.path(),
            r#"[package]
name = "single_example_fixture"
version = "0.1.0"
edition = "2024"
autobins = false
autoexamples = false

[[example]]
name = "demo"
path = "examples/demo.rs"
"#,
        );
        fs::write(temp.path().join("examples/demo.rs"), "fn main() {}\n")
            .unwrap_or_else(|error| panic!("write example: {error}"));
        let selection = resolve_cargo_selection(Some(&temp.path().join("Cargo.toml")))
            .unwrap_or_else(|error| panic!("resolve fixture selection: {error}"));

        assert_eq!(
            selection.packages[0].target,
            TargetSelector::Example("demo".to_string())
        );
    }

    #[test]
    fn select_primary_target_uses_named_target_for_single_bin() {
        let temp =
            tempfile::tempdir().unwrap_or_else(|error| panic!("create temp fixture dir: {error}"));
        fs::create_dir_all(temp.path().join("src/bin"))
            .unwrap_or_else(|error| panic!("create bin dir: {error}"));
        write_fixture_manifest(
            temp.path(),
            r#"[package]
name = "single_bin_fixture"
version = "0.1.0"
edition = "2024"
autobins = false

[[bin]]
name = "demo"
path = "src/bin/demo.rs"
"#,
        );
        fs::write(temp.path().join("src/bin/demo.rs"), "fn main() {}\n")
            .unwrap_or_else(|error| panic!("write bin: {error}"));
        let selection = resolve_cargo_selection(Some(&temp.path().join("Cargo.toml")))
            .unwrap_or_else(|error| panic!("resolve fixture selection: {error}"));

        assert_eq!(
            selection.packages[0].target,
            TargetSelector::Bin("demo".to_string())
        );
    }
}
