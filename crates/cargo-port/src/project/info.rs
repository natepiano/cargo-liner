use super::git::LocalGitState;
use super::git::Submodule;
use crate::ci::CiRun;
use crate::ci::OwnerRepo;

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

/// Persisted pull-request metadata for a single GitHub repository.
#[derive(Clone, Default)]
pub(crate) enum ProjectPrData {
    #[default]
    Unfetched,
    Loading(Option<ProjectPrInfo>),
    Loaded(ProjectPrInfo),
    Unavailable(ProjectPrUnavailable),
}

impl ProjectPrData {
    pub(crate) const fn info(&self) -> Option<&ProjectPrInfo> {
        match self {
            Self::Loaded(info) => Some(info),
            Self::Loading(stale) => stale.as_ref(),
            Self::Unavailable(unavailable) => unavailable.stale.as_ref(),
            Self::Unfetched => None,
        }
    }

    pub(crate) const fn needs_fetch(&self) -> bool {
        matches!(self, Self::Unfetched | Self::Unavailable(_))
    }
}

#[derive(Clone)]
pub(crate) struct ProjectPrInfo {
    pub open:           Vec<PullRequestInfo>,
    pub default_branch: String,
    pub fetched_at:     String,
    pub completeness:   PullRequestCompleteness,
    pub viewer_login:   String,
    pub owner_repo:     OwnerRepo,
}

#[derive(Clone)]
pub(crate) struct ProjectPrUnavailable {
    pub reason:     PullRequestUnavailableReason,
    pub stale:      Option<ProjectPrInfo>,
    pub fetched_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PullRequestInfo {
    pub number:     u32,
    pub title:      String,
    pub url:        String,
    pub state:      PullRequestState,
    pub head:       String,
    pub head_owner: Option<String>,
    pub head_repo:  Option<String>,
    pub base:       String,
}

impl PullRequestInfo {
    pub(crate) fn branch_label(&self, base_default: &str) -> String {
        let head = self.head_owner.as_ref().map_or_else(
            || self.head.clone(),
            |owner| format!("{owner}:{}", self.head),
        );
        if self.base == base_default {
            head
        } else {
            format!("{head} -> {}", self.base)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PullRequestState {
    Draft,
    ChangesRequested,
    ChecksFailing,
    Blocked,
    Behind,
    ReviewRequired,
    Approved,
    Ready,
    Unknown,
}

impl PullRequestState {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::ChangesRequested => "changes",
            Self::ChecksFailing => "checks",
            Self::Blocked => "blocked",
            Self::Behind => "behind",
            Self::ReviewRequired => "review",
            Self::Approved => "approved",
            Self::Ready => "ready",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PullRequestUnavailableReason {
    Unauthenticated,
    RateLimited,
    Network,
    Forbidden,
    RepositoryMissing,
    GraphQlError,
    IncompletePagination,
}

impl PullRequestUnavailableReason {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Unauthenticated => "unauthenticated",
            Self::RateLimited => "rate limited",
            Self::Network => "network unavailable",
            Self::Forbidden => "forbidden",
            Self::RepositoryMissing => "repository missing",
            Self::GraphQlError => "github query failed",
            Self::IncompletePagination => "incomplete results",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PullRequestGoneReason {
    Merged { base: String },
    Closed,
    Missing,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PullRequestCompleteness {
    Complete,
    Truncated { shown: usize },
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

/// Per-project test-function counts collected by a source scan that
/// matches `#[test]`-family attributes in `.rs` files, bucketed by the
/// directory the file lives in.
///
/// `None` on `ProjectInfo` means the scan hasn't completed yet. The
/// counts are a heuristic: they match a fixed attribute set (see
/// `scan::test_counts`) and do not include doctests.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TestCounts {
    /// `#[test]`-family functions under `src/` (unit tests).
    pub unit:        usize,
    /// `#[test]`-family functions under `tests/` (integration tests).
    pub integration: usize,
}

impl TestCounts {
    /// Sum two count snapshots — used to fold a workspace's members into
    /// the workspace-level total.
    pub(crate) const fn merged(self, other: Self) -> Self {
        Self {
            unit:        self.unit + other.unit,
            integration: self.integration + other.integration,
        }
    }
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
    pub test_counts:           Option<TestCounts>,
    pub visibility:            Visibility,
    pub worktree_health:       WorktreeHealth,
    pub submodules:            Vec<Submodule>,
}

impl ProjectInfo {
    #[cfg(test)]
    #[expect(dead_code, reason = "Reserved for later-stage test helpers")]
    pub(crate) fn for_tests() -> Self { Self::default() }
}
