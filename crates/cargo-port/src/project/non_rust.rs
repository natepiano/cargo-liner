use std::ops::Deref;
use std::ops::DerefMut;
use std::path::Path;
use std::path::PathBuf;

use super::git::GitInfo;
use super::info::ProjectInfo;
use super::info::Visibility;
use super::info::WorktreeHealth;
use super::paths;
use super::paths::AbsolutePath;
use super::paths::DisplayPath;
use super::paths::RootDirectoryName;
use super::project_fields::ProjectFields;

/// A non-Rust project (git repo without `Cargo.toml`).
/// Derefs to `ProjectInfo` for uniform metadata access.
#[derive(Clone)]
pub(crate) struct NonRustProject {
    pub(super) path: AbsolutePath,
    pub(super) name: Option<String>,
    pub(super) info: ProjectInfo,
}

impl NonRustProject {
    pub(crate) fn new(path: PathBuf, name: Option<String>) -> Self {
        Self {
            path: path.into(),
            name,
            info: ProjectInfo::default(),
        }
    }
}

impl ProjectFields for NonRustProject {
    fn path(&self) -> &Path { self.path.as_path() }

    fn name(&self) -> Option<&str> { self.name.as_deref() }

    fn visibility(&self) -> Visibility { self.info.visibility }

    fn worktree_health(&self) -> WorktreeHealth { self.info.worktree_health }

    fn disk_usage_bytes(&self) -> Option<u64> { self.info.disk_usage_bytes }

    fn git_info(&self) -> Option<&GitInfo> { self.info.git_info.as_ref() }

    fn info(&self) -> &ProjectInfo { &self.info }

    fn info_mut(&mut self) -> &mut ProjectInfo { &mut self.info }

    fn display_path(&self) -> DisplayPath { self.path.display_path() }

    fn root_directory_name(&self) -> RootDirectoryName {
        RootDirectoryName(paths::directory_leaf(self.path.as_path()))
    }
}

impl Deref for NonRustProject {
    type Target = ProjectInfo;

    fn deref(&self) -> &ProjectInfo { &self.info }
}

impl DerefMut for NonRustProject {
    fn deref_mut(&mut self) -> &mut ProjectInfo { &mut self.info }
}
