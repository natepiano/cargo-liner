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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionScope {
    Workspace,
    SinglePackage,
}

#[derive(Debug)]
pub struct Selection {
    pub manifest_path:    PathBuf,
    pub manifest_dir:     PathBuf,
    pub workspace_root:   PathBuf,
    pub target_directory: PathBuf,
    pub analysis_root:    PathBuf,
    pub scope:            SelectionScope,
    pub package_roots:    Vec<PathBuf>,
    pub packages:         Vec<SelectedPackage>,
}

#[derive(Debug, Clone)]
pub struct SelectedPackage {
    pub name:          String,
    pub manifest_path: PathBuf,
    pub root:          PathBuf,
    pub source_root:   PathBuf,
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
    let manifest_is_workspace_root = manifest_path == workspace_manifest;
    let manifest_matches_package = metadata
        .packages
        .iter()
        .any(|pkg| pkg.manifest_path.as_std_path() == manifest_path);
    let scope = if manifest_is_workspace_root
        && (!manifest_matches_package || metadata.workspace_members.len() > 1)
    {
        SelectionScope::Workspace
    } else {
        SelectionScope::SinglePackage
    };

    let packages: Vec<SelectedPackage> = match scope {
        SelectionScope::Workspace => metadata
            .workspace_members
            .iter()
            .filter_map(|id| metadata.packages.iter().find(|pkg| &pkg.id == id))
            .map(selected_package_from_metadata)
            .collect::<Result<Vec<_>>>()?,
        SelectionScope::SinglePackage => {
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
        },
    };
    let package_roots = packages
        .iter()
        .map(|package| package.root.clone())
        .collect();

    let analysis_root = match scope {
        SelectionScope::Workspace => workspace_root.clone(),
        SelectionScope::SinglePackage => manifest_dir.clone(),
    };

    Ok(Selection {
        manifest_path,
        manifest_dir,
        workspace_root,
        target_directory,
        analysis_root,
        scope,
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
    let target = select_primary_target_metadata(package.targets.as_slice())
        .context("package metadata did not contain a selectable target")?;
    let source_root = target
        .src_path
        .as_std_path()
        .parent()
        .context("target source path had no parent directory")?
        .to_path_buf();
    Ok(SelectedPackage {
        name: package.name.to_string(),
        manifest_path,
        root,
        source_root,
        target: target_selector(target),
    })
}

fn target_selector(target: &Target) -> TargetSelector {
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

    TargetSelector::Implicit
}

fn select_primary_target_metadata(targets: &[Target]) -> Option<&Target> {
    if targets.len() == 1 {
        return targets.first();
    }

    let select_named = |kind: TargetKind| targets.iter().find(|target| target.kind.contains(&kind));

    if targets.iter().any(|target| {
        target
            .kind
            .iter()
            .any(|kind| *kind == TargetKind::Lib || *kind == TargetKind::ProcMacro)
    }) {
        return targets.iter().find(|target| {
            target
                .kind
                .iter()
                .any(|kind| *kind == TargetKind::Lib || *kind == TargetKind::ProcMacro)
        });
    }

    select_named(TargetKind::Bin)
        .or_else(|| select_named(TargetKind::Example))
        .or_else(|| select_named(TargetKind::Test))
        .or_else(|| select_named(TargetKind::Bench))
        .or_else(|| targets.first())
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
#[allow(clippy::unwrap_used, reason = "tests should panic on unexpected values")]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::fs;

    use super::SelectionScope;
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
        assert_eq!(
            fs::canonicalize(&selection.packages[0].source_root).unwrap_or_else(|error| panic!(
                "canonicalize selected example source root: {error}"
            )),
            fs::canonicalize(temp.path().join("examples")).unwrap_or_else(|error| panic!(
                "canonicalize expected example source root: {error}"
            ))
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
        assert_eq!(
            fs::canonicalize(&selection.packages[0].source_root)
                .unwrap_or_else(|error| panic!("canonicalize selected bin source root: {error}")),
            fs::canonicalize(temp.path().join("src/bin"))
                .unwrap_or_else(|error| panic!("canonicalize expected bin source root: {error}"))
        );
    }

    #[test]
    fn resolve_virtual_workspace_root_with_single_member_selects_workspace() {
        let temp =
            tempfile::tempdir().unwrap_or_else(|error| panic!("create temp fixture dir: {error}"));
        fs::create_dir_all(temp.path().join("member/src"))
            .unwrap_or_else(|error| panic!("create member src dir: {error}"));
        write_fixture_manifest(
            temp.path(),
            r#"[workspace]
members = ["member"]
resolver = "3"
"#,
        );
        write_fixture_manifest(
            &temp.path().join("member"),
            r#"[package]
name = "member_fixture"
version = "0.1.0"
edition = "2024"
"#,
        );
        fs::write(temp.path().join("member/src/main.rs"), "fn main() {}\n")
            .unwrap_or_else(|error| panic!("write member main: {error}"));

        let selection = resolve_cargo_selection(Some(&temp.path().join("Cargo.toml")))
            .unwrap_or_else(|error| panic!("resolve workspace selection: {error}"));

        assert_eq!(selection.scope, SelectionScope::Workspace);
        assert_eq!(selection.packages.len(), 1);
        assert_eq!(selection.packages[0].name, "member_fixture");
    }
}
