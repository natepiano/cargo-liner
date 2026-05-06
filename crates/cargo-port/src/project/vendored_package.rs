use super::git::CheckoutInfo;
use super::git::WorktreeStatus;
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

/// A crate vendored under a parent Rust project.
///
/// Distinct from [`Package`](super::package::Package) because vendored crates do
/// not own lint state and cannot themselves have nested vendored children.
/// Keeping this as its own type makes those invariants structural rather than
/// relying on convention.
#[derive(Clone, Default)]
pub(crate) struct VendoredPackage {
    pub(crate) path:             AbsolutePath,
    pub(crate) name:             Option<String>,
    pub(crate) worktree_status:  WorktreeStatus,
    pub(crate) info:             ProjectInfo,
    pub(crate) cargo:            Cargo,
    pub(crate) crates_version:   Option<String>,
    pub(crate) crates_downloads: Option<u64>,
}

impl VendoredPackage {
    pub(crate) fn package_name(&self) -> PackageName {
        PackageName(self.name.as_deref().map_or_else(
            || paths::directory_leaf(self.path.as_path()),
            str::to_string,
        ))
    }

    pub(crate) fn crates_version(&self) -> Option<&str> { self.crates_version.as_deref() }

    pub(crate) const fn crates_downloads(&self) -> Option<u64> { self.crates_downloads }

    pub(crate) fn set_crates_io(&mut self, version: String, downloads: u64) {
        self.crates_version = Some(version);
        self.crates_downloads = Some(downloads);
    }
}

impl ProjectFields for VendoredPackage {
    fn path(&self) -> &AbsolutePath { &self.path }

    fn name(&self) -> Option<&str> { self.name.as_deref() }

    fn visibility(&self) -> Visibility { self.info.visibility }

    fn worktree_health(&self) -> WorktreeHealth { self.info.worktree_health }

    fn disk_usage_bytes(&self) -> Option<u64> { self.info.disk_usage_bytes }

    fn git_info(&self) -> Option<&CheckoutInfo> { self.info.local_git_state.info() }

    fn info(&self) -> &ProjectInfo { &self.info }

    fn display_path(&self) -> DisplayPath { self.path.display_path() }

    fn root_directory_name(&self) -> RootDirectoryName {
        RootDirectoryName(paths::directory_leaf(self.path.as_path()))
    }

    fn worktree_status(&self) -> &WorktreeStatus { &self.worktree_status }

    fn crates_io_name(&self) -> Option<&str> {
        self.name.as_deref().filter(|_| self.cargo.publishable())
    }
}
