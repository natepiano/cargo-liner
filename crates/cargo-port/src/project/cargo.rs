use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;
use toml::Table;
use toml::Value;

use super::git;
use super::info::ProjectInfo;
use super::member_group;
use super::non_rust::NonRustProject;
use super::package::Package;
use super::paths::AbsolutePath;
use super::project_fields::ProjectFields;
use super::rust_info::Cargo;
use super::rust_info::RustInfo;
use super::workspace::Workspace;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ProjectType {
    Workspace,
    Binary,
    Library,
    ProcMacro,
}

impl fmt::Display for ProjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

pub(crate) enum ProjectParseError {
    Read(io::Error),
    Parse(toml::de::Error),
}

impl fmt::Display for ProjectParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

    let version = table
        .get("package")
        .and_then(|p| p.get("version"))
        .map(|v| {
            v.as_str().map_or_else(
                || {
                    if v.get("workspace").and_then(Value::as_bool) == Some(true) {
                        "(workspace)".to_string()
                    } else {
                        "-".to_string()
                    }
                },
                |s| (*s).to_string(),
            )
        });

    let description = table
        .get("package")
        .and_then(|p| p.get("description"))
        .and_then(|n| n.as_str())
        .map(|s| (*s).to_string());

    let worktree_status = git::get_worktree_status(project_dir);
    let worktree_health = git::get_worktree_health(project_dir);

    let publishable = match table.get("package").and_then(|p| p.get("publish")) {
        None => true,
        Some(v) if v.as_bool() == Some(false) => false,
        Some(v) => v.as_array().is_none_or(|arr| !arr.is_empty()),
    };

    let types = get_types(&table, project_dir);
    let examples = collect_examples(&table, project_dir);
    let benches = collect_target_names(&table, project_dir, "bench", "benches");
    let test_count = count_targets(&table, project_dir, "test", "tests");

    let cargo = Cargo {
        version,
        description,
        types,
        examples,
        benches,
        test_count,
        publishable,
    };

    let rust = RustInfo {
        info: ProjectInfo {
            worktree_health,
            ..ProjectInfo::default()
        },
        cargo,
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
    project.info_mut().worktree_health = git::get_worktree_health(project_dir);
    project.worktree_status = git::get_worktree_status(project_dir);
    project
}

fn get_types(table: &Table, project_dir: &Path) -> Vec<ProjectType> {
    let mut types = Vec::new();

    if table.get("workspace").is_some() {
        types.push(ProjectType::Workspace);
    }

    let is_proc_macro = table
        .get("lib")
        .and_then(|lib| lib.get("proc-macro"))
        .and_then(Value::as_bool)
        == Some(true);

    if is_proc_macro {
        types.push(ProjectType::ProcMacro);
    } else {
        let has_lib_section = table.get("lib").is_some();
        let has_lib_rs = project_dir.join("src/lib.rs").exists();
        if has_lib_section || has_lib_rs {
            types.push(ProjectType::Library);
        }
    }

    let has_bin_section = table.get("bin").is_some();
    let has_main_rs = project_dir.join("src/main.rs").exists();
    if has_bin_section || has_main_rs {
        types.push(ProjectType::Binary);
    }

    types
}

/// Collect examples grouped by category. Prefers `[[example]]` declarations, falls back to
/// file discovery.
fn collect_examples(table: &Table, project_dir: &Path) -> Vec<ExampleGroup> {
    // Collect from `[[example]]` entries in `Cargo.toml`
    if let Some(arr) = table.get("example").and_then(|v| v.as_array())
        && !arr.is_empty()
    {
        let mut groups: HashMap<String, Vec<String>> = HashMap::new();
        for entry in arr {
            let name = entry
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_string();
            if name.is_empty() {
                continue;
            }
            // Derive category from path: "examples/2d/foo.rs" -> "2d"
            let category = entry
                .get("path")
                .and_then(|p| p.as_str())
                .and_then(|p| {
                    let parts: Vec<&str> = p.split('/').collect();
                    // "examples/category/file.rs" -> category
                    if parts.len() >= 3 {
                        Some(parts[1].to_string())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            groups.entry(category).or_default().push(name);
        }
        return build_sorted_groups(groups);
    }

    // Auto-discover from examples/ directory
    let examples_dir = project_dir.join("examples");
    if !examples_dir.is_dir() {
        return Vec::new();
    }

    discover_examples_grouped(&examples_dir)
}

fn build_sorted_groups(
    mut groups: std::collections::HashMap<String, Vec<String>>,
) -> Vec<ExampleGroup> {
    let mut result: Vec<ExampleGroup> = groups
        .drain()
        .map(|(category, mut names)| {
            names.sort();
            ExampleGroup { category, names }
        })
        .collect();
    // Root-level first, then alphabetically by category
    result.sort_by(|a, b| {
        let a_root = a.category.is_empty();
        let b_root = b.category.is_empty();
        match (a_root, b_root) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.category.cmp(&b.category),
        }
    });
    result
}

/// Auto-discover examples from a directory, grouping by subdirectory.
fn discover_examples_grouped(examples_dir: &Path) -> Vec<ExampleGroup> {
    let Ok(entries) = std::fs::read_dir(examples_dir) else {
        return Vec::new();
    };

    let mut groups: HashMap<String, Vec<String>> = HashMap::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "rs") {
            if let Some(stem) = path.file_stem() {
                groups
                    .entry(String::new())
                    .or_default()
                    .push(stem.to_string_lossy().to_string());
            }
        } else if path.is_dir() {
            let dir_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            // Collect `.rs` files and `main.rs` subdirs within this category
            if let Ok(sub_entries) = std::fs::read_dir(&path) {
                for sub in sub_entries.flatten() {
                    let sub_path = sub.path();
                    if sub_path.is_file() && sub_path.extension().is_some_and(|e| e == "rs") {
                        if let Some(stem) = sub_path.file_stem() {
                            groups
                                .entry(dir_name.clone())
                                .or_default()
                                .push(stem.to_string_lossy().to_string());
                        }
                    } else if sub_path.is_dir()
                        && sub_path.join("main.rs").exists()
                        && let Some(name) = sub_path.file_name()
                    {
                        groups
                            .entry(dir_name.clone())
                            .or_default()
                            .push(name.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    build_sorted_groups(groups)
}

/// Collect target names (e.g. benches). Prefers `[[toml_key]]` declarations, falls back to
/// file discovery in `dir_name/`.
fn collect_target_names(
    table: &Table,
    project_dir: &Path,
    toml_key: &str,
    dir_name: &str,
) -> Vec<String> {
    if let Some(arr) = table.get(toml_key).and_then(|v| v.as_array())
        && !arr.is_empty()
    {
        let mut names: Vec<String> = arr
            .iter()
            .filter_map(|entry| {
                entry
                    .get("name")
                    .and_then(|n| n.as_str())
                    .map(std::string::ToString::to_string)
            })
            .collect();
        names.sort();
        return names;
    }

    let dir = project_dir.join(dir_name);
    if !dir.is_dir() {
        return Vec::new();
    }

    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "rs") {
            if let Some(stem) = path.file_stem() {
                names.push(stem.to_string_lossy().to_string());
            }
        } else if path.is_dir()
            && path.join("main.rs").exists()
            && let Some(name) = path.file_name()
        {
            names.push(name.to_string_lossy().to_string());
        }
    }
    names.sort();
    names
}

fn count_targets(table: &Table, project_dir: &Path, toml_key: &str, dir_name: &str) -> usize {
    let declared = table
        .get(toml_key)
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len);

    if declared > 0 {
        return declared;
    }

    let dir = project_dir.join(dir_name);
    if !dir.is_dir() {
        return 0;
    }

    member_group::count_rs_files_recursive(&dir)
}
