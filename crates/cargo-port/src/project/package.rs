use std::ops::Deref;
use std::ops::DerefMut;

use super::git::GitInfo;
use super::info::ProjectInfo;
use super::info::Visibility;
use super::info::WorktreeHealth;
use super::paths;
use super::paths::AbsolutePath;
use super::paths::DisplayPath;
use super::paths::PackageName;
use super::paths::RootDirectoryName;
use super::project_fields::ProjectFields;
use super::rust_info::Cargo;
use super::rust_info::RustInfo;
use crate::lint::LintRuns;

/// A standalone Rust package project. Derefs to `RustInfo` for uniform access.
#[derive(Clone)]
pub(crate) struct PackageProject {
    pub(super) path: AbsolutePath,
    pub(super) name: Option<String>,
    pub(super) rust: RustInfo,
}

impl PackageProject {
    pub(crate) fn new(
        path: AbsolutePath,
        name: Option<String>,
        cargo: Cargo,
        vendored: Vec<Self>,
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
        }
    }

    /// Cargo package name when present, otherwise directory leaf.
    pub(crate) fn package_name(&self) -> PackageName {
        PackageName(self.name.as_deref().map_or_else(
            || paths::directory_leaf(self.path.as_path()),
            str::to_string,
        ))
    }

    /// Language icon for the project list.
    pub(crate) const fn lang_icon() -> &'static str { "\u{1f980}" }
}

impl ProjectFields for PackageProject {
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

impl Deref for PackageProject {
    type Target = RustInfo;

    fn deref(&self) -> &RustInfo { &self.rust }
}

impl DerefMut for PackageProject {
    fn deref_mut(&mut self) -> &mut RustInfo { &mut self.rust }
}
