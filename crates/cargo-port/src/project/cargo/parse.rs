use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::io;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;
use toml::Table;

use super::package::Package;
use super::rust_info::Cargo;
use super::rust_info::RustInfo;
use super::workspace::Workspace;
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

pub enum ProjectParseError {
    Read(io::Error),
    Parse(toml::de::Error),
}

impl Display for ProjectParseError {
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
) -> Result<CargoParseResult, ProjectParseError> {
    let contents = std::fs::read_to_string(cargo_toml_path).map_err(ProjectParseError::Read)?;
    let table: Table = contents.parse().map_err(ProjectParseError::Parse)?;

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
        info: ProjectInfo {
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

/// Create a project entry for a non-Rust git repository (no `Cargo.toml`).
pub(crate) fn from_git_dir(project_dir: &Path) -> NonRustProject {
    let name = project_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string());

    let mut project = NonRustProject::new(AbsolutePath::from(project_dir), name);
    project.info.worktree_health = git::get_worktree_health(project_dir);
    project.worktree_status = git::get_worktree_status(project_dir);
    project
}
