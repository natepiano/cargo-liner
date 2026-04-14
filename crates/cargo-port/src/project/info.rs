use super::git::GitState;
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

/// Shared metadata for all project types (Rust and non-Rust).
///
/// Identity fields (`path`, `name`) live on each project struct directly —
/// they must not be exposed through `info_mut()` to prevent accidental
/// mutation of lookup keys.
#[derive(Clone, Default)]
pub(crate) struct ProjectInfo {
    pub disk_usage_bytes: Option<u64>,
    pub git_state:        GitState,
    pub visibility:       Visibility,
    pub worktree_health:  WorktreeHealth,
    pub submodules:       Vec<SubmoduleInfo>,
}
