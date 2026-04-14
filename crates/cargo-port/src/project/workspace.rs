use std::ops::Deref;
use std::ops::DerefMut;

use super::git::GitInfo;
use super::info::ProjectInfo;
use super::info::Visibility;
use super::info::WorktreeHealth;
use super::member_group::MemberGroup;
use super::package::PackageProject;
use super::paths;
use super::paths::AbsolutePath;
use super::paths::DisplayPath;
use super::paths::PackageName;
use super::paths::RootDirectoryName;
use super::project_fields::ProjectFields;
use super::rust_info::Cargo;
use super::rust_info::RustInfo;
use crate::lint::LintRuns;

/// A Rust workspace project. Contains member groups in addition to the
/// shared `RustInfo` data. Derefs to `RustInfo` for uniform access.
#[derive(Clone)]
pub(crate) struct WorkspaceProject {
    pub(super) path:   AbsolutePath,
    pub(super) name:   Option<String>,
    pub(super) rust:   RustInfo,
    pub(super) groups: Vec<MemberGroup>,
}

impl WorkspaceProject {
    pub(crate) fn new(
        path: AbsolutePath,
        name: Option<String>,
        cargo: Cargo,
        groups: Vec<MemberGroup>,
        vendored: Vec<PackageProject>,
        worktree_name: Option<String>,
        worktree_primary_abs_path: Option<AbsolutePath>,
    ) -> Self {
        Self {
            path,
            name,
            rust: RustInfo {
                info: ProjectInfo::default(),
                cargo,
                vendored,
                worktree_name,
                worktree_primary_abs_path,
                lint_runs: LintRuns::default(),
            },
            groups,
        }
    }

    pub(crate) fn groups(&self) -> &[MemberGroup] { &self.groups }

    pub(crate) const fn groups_mut(&mut self) -> &mut Vec<MemberGroup> { &mut self.groups }

    pub(crate) fn has_members(&self) -> bool { self.groups.iter().any(|g| !g.members().is_empty()) }

    /// Cargo package name when present, otherwise directory leaf.
    pub(crate) fn package_name(&self) -> PackageName {
        PackageName(self.name.as_deref().map_or_else(
            || paths::directory_leaf(self.path.as_path()),
            str::to_string,
        ))
    }
}

impl ProjectFields for WorkspaceProject {
    fn path(&self) -> &AbsolutePath { &self.path }

    fn name(&self) -> Option<&str> { self.name.as_deref() }

    fn visibility(&self) -> Visibility { self.rust.info.visibility }

    fn worktree_health(&self) -> WorktreeHealth { self.rust.info.worktree_health }

    fn disk_usage_bytes(&self) -> Option<u64> { self.rust.info.disk_usage_bytes }

    fn git_info(&self) -> Option<&GitInfo> { self.rust.info.local_git_state.info() }

    fn info(&self) -> &ProjectInfo { &self.rust.info }

    fn info_mut(&mut self) -> &mut ProjectInfo { &mut self.rust.info }

    fn display_path(&self) -> DisplayPath { self.path.display_path() }

    fn root_directory_name(&self) -> RootDirectoryName {
        RootDirectoryName(paths::directory_leaf(self.path.as_path()))
    }
}

impl Deref for WorkspaceProject {
    type Target = RustInfo;

    fn deref(&self) -> &RustInfo { &self.rust }
}

impl DerefMut for WorkspaceProject {
    fn deref_mut(&mut self) -> &mut RustInfo { &mut self.rust }
}
