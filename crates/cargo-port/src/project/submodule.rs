use std::path::Path;

use super::info::ProjectInfo;
use super::paths::AbsolutePath;

/// Metadata for a git submodule nested inside a project.
#[derive(Clone)]
pub(crate) struct Submodule {
    /// The submodule name from `.gitmodules` (e.g. `glTF-IBL-Sampler`).
    pub name:          String,
    /// Absolute path on disk.
    pub path:          AbsolutePath,
    /// Relative path within the parent repo (the `path =` value).
    pub relative_path: String,
    /// Remote URL from `.gitmodules`.
    pub url:           Option<String>,
    /// Tracking branch from `.gitmodules` (if specified).
    pub branch:        Option<String>,
    /// Pinned commit SHA from `git ls-tree HEAD`.
    pub commit:        Option<String>,
    /// Shared metadata (git info, disk usage, etc.) — populated by
    /// background messages through the standard `at_path_mut` lookup.
    pub info:          ProjectInfo,
}

/// Parse `.gitmodules` and resolve pinned commits for all submodules.
pub(crate) fn detect_submodules(project_root: &Path) -> Vec<Submodule> {
    let gitmodules_path = project_root.join(".gitmodules");
    let Ok(content) = std::fs::read_to_string(&gitmodules_path) else {
        return Vec::new();
    };

    let mut entries = parse_gitmodules(&content);
    if entries.is_empty() {
        return Vec::new();
    }

    // Resolve absolute paths and pinned commits.
    let commits = ls_tree_submodule_commits(project_root);
    for entry in &mut entries {
        entry.path = AbsolutePath::from(project_root.join(&entry.relative_path));
        if let Some(sha) = commits.get(&entry.relative_path) {
            entry.commit = Some(sha.clone());
        }
    }

    entries
}

/// Parse the INI-like `.gitmodules` format into partially filled entries.
fn parse_gitmodules(content: &str) -> Vec<Submodule> {
    let mut entries: Vec<Submodule> = Vec::new();
    let mut current: Option<Submodule> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(header) = trimmed
            .strip_prefix("[submodule \"")
            .and_then(|s| s.strip_suffix("\"]"))
        {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            current = Some(Submodule {
                name:          header.to_string(),
                path:          "/".into(),
                relative_path: String::new(),
                url:           None,
                branch:        None,
                commit:        None,
                info:          ProjectInfo::default(),
            });
        } else if let Some(ref mut entry) = current
            && let Some((key, value)) = parse_key_value(trimmed)
        {
            match key {
                "path" => entry.relative_path = value.to_string(),
                "url" => entry.url = Some(value.to_string()),
                "branch" => entry.branch = Some(value.to_string()),
                _ => {},
            }
        }
    }
    if let Some(entry) = current {
        entries.push(entry);
    }

    entries
}

/// Extract `key = value` from a trimmed config line.
fn parse_key_value(line: &str) -> Option<(&str, &str)> {
    let (key, rest) = line.split_once('=')?;
    Some((key.trim(), rest.trim()))
}

/// Run `git ls-tree HEAD` to get pinned commit SHAs for submodule paths.
///
/// Returns a map of `relative_path` → short SHA.
fn ls_tree_submodule_commits(project_root: &Path) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let output = std::process::Command::new("git")
        .args(["ls-tree", "HEAD"])
        .current_dir(project_root)
        .output();
    let Ok(output) = output else {
        return map;
    };
    if !output.status.success() {
        return map;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        // Format: "160000 commit <sha>\t<path>"
        if !line.starts_with("160000") {
            continue;
        }
        let Some((meta, path)) = line.split_once('\t') else {
            continue;
        };
        // meta = "160000 commit <sha>"
        let sha = meta
            .rsplit_once(' ')
            .map(|(_, sha)| &sha[..sha.len().min(8)]);
        if let Some(sha) = sha {
            map.insert(path.to_string(), sha.to_string());
        }
    }
    map
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_submodule() {
        let content = r#"[submodule "glTF-IBL-Sampler"]
	path = glTF-IBL-Sampler
	url = https://github.com/pcwalton/glTF-IBL-Sampler.git
	branch = lite
"#;
        let entries = parse_gitmodules(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "glTF-IBL-Sampler");
        assert_eq!(entries[0].relative_path, "glTF-IBL-Sampler");
        assert_eq!(
            entries[0].url.as_deref(),
            Some("https://github.com/pcwalton/glTF-IBL-Sampler.git")
        );
        assert_eq!(entries[0].branch.as_deref(), Some("lite"));
    }

    #[test]
    fn parse_multiple_submodules() {
        let content = r#"[submodule "lib-a"]
	path = vendor/lib-a
	url = https://example.com/lib-a.git
[submodule "lib-b"]
	path = vendor/lib-b
	url = https://example.com/lib-b.git
	branch = main
"#;
        let entries = parse_gitmodules(content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "lib-a");
        assert_eq!(entries[0].relative_path, "vendor/lib-a");
        assert_eq!(entries[1].name, "lib-b");
        assert_eq!(entries[1].branch.as_deref(), Some("main"));
    }

    #[test]
    fn parse_empty_returns_empty() {
        let entries = parse_gitmodules("");
        assert!(entries.is_empty());
    }
}
