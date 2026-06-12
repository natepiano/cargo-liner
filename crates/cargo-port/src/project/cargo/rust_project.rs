use std::path::Path;

use super::package::Package;
use super::rust_info::RustInfo;
use super::workspace::Workspace;
use crate::lint::LintRuns;
use crate::project::git::CheckoutInfo;
use crate::project::git::WorktreeStatus;
use crate::project::info::ProjectInfo;
use crate::project::info::Visibility;
use crate::project::info::WorktreeHealth;
use crate::project::paths::AbsolutePath;
use crate::project::paths::DisplayPath;
use crate::project::paths::RootDirectoryName;
use crate::project::project_fields::ProjectFields;
use crate::project::vendored_package::VendoredPackage;

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
    pub fn path(&self) -> &AbsolutePath {
        match self {
            Self::Workspace(ws) => ws.path(),
            Self::Package(pkg) => pkg.path(),
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Workspace(ws) => ws.name(),
            Self::Package(pkg) => pkg.name(),
        }
    }

    pub fn worktree_status(&self) -> &WorktreeStatus {
        match self {
            Self::Workspace(ws) => ws.worktree_status(),
            Self::Package(pkg) => pkg.worktree_status(),
        }
    }

    pub fn display_path(&self) -> DisplayPath {
        match self {
            Self::Workspace(ws) => ws.display_path(),
            Self::Package(pkg) => pkg.display_path(),
        }
    }

    pub fn linked_primary_root(&self) -> Option<AbsolutePath> {
        let status = self.worktree_status();
        if status.is_linked_worktree() {
            status.primary_root().cloned()
        } else {
            None
        }
    }

    pub fn root_directory_name(&self) -> RootDirectoryName {
        match self {
            Self::Workspace(ws) => ws.root_directory_name(),
            Self::Package(pkg) => pkg.root_directory_name(),
        }
    }

    pub fn visibility(&self) -> Visibility {
        match self {
            Self::Workspace(ws) => ws.visibility(),
            Self::Package(pkg) => pkg.visibility(),
        }
    }

    pub fn worktree_health(&self) -> WorktreeHealth {
        match self {
            Self::Workspace(ws) => ws.worktree_health(),
            Self::Package(pkg) => pkg.worktree_health(),
        }
    }

    pub fn disk_usage_bytes(&self) -> Option<u64> {
        match self {
            Self::Workspace(ws) => ws.disk_usage_bytes(),
            Self::Package(pkg) => pkg.disk_usage_bytes(),
        }
    }

    pub fn git_info(&self) -> Option<&CheckoutInfo> {
        match self {
            Self::Workspace(ws) => ws.git_info(),
            Self::Package(pkg) => pkg.git_info(),
        }
    }

    pub const fn rust_info(&self) -> &RustInfo {
        match self {
            Self::Workspace(ws) => &ws.rust,
            Self::Package(pkg) => &pkg.rust,
        }
    }

    pub const fn rust_info_mut(&mut self) -> &mut RustInfo {
        match self {
            Self::Workspace(ws) => &mut ws.rust,
            Self::Package(pkg) => &mut pkg.rust,
        }
    }

    pub fn at_path(&self, path: &Path) -> Option<&ProjectInfo> {
        match self {
            Self::Workspace(ws) => info_in_workspace(ws, path),
            Self::Package(pkg) => info_in_package(pkg, path),
        }
    }

    pub fn rust_info_at_path(&self, path: &Path) -> Option<&RustInfo> {
        match self {
            Self::Workspace(ws) => rust_info_in_workspace(ws, path),
            Self::Package(pkg) => rust_info_in_package(pkg, path),
        }
    }

    pub fn at_path_mut(&mut self, path: &Path) -> Option<&mut ProjectInfo> {
        match self {
            Self::Workspace(ws) => info_in_workspace_mut(ws, path),
            Self::Package(pkg) => info_in_package_mut(pkg, path),
        }
    }

    pub fn rust_info_at_path_mut(&mut self, path: &Path) -> Option<&mut RustInfo> {
        match self {
            Self::Workspace(ws) => rust_info_in_workspace_mut(ws, path),
            Self::Package(pkg) => rust_info_in_package_mut(pkg, path),
        }
    }

    /// Returns the `LintRuns` for the lint-owning node that contains `path`.
    ///
    /// Lint runs at the workspace/package level. Members and vendored packages
    /// resolve to their owning parent's `LintRuns`.
    pub fn lint_at_path(&self, path: &Path) -> Option<&LintRuns> {
        match self {
            Self::Workspace(ws) => lint_in_workspace(ws, path),
            Self::Package(pkg) => lint_in_package(pkg, path),
        }
    }

    pub fn lint_at_path_mut(&mut self, path: &Path) -> Option<&mut LintRuns> {
        match self {
            Self::Workspace(ws) => lint_in_workspace_mut(ws, path),
            Self::Package(pkg) => lint_in_package_mut(pkg, path),
        }
    }

    pub fn lint_owner_path(&self, path: &Path) -> Option<&AbsolutePath> {
        match self {
            Self::Workspace(ws) => lint_owner_in_workspace(ws, path),
            Self::Package(pkg) => lint_owner_in_package(pkg, path),
        }
    }

    pub fn vendored_at_path(&self, path: &Path) -> Option<&VendoredPackage> {
        match self {
            Self::Workspace(ws) => vendored_in_workspace(ws, path),
            Self::Package(pkg) => vendored_in_package(pkg, path),
        }
    }

    pub fn vendored_at_path_mut(&mut self, path: &Path) -> Option<&mut VendoredPackage> {
        match self {
            Self::Workspace(ws) => vendored_in_workspace_mut(ws, path),
            Self::Package(pkg) => vendored_in_package_mut(pkg, path),
        }
    }

    pub fn collect_project_info(&self, out: &mut Vec<(AbsolutePath, ProjectInfo)>) {
        match self {
            Self::Workspace(ws) => collect_project_info_from_workspace(ws, out),
            Self::Package(pkg) => collect_project_info_from_package(pkg, out),
        }
    }
}

impl ProjectFields for RustProject {
    fn path(&self) -> &AbsolutePath { Self::path(self) }

    fn name(&self) -> Option<&str> { Self::name(self) }

    fn visibility(&self) -> Visibility { Self::visibility(self) }

    fn worktree_health(&self) -> WorktreeHealth { Self::worktree_health(self) }

    fn disk_usage_bytes(&self) -> Option<u64> { Self::disk_usage_bytes(self) }

    fn git_info(&self) -> Option<&CheckoutInfo> { Self::git_info(self) }

    fn info(&self) -> &ProjectInfo {
        match self {
            Self::Workspace(ws) => ws.info(),
            Self::Package(pkg) => pkg.info(),
        }
    }

    fn display_path(&self) -> DisplayPath { Self::display_path(self) }

    fn root_directory_name(&self) -> RootDirectoryName { Self::root_directory_name(self) }

    fn worktree_status(&self) -> &WorktreeStatus { Self::worktree_status(self) }

    fn crates_io_name(&self) -> Option<&str> {
        match self {
            Self::Workspace(ws) => ws.crates_io_name(),
            Self::Package(pkg) => pkg.crates_io_name(),
        }
    }
}

// ── Traversal helpers ────────────────────────────────────────────────

pub(super) fn info_in_workspace<'a>(ws: &'a Workspace, path: &Path) -> Option<&'a ProjectInfo> {
    if ws.path() == path {
        return Some(&ws.rust.project_info);
    }
    for group in ws.groups() {
        for member in group.members() {
            if member.path() == path {
                return Some(&member.rust.project_info);
            }
            for vendored in member.vendored() {
                if vendored.path() == path {
                    return Some(&vendored.project_info);
                }
            }
        }
    }
    for vendored in ws.vendored() {
        if vendored.path() == path {
            return Some(&vendored.project_info);
        }
    }
    None
}

pub(super) fn info_in_package<'a>(pkg: &'a Package, path: &Path) -> Option<&'a ProjectInfo> {
    if pkg.path() == path {
        return Some(&pkg.rust.project_info);
    }
    for vendored in pkg.vendored() {
        if vendored.path() == path {
            return Some(&vendored.project_info);
        }
    }
    None
}

pub(super) fn info_in_workspace_mut<'a>(
    ws: &'a mut Workspace,
    path: &Path,
) -> Option<&'a mut ProjectInfo> {
    if ws.path() == path {
        return Some(&mut ws.rust.project_info);
    }
    match workspace_info_target(ws, path)? {
        WorkspaceInfoTarget::Member {
            group_index,
            member_index,
        } => Some(
            &mut ws.groups_mut()[group_index].members_mut()[member_index]
                .rust
                .project_info,
        ),
        WorkspaceInfoTarget::MemberVendored {
            group_index,
            member_index,
            vendored_index,
        } => Some(
            &mut ws.groups_mut()[group_index].members_mut()[member_index].vendored_mut()
                [vendored_index]
                .project_info,
        ),
        WorkspaceInfoTarget::RootVendored { vendored_index } => {
            Some(&mut ws.vendored_mut()[vendored_index].project_info)
        },
    }
}

enum WorkspaceInfoTarget {
    Member {
        group_index:  usize,
        member_index: usize,
    },
    MemberVendored {
        group_index:    usize,
        member_index:   usize,
        vendored_index: usize,
    },
    RootVendored {
        vendored_index: usize,
    },
}

fn workspace_info_target(ws: &Workspace, path: &Path) -> Option<WorkspaceInfoTarget> {
    for (group_index, group) in ws.groups().iter().enumerate() {
        for (member_index, member) in group.members().iter().enumerate() {
            if member.path() == path {
                return Some(WorkspaceInfoTarget::Member {
                    group_index,
                    member_index,
                });
            }
            if let Some(vendored_index) = member
                .vendored()
                .iter()
                .position(|vendored| vendored.path() == path)
            {
                return Some(WorkspaceInfoTarget::MemberVendored {
                    group_index,
                    member_index,
                    vendored_index,
                });
            }
        }
    }
    ws.vendored()
        .iter()
        .position(|vendored| vendored.path() == path)
        .map(|vendored_index| WorkspaceInfoTarget::RootVendored { vendored_index })
}

pub(super) fn info_in_package_mut<'a>(
    pkg: &'a mut Package,
    path: &Path,
) -> Option<&'a mut ProjectInfo> {
    if pkg.path() == path {
        return Some(&mut pkg.rust.project_info);
    }
    for vendored in pkg.vendored_mut() {
        if vendored.path() == path {
            return Some(&mut vendored.project_info);
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
    None
}

pub(super) fn rust_info_in_package<'a>(pkg: &'a Package, path: &Path) -> Option<&'a RustInfo> {
    if pkg.path() == path {
        return Some(&pkg.rust);
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
    None
}

pub(super) fn rust_info_in_package_mut<'a>(
    pkg: &'a mut Package,
    path: &Path,
) -> Option<&'a mut RustInfo> {
    if pkg.path() == path {
        return Some(&mut pkg.rust);
    }
    None
}

// ── VendoredPackage traversal helpers ────────────────────────────────

pub(super) fn vendored_in_workspace<'a>(
    ws: &'a Workspace,
    path: &Path,
) -> Option<&'a VendoredPackage> {
    for group in ws.groups() {
        for member in group.members() {
            if let Some(vendored) = vendored_in_package(member, path) {
                return Some(vendored);
            }
        }
    }
    ws.vendored().iter().find(|v| v.path() == path)
}

pub(super) fn vendored_in_package<'a>(
    pkg: &'a Package,
    path: &Path,
) -> Option<&'a VendoredPackage> {
    pkg.vendored().iter().find(|v| v.path() == path)
}

pub(super) fn vendored_in_workspace_mut<'a>(
    ws: &'a mut Workspace,
    path: &Path,
) -> Option<&'a mut VendoredPackage> {
    match workspace_vendored_target(ws, path)? {
        WorkspaceVendoredTarget::Member {
            group_index,
            member_index,
            vendored_index,
        } => Some(
            &mut ws.groups_mut()[group_index].members_mut()[member_index].vendored_mut()
                [vendored_index],
        ),
        WorkspaceVendoredTarget::Root { vendored_index } => {
            Some(&mut ws.vendored_mut()[vendored_index])
        },
    }
}

enum WorkspaceVendoredTarget {
    Member {
        group_index:    usize,
        member_index:   usize,
        vendored_index: usize,
    },
    Root {
        vendored_index: usize,
    },
}

fn workspace_vendored_target(ws: &Workspace, path: &Path) -> Option<WorkspaceVendoredTarget> {
    for (group_index, group) in ws.groups().iter().enumerate() {
        for (member_index, member) in group.members().iter().enumerate() {
            if let Some(vendored_index) = member
                .vendored()
                .iter()
                .position(|vendored| vendored.path() == path)
            {
                return Some(WorkspaceVendoredTarget::Member {
                    group_index,
                    member_index,
                    vendored_index,
                });
            }
        }
    }
    ws.vendored()
        .iter()
        .position(|vendored| vendored.path() == path)
        .map(|vendored_index| WorkspaceVendoredTarget::Root { vendored_index })
}

pub(super) fn vendored_in_package_mut<'a>(
    pkg: &'a mut Package,
    path: &Path,
) -> Option<&'a mut VendoredPackage> {
    pkg.vendored_mut().iter_mut().find(|v| v.path() == path)
}

// ── Lint traversal helpers ───────────────────────────────────────────
//
// Lint runs at the workspace/package level. Workspace members resolve to the
// owning root's `LintRuns`. Vendored crates never resolve to lint — they are
// not lint-owning nodes, and the type system guarantees they cannot be.

pub(super) fn lint_in_workspace<'a>(ws: &'a Workspace, path: &Path) -> Option<&'a LintRuns> {
    if ws.path() == path {
        return Some(&ws.rust.lint_runs);
    }
    let is_member = ws
        .groups()
        .iter()
        .any(|g| g.members().iter().any(|m| m.path() == path));
    is_member.then_some(&ws.rust.lint_runs)
}

pub(super) fn lint_in_package<'a>(pkg: &'a Package, path: &Path) -> Option<&'a LintRuns> {
    (pkg.path() == path).then_some(&pkg.rust.lint_runs)
}

pub(super) fn lint_owner_in_workspace<'a>(
    ws: &'a Workspace,
    path: &Path,
) -> Option<&'a AbsolutePath> {
    lint_in_workspace(ws, path).map(|_| ws.path())
}

pub(super) fn lint_owner_in_package<'a>(pkg: &'a Package, path: &Path) -> Option<&'a AbsolutePath> {
    lint_in_package(pkg, path).map(|_| pkg.path())
}

pub(super) fn lint_in_workspace_mut<'a>(
    ws: &'a mut Workspace,
    path: &Path,
) -> Option<&'a mut LintRuns> {
    let is_member = ws
        .groups()
        .iter()
        .any(|g| g.members().iter().any(|m| m.path() == path));
    if ws.path() == path || is_member {
        return Some(&mut ws.rust.lint_runs);
    }
    None
}

pub(super) fn lint_in_package_mut<'a>(
    pkg: &'a mut Package,
    path: &Path,
) -> Option<&'a mut LintRuns> {
    if pkg.path() == path {
        Some(&mut pkg.rust.lint_runs)
    } else {
        None
    }
}

fn collect_project_info_from_workspace(ws: &Workspace, out: &mut Vec<(AbsolutePath, ProjectInfo)>) {
    out.push((ws.path().clone(), ws.rust.project_info.clone()));
    for group in ws.groups() {
        for member in group.members() {
            out.push((member.path().clone(), member.rust.project_info.clone()));
            for vendored in member.vendored() {
                out.push((vendored.path().clone(), vendored.project_info.clone()));
            }
        }
    }
    for vendored in ws.vendored() {
        out.push((vendored.path().clone(), vendored.project_info.clone()));
    }
}

fn collect_project_info_from_package(pkg: &Package, out: &mut Vec<(AbsolutePath, ProjectInfo)>) {
    out.push((pkg.path().clone(), pkg.rust.project_info.clone()));
    for vendored in pkg.vendored() {
        out.push((vendored.path().clone(), vendored.project_info.clone()));
    }
}
