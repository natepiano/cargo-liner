use std::path::Path;
use std::path::PathBuf;

use super::git::GitInfo;
use super::info::ProjectInfo;
use super::info::Visibility;
use super::info::WorktreeHealth;
use super::package::PackageProject;
use super::paths::DisplayPath;
use super::paths::RootDirectoryName;
use super::project_fields::ProjectFields;
use super::workspace::WorkspaceProject;

/// A Rust project — either a workspace or a standalone package.
///
/// Delegation methods forward to the concrete type via 2-arm matches.
/// For kind-specific access (e.g. `.groups()`), match on the variant.
#[derive(Clone)]
pub(crate) enum RustProject {
    Workspace(WorkspaceProject),
    Package(PackageProject),
}

impl RustProject {
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Workspace(ws) => ws.path(),
            Self::Package(pkg) => pkg.path(),
        }
    }

    pub(crate) fn name(&self) -> Option<&str> {
        match self {
            Self::Workspace(ws) => ws.name(),
            Self::Package(pkg) => pkg.name(),
        }
    }

    pub(crate) fn worktree_name(&self) -> Option<&str> {
        match self {
            Self::Workspace(ws) => ws.worktree_name(),
            Self::Package(pkg) => pkg.worktree_name(),
        }
    }

    pub(crate) fn worktree_primary_abs_path(&self) -> Option<&Path> {
        match self {
            Self::Workspace(ws) => ws.worktree_primary_abs_path(),
            Self::Package(pkg) => pkg.worktree_primary_abs_path(),
        }
    }

    pub(crate) fn display_path(&self) -> DisplayPath {
        match self {
            Self::Workspace(ws) => ws.display_path(),
            Self::Package(pkg) => pkg.display_path(),
        }
    }

    pub(crate) fn root_directory_name(&self) -> RootDirectoryName {
        match self {
            Self::Workspace(ws) => ws.root_directory_name(),
            Self::Package(pkg) => pkg.root_directory_name(),
        }
    }

    pub(crate) fn visibility(&self) -> Visibility {
        match self {
            Self::Workspace(ws) => ws.visibility(),
            Self::Package(pkg) => pkg.visibility(),
        }
    }

    pub(crate) fn worktree_health(&self) -> WorktreeHealth {
        match self {
            Self::Workspace(ws) => ws.worktree_health(),
            Self::Package(pkg) => pkg.worktree_health(),
        }
    }

    pub(crate) fn disk_usage_bytes(&self) -> Option<u64> {
        match self {
            Self::Workspace(ws) => ws.disk_usage_bytes(),
            Self::Package(pkg) => pkg.disk_usage_bytes(),
        }
    }

    pub(crate) fn git_info(&self) -> Option<&GitInfo> {
        match self {
            Self::Workspace(ws) => ws.git_info(),
            Self::Package(pkg) => pkg.git_info(),
        }
    }

    pub(crate) fn at_path(&self, path: &Path) -> Option<&ProjectInfo> {
        match self {
            Self::Workspace(ws) => info_in_workspace(ws, path),
            Self::Package(pkg) => info_in_package(pkg, path),
        }
    }

    pub(crate) fn at_path_mut(&mut self, path: &Path) -> Option<&mut ProjectInfo> {
        match self {
            Self::Workspace(ws) => info_in_workspace_mut(ws, path),
            Self::Package(pkg) => info_in_package_mut(pkg, path),
        }
    }

    pub(crate) fn collect_project_info(&self, out: &mut Vec<(PathBuf, ProjectInfo)>) {
        match self {
            Self::Workspace(ws) => collect_project_info_from_workspace(ws, out),
            Self::Package(pkg) => collect_project_info_from_package(pkg, out),
        }
    }
}

// ── Traversal helpers ────────────────────────────────────────────────

pub(super) fn info_in_workspace<'a>(
    ws: &'a WorkspaceProject,
    path: &Path,
) -> Option<&'a ProjectInfo> {
    if ws.path() == path {
        return Some(ws.rust.info());
    }
    for group in ws.groups() {
        for member in group.members() {
            if member.path() == path {
                return Some(member.rust.info());
            }
        }
    }
    for vendored in ws.vendored() {
        if vendored.path() == path {
            return Some(vendored.rust.info());
        }
    }
    None
}

pub(super) fn info_in_package<'a>(pkg: &'a PackageProject, path: &Path) -> Option<&'a ProjectInfo> {
    if pkg.path() == path {
        return Some(pkg.rust.info());
    }
    for vendored in pkg.vendored() {
        if vendored.path() == path {
            return Some(vendored.rust.info());
        }
    }
    None
}

pub(super) fn info_in_workspace_mut<'a>(
    ws: &'a mut WorkspaceProject,
    path: &Path,
) -> Option<&'a mut ProjectInfo> {
    if ws.path() == path {
        return Some(ws.rust.info_mut());
    }
    let member_index = ws
        .groups()
        .iter()
        .enumerate()
        .find_map(|(group_index, group)| {
            group
                .members()
                .iter()
                .position(|member| member.path() == path)
                .map(|member_index| (group_index, member_index))
        });
    if let Some((group_index, member_index)) = member_index {
        return Some(
            ws.groups_mut()[group_index].members_mut()[member_index]
                .rust
                .info_mut(),
        );
    }
    let vendored_index = ws
        .vendored()
        .iter()
        .position(|vendored| vendored.path() == path);
    if let Some(vendored_index) = vendored_index {
        return Some(ws.vendored_mut()[vendored_index].rust.info_mut());
    }
    None
}

pub(super) fn info_in_package_mut<'a>(
    pkg: &'a mut PackageProject,
    path: &Path,
) -> Option<&'a mut ProjectInfo> {
    if pkg.path() == path {
        return Some(pkg.rust.info_mut());
    }
    for vendored in pkg.vendored_mut() {
        if vendored.path() == path {
            return Some(vendored.rust.info_mut());
        }
    }
    None
}

fn collect_project_info_from_workspace(
    ws: &WorkspaceProject,
    out: &mut Vec<(PathBuf, ProjectInfo)>,
) {
    out.push((ws.path().to_path_buf(), ws.rust.info().clone()));
    for group in ws.groups() {
        for member in group.members() {
            out.push((member.path().to_path_buf(), member.rust.info().clone()));
        }
    }
    for vendored in ws.vendored() {
        out.push((vendored.path().to_path_buf(), vendored.rust.info().clone()));
    }
}

fn collect_project_info_from_package(pkg: &PackageProject, out: &mut Vec<(PathBuf, ProjectInfo)>) {
    out.push((pkg.path().to_path_buf(), pkg.rust.info().clone()));
    for vendored in pkg.vendored() {
        out.push((vendored.path().to_path_buf(), vendored.rust.info().clone()));
    }
}
