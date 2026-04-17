use super::git::GitInfo;
use super::info::ProjectInfo;
use super::info::Visibility;
use super::info::WorktreeHealth;
use super::paths::AbsolutePath;
use super::paths::DisplayPath;
use super::paths::RootDirectoryName;

/// Read-only access shared by all concrete project-list nodes.
///
/// This is the minimal contract needed to treat a path-backed list entry as
/// “real” in generic code: once a type exposes a path and `ProjectInfo`, it
/// automatically participates in shared disk/git/visibility logic instead of
/// each caller deciding which metadata to respect.
pub(crate) trait ProjectListEntry {
    fn path(&self) -> &AbsolutePath;
    fn info(&self) -> &ProjectInfo;
}

/// Shared field access for project types that also expose naming and mutation
/// helpers beyond the basic project-list entry contract.
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
}

impl<T: ProjectFields + ?Sized> ProjectListEntry for T {
    fn path(&self) -> &AbsolutePath { ProjectFields::path(self) }

    fn info(&self) -> &ProjectInfo { ProjectFields::info(self) }
}
