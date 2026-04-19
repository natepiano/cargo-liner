use super::git::GitInfo;
use super::git::WorktreeStatus;
use super::info::ProjectInfo;
use super::info::Visibility;
use super::info::WorktreeHealth;
use super::paths::AbsolutePath;
use super::paths::DisplayPath;
use super::paths::RootDirectoryName;

/// Shared field access for every concrete project-list node.
///
/// Once a type implements this trait it participates in all generic
/// disk/git/visibility/enrichment logic — there is no looser tier.
pub(crate) trait ProjectFields {
    fn path(&self) -> &AbsolutePath;
    fn name(&self) -> Option<&str>;
    fn visibility(&self) -> Visibility;
    fn worktree_health(&self) -> WorktreeHealth;
    fn disk_usage_bytes(&self) -> Option<u64>;
    fn git_info(&self) -> Option<&GitInfo>;
    fn info(&self) -> &ProjectInfo;
    fn info_mut(&mut self) -> &mut ProjectInfo;
    fn display_path(&self) -> DisplayPath;
    fn root_directory_name(&self) -> RootDirectoryName;

    /// Git worktree status (not-git / primary / linked). Lives as a
    /// top-level identity field on each project type — not inside
    /// `ProjectInfo` — so `handle_project_refreshed` cannot accidentally
    /// overwrite the freshly-detected value when copying runtime data
    /// across a refresh.
    fn worktree_status(&self) -> &WorktreeStatus;

    /// Crates.io package name to query, when the entry corresponds to a
    /// publishable crate. Default `None` — opt in by overriding.
    fn crates_io_name(&self) -> Option<&str> { None }
}
