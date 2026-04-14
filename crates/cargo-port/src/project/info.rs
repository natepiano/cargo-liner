use super::git::LocalGitState;
use super::submodule::SubmoduleInfo;

/// Visibility state for projects and worktree groups.
/// Progression: `Visible -> Deleted -> Dismissed`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Visibility {
    #[default]
    Visible,
    Deleted,
    Dismissed,
}

/// Whether a worktree's `.git` file points to a valid gitdir.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum WorktreeHealth {
    /// Not a worktree, or health not yet checked.
    #[default]
    Normal,
    /// The `.git` file's gitdir target does not exist on disk.
    Broken,
}

/// GitHub repository metadata fetched from the GitHub API.
///
/// `None` on `ProjectInfo` means the fetch hasn't completed yet.
/// `Some(...)` means the API responded — stars of 0 is a valid value.
#[derive(Clone, Debug)]
pub(crate) struct GitHubInfo {
    pub stars:       u64,
    pub description: Option<String>,
}

/// A single language entry in the language statistics breakdown.
#[derive(Clone, Debug)]
pub(crate) struct LangEntry {
    /// Tokei language name (e.g., "Rust", "C++", "Python").
    pub language:   String,
    pub file_count: usize,
    /// Code lines (tokei's "code" count, excludes comments and blanks).
    pub line_count: usize,
}

/// Per-project language statistics collected by tokei.
///
/// `None` on `ProjectInfo` means the scan hasn't completed yet.
#[derive(Clone, Debug, Default)]
pub(crate) struct LanguageStats {
    /// Sorted by `line_count` descending (dominant language first).
    pub entries: Vec<LangEntry>,
}

/// Shared metadata for all project types (Rust and non-Rust).
///
/// Identity fields (`path`, `name`) live on each project struct directly —
/// they must not be exposed through `info_mut()` to prevent accidental
/// mutation of lookup keys.
#[derive(Clone, Default)]
pub(crate) struct ProjectInfo {
    pub disk_usage_bytes: Option<u64>,
    pub local_git_state:  LocalGitState,
    pub github_info:      Option<GitHubInfo>,
    pub language_stats:   Option<LanguageStats>,
    pub visibility:       Visibility,
    pub worktree_health:  WorktreeHealth,
    pub submodules:       Vec<SubmoduleInfo>,
}
