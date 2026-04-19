use std::path::Path;

use super::git::GitInfo;
use super::git::WorktreeStatus;
use super::info::ProjectInfo;
use super::info::Visibility;
use super::info::WorktreeHealth;
use super::package::Package;
use super::paths::AbsolutePath;
use super::paths::DisplayPath;
use super::paths::RootDirectoryName;
use super::project_fields::ProjectFields;
use super::rust_info::RustInfo;
use super::workspace::Workspace;
use crate::lint::LintRuns;

/// A Rust project — either a workspace or a standalone package.
///
/// Delegation methods forward to the concrete type via 2-arm matches.
/// For kind-specific access (e.g. `.groups()`), match on the variant.
#[derive(Clone)]
pub(crate) enum RustProject {
    Workspace(Workspace),
    Package(Package),
}

impl RustProject {
    pub(crate) fn path(&self) -> &AbsolutePath {
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

    pub(crate) fn worktree_status(&self) -> &WorktreeStatus {
        match self {
            Self::Workspace(ws) => ws.worktree_status(),
            Self::Package(pkg) => pkg.worktree_status(),
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

    pub(crate) fn rust_info_at_path(&self, path: &Path) -> Option<&RustInfo> {
        match self {
            Self::Workspace(ws) => rust_info_in_workspace(ws, path),
            Self::Package(pkg) => rust_info_in_package(pkg, path),
        }
    }

    pub(crate) fn at_path_mut(&mut self, path: &Path) -> Option<&mut ProjectInfo> {
        match self {
            Self::Workspace(ws) => info_in_workspace_mut(ws, path),
            Self::Package(pkg) => info_in_package_mut(pkg, path),
        }
    }

    pub(crate) fn rust_info_at_path_mut(&mut self, path: &Path) -> Option<&mut RustInfo> {
        match self {
            Self::Workspace(ws) => rust_info_in_workspace_mut(ws, path),
            Self::Package(pkg) => rust_info_in_package_mut(pkg, path),
        }
    }

    /// Returns the `LintRuns` for the lint-owning node that contains `path`.
    ///
    /// Lint runs at the workspace/package level. Members and vendored packages
    /// resolve to their owning parent's `LintRuns`.
    pub(crate) fn lint_at_path(&self, path: &Path) -> Option<&LintRuns> {
        match self {
            Self::Workspace(ws) => lint_in_workspace(ws, path),
            Self::Package(pkg) => lint_in_package(pkg, path),
        }
    }

    pub(crate) fn lint_at_path_mut(&mut self, path: &Path) -> Option<&mut LintRuns> {
        match self {
            Self::Workspace(ws) => lint_in_workspace_mut(ws, path),
            Self::Package(pkg) => lint_in_package_mut(pkg, path),
        }
    }

    pub(crate) fn collect_project_info(&self, out: &mut Vec<(AbsolutePath, ProjectInfo)>) {
        match self {
            Self::Workspace(ws) => collect_project_info_from_workspace(ws, out),
            Self::Package(pkg) => collect_project_info_from_package(pkg, out),
        }
    }
}

// ── Traversal helpers ────────────────────────────────────────────────

pub(super) fn info_in_workspace<'a>(ws: &'a Workspace, path: &Path) -> Option<&'a ProjectInfo> {
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

pub(super) fn info_in_package<'a>(pkg: &'a Package, path: &Path) -> Option<&'a ProjectInfo> {
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
    ws: &'a mut Workspace,
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
    pkg: &'a mut Package,
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

// ── RustInfo traversal helpers ──────────────────────────────────────

pub(super) fn rust_info_in_workspace<'a>(ws: &'a Workspace, path: &Path) -> Option<&'a RustInfo> {
    if ws.path() == path {
        return Some(&ws.rust);
    }
    for group in ws.groups() {
        for member in group.members() {
            if member.path() == path {
                return Some(&member.rust);
            }
        }
    }
    for vendored in ws.vendored() {
        if vendored.path() == path {
            return Some(&vendored.rust);
        }
    }
    None
}

pub(super) fn rust_info_in_package<'a>(pkg: &'a Package, path: &Path) -> Option<&'a RustInfo> {
    if pkg.path() == path {
        return Some(&pkg.rust);
    }
    for vendored in pkg.vendored() {
        if vendored.path() == path {
            return Some(&vendored.rust);
        }
    }
    None
}

pub(super) fn rust_info_in_workspace_mut<'a>(
    ws: &'a mut Workspace,
    path: &Path,
) -> Option<&'a mut RustInfo> {
    if ws.path() == path {
        return Some(&mut ws.rust);
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
        return Some(&mut ws.groups_mut()[group_index].members_mut()[member_index].rust);
    }
    let vendored_index = ws
        .vendored()
        .iter()
        .position(|vendored| vendored.path() == path);
    if let Some(vendored_index) = vendored_index {
        return Some(&mut ws.vendored_mut()[vendored_index].rust);
    }
    None
}

pub(super) fn rust_info_in_package_mut<'a>(
    pkg: &'a mut Package,
    path: &Path,
) -> Option<&'a mut RustInfo> {
    if pkg.path() == path {
        return Some(&mut pkg.rust);
    }
    for vendored in pkg.vendored_mut() {
        if vendored.path() == path {
            return Some(&mut vendored.rust);
        }
    }
    None
}

// ── Lint traversal helpers ───────────────────────────────────────────
//
// Lint runs at the workspace/package level. Members and vendored packages
// resolve to the owning root's `LintRuns`.

pub(super) fn lint_in_workspace<'a>(ws: &'a Workspace, path: &Path) -> Option<&'a LintRuns> {
    if ws.path() == path {
        return Some(ws.lint_runs());
    }
    let is_child = ws
        .groups()
        .iter()
        .any(|g| g.members().iter().any(|m| m.path() == path))
        || ws.vendored().iter().any(|v| v.path() == path);
    is_child.then(|| ws.lint_runs())
}

pub(super) fn lint_in_package<'a>(pkg: &'a Package, path: &Path) -> Option<&'a LintRuns> {
    if pkg.path() == path {
        return Some(pkg.lint_runs());
    }
    let is_child = pkg.vendored().iter().any(|v| v.path() == path);
    is_child.then(|| pkg.lint_runs())
}

pub(super) fn lint_in_workspace_mut<'a>(
    ws: &'a mut Workspace,
    path: &Path,
) -> Option<&'a mut LintRuns> {
    if ws.path() == path {
        return Some(ws.lint_runs_mut());
    }
    let is_child = ws
        .groups()
        .iter()
        .any(|g| g.members().iter().any(|m| m.path() == path))
        || ws.vendored().iter().any(|v| v.path() == path);
    is_child.then(|| ws.lint_runs_mut())
}

pub(super) fn lint_in_package_mut<'a>(
    pkg: &'a mut Package,
    path: &Path,
) -> Option<&'a mut LintRuns> {
    if pkg.path() == path {
        return Some(pkg.lint_runs_mut());
    }
    let is_child = pkg.vendored().iter().any(|v| v.path() == path);
    is_child.then(|| pkg.lint_runs_mut())
}

fn collect_project_info_from_workspace(ws: &Workspace, out: &mut Vec<(AbsolutePath, ProjectInfo)>) {
    out.push((ws.path().clone(), ws.rust.info().clone()));
    for group in ws.groups() {
        for member in group.members() {
            out.push((member.path().clone(), member.rust.info().clone()));
        }
    }
    for vendored in ws.vendored() {
        out.push((vendored.path().clone(), vendored.rust.info().clone()));
    }
}

fn collect_project_info_from_package(pkg: &Package, out: &mut Vec<(AbsolutePath, ProjectInfo)>) {
    out.push((pkg.path().clone(), pkg.rust.info().clone()));
    for vendored in pkg.vendored() {
        out.push((vendored.path().clone(), vendored.rust.info().clone()));
    }
}
