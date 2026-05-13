use std::ops::Deref;
use std::ops::DerefMut;

use super::rust_info::RustInfo;
use crate::project::git::CheckoutInfo;
use crate::project::git::WorktreeStatus;
use crate::project::info::ProjectInfo;
use crate::project::info::Visibility;
use crate::project::info::WorktreeHealth;
use crate::project::member_group::MemberGroup;
use crate::project::paths;
use crate::project::paths::AbsolutePath;
use crate::project::paths::DisplayPath;
use crate::project::paths::PackageName;
use crate::project::paths::RootDirectoryName;
use crate::project::project_fields::ProjectFields;

/// A Rust workspace project. Contains member groups in addition to the
/// shared `RustInfo` data. Implements `Deref<Target = RustInfo>` for uniform
/// access.
///
/// Construct via struct literal — all fields default to empty/none, so tests
/// can use `..Default::default()`.
#[derive(Clone, Default)]
pub struct Workspace {
    pub path:            AbsolutePath,
    pub name:            Option<String>,
    pub worktree_status: WorktreeStatus,
    pub rust:            RustInfo,
    pub groups:          Vec<MemberGroup>,
}

impl Workspace {
    pub fn groups(&self) -> &[MemberGroup] { &self.groups }

    pub const fn groups_mut(&mut self) -> &mut Vec<MemberGroup> { &mut self.groups }

    pub fn has_members(&self) -> bool { self.groups.iter().any(|g| !g.members().is_empty()) }

    /// Cargo package name when present, otherwise directory leaf.
    pub fn package_name(&self) -> PackageName {
        PackageName(self.name.as_deref().map_or_else(
            || paths::directory_leaf(self.path.as_path()),
            str::to_string,
        ))
    }
}

impl ProjectFields for Workspace {
    fn path(&self) -> &AbsolutePath { &self.path }

    fn name(&self) -> Option<&str> { self.name.as_deref() }

    fn visibility(&self) -> Visibility { self.rust.info.visibility }

    fn worktree_health(&self) -> WorktreeHealth { self.rust.info.worktree_health }

    fn disk_usage_bytes(&self) -> Option<u64> { self.rust.info.disk_usage_bytes }

    fn git_info(&self) -> Option<&CheckoutInfo> { self.rust.info.local_git_state.info() }

    fn info(&self) -> &ProjectInfo { &self.rust.info }

    fn display_path(&self) -> DisplayPath { self.path.display_path() }

    fn root_directory_name(&self) -> RootDirectoryName {
        RootDirectoryName(paths::directory_leaf(self.path.as_path()))
    }

    fn worktree_status(&self) -> &WorktreeStatus { &self.worktree_status }

    fn crates_io_name(&self) -> Option<&str> {
        self.name
            .as_deref()
            .filter(|_| self.rust.cargo.publishable())
    }
}

impl Deref for Workspace {
    type Target = RustInfo;

    fn deref(&self) -> &RustInfo { &self.rust }
}

impl DerefMut for Workspace {
    fn deref_mut(&mut self) -> &mut RustInfo { &mut self.rust }
}
