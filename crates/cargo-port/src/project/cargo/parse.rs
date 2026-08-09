use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::io;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;
use toml::Table;
use toml::Value;

use super::package::Package;
use super::rust_info::Cargo;
use super::rust_info::RustInfo;
use super::workspace::Workspace;
use crate::constants::CARGO_BIN_TARGET_DIR;
use crate::constants::CARGO_LIB_TARGET;
use crate::constants::CARGO_MAIN_FILE;
use crate::constants::CARGO_MAIN_TARGET;
use crate::constants::CARGO_TARGET_TABLES;
use crate::constants::CARGO_TOML;
use crate::constants::RUST_SOURCE_EXTENSION;
use crate::project::git;
use crate::project::info::ProjectInfo;
use crate::project::non_rust::NonRustProject;
use crate::project::paths::AbsolutePath;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ProjectType {
    Workspace,
    Binary,
    Library,
    ProcMacro,
}

impl Display for ProjectType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace => write!(f, "workspace"),
            Self::Binary => write!(f, "binary"),
            Self::Library => write!(f, "library"),
            Self::ProcMacro => write!(f, "proc-macro"),
        }
    }
}

/// A group of examples in a subdirectory, or root-level examples (empty category).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExampleGroup {
    /// Subdirectory name, or empty for root-level examples.
    pub category: String,
    pub names:    Vec<String>,
}

pub(crate) enum ProjectLoadError {
    Read(io::Error),
    Parse(toml::de::Error),
}

impl Display for ProjectLoadError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(e) => write!(f, "read error: {e}"),
            Self::Parse(e) => write!(f, "parse error: {e}"),
        }
    }
}

/// Result of parsing a `Cargo.toml`: either a workspace or a standalone package.
pub(crate) enum CargoParseResult {
    Workspace(Workspace),
    Package(Package),
}

/// Whether a `Cargo.toml` names anything cargo could actually build.
#[derive(Clone, Copy)]
pub(crate) enum ManifestTargets {
    Resolvable,
    Missing,
}

impl From<bool> for ManifestTargets {
    fn from(resolvable: bool) -> Self {
        if resolvable {
            Self::Resolvable
        } else {
            Self::Missing
        }
    }
}

/// Parse a `Cargo.toml` and return either a workspace or a package project.
///
/// Step 3b full retirement: hand-parsing of `version`, `description`,
/// `publish`, `[lib]` / `[[bin]]` / `[[example]]` / `[[bench]]` /
/// `[[test]]` is dropped. The authoritative source is the
/// `WorkspaceMetadata` populated by `cargo metadata`; detail-pane and
/// finder-index readers prefer the metadata when present and silently
/// fall back to empty data pre-metadata — matching the Targets-pane
/// "Loading…" UX established in Step 3a. This function now only
/// extracts the fields needed to classify a project at parse time
/// (`[package] name`, `[workspace]` presence) and the on-disk
/// worktree state.
pub(crate) fn from_cargo_toml(
    cargo_toml_path: &Path,
) -> Result<CargoParseResult, ProjectLoadError> {
    let table = read_manifest_table(cargo_toml_path)?;

    let project_dir = cargo_toml_path.parent().unwrap_or(cargo_toml_path);
    let abs_path = AbsolutePath::from(project_dir);

    let name = table
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| (*s).to_string());

    let worktree_status = git::get_worktree_status(project_dir);
    let worktree_health = git::get_worktree_health(project_dir);

    let rust = RustInfo {
        project_info: ProjectInfo {
            worktree_health,
            ..ProjectInfo::default()
        },
        cargo: Cargo::default(),
        ..RustInfo::default()
    };

    if table.get("workspace").is_some() {
        Ok(CargoParseResult::Workspace(Workspace {
            path: abs_path,
            name,
            worktree_status,
            rust,
            ..Workspace::default()
        }))
    } else {
        Ok(CargoParseResult::Package(Package {
            path: abs_path,
            name,
            worktree_status,
            rust,
        }))
    }
}

/// Does this `Cargo.toml` resolve to a target cargo could build?
///
/// `from_cargo_toml` accepts every manifest that parses, so a `Cargo.toml`
/// copied into a scratch directory — no `src/`, `[workspace] members` naming
/// directories that aren't there — registers as a project in the tree. Scan
/// discovery calls this first and skips the manifests that resolve to
/// nothing, which also avoids the `git::get_worktree_status` and
/// `git::get_worktree_health` subprocesses `from_cargo_toml` runs per
/// manifest.
pub(crate) fn manifest_targets(cargo_toml_path: &Path) -> ManifestTargets {
    let Ok(table) = read_manifest_table(cargo_toml_path) else {
        return ManifestTargets::Missing;
    };
    let project_dir = cargo_toml_path.parent().unwrap_or(cargo_toml_path);

    match workspace_targets(project_dir, &table) {
        ManifestTargets::Resolvable => ManifestTargets::Resolvable,
        ManifestTargets::Missing => package_targets(project_dir, &table),
    }
}

/// Create a project entry for a non-Rust git repository (no `Cargo.toml`).
pub(crate) fn from_git_dir(project_dir: &Path) -> NonRustProject {
    let name = project_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string());

    let mut project = NonRustProject::new(AbsolutePath::from(project_dir), name);
    project.project_info.worktree_health = git::get_worktree_health(project_dir);
    project.worktree_status = git::get_worktree_status(project_dir);
    project
}

fn read_manifest_table(cargo_toml_path: &Path) -> Result<Table, ProjectLoadError> {
    let contents = std::fs::read_to_string(cargo_toml_path).map_err(ProjectLoadError::Read)?;
    contents.parse().map_err(ProjectLoadError::Parse)
}

/// A `[workspace]` resolves once any one of its `members` entries names a
/// directory holding a `Cargo.toml`. A glob entry (`crates/*`) resolves when
/// any child of the literal prefix does — close enough to cargo's expansion
/// to tell a live workspace from a stray manifest, and it needs no glob
/// dependency.
fn workspace_targets(project_dir: &Path, table: &Table) -> ManifestTargets {
    table
        .get("workspace")
        .and_then(Value::as_table)
        .and_then(|workspace| workspace.get("members"))
        .and_then(Value::as_array)
        .is_some_and(|members| {
            members.iter().filter_map(Value::as_str).any(|member| {
                member.split_once('*').map_or_else(
                    || project_dir.join(member).join(CARGO_TOML).is_file(),
                    |(prefix, _)| {
                        std::fs::read_dir(project_dir.join(prefix))
                            .into_iter()
                            .flatten()
                            .flatten()
                            .any(|entry| entry.path().join(CARGO_TOML).is_file())
                    },
                )
            })
        })
        .into()
}

/// A `[package]` resolves through cargo's conventional layout — `src/lib.rs`,
/// `src/main.rs`, or an entry under `src/bin` — or through an explicit `path`
/// in one of the `CARGO_TARGET_TABLES`.
fn package_targets(project_dir: &Path, table: &Table) -> ManifestTargets {
    if !table.contains_key("package") {
        return ManifestTargets::Missing;
    }

    (project_dir.join(CARGO_LIB_TARGET).is_file()
        || project_dir.join(CARGO_MAIN_TARGET).is_file()
        || std::fs::read_dir(project_dir.join(CARGO_BIN_TARGET_DIR))
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| {
                let path = entry.path();
                path.extension()
                    .is_some_and(|extension| extension == RUST_SOURCE_EXTENSION)
                    || path.join(CARGO_MAIN_FILE).is_file()
            })
        || explicit_target_paths(table).any(|path| project_dir.join(path).exists()))
    .into()
}

/// Collect every `path` named by a `[lib]` / `[[bin]]` / `[[example]]` /
/// `[[test]]` / `[[bench]]` table. Cargo accepts each key in both the single-
/// table and array-of-tables form, so both are read.
fn explicit_target_paths(table: &Table) -> impl Iterator<Item = &str> {
    CARGO_TARGET_TABLES
        .into_iter()
        .flat_map(move |key| {
            let node = table.get(key);
            node.and_then(Value::as_table).into_iter().chain(
                node.and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_table),
            )
        })
        .filter_map(|target| target.get("path").and_then(Value::as_str))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    const PACKAGE_HEADER: &str = "[package]\nname = \"probe\"\nversion = \"0.1.0\"\n";

    /// Write `manifest` as the root `Cargo.toml` and create each path in
    /// `sources` — a trailing `/` marks a directory, anything else a file.
    fn manifest_dir(manifest: &str, sources: &[&str]) -> TempDir {
        let dir = TempDir::new().expect("temp dir");
        std::fs::write(dir.path().join(CARGO_TOML), manifest).expect("write manifest");
        for source in sources {
            let path = dir.path().join(source.trim_end_matches('/'));
            if source.ends_with('/') {
                std::fs::create_dir_all(&path).expect("create dir");
            } else {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).expect("create parent");
                }
                std::fs::write(&path, "").expect("write source");
            }
        }
        dir
    }

    fn resolves(dir: &TempDir) -> bool {
        matches!(
            manifest_targets(&dir.path().join(CARGO_TOML)),
            ManifestTargets::Resolvable
        )
    }

    #[test]
    fn package_without_sources_is_missing() {
        let dir = manifest_dir(PACKAGE_HEADER, &[]);
        assert!(!resolves(&dir));
    }

    #[test]
    fn package_with_conventional_targets_resolves() {
        let bin_target = format!("{CARGO_BIN_TARGET_DIR}/tool.rs");
        for source in [CARGO_LIB_TARGET, CARGO_MAIN_TARGET, bin_target.as_str()] {
            let dir = manifest_dir(PACKAGE_HEADER, &[source]);
            assert!(resolves(&dir), "{source} should resolve");
        }
    }

    #[test]
    fn package_with_bin_subdirectory_target_resolves() {
        let bin_main = format!("{CARGO_BIN_TARGET_DIR}/tool/{CARGO_MAIN_FILE}");
        let dir = manifest_dir(PACKAGE_HEADER, &[bin_main.as_str()]);
        assert!(resolves(&dir));
    }

    #[test]
    fn package_with_explicit_target_path_resolves() {
        let manifest = format!("{PACKAGE_HEADER}\n[lib]\npath = \"other/entry.rs\"\n");
        let dir = manifest_dir(&manifest, &["other/entry.rs"]);
        assert!(resolves(&dir));
    }

    #[test]
    fn package_with_dangling_explicit_target_path_is_missing() {
        let manifest = format!("{PACKAGE_HEADER}\n[[bin]]\nname = \"tool\"\npath = \"gone.rs\"\n");
        let dir = manifest_dir(&manifest, &[]);
        assert!(!resolves(&dir));
    }

    #[test]
    fn workspace_with_existing_member_resolves() {
        let dir = manifest_dir(
            "[workspace]\nmembers = [\"member\"]\n",
            &["member/Cargo.toml"],
        );
        assert!(resolves(&dir));
    }

    #[test]
    fn workspace_with_glob_member_resolves() {
        let dir = manifest_dir(
            "[workspace]\nmembers = [\"crates/*\"]\n",
            &["crates/inner/Cargo.toml"],
        );
        assert!(resolves(&dir));
    }

    /// The `$TMPDIR` manifest that prompted this check: a stray copy of a
    /// real workspace root, with neither its `src/` nor its members beside it.
    #[test]
    fn workspace_with_missing_member_and_no_sources_is_missing() {
        let manifest = format!("{PACKAGE_HEADER}\n[workspace]\nmembers = [\"member\"]\n");
        let dir = manifest_dir(&manifest, &[]);
        assert!(!resolves(&dir));
    }

    #[test]
    fn absent_manifest_is_missing() {
        let dir = TempDir::new().expect("temp dir");
        assert!(!resolves(&dir));
    }
}
