use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;
use serde::Serialize;
use toml::Value;

/// Whether a project is a plain clone or a fork (has an "upstream" remote).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GitOrigin {
    /// A local-only repo (no origin remote).
    Local,
    /// A plain git clone (has "origin" remote).
    Clone,
    /// A fork (has an "upstream" remote).
    Fork,
}

impl GitOrigin {
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Local => "●",
            Self::Clone => "⊙",
            Self::Fork => "⑂",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Clone => "clone",
            Self::Fork => "fork",
        }
    }
}

/// Git metadata for a project: origin type, owner, repo URL, and current branch.
#[derive(Debug, Clone, Serialize)]
pub struct GitInfo {
    /// Whether this is a clone or a fork.
    pub origin:       GitOrigin,
    /// The current branch name.
    pub branch:       Option<String>,
    /// The GitHub/GitLab owner (e.g. "natepiano").
    pub owner:        Option<String>,
    /// The HTTPS URL to the repository.
    pub url:          Option<String>,
    /// ISO 8601 date of the first commit (inception).
    pub first_commit: Option<String>,
    /// ISO 8601 date of the most recent commit.
    pub last_commit:  Option<String>,
    /// Commits ahead and behind the upstream tracking branch (ahead, behind).
    pub ahead_behind: Option<(usize, usize)>,
}

impl GitInfo {
    /// Detect git info for a project directory.
    pub fn detect(project_dir: &Path) -> Option<Self> {
        if !project_dir.join(".git").exists() {
            return None;
        }

        let remote_output = Command::new("git")
            .args(["remote"])
            .current_dir(project_dir)
            .output()
            .ok()?;
        let remotes = String::from_utf8_lossy(&remote_output.stdout);
        let has_origin = remotes.lines().any(|line| line.trim() == "origin");
        let has_upstream = remotes.lines().any(|line| line.trim() == "upstream");
        let origin = if !has_origin {
            GitOrigin::Local
        } else if has_upstream {
            GitOrigin::Fork
        } else {
            GitOrigin::Clone
        };

        let url_output = Command::new("git")
            .args(["remote", "get-url", "origin"])
            .current_dir(project_dir)
            .output()
            .ok()?;
        let raw_url = String::from_utf8_lossy(&url_output.stdout)
            .trim()
            .to_string();

        let (owner, url) = parse_remote_url(&raw_url);

        let branch = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(project_dir)
            .output()
            .ok()
            .and_then(|o| {
                let b = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if b.is_empty() { None } else { Some(b) }
            });

        let ahead_behind = Command::new("git")
            .args(["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])
            .current_dir(project_dir)
            .output()
            .ok()
            .and_then(|o| {
                let s = String::from_utf8_lossy(&o.stdout);
                let mut parts = s.trim().split('\t');
                let ahead = parts.next()?.parse::<usize>().ok()?;
                let behind = parts.next()?.parse::<usize>().ok()?;
                Some((ahead, behind))
            });

        let first_commit = Command::new("git")
            .args(["log", "--reverse", "--format=%aI", "--diff-filter=A"])
            .current_dir(project_dir)
            .output()
            .ok()
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .next()
                    .filter(|s| !s.is_empty())
                    .map(std::string::ToString::to_string)
            });

        let last_commit = Command::new("git")
            .args(["log", "-1", "--format=%aI"])
            .current_dir(project_dir)
            .output()
            .ok()
            .and_then(|o| {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() { None } else { Some(s) }
            });

        Some(Self {
            origin,
            branch,
            owner,
            url,
            first_commit,
            last_commit,
            ahead_behind,
        })
    }
}

/// Extract the owner and HTTPS URL from a git remote URL.
///
/// Handles:
/// - `https://github.com/owner/repo.git`
/// - `git@github.com:owner/repo.git`
fn parse_remote_url(raw: &str) -> (Option<String>, Option<String>) {
    // SSH: git@github.com:owner/repo.git
    if let Some(after_at) = raw.strip_prefix("git@")
        && let Some((host, path)) = after_at.split_once(':')
    {
        let path = path.strip_suffix(".git").unwrap_or(path);
        let owner = path.split('/').next().map(|s| (*s).to_string());
        let url = format!("https://{host}/{path}");
        return (owner, Some(url));
    }

    // HTTPS: https://github.com/owner/repo.git
    if raw.starts_with("https://") || raw.starts_with("http://") {
        let clean = raw.strip_suffix(".git").unwrap_or(raw);
        // Extract owner from path: https://host/owner/repo
        let owner = clean.split('/').nth(3).map(|s| (*s).to_string());
        return (owner, Some((*clean).to_string()));
    }

    (None, None)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectType {
    Binary,
    Library,
    ProcMacro,
    BuildScript,
}

impl fmt::Display for ProjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary => write!(f, "binary"),
            Self::Library => write!(f, "library"),
            Self::ProcMacro => write!(f, "proc-macro"),
            Self::BuildScript => write!(f, "build-script"),
        }
    }
}

/// A group of examples in a subdirectory, or root-level examples (empty category).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExampleGroup {
    /// Subdirectory name, or empty for root-level examples.
    pub category: String,
    pub names:    Vec<String>,
}

/// Serde default helper that returns `true`.
const fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustProject {
    /// Display path (e.g. `~/rust/bevy`).
    pub path:          String,
    /// Absolute filesystem path for operations that need to access the project on disk.
    #[serde(skip)]
    pub abs_path:      String,
    pub name:          Option<String>,
    pub version:       Option<String>,
    pub description:   Option<String>,
    pub worktree_name: Option<String>,
    /// Whether this project has a `[workspace]` section.
    #[serde(default)]
    pub is_workspace:  bool,
    pub types:         Vec<ProjectType>,
    pub examples:      Vec<ExampleGroup>,
    pub benches:       Vec<String>,
    pub test_count:    usize,
    /// Whether this project is a Rust project (has `Cargo.toml`).
    #[serde(default = "default_true")]
    pub is_rust:       bool,
}

impl RustProject {
    /// Total number of examples across all groups.
    pub fn example_count(&self) -> usize { self.examples.iter().map(|g| g.names.len()).sum() }

    /// Language icon for the project list.
    pub const fn lang_icon(&self) -> &'static str { if self.is_rust { "🦀" } else { "  " } }

    /// Display name for the project list.
    /// Shows `name (worktree_dir)` for worktrees, just `name` otherwise.
    /// Falls back to the last path component for workspace-only projects.
    pub fn display_name(&self) -> String {
        let name = self
            .name
            .as_deref()
            .unwrap_or_else(|| self.path.rsplit('/').next().unwrap_or(&self.path));
        self.worktree_name
            .as_ref()
            .map_or_else(|| name.to_string(), |wt| format!("{name} ({wt})"))
    }
}

pub enum ProjectParseError {
    ReadError(io::Error),
    ParseError(toml::de::Error),
}

impl fmt::Display for ProjectParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadError(e) => write!(f, "read error: {e}"),
            Self::ParseError(e) => write!(f, "parse error: {e}"),
        }
    }
}

impl RustProject {
    pub fn from_cargo_toml(cargo_toml_path: &Path) -> Result<Self, ProjectParseError> {
        let contents =
            std::fs::read_to_string(cargo_toml_path).map_err(ProjectParseError::ReadError)?;
        let table: Value = contents.parse().map_err(ProjectParseError::ParseError)?;

        let project_dir = cargo_toml_path.parent().unwrap_or(cargo_toml_path);

        let path_str = home_relative_path(project_dir);

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

        // A `.git` file (not directory) indicates a git worktree
        let worktree_name = detect_worktree_name(project_dir);

        let is_workspace = table.get("workspace").is_some();
        let types = detect_types(&table, project_dir);
        let examples = collect_examples(&table, project_dir);
        let benches = collect_target_names(&table, project_dir, "bench", "benches");
        let test_count = count_targets(&table, project_dir, "test", "tests");

        let abs_path = project_dir.display().to_string();

        Ok(Self {
            path: path_str,
            abs_path,
            name,
            version,
            description,
            worktree_name,
            is_workspace,
            types,
            examples,
            benches,
            test_count,
            is_rust: true,
        })
    }

    /// Create a project entry for a non-Rust git repository (no `Cargo.toml`).
    pub fn from_git_dir(project_dir: &Path) -> Self {
        let name = project_dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string());
        let path = home_relative_path(project_dir);
        let abs_path = project_dir.display().to_string();
        let worktree_name = detect_worktree_name(project_dir);

        Self {
            path,
            abs_path,
            name,
            version: None,
            description: None,
            worktree_name,
            is_workspace: false,
            types: Vec::new(),
            examples: Vec::new(),
            benches: Vec::new(),
            test_count: 0,
            is_rust: false,
        }
    }

    pub const fn is_workspace(&self) -> bool { self.is_workspace }
}

fn detect_types(table: &Value, project_dir: &Path) -> Vec<ProjectType> {
    let mut types = Vec::new();

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

    if project_dir.join("build.rs").exists() {
        types.push(ProjectType::BuildScript);
    }

    types
}

/// Collect examples grouped by category. Prefers `[[example]]` declarations, falls back to
/// file discovery.
fn collect_examples(table: &Value, project_dir: &Path) -> Vec<ExampleGroup> {
    // Collect from [[example]] entries in Cargo.toml
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
            // Collect .rs files and main.rs subdirs within this category
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
    table: &Value,
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

fn count_targets(table: &Value, project_dir: &Path, toml_key: &str, dir_name: &str) -> usize {
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

    count_rs_files_recursive(&dir)
}

/// Detect if a project directory is inside a git worktree.
/// Walks up the directory tree looking for a `.git` file (not directory).
/// Returns the worktree root directory name as the label.
/// Returns a `~/`-prefixed path if under the home directory, otherwise the absolute path.
/// Returns a `~/`-prefixed path if under the home directory, otherwise the absolute path.
pub fn home_relative_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rel) = path.strip_prefix(&home)
    {
        return format!("~/{}", rel.display());
    }
    path.display().to_string()
}

fn detect_worktree_name(project_dir: &Path) -> Option<String> {
    let mut dir = project_dir;
    loop {
        let git_path = dir.join(".git");
        if git_path.is_file() {
            return dir.file_name().map(|n| n.to_string_lossy().to_string());
        }
        if git_path.is_dir() {
            // Found a real .git directory — not a worktree
            return None;
        }
        dir = dir.parent()?;
    }
}

fn count_rs_files_recursive(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };

    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "rs") {
            count += 1;
        } else if path.is_dir() {
            // Subdirectories can contain examples too (e.g., examples/foo/main.rs)
            // Count the directory as one example if it has a main.rs
            if path.join("main.rs").exists() {
                count += 1;
            }
        }
    }
    count
}
