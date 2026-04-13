use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use cargo_metadata::Metadata;
use cargo_metadata::MetadataCommand;
use cargo_metadata::Package;

use super::cli::CargoCheckCli;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionScope {
    Workspace,
    SinglePackage,
}

#[derive(Debug)]
pub(crate) struct Selection {
    pub manifest_path:    PathBuf,
    pub manifest_dir:     PathBuf,
    pub workspace_root:   PathBuf,
    pub target_directory: PathBuf,
    pub analysis_root:    PathBuf,
    pub scope:            SelectionScope,
    pub package_roots:    Vec<PathBuf>,
    pub packages:         Vec<PackageMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CargoCheckPlan {
    pub manifest_path:    PathBuf,
    pub workspace_root:   PathBuf,
    pub target_directory: PathBuf,
    pub analysis_root:    PathBuf,
    pub cargo_args:       Vec<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PackageMetadata {
    pub package_id:    String,
    pub manifest_path: PathBuf,
    pub root:          PathBuf,
    pub targets:       Vec<TargetMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetMetadata {
    pub kind:              Vec<String>,
    pub crate_types:       Vec<String>,
    pub name:              String,
    pub src_path:          PathBuf,
    pub edition:           String,
    pub required_features: Vec<String>,
    pub doc:               bool,
    pub doctest:           bool,
    pub test:              bool,
}

pub(crate) fn resolve_cargo_selection(explicit_manifest_path: Option<&Path>) -> Result<Selection> {
    let manifest_path = match explicit_manifest_path {
        Some(path) => normalize_explicit_manifest_path(path)?,
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

    let selected_packages: Vec<&Package> = match scope {
        SelectionScope::Workspace => metadata
            .workspace_members
            .iter()
            .filter_map(|id| metadata.packages.iter().find(|pkg| &pkg.id == id))
            .collect(),
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
            vec![package]
        },
    };
    let package_roots = selected_packages
        .iter()
        .map(|package| package_root_from_metadata(package))
        .collect::<Result<Vec<_>>>()?;
    let packages = selected_packages
        .into_iter()
        .map(package_metadata_from_cargo)
        .collect::<Result<Vec<_>>>()?;

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

pub(crate) fn build_cargo_check_plan(
    selection: &Selection,
    cargo_cli: &CargoCheckCli,
) -> CargoCheckPlan {
    let mut cargo_args = vec![
        OsString::from("--manifest-path"),
        selection.manifest_path.as_os_str().to_owned(),
    ];

    let default_workspace = selection.scope == SelectionScope::Workspace
        && cargo_cli.package.is_empty()
        && cargo_cli.exclude.is_empty();
    let use_workspace = cargo_cli.workspace() || !cargo_cli.exclude.is_empty() || default_workspace;
    if use_workspace {
        cargo_args.push(OsString::from("--workspace"));
    }

    append_repeated_flag(&mut cargo_args, "--package", &cargo_cli.package);
    append_repeated_flag(&mut cargo_args, "--exclude", &cargo_cli.exclude);
    append_bool_flag(&mut cargo_args, "--all-targets", cargo_cli.all_targets());
    append_bool_flag(&mut cargo_args, "--lib", cargo_cli.lib());
    append_bool_flag(&mut cargo_args, "--bins", cargo_cli.bins());
    append_bool_flag(&mut cargo_args, "--examples", cargo_cli.examples());
    append_bool_flag(&mut cargo_args, "--tests", cargo_cli.tests());
    append_bool_flag(&mut cargo_args, "--benches", cargo_cli.benches());
    append_repeated_flag(&mut cargo_args, "--bin", &cargo_cli.bin);
    append_repeated_flag(&mut cargo_args, "--example", &cargo_cli.example);
    append_repeated_flag(&mut cargo_args, "--test", &cargo_cli.test);
    append_repeated_flag(&mut cargo_args, "--bench", &cargo_cli.bench);

    CargoCheckPlan {
        manifest_path: selection.manifest_path.clone(),
        workspace_root: selection.workspace_root.clone(),
        target_directory: selection.target_directory.clone(),
        analysis_root: selection.analysis_root.clone(),
        cargo_args,
    }
}

fn append_bool_flag(args: &mut Vec<OsString>, flag: &'static str, enabled: bool) {
    if enabled {
        args.push(OsString::from(flag));
    }
}

fn append_repeated_flag(args: &mut Vec<OsString>, flag: &'static str, values: &[String]) {
    for value in values {
        args.push(OsString::from(flag));
        args.push(OsString::from(value));
    }
}

fn package_root_from_metadata(package: &Package) -> Result<PathBuf> {
    let package_root = package
        .manifest_path
        .as_std_path()
        .parent()
        .context("package manifest path had no parent directory")?;
    package_root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", package_root.display()))
}

fn package_metadata_from_cargo(package: &Package) -> Result<PackageMetadata> {
    Ok(PackageMetadata {
        package_id:    package.id.to_string(),
        manifest_path: package.manifest_path.clone().into_std_path_buf(),
        root:          package_root_from_metadata(package)?,
        targets:       package
            .targets
            .iter()
            .map(target_metadata_from_cargo)
            .collect(),
    })
}

fn target_metadata_from_cargo(target: &cargo_metadata::Target) -> TargetMetadata {
    TargetMetadata {
        kind:              target.kind.iter().map(ToString::to_string).collect(),
        crate_types:       target.crate_types.iter().map(ToString::to_string).collect(),
        name:              target.name.clone(),
        src_path:          target.src_path.clone().into_std_path_buf(),
        edition:           target.edition.to_string(),
        required_features: target.required_features.clone(),
        doc:               target.doc,
        doctest:           target.doctest,
        test:              target.test,
    }
}

fn cargo_metadata_for(manifest_path: &Path) -> Result<Metadata> {
    let mut command = MetadataCommand::new();
    command.no_deps();
    command.manifest_path(manifest_path);
    command.exec().context("failed to run cargo metadata")
}

fn normalize_explicit_manifest_path(path: &Path) -> Result<PathBuf> {
    if path.is_dir() {
        let manifest_path = path.join("Cargo.toml");
        if !manifest_path.is_file() {
            bail!("directory {} does not contain Cargo.toml", path.display());
        }

        return manifest_path
            .canonicalize()
            .with_context(|| format!("failed to canonicalize {}", manifest_path.display()));
    }

    path.canonicalize()
        .with_context(|| format!("failed to canonicalize {}", path.display()))
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
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::path::PathBuf;

    use super::CargoCheckPlan;
    use super::PackageMetadata;
    use super::Selection;
    use super::SelectionScope;
    use super::TargetMetadata;
    use super::build_cargo_check_plan;
    use super::resolve_cargo_selection;
    use crate::cli::CargoCheckCli;
    use crate::cli::PrimaryTargetCli;
    use crate::cli::SecondaryTargetCli;
    use crate::cli::WorkspaceCli;

    fn fixture_selection(scope: SelectionScope) -> Selection {
        Selection {
            manifest_path: PathBuf::from("/workspace/Cargo.toml"),
            manifest_dir: PathBuf::from("/workspace"),
            workspace_root: PathBuf::from("/workspace"),
            target_directory: PathBuf::from("/workspace/target"),
            analysis_root: PathBuf::from("/workspace"),
            scope,
            package_roots: vec![PathBuf::from("/workspace/member")],
            packages: vec![PackageMetadata {
                package_id:    String::from("path+file:///workspace/member#member@0.1.0"),
                manifest_path: PathBuf::from("/workspace/member/Cargo.toml"),
                root:          PathBuf::from("/workspace/member"),
                targets:       vec![TargetMetadata {
                    kind:              vec![String::from("lib")],
                    crate_types:       vec![String::from("lib")],
                    name:              String::from("member"),
                    src_path:          PathBuf::from("/workspace/member/src/lib.rs"),
                    edition:           String::from("2024"),
                    required_features: Vec::new(),
                    doc:               true,
                    doctest:           true,
                    test:              true,
                }],
            }],
        }
    }

    fn cargo_args_strings(plan: CargoCheckPlan) -> Vec<String> {
        plan.cargo_args
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn default_workspace_plan_checks_workspace() {
        let selection = fixture_selection(SelectionScope::Workspace);

        let args = cargo_args_strings(build_cargo_check_plan(
            &selection,
            &CargoCheckCli::default(),
        ));

        assert_eq!(
            args,
            vec!["--manifest-path", "/workspace/Cargo.toml", "--workspace"]
        );
    }

    #[test]
    fn default_single_package_plan_checks_manifest_only() {
        let selection = fixture_selection(SelectionScope::SinglePackage);

        let args = cargo_args_strings(build_cargo_check_plan(
            &selection,
            &CargoCheckCli::default(),
        ));

        assert_eq!(args, vec!["--manifest-path", "/workspace/Cargo.toml"]);
    }

    #[test]
    fn plan_includes_workspace_all_targets() {
        let selection = fixture_selection(SelectionScope::Workspace);
        let cargo_cli = CargoCheckCli {
            workspace: WorkspaceCli { workspace: true },
            primary_targets: PrimaryTargetCli {
                all_targets: true,
                ..Default::default()
            },
            ..CargoCheckCli::default()
        };

        let args = cargo_args_strings(build_cargo_check_plan(&selection, &cargo_cli));

        assert_eq!(
            args,
            vec![
                "--manifest-path",
                "/workspace/Cargo.toml",
                "--workspace",
                "--all-targets",
            ]
        );
    }

    #[test]
    fn plan_includes_named_package_and_tests() {
        let selection = fixture_selection(SelectionScope::Workspace);
        let cargo_cli = CargoCheckCli {
            package: vec!["demo".to_string()],
            secondary_targets: SecondaryTargetCli {
                tests: true,
                ..Default::default()
            },
            ..CargoCheckCli::default()
        };

        let args = cargo_args_strings(build_cargo_check_plan(&selection, &cargo_cli));

        assert_eq!(
            args,
            vec![
                "--manifest-path",
                "/workspace/Cargo.toml",
                "--package",
                "demo",
                "--tests",
            ]
        );
    }

    #[test]
    fn plan_includes_workspace_excludes() {
        let selection = fixture_selection(SelectionScope::Workspace);
        let cargo_cli = CargoCheckCli {
            exclude: vec!["demo".to_string()],
            ..CargoCheckCli::default()
        };

        let args = cargo_args_strings(build_cargo_check_plan(&selection, &cargo_cli));

        assert_eq!(
            args,
            vec![
                "--manifest-path",
                "/workspace/Cargo.toml",
                "--workspace",
                "--exclude",
                "demo",
            ]
        );
    }

    #[test]
    fn plan_includes_specific_named_targets() {
        let selection = fixture_selection(SelectionScope::SinglePackage);
        let cargo_cli = CargoCheckCli {
            bin: vec!["cli".to_string()],
            example: vec!["demo".to_string()],
            test: vec!["integration".to_string()],
            bench: vec!["perf".to_string()],
            ..CargoCheckCli::default()
        };

        let args = cargo_args_strings(build_cargo_check_plan(&selection, &cargo_cli));

        assert_eq!(
            args,
            vec![
                "--manifest-path",
                "/workspace/Cargo.toml",
                "--bin",
                "cli",
                "--example",
                "demo",
                "--test",
                "integration",
                "--bench",
                "perf",
            ]
        );
    }

    #[test]
    fn resolve_virtual_workspace_root_with_single_member_selects_workspace() {
        let temp =
            tempfile::tempdir().unwrap_or_else(|error| panic!("create temp fixture dir: {error}"));
        std::fs::create_dir_all(temp.path().join("member/src"))
            .unwrap_or_else(|error| panic!("create member src dir: {error}"));
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"member\"]\nresolver = \"3\"\n",
        )
        .unwrap_or_else(|error| panic!("write workspace manifest: {error}"));
        std::fs::write(
            temp.path().join("member/Cargo.toml"),
            "[package]\nname = \"member_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap_or_else(|error| panic!("write member manifest: {error}"));
        std::fs::write(temp.path().join("member/src/main.rs"), "fn main() {}\n")
            .unwrap_or_else(|error| panic!("write member main: {error}"));

        let selection = resolve_cargo_selection(Some(&temp.path().join("Cargo.toml")))
            .unwrap_or_else(|error| panic!("resolve workspace selection: {error}"));

        assert_eq!(selection.scope, SelectionScope::Workspace);
        assert_eq!(selection.package_roots.len(), 1);
        assert_eq!(
            std::fs::canonicalize(&selection.package_roots[0])
                .unwrap_or_else(|error| panic!("canonicalize selected package root: {error}")),
            std::fs::canonicalize(temp.path().join("member"))
                .unwrap_or_else(|error| panic!("canonicalize expected package root: {error}"))
        );
    }

    #[test]
    fn resolve_project_directory_uses_its_manifest() {
        let temp =
            tempfile::tempdir().unwrap_or_else(|error| panic!("create temp fixture dir: {error}"));
        std::fs::create_dir_all(temp.path().join("src"))
            .unwrap_or_else(|error| panic!("create src dir: {error}"));
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"dir_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap_or_else(|error| panic!("write manifest: {error}"));
        std::fs::write(temp.path().join("src/lib.rs"), "pub fn exported() {}\n")
            .unwrap_or_else(|error| panic!("write lib: {error}"));

        let selection = resolve_cargo_selection(Some(temp.path()))
            .unwrap_or_else(|error| panic!("resolve directory selection: {error}"));

        assert_eq!(
            selection.manifest_path,
            std::fs::canonicalize(temp.path().join("Cargo.toml"))
                .unwrap_or_else(|error| panic!("canonicalize manifest: {error}"))
        );
        assert_eq!(selection.scope, SelectionScope::SinglePackage);
    }
}
