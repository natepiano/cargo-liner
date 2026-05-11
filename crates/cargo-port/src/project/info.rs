use super::git::LocalGitState;
use super::submodule::Submodule;
use crate::ci::CiRun;

// Per-repo metadata (`github_info`, `ci_data`, ...) lives on
// `ProjectEntry::git_repo`, not here. Submodules in particular get neither
// — `is_submodule_path` suppresses fetches at the parent's level, so
// per-submodule storage would be dead.

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

/// Persisted CI metadata for a single project hierarchy node.
#[derive(Clone)]
pub(crate) struct ProjectCiInfo {
    pub runs:         Vec<CiRun>,
    pub github_total: u32,
    pub exhausted:    bool,
}

/// Hierarchy-backed CI ownership. `Unfetched` means we have not resolved CI
/// data for this project yet.
#[derive(Clone, Default)]
pub(crate) enum ProjectCiData {
    #[default]
    Unfetched,
    Loaded(ProjectCiInfo),
}

impl ProjectCiData {
    pub(crate) const fn info(&self) -> Option<&ProjectCiInfo> {
        match self {
            Self::Unfetched => None,
            Self::Loaded(info) => Some(info),
        }
    }

    pub(crate) const fn github_total(&self) -> u32 {
        match self {
            Self::Unfetched => 0,
            Self::Loaded(info) => info.github_total,
        }
    }
}

/// A single language entry in the language statistics breakdown.
#[derive(Clone, Debug)]
pub(crate) struct LangEntry {
    /// Tokei language name (e.g., "Rust", "C++", "Python").
    pub language: String,
    pub files:    usize,
    pub code:     usize,
    pub comments: usize,
    pub blanks:   usize,
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
    pub disk_usage_bytes:      Option<u64>,
    /// Bytes rooted at this project's path that do **not** live inside
    /// any `target/` subtree (source, docs, .git, etc.). Populated by
    /// the scan walker in a single pass alongside `disk_usage_bytes`.
    /// Step 5's detail-pane breakdown renders this as the "non-target"
    /// portion; the sum `in_project_non_target + in_project_target`
    /// equals `disk_usage_bytes` for every owner (target is in-tree)
    /// and stays smaller for a sharer (its `in_project_target == 0`
    /// because the real target lives elsewhere).
    pub in_project_non_target: Option<u64>,
    /// Bytes rooted at this project's path that live inside any
    /// `target/` subtree. Zero for sharers whose workspace redirects
    /// the target dir out-of-tree (e.g. via `CARGO_TARGET_DIR`).
    pub in_project_target:     Option<u64>,
    pub local_git_state:       LocalGitState,
    pub language_stats:        Option<LanguageStats>,
    pub visibility:            Visibility,
    pub worktree_health:       WorktreeHealth,
    pub submodules:            Vec<Submodule>,
}

impl ProjectInfo {
    #[cfg(test)]
    #[expect(dead_code, reason = "Reserved for later-stage test helpers")]
    pub(crate) fn for_tests() -> Self { Self::default() }
}
