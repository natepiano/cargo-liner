use std::path::Path;

use super::git::CheckoutInfo;
use super::info::ProjectInfo;
use super::info::Visibility;
use super::info::WorktreeHealth;
use super::non_rust::NonRustProject;
use super::paths::AbsolutePath;
use super::paths::DisplayPath;
use super::paths::RootDirectoryName;
use super::project_fields::ProjectFields;
use super::rust_project::RustProject;
use super::submodule::Submodule;
use super::vendored_package::VendoredPackage;
use super::worktree_group::WorktreeGroup;
use crate::lint::LintRuns;
use crate::lint::LintStatus;

/// The top-level enum for the project list — 3 variants.
#[derive(Clone)]
pub(crate) enum RootItem {
    Rust(RustProject),
    NonRust(NonRustProject),
    Worktrees(WorktreeGroup),
}

impl RootItem {
    pub(crate) fn visibility(&self) -> Visibility {
        match self {
            Self::Rust(p) => p.visibility(),
            Self::NonRust(p) => p.visibility(),
            Self::Worktrees(g) => g.derived_visibility(),
        }
    }

    pub(crate) fn worktree_health(&self) -> WorktreeHealth {
        match self {
            Self::Rust(p) => p.worktree_health(),
            Self::NonRust(p) => p.worktree_health(),
            Self::Worktrees(g) => g.primary_worktree_health(),
        }
    }

    /// Absolute path to the primary project root.
    pub(crate) fn path(&self) -> &AbsolutePath {
        match self {
            Self::Rust(p) => p.path(),
            Self::NonRust(p) => p.path(),
            Self::Worktrees(g) => g.primary_path(),
        }
    }

    pub(crate) fn name(&self) -> Option<&str> {
        match self {
            Self::Rust(p) => p.name(),
            Self::NonRust(p) => p.name(),
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces { primary, .. } => primary.name(),
                WorktreeGroup::Packages { primary, .. } => primary.name(),
            },
        }
    }

    pub(crate) fn display_path(&self) -> DisplayPath {
        match self {
            Self::Rust(p) => p.display_path(),
            Self::NonRust(p) => p.display_path(),
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces { primary, .. } => primary.display_path(),
                WorktreeGroup::Packages { primary, .. } => primary.display_path(),
            },
        }
    }

    pub(crate) fn git_directory(&self) -> Option<AbsolutePath> { git::resolve_git_dir(self.path()) }

    /// Directory leaf name for top-level root labels and disambiguation.
    pub(crate) fn root_directory_name(&self) -> RootDirectoryName {
        match self {
            Self::Rust(p) => p.root_directory_name(),
            Self::NonRust(p) => p.root_directory_name(),
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces { primary, .. } => primary.root_directory_name(),
                WorktreeGroup::Packages { primary, .. } => primary.root_directory_name(),
            },
        }
    }

    pub(crate) fn worktree_badge_suffix(&self) -> Option<String> {
        let live_worktrees = match self {
            Self::Worktrees(g) if g.renders_as_group() => g.live_entry_count(),
            _ => 0,
        };
        (live_worktrees > 0).then(|| format!(" {}:{live_worktrees}", crate::constants::WORKTREE))
    }

    /// Whether this item has expandable children.
    pub(crate) fn has_children(&self) -> bool {
        if !self.submodules().is_empty() {
            return true;
        }
        match self {
            Self::Rust(RustProject::Workspace(ws)) => {
                ws.groups().iter().any(|g| !g.members().is_empty()) || !ws.vendored().is_empty()
            },
            Self::Rust(RustProject::Package(pkg)) => !pkg.vendored().is_empty(),
            Self::NonRust(_) => false,
            Self::Worktrees(g) => {
                if g.renders_as_group() {
                    true
                } else {
                    match g {
                        WorktreeGroup::Workspaces {
                            primary, linked, ..
                        } => single_live_workspace(primary, linked)
                            .is_some_and(|ws| ws.has_members() || !ws.vendored().is_empty()),
                        WorktreeGroup::Packages {
                            primary, linked, ..
                        } => single_live_package(primary, linked)
                            .is_some_and(|pkg| !pkg.vendored().is_empty()),
                    }
                }
            },
        }
    }

    /// Language icon for the project list.
    pub(crate) const fn lang_icon(&self) -> &'static str {
        match self {
            Self::Rust(_) | Self::Worktrees(_) => "\u{1f980}",
            Self::NonRust(_) => "  ",
        }
    }

    /// Git submodules for this item's primary project info.
    pub(crate) fn submodules(&self) -> &[Submodule] {
        match self {
            Self::Rust(RustProject::Workspace(ws)) => &ws.info().submodules,
            Self::Rust(RustProject::Package(pkg)) => &pkg.info().submodules,
            Self::NonRust(p) => &p.info().submodules,
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces { primary, .. } => &primary.info().submodules,
                WorktreeGroup::Packages { primary, .. } => &primary.info().submodules,
            },
        }
    }

    /// Whether this is a Rust project (has `Cargo.toml`).
    pub(crate) const fn is_rust(&self) -> bool {
        matches!(self, Self::Rust(_) | Self::Worktrees(_))
    }

    /// Disk usage for this item. Worktree groups sum primary + linked.
    pub(crate) fn disk_usage_bytes(&self) -> Option<u64> {
        match self {
            Self::Rust(p) => p.disk_usage_bytes(),
            Self::NonRust(p) => p.disk_usage_bytes(),
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces {
                    primary, linked, ..
                } => sum_disk(
                    primary.disk_usage_bytes(),
                    linked.iter().map(ProjectFields::disk_usage_bytes),
                ),
                WorktreeGroup::Packages {
                    primary, linked, ..
                } => sum_disk(
                    primary.disk_usage_bytes(),
                    linked.iter().map(ProjectFields::disk_usage_bytes),
                ),
            },
        }
    }

    pub(crate) fn git_info(&self) -> Option<&CheckoutInfo> {
        match self {
            Self::Rust(p) => p.git_info(),
            Self::NonRust(p) => p.git_info(),
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces { primary, .. } => primary.git_info(),
                WorktreeGroup::Packages { primary, .. } => primary.git_info(),
            },
        }
    }

    pub(crate) fn at_path(&self, path: &Path) -> Option<&ProjectInfo> {
        let result = match self {
            Self::Rust(p) => p.at_path(path),
            Self::NonRust(p) => (p.path() == path).then(|| p.info()),
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces {
                    primary, linked, ..
                } => rust_project::info_in_workspace(primary, path).or_else(|| {
                    linked
                        .iter()
                        .find_map(|l| rust_project::info_in_workspace(l, path))
                }),
                WorktreeGroup::Packages {
                    primary, linked, ..
                } => rust_project::info_in_package(primary, path).or_else(|| {
                    linked
                        .iter()
                        .find_map(|l| rust_project::info_in_package(l, path))
                }),
            },
        };
        result.or_else(|| {
            self.submodules()
                .iter()
                .find(|s| s.path.as_path() == path)
                .map(|s| &s.info)
        })
    }

    pub(crate) fn rust_info_at_path(&self, path: &Path) -> Option<&RustInfo> {
        match self {
            Self::Rust(p) => p.rust_info_at_path(path),
            Self::NonRust(_) => None,
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces {
                    primary, linked, ..
                } => rust_project::rust_info_in_workspace(primary, path).or_else(|| {
                    linked
                        .iter()
                        .find_map(|l| rust_project::rust_info_in_workspace(l, path))
                }),
                WorktreeGroup::Packages {
                    primary, linked, ..
                } => rust_project::rust_info_in_package(primary, path).or_else(|| {
                    linked
                        .iter()
                        .find_map(|l| rust_project::rust_info_in_package(l, path))
                }),
            },
        }
    }

    pub(crate) fn at_path_mut(&mut self, path: &Path) -> Option<&mut ProjectInfo> {
        // Check submodules first to avoid double-borrowing through the main
        // hierarchy and then falling back to submodules on the same `&mut self`.
        if self.submodules().iter().any(|s| s.path.as_path() == path) {
            return submodule_info_mut(self, path);
        }
        match self {
            Self::Rust(p) => p.at_path_mut(path),
            Self::NonRust(p) => (p.path() == path).then(|| p.info_mut()),
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces {
                    primary, linked, ..
                } => {
                    if rust_project::info_in_workspace(primary, path).is_some() {
                        return rust_project::info_in_workspace_mut(primary, path);
                    }
                    let idx = linked
                        .iter()
                        .position(|l| rust_project::info_in_workspace(l, path).is_some())?;
                    rust_project::info_in_workspace_mut(&mut linked[idx], path)
                },
                WorktreeGroup::Packages {
                    primary, linked, ..
                } => {
                    if rust_project::info_in_package(primary, path).is_some() {
                        return rust_project::info_in_package_mut(primary, path);
                    }
                    let idx = linked
                        .iter()
                        .position(|l| rust_project::info_in_package(l, path).is_some())?;
                    rust_project::info_in_package_mut(&mut linked[idx], path)
                },
            },
        }
    }

    pub(crate) fn rust_info_at_path_mut(&mut self, path: &Path) -> Option<&mut RustInfo> {
        match self {
            Self::Rust(p) => p.rust_info_at_path_mut(path),
            Self::NonRust(_) => None,
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces {
                    primary, linked, ..
                } => {
                    if rust_project::info_in_workspace(primary, path).is_some() {
                        return rust_project::rust_info_in_workspace_mut(primary, path);
                    }
                    let idx = linked
                        .iter()
                        .position(|l| rust_project::info_in_workspace(l, path).is_some())?;
                    rust_project::rust_info_in_workspace_mut(&mut linked[idx], path)
                },
                WorktreeGroup::Packages {
                    primary, linked, ..
                } => {
                    if rust_project::info_in_package(primary, path).is_some() {
                        return rust_project::rust_info_in_package_mut(primary, path);
                    }
                    let idx = linked
                        .iter()
                        .position(|l| rust_project::info_in_package(l, path).is_some())?;
                    rust_project::rust_info_in_package_mut(&mut linked[idx], path)
                },
            },
        }
    }

    /// Returns the `LintRuns` for the lint-owning node that contains `path`.
    pub(crate) fn lint_at_path(&self, path: &Path) -> Option<&LintRuns> {
        match self {
            Self::Rust(p) => p.lint_at_path(path),
            Self::NonRust(_) => None,
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces {
                    primary, linked, ..
                } => rust_project::lint_in_workspace(primary, path).or_else(|| {
                    linked
                        .iter()
                        .find_map(|l| rust_project::lint_in_workspace(l, path))
                }),
                WorktreeGroup::Packages {
                    primary, linked, ..
                } => rust_project::lint_in_package(primary, path).or_else(|| {
                    linked
                        .iter()
                        .find_map(|l| rust_project::lint_in_package(l, path))
                }),
            },
        }
    }

    pub(crate) fn vendored_at_path(&self, path: &Path) -> Option<&VendoredPackage> {
        match self {
            Self::Rust(p) => p.vendored_at_path(path),
            Self::NonRust(_) => None,
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces {
                    primary, linked, ..
                } => rust_project::vendored_in_workspace(primary, path).or_else(|| {
                    linked
                        .iter()
                        .find_map(|l| rust_project::vendored_in_workspace(l, path))
                }),
                WorktreeGroup::Packages {
                    primary, linked, ..
                } => rust_project::vendored_in_package(primary, path).or_else(|| {
                    linked
                        .iter()
                        .find_map(|l| rust_project::vendored_in_package(l, path))
                }),
            },
        }
    }

    pub(crate) fn vendored_at_path_mut(&mut self, path: &Path) -> Option<&mut VendoredPackage> {
        match self {
            Self::Rust(p) => p.vendored_at_path_mut(path),
            Self::NonRust(_) => None,
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces {
                    primary, linked, ..
                } => {
                    if rust_project::vendored_in_workspace(primary, path).is_some() {
                        return rust_project::vendored_in_workspace_mut(primary, path);
                    }
                    let idx = linked
                        .iter()
                        .position(|l| rust_project::vendored_in_workspace(l, path).is_some())?;
                    rust_project::vendored_in_workspace_mut(&mut linked[idx], path)
                },
                WorktreeGroup::Packages {
                    primary, linked, ..
                } => {
                    if rust_project::vendored_in_package(primary, path).is_some() {
                        return rust_project::vendored_in_package_mut(primary, path);
                    }
                    let idx = linked
                        .iter()
                        .position(|l| rust_project::vendored_in_package(l, path).is_some())?;
                    rust_project::vendored_in_package_mut(&mut linked[idx], path)
                },
            },
        }
    }

    pub(crate) fn lint_at_path_mut(&mut self, path: &Path) -> Option<&mut LintRuns> {
        match self {
            Self::Rust(p) => p.lint_at_path_mut(path),
            Self::NonRust(_) => None,
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces {
                    primary, linked, ..
                } => {
                    if rust_project::lint_in_workspace(primary, path).is_some() {
                        return rust_project::lint_in_workspace_mut(primary, path);
                    }
                    let idx = linked
                        .iter()
                        .position(|l| rust_project::lint_in_workspace(l, path).is_some())?;
                    rust_project::lint_in_workspace_mut(&mut linked[idx], path)
                },
                WorktreeGroup::Packages {
                    primary, linked, ..
                } => {
                    if rust_project::lint_in_package(primary, path).is_some() {
                        return rust_project::lint_in_package_mut(primary, path);
                    }
                    let idx = linked
                        .iter()
                        .position(|l| rust_project::lint_in_package(l, path).is_some())?;
                    rust_project::lint_in_package_mut(&mut linked[idx], path)
                },
            },
        }
    }

    /// Lint runs owned by the parent of a vendored crate at `path`.
    ///
    /// Vendored crates do not own lint state (see `VendoredPackage`), but the
    /// detail pane surfaces the owning root's runs when a vendored row is
    /// selected — mirroring how workspace members inherit their workspace's
    /// lint history.
    pub(crate) fn vendored_owner_lint(&self, path: &Path) -> Option<&LintRuns> {
        match self {
            Self::Rust(RustProject::Workspace(ws)) => ws
                .vendored()
                .iter()
                .any(|v| v.path() == path)
                .then(|| ws.lint_runs()),
            Self::Rust(RustProject::Package(pkg)) => pkg
                .vendored()
                .iter()
                .any(|v| v.path() == path)
                .then(|| pkg.lint_runs()),
            Self::NonRust(_) => None,
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces {
                    primary, linked, ..
                } => std::iter::once(primary)
                    .chain(linked.iter())
                    .find(|ws| ws.vendored().iter().any(|v| v.path() == path))
                    .map(|ws| ws.lint_runs()),
                WorktreeGroup::Packages {
                    primary, linked, ..
                } => std::iter::once(primary)
                    .chain(linked.iter())
                    .find(|pkg| pkg.vendored().iter().any(|v| v.path() == path))
                    .map(|pkg| pkg.lint_runs()),
            },
        }
    }

    /// Aggregate lint status for this root item.
    ///
    /// For `Worktrees`, aggregates across all entries.
    /// For `Rust`, returns the node's own lint status.
    /// For `NonRust`, returns `NoLog`.
    pub(crate) fn lint_rollup_status(&self) -> LintStatus {
        match self {
            Self::Rust(p) => p
                .lint_at_path(p.path())
                .map_or(LintStatus::NoLog, |lr| lr.status().clone()),
            Self::NonRust(_) => LintStatus::NoLog,
            Self::Worktrees(g) => g.lint_rollup_status(),
        }
    }

    pub(crate) fn collect_project_info(&self) -> Vec<(AbsolutePath, ProjectInfo)> {
        let mut out = Vec::new();
        match self {
            Self::Rust(p) => p.collect_project_info(&mut out),
            Self::NonRust(p) => {
                out.push((p.path().clone(), p.info().clone()));
            },
            Self::Worktrees(g) => match g {
                WorktreeGroup::Workspaces {
                    primary, linked, ..
                } => {
                    RustProject::Workspace(primary.clone()).collect_project_info(&mut out);
                    for l in linked {
                        RustProject::Workspace(l.clone()).collect_project_info(&mut out);
                    }
                },
                WorktreeGroup::Packages {
                    primary, linked, ..
                } => {
                    RustProject::Package(primary.clone()).collect_project_info(&mut out);
                    for l in linked {
                        RustProject::Package(l.clone()).collect_project_info(&mut out);
                    }
                },
            },
        }
        out
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Mutable access to a submodule's `ProjectInfo` by path.
///
/// Separated from `at_path_mut` to avoid borrow-checker conflicts when
/// the caller has already borrowed `self` immutably for the submodule check.
fn submodule_info_mut<'a>(item: &'a mut RootItem, path: &Path) -> Option<&'a mut ProjectInfo> {
    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => ws
            .info_mut()
            .submodules
            .iter_mut()
            .find(|s| s.path.as_path() == path)
            .map(|s| &mut s.info),
        RootItem::Rust(RustProject::Package(pkg)) => pkg
            .info_mut()
            .submodules
            .iter_mut()
            .find(|s| s.path.as_path() == path)
            .map(|s| &mut s.info),
        RootItem::NonRust(nr) => nr
            .info_mut()
            .submodules
            .iter_mut()
            .find(|s| s.path.as_path() == path)
            .map(|s| &mut s.info),
        RootItem::Worktrees(g) => match g {
            WorktreeGroup::Workspaces { primary, .. } => primary
                .info_mut()
                .submodules
                .iter_mut()
                .find(|s| s.path.as_path() == path)
                .map(|s| &mut s.info),
            WorktreeGroup::Packages { primary, .. } => primary
                .info_mut()
                .submodules
                .iter_mut()
                .find(|s| s.path.as_path() == path)
                .map(|s| &mut s.info),
        },
    }
}

fn sum_disk(primary: Option<u64>, linked: impl Iterator<Item = Option<u64>>) -> Option<u64> {
    let mut total = 0u64;
    let mut any = false;
    for b in std::iter::once(primary).chain(linked).flatten() {
        total += b;
        any = true;
    }
    any.then_some(total)
}

use super::git;
use super::package::Package;
use super::rust_info::RustInfo;
use super::rust_project;
use super::workspace::Workspace;

pub(super) fn single_live_workspace<'a>(
    primary: &'a Workspace,
    linked: &'a [Workspace],
) -> Option<&'a Workspace> {
    let live_count = std::iter::once(primary.visibility())
        .chain(linked.iter().map(Workspace::visibility))
        .filter(|v| !matches!(v, Visibility::Dismissed))
        .count();
    if live_count != 1 {
        return None;
    }
    std::iter::once(primary)
        .chain(linked.iter())
        .find(|p| !matches!(p.visibility(), Visibility::Dismissed))
}

pub(super) fn single_live_package<'a>(
    primary: &'a Package,
    linked: &'a [Package],
) -> Option<&'a Package> {
    let live_count = std::iter::once(primary.visibility())
        .chain(linked.iter().map(Package::visibility))
        .filter(|v| !matches!(v, Visibility::Dismissed))
        .count();
    if live_count != 1 {
        return None;
    }
    std::iter::once(primary)
        .chain(linked.iter())
        .find(|p| !matches!(p.visibility(), Visibility::Dismissed))
}
