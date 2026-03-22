use std::fmt;
use std::io;
use std::path::Path;
use std::process::Command;

use serde::Serialize;
use toml::Value;

/// Whether a project is a plain clone or a fork (has an "upstream" remote).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GitOrigin {
    /// A plain git clone (only "origin" remote).
    Clone,
    /// A fork (has an "upstream" remote).
    Fork,
}

impl GitOrigin {
    /// Returns a single-character icon: `⑂` for fork, `⊙` for clone.
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Clone => "⊙",
            Self::Fork => "⑂",
        }
    }

    /// Returns the label: "clone" or "fork".
    pub const fn label(self) -> &'static str {
        match self {
            Self::Clone => "clone",
            Self::Fork => "fork",
        }
    }
}

/// Git metadata for a project: origin type, owner, and repo URL.
#[derive(Debug, Clone, Serialize)]
pub struct GitInfo {
    /// Whether this is a clone or a fork.
    pub origin: GitOrigin,
    /// The GitHub/GitLab owner (e.g. "natepiano").
    pub owner:  Option<String>,
    /// The HTTPS URL to the repository.
    pub url:    Option<String>,
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
        let has_upstream = remotes.lines().any(|line| line.trim() == "upstream");
        let origin = if has_upstream {
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

        Some(Self { origin, owner, url })
    }
}

/// Extract the owner and HTTPS URL from a git remote URL.
///
/// Handles:
/// - `https://github.com/owner/repo.git`
/// - `git@github.com:owner/repo.git`
fn parse_remote_url(raw: &str) -> (Option<String>, Option<String>) {
    // SSH: git@github.com:owner/repo.git
    if let Some(after_at) = raw.strip_prefix("git@") {
        if let Some((host, path)) = after_at.split_once(':') {
            let path = path.strip_suffix(".git").unwrap_or(path);
            let owner = path.split('/').next().map(|s| (*s).to_string());
            let url = format!("https://{host}/{path}");
            return (owner, Some(url));
        }
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectType {
    Workspace,
    Binary,
    Library,
    ProcMacro,
    BuildScript,
}

impl fmt::Display for ProjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Workspace => write!(f, "workspace"),
            Self::Binary => write!(f, "binary"),
            Self::Library => write!(f, "library"),
            Self::ProcMacro => write!(f, "proc-macro"),
            Self::BuildScript => write!(f, "build-script"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RustProject {
    pub path:          String,
    pub name:          Option<String>,
    pub version:       Option<String>,
    pub description:   Option<String>,
    pub types:         Vec<ProjectType>,
    pub example_count: usize,
    pub bench_count:   usize,
    pub test_count:    usize,
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
    pub fn from_cargo_toml(
        cargo_toml_path: &Path,
        scan_root: &Path,
    ) -> Result<Self, ProjectParseError> {
        let contents =
            std::fs::read_to_string(cargo_toml_path).map_err(ProjectParseError::ReadError)?;
        let table: Value = contents.parse().map_err(ProjectParseError::ParseError)?;

        let project_dir = cargo_toml_path.parent().unwrap_or(cargo_toml_path);

        let relative_path = project_dir.strip_prefix(scan_root).unwrap_or(project_dir);
        let path_str = if relative_path == Path::new("") {
            ".".to_string()
        } else {
            relative_path.display().to_string()
        };

        let name = table
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .map(|s| (*s).to_string());

        let version = table
            .get("package")
            .and_then(|p| p.get("version"))
            .map(|v| {
                if v.is_str() {
                    (*v.as_str().unwrap()).to_string()
                } else if v.get("workspace").and_then(|w| w.as_bool()) == Some(true) {
                    "(workspace)".to_string()
                } else {
                    "-".to_string()
                }
            });

        let description = table
            .get("package")
            .and_then(|p| p.get("description"))
            .and_then(|n| n.as_str())
            .map(|s| (*s).to_string());

        let types = detect_types(&table, project_dir);
        let example_count = count_examples(&table, project_dir);
        let bench_count = count_targets(&table, project_dir, "bench", "benches");
        let test_count = count_targets(&table, project_dir, "test", "tests");

        Ok(Self {
            path: path_str,
            name,
            version,
            description,
            types,
            example_count,
            bench_count,
            test_count,
        })
    }

    pub fn is_workspace(&self) -> bool {
        self.types
            .iter()
            .any(|t| matches!(t, ProjectType::Workspace))
    }
}

fn detect_types(table: &Value, project_dir: &Path) -> Vec<ProjectType> {
    let mut types = Vec::new();

    if table.get("workspace").is_some() {
        types.push(ProjectType::Workspace);
    }

    let is_proc_macro = table
        .get("lib")
        .and_then(|lib| lib.get("proc-macro"))
        .and_then(|v| v.as_bool())
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

fn count_examples(table: &Value, project_dir: &Path) -> usize {
    // Count [[example]] entries in Cargo.toml
    let declared = table
        .get("example")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    if declared > 0 {
        return declared;
    }

    // Auto-discover: count .rs files in examples/ directory
    let examples_dir = project_dir.join("examples");
    if !examples_dir.is_dir() {
        return 0;
    }

    count_rs_files_recursive(&examples_dir)
}

fn count_targets(table: &Value, project_dir: &Path, toml_key: &str, dir_name: &str) -> usize {
    let declared = table
        .get(toml_key)
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    if declared > 0 {
        return declared;
    }

    let dir = project_dir.join(dir_name);
    if !dir.is_dir() {
        return 0;
    }

    count_rs_files_recursive(&dir)
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
