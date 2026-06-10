use std::env;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use cargo_metadata::Metadata;
use cargo_metadata::MetadataCommand;
use cargo_metadata::Package;
use cargo_metadata::Target;
use serde::Serialize;
use serde::Serializer;

use crate::compiler::CARGO_FLAG_ALL_TARGETS;
use crate::compiler::CARGO_FLAG_EXCLUDE;
use crate::compiler::CARGO_FLAG_MANIFEST_PATH;
use crate::compiler::CARGO_FLAG_PACKAGE;
use crate::compiler::CARGO_FLAG_WORKSPACE;
use crate::config::CargoCheckCli;
use crate::config::WorkspaceSelection;

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
    pub id:            String,
    pub manifest_path: PathBuf,
    pub root:          PathBuf,
    pub targets:       Vec<TargetMetadata>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetSupport {
    Disabled,
    Enabled,
}

impl TargetSupport {
    pub(crate) const fn is_enabled(self) -> bool { matches!(self, Self::Enabled) }
}

impl From<bool> for TargetSupport {
    fn from(value: bool) -> Self { if value { Self::Enabled } else { Self::Disabled } }
}

impl Serialize for TargetSupport {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bool(self.is_enabled())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetMetadata {
    pub kind:              Vec<String>,
    pub crate_types:       Vec<String>,
    pub name:              String,
    pub src_path:          PathBuf,
    pub edition:           String,
    pub required_features: Vec<String>,
    pub doc:               TargetSupport,
    pub doctest:           TargetSupport,
    pub test:              TargetSupport,
}

pub(crate) fn resolve_cargo_selection(explicit_manifest_path: Option<&Path>) -> Result<Selection> {
    let manifest_path = match explicit_manifest_path {
        Some(path) => normalize_explicit_manifest_path(path)?,
        None => find_nearest_manifest(&env::current_dir()?)?,
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
        .copied()
        .map(package_root_from_metadata)
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
        OsString::from(CARGO_FLAG_MANIFEST_PATH),
        selection.manifest_path.as_os_str().to_owned(),
    ];

    let default_workspace = selection.scope == SelectionScope::Workspace
        && cargo_cli.package.is_empty()
        && cargo_cli.exclude.is_empty();
    let use_workspace = matches!(cargo_cli.workspace_selection, WorkspaceSelection::Workspace)
        || !cargo_cli.exclude.is_empty()
        || default_workspace;
    if use_workspace {
        cargo_args.push(OsString::from(CARGO_FLAG_WORKSPACE));
    }

    append_repeated_flag(&mut cargo_args, CARGO_FLAG_PACKAGE, &cargo_cli.package);
    append_repeated_flag(&mut cargo_args, CARGO_FLAG_EXCLUDE, &cargo_cli.exclude);

    // Mend's reachability/visibility analyses require seeing every target's
    // call graph. Lib-only compilation strips `#[cfg(test)]` blocks and
    // hides callers that exist in test or example code. Always include all
    // targets so the analysis is sound regardless of the user's display
    // filter (target_selections, bin/example/test/bench lists).
    cargo_args.push(OsString::from(CARGO_FLAG_ALL_TARGETS));

    CargoCheckPlan {
        manifest_path: selection.manifest_path.clone(),
        workspace_root: selection.workspace_root.clone(),
        target_directory: selection.target_directory.clone(),
        analysis_root: selection.analysis_root.clone(),
        cargo_args,
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
        id:            package.id.to_string(),
        manifest_path: package.manifest_path.clone().into_std_path_buf(),
        root:          package_root_from_metadata(package)?,
        targets:       package
            .targets
            .iter()
            .map(target_metadata_from_cargo)
            .collect(),
    })
}

fn target_metadata_from_cargo(target: &Target) -> TargetMetadata {
    TargetMetadata {
        kind:              target.kind.iter().map(ToString::to_string).collect(),
        crate_types:       target.crate_types.iter().map(ToString::to_string).collect(),
        name:              target.name.clone(),
        src_path:          target.src_path.clone().into_std_path_buf(),
        edition:           target.edition.to_string(),
        required_features: target.required_features.clone(),
        doc:               TargetSupport::from(target.doc),
        doctest:           TargetSupport::from(target.doctest),
        test:              TargetSupport::from(target.test),
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
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::CargoCheckPlan;
    use super::PackageMetadata;
    use super::Selection;
    use super::SelectionScope;
    use super::TargetMetadata;
    use super::TargetSupport;
    use super::build_cargo_check_plan;
    use super::resolve_cargo_selection;
    use crate::compiler::CARGO_FLAG_ALL_TARGETS;
    use crate::compiler::CARGO_FLAG_EXCLUDE;
    use crate::compiler::CARGO_FLAG_MANIFEST_PATH;
    use crate::compiler::CARGO_FLAG_PACKAGE;
    use crate::compiler::CARGO_FLAG_WORKSPACE;
    use crate::config::CargoCheckCli;
    use crate::config::TargetSelection;
    use crate::config::WorkspaceSelection;
    use crate::selection::CARGO_TARGET_KIND_LIB;

    fn workspace_manifest_path() -> PathBuf { PathBuf::from("/workspace").join("Cargo.toml") }

    fn workspace_manifest_arg() -> String {
        workspace_manifest_path().to_string_lossy().into_owned()
    }

    fn fixture_selection(scope: SelectionScope) -> Selection {
        Selection {
            manifest_path: workspace_manifest_path(),
            manifest_dir: PathBuf::from("/workspace"),
            workspace_root: PathBuf::from("/workspace"),
            target_directory: PathBuf::from("/workspace/target"),
            analysis_root: PathBuf::from("/workspace"),
            scope,
            package_roots: vec![PathBuf::from("/workspace/member")],
            packages: vec![PackageMetadata {
                id:            String::from("path+file:///workspace/member#member@0.1.0"),
                manifest_path: PathBuf::from("/workspace/member").join("Cargo.toml"),
                root:          PathBuf::from("/workspace/member"),
                targets:       vec![TargetMetadata {
                    kind:              vec![String::from(CARGO_TARGET_KIND_LIB)],
                    crate_types:       vec![String::from(CARGO_TARGET_KIND_LIB)],
                    name:              String::from("member"),
                    src_path:          PathBuf::from("/workspace/member/src/lib.rs"),
                    edition:           String::from("2024"),
                    required_features: Vec::new(),
                    doc:               TargetSupport::Enabled,
                    doctest:           TargetSupport::Enabled,
                    test:              TargetSupport::Enabled,
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
    fn default_workspace_plan_checks_workspace_with_all_targets() {
        // Mend always passes `--all-targets` for analysis so test-only
        // callers are visible in the call graph.
        let selection = fixture_selection(SelectionScope::Workspace);

        let args = cargo_args_strings(build_cargo_check_plan(
            &selection,
            &CargoCheckCli::default(),
        ));

        assert_eq!(
            args,
            vec![
                CARGO_FLAG_MANIFEST_PATH.to_string(),
                workspace_manifest_arg(),
                CARGO_FLAG_WORKSPACE.to_string(),
                CARGO_FLAG_ALL_TARGETS.to_string(),
            ]
        );
    }

    #[test]
    fn default_single_package_plan_includes_all_targets() {
        let selection = fixture_selection(SelectionScope::SinglePackage);

        let args = cargo_args_strings(build_cargo_check_plan(
            &selection,
            &CargoCheckCli::default(),
        ));

        assert_eq!(
            args,
            vec![
                CARGO_FLAG_MANIFEST_PATH.to_string(),
                workspace_manifest_arg(),
                CARGO_FLAG_ALL_TARGETS.to_string(),
            ]
        );
    }

    #[test]
    fn plan_includes_workspace_all_targets() {
        // User-facing target-selection flags are now display filters and do
        // not duplicate the always-on `--all-targets`.
        let selection = fixture_selection(SelectionScope::Workspace);
        let cargo_cli = CargoCheckCli {
            workspace_selection: WorkspaceSelection::Workspace,
            target_selections: BTreeSet::from([TargetSelection::All]),
            ..CargoCheckCli::default()
        };

        let args = cargo_args_strings(build_cargo_check_plan(&selection, &cargo_cli));

        assert_eq!(
            args,
            vec![
                CARGO_FLAG_MANIFEST_PATH.to_string(),
                workspace_manifest_arg(),
                CARGO_FLAG_WORKSPACE.to_string(),
                CARGO_FLAG_ALL_TARGETS.to_string(),
            ]
        );
    }

    #[test]
    fn plan_includes_named_package_and_tests() {
        let selection = fixture_selection(SelectionScope::Workspace);
        let cargo_cli = CargoCheckCli {
            package: vec!["demo".to_string()],
            target_selections: BTreeSet::from([TargetSelection::Tests]),
            ..CargoCheckCli::default()
        };

        let args = cargo_args_strings(build_cargo_check_plan(&selection, &cargo_cli));

        assert_eq!(
            args,
            vec![
                CARGO_FLAG_MANIFEST_PATH.to_string(),
                workspace_manifest_arg(),
                CARGO_FLAG_PACKAGE.to_string(),
                "demo".to_string(),
                CARGO_FLAG_ALL_TARGETS.to_string(),
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
                CARGO_FLAG_MANIFEST_PATH.to_string(),
                workspace_manifest_arg(),
                CARGO_FLAG_WORKSPACE.to_string(),
                CARGO_FLAG_EXCLUDE.to_string(),
                "demo".to_string(),
                CARGO_FLAG_ALL_TARGETS.to_string(),
            ]
        );
    }

    #[test]
    fn plan_includes_specific_named_targets() {
        // Named-target flags are display filters; analysis still runs with
        // `--all-targets` so the call graph stays complete.
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
                CARGO_FLAG_MANIFEST_PATH.to_string(),
                workspace_manifest_arg(),
                CARGO_FLAG_ALL_TARGETS.to_string(),
            ]
        );
    }

    #[test]
    fn resolve_virtual_workspace_root_with_single_member_selects_workspace() {
        let temp = tempdir().unwrap_or_else(|error| panic!("create temp fixture dir: {error}"));
        fs::create_dir_all(temp.path().join("member/src"))
            .unwrap_or_else(|error| panic!("create member src dir: {error}"));
        fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"member\"]\nresolver = \"3\"\n",
        )
        .unwrap_or_else(|error| panic!("write workspace manifest: {error}"));
        fs::write(
            temp.path().join("member").join("Cargo.toml"),
            "[package]\nname = \"member_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap_or_else(|error| panic!("write member manifest: {error}"));
        fs::write(temp.path().join("member/src/main.rs"), "fn main() {}\n")
            .unwrap_or_else(|error| panic!("write member main: {error}"));

        let selection = resolve_cargo_selection(Some(&temp.path().join("Cargo.toml")))
            .unwrap_or_else(|error| panic!("resolve workspace selection: {error}"));

        assert_eq!(selection.scope, SelectionScope::Workspace);
        assert_eq!(selection.package_roots.len(), 1);
        assert_eq!(
            fs::canonicalize(&selection.package_roots[0])
                .unwrap_or_else(|error| panic!("canonicalize selected package root: {error}")),
            fs::canonicalize(temp.path().join("member"))
                .unwrap_or_else(|error| panic!("canonicalize expected package root: {error}"))
        );
    }

    #[test]
    fn resolve_project_directory_uses_its_manifest() {
        let temp = tempdir().unwrap_or_else(|error| panic!("create temp fixture dir: {error}"));
        fs::create_dir_all(temp.path().join("src"))
            .unwrap_or_else(|error| panic!("create src dir: {error}"));
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"dir_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap_or_else(|error| panic!("write manifest: {error}"));
        fs::write(temp.path().join("src/lib.rs"), "pub fn exported() {}\n")
            .unwrap_or_else(|error| panic!("write lib: {error}"));

        let selection = resolve_cargo_selection(Some(temp.path()))
            .unwrap_or_else(|error| panic!("resolve directory selection: {error}"));

        assert_eq!(
            selection.manifest_path,
            fs::canonicalize(temp.path().join("Cargo.toml"))
                .unwrap_or_else(|error| panic!("canonicalize manifest: {error}"))
        );
        assert_eq!(selection.scope, SelectionScope::SinglePackage);
    }
}
