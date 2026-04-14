use super::git::GitInfo;
use super::info::ProjectInfo;
use super::info::Visibility;
use super::info::WorktreeHealth;
use super::paths::AbsolutePath;
use super::paths::DisplayPath;
use super::paths::RootDirectoryName;

/// Shared field access for all project types.
///
/// Implemented by `WorkspaceProject`, `PackageProject`, and `NonRustProject`.
/// Enables generic iteration and ensures all project types expose the same
/// identity and metadata surface.
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
