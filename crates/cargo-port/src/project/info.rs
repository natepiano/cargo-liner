use super::git::LocalGitState;
use super::git::WorktreeStatus;
use super::submodule::Submodule;
use crate::ci::CiRun;

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

    pub(crate) fn runs(&self) -> &[CiRun] { self.info().map_or(&[], |info| &info.runs) }

    pub(crate) const fn github_total(&self) -> u32 {
        match self {
            Self::Unfetched => 0,
            Self::Loaded(info) => info.github_total,
        }
    }

    pub(crate) const fn is_exhausted(&self) -> bool {
        match self {
            Self::Unfetched => false,
            Self::Loaded(info) => info.exhausted,
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
    pub disk_usage_bytes: Option<u64>,
    pub local_git_state:  LocalGitState,
    pub github_info:      Option<GitHubInfo>,
    pub ci_data:          ProjectCiData,
    pub language_stats:   Option<LanguageStats>,
    pub visibility:       Visibility,
    pub worktree_health:  WorktreeHealth,
    pub worktree_status:  WorktreeStatus,
    pub submodules:       Vec<Submodule>,
}
