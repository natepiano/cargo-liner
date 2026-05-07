use std::path::Path;

use super::git::CheckoutInfo;
use super::info::ProjectInfo;
use super::info::Visibility;
use super::info::WorktreeHealth;
use super::non_rust::NonRustProject;
use super::package::Package;
use super::paths::AbsolutePath;
use super::paths::DisplayPath;
use super::paths::RootDirectoryName;
use super::project_fields::ProjectFields;
use super::rust_project::RustProject;
use super::submodule::Submodule;
use super::vendored_package::VendoredPackage;
use super::worktree_group::WorktreeGroup;
use crate::ci::CiStatus;
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
            Self::Worktrees(g) => g.primary.name(),
        }
    }

    pub(crate) fn display_path(&self) -> DisplayPath {
        match self {
            Self::Rust(p) => p.display_path(),
            Self::NonRust(p) => p.display_path(),
            Self::Worktrees(g) => g.primary.display_path(),
        }
    }

    pub(crate) fn git_directory(&self) -> Option<AbsolutePath> { git::resolve_git_dir(self.path()) }

    /// Directory leaf name for top-level root labels and disambiguation.
    pub(crate) fn root_directory_name(&self) -> RootDirectoryName {
        match self {
            Self::Rust(p) => p.root_directory_name(),
            Self::NonRust(p) => p.root_directory_name(),
            Self::Worktrees(g) => g.primary.root_directory_name(),
        }
    }

    pub(crate) fn worktree_badge_suffix(&self) -> Option<String> {
        let visible_worktrees = match self {
            Self::Worktrees(g) if g.renders_as_group() => g.visible_entry_count(),
            _ => 0,
        };
        (visible_worktrees > 0)
            .then(|| format!(" {}:{visible_worktrees}", crate::constants::WORKTREE))
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
                    return true;
                }
                g.single_live().is_some_and(|p| match p {
                    RustProject::Workspace(ws) => ws.has_members() || !ws.vendored().is_empty(),
                    RustProject::Package(pkg) => !pkg.vendored().is_empty(),
                })
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
            Self::Rust(RustProject::Workspace(ws)) => &ws.info.submodules,
            Self::Rust(RustProject::Package(pkg)) => &pkg.info.submodules,
            Self::NonRust(p) => &p.info.submodules,
            Self::Worktrees(g) => &g.primary.rust_info().info.submodules,
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
            Self::Worktrees(g) => sum_disk(
                g.primary.disk_usage_bytes(),
                g.linked.iter().map(ProjectFields::disk_usage_bytes),
            ),
        }
    }

    pub(crate) fn git_info(&self) -> Option<&CheckoutInfo> {
        match self {
            Self::Rust(p) => p.git_info(),
            Self::NonRust(p) => p.git_info(),
            Self::Worktrees(g) => g.primary.git_info(),
        }
    }

    pub(crate) fn at_path(&self, path: &Path) -> Option<&ProjectInfo> {
        let result = match self {
            Self::Rust(p) => p.at_path(path),
            Self::NonRust(p) => (p.path() == path).then_some(&p.info),
            Self::Worktrees(g) => g.iter_entries().find_map(|p| p.at_path(path)),
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
            Self::Worktrees(g) => g.iter_entries().find_map(|p| p.rust_info_at_path(path)),
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
            Self::NonRust(p) => (p.path() == path).then_some(&mut p.info),
            Self::Worktrees(g) => find_in_group_mut(g, path, RustProject::at_path_mut),
        }
    }

    pub(crate) fn rust_info_at_path_mut(&mut self, path: &Path) -> Option<&mut RustInfo> {
        match self {
            Self::Rust(p) => p.rust_info_at_path_mut(path),
            Self::NonRust(_) => None,
            Self::Worktrees(g) => find_in_group_mut(g, path, RustProject::rust_info_at_path_mut),
        }
    }

    /// Returns the `LintRuns` for the lint-owning node that contains `path`.
    pub(crate) fn lint_at_path(&self, path: &Path) -> Option<&LintRuns> {
        match self {
            Self::Rust(p) => p.lint_at_path(path),
            Self::NonRust(_) => None,
            Self::Worktrees(g) => g.iter_entries().find_map(|p| p.lint_at_path(path)),
        }
    }

    pub(crate) fn vendored_at_path(&self, path: &Path) -> Option<&VendoredPackage> {
        match self {
            Self::Rust(p) => p.vendored_at_path(path),
            Self::NonRust(_) => None,
            Self::Worktrees(g) => g.iter_entries().find_map(|p| p.vendored_at_path(path)),
        }
    }

    pub(crate) fn vendored_at_path_mut(&mut self, path: &Path) -> Option<&mut VendoredPackage> {
        match self {
            Self::Rust(p) => p.vendored_at_path_mut(path),
            Self::NonRust(_) => None,
            Self::Worktrees(g) => find_in_group_mut(g, path, RustProject::vendored_at_path_mut),
        }
    }

    pub(crate) fn lint_at_path_mut(&mut self, path: &Path) -> Option<&mut LintRuns> {
        match self {
            Self::Rust(p) => p.lint_at_path_mut(path),
            Self::NonRust(_) => None,
            Self::Worktrees(g) => find_in_group_mut(g, path, RustProject::lint_at_path_mut),
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
                .then_some(&ws.rust.lint_runs),
            Self::Rust(RustProject::Package(pkg)) => pkg
                .vendored()
                .iter()
                .any(|v| v.path() == path)
                .then_some(&pkg.rust.lint_runs),
            Self::NonRust(_) => None,
            Self::Worktrees(g) => g
                .iter_entries()
                .find(|p| p.rust_info().vendored().iter().any(|v| v.path() == path))
                .map(|p| &p.rust_info().lint_runs),
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
                out.push((p.path().clone(), p.info.clone()));
            },
            Self::Worktrees(g) => {
                for entry in g.iter_entries() {
                    entry.collect_project_info(&mut out);
                }
            },
        }
        out
    }

    /// Borrowed `Path` for a member by group/member index (workspace
    /// member or single-live worktree-workspace member).
    pub(crate) fn member_path_ref(&self, group_index: usize, member_index: usize) -> Option<&Path> {
        match self {
            Self::Rust(RustProject::Workspace(ws)) => {
                let group = ws.groups().get(group_index)?;
                let member = group.members().get(member_index)?;
                Some(member.path().as_path())
            },
            Self::Worktrees(wtg) if !wtg.renders_as_group() => {
                let group = wtg.single_live_workspace()?.groups().get(group_index)?;
                let member = group.members().get(member_index)?;
                Some(member.path().as_path())
            },
            _ => None,
        }
    }

    /// Borrowed `Path` for a vendored package by index.
    pub(crate) fn vendored_path_ref(&self, vendored_index: usize) -> Option<&Path> {
        match self {
            Self::Rust(RustProject::Workspace(ws)) => ws
                .vendored()
                .get(vendored_index)
                .map(|p| p.path().as_path()),
            Self::Rust(RustProject::Package(pkg)) => pkg
                .vendored()
                .get(vendored_index)
                .map(|p| p.path().as_path()),
            Self::Worktrees(wtg) if !wtg.renders_as_group() => wtg
                .single_live()?
                .rust_info()
                .vendored()
                .get(vendored_index)
                .map(|p| p.path().as_path()),
            _ => None,
        }
    }

    /// Resolve a member `Package` from this item (workspace member or
    /// single-live worktree-workspace member).
    pub(crate) fn resolve_member(
        &self,
        group_index: usize,
        member_index: usize,
    ) -> Option<&Package> {
        match self {
            Self::Rust(RustProject::Workspace(ws)) => {
                ws.groups().get(group_index)?.members().get(member_index)
            },
            Self::Worktrees(wtg) if !wtg.renders_as_group() => wtg
                .single_live_workspace()?
                .groups()
                .get(group_index)?
                .members()
                .get(member_index),
            _ => None,
        }
    }

    /// Resolve a vendored package from this item (workspace, package,
    /// or single-live worktree workspace/package).
    pub(crate) fn resolve_vendored(&self, vendored_index: usize) -> Option<&VendoredPackage> {
        match self {
            Self::Rust(RustProject::Workspace(ws)) => ws.vendored().get(vendored_index),
            Self::Rust(RustProject::Package(pkg)) => pkg.vendored().get(vendored_index),
            Self::Worktrees(wtg) if !wtg.renders_as_group() => wtg
                .single_live()?
                .rust_info()
                .vendored()
                .get(vendored_index),
            _ => None,
        }
    }

    /// Aggregate CI status across this item's paths.
    ///
    /// `status_for_path` resolves each path's status (caller threads
    /// display-mode and unpublished-branch suppression). Single-path items
    /// return the resolver's answer directly. Multi-path items reduce:
    /// any-`Failed` → `Failed`; every path with data is `Passed` → `Passed`;
    /// no path has data → `None`; mixed non-`Failed` (some `Passed`, some
    /// `Cancelled`) → `None`.
    pub(crate) fn ci_status<F>(&self, status_for_path: F) -> Option<CiStatus>
    where
        F: Fn(&Path) -> Option<CiStatus>,
    {
        let paths = self.unique_paths();
        if paths.len() == 1 {
            return status_for_path(&paths[0]);
        }
        let mut any_failure = false;
        let mut all_success = true;
        let mut any_data = false;
        for path in &paths {
            if let Some(status) = status_for_path(path) {
                any_data = true;
                if status.is_failure() {
                    any_failure = true;
                    all_success = false;
                } else if !status.is_success() {
                    all_success = false;
                }
            }
        }
        if !any_data {
            None
        } else if any_failure {
            Some(CiStatus::Failed)
        } else if all_success {
            Some(CiStatus::Passed)
        } else {
            None
        }
    }

    /// All absolute paths for this item (root + worktrees, deduplicated).
    pub(crate) fn unique_paths(&self) -> Vec<AbsolutePath> {
        let mut paths = Vec::new();
        paths.push(self.path().clone());
        if let Self::Worktrees(g) = self {
            for l in &g.linked {
                let p = l.path().clone();
                if !paths.contains(&p) {
                    paths.push(p);
                }
            }
        }
        paths
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Mutable access to a submodule's `ProjectInfo` by path.
///
/// Separated from `at_path_mut` to avoid borrow-checker conflicts when
/// the caller has already borrowed `self` immutably for the submodule check.
fn submodule_info_mut<'a>(item: &'a mut RootItem, path: &Path) -> Option<&'a mut ProjectInfo> {
    let submodules = match item {
        RootItem::Rust(RustProject::Workspace(ws)) => &mut ws.info.submodules,
        RootItem::Rust(RustProject::Package(pkg)) => &mut pkg.info.submodules,
        RootItem::NonRust(nr) => &mut nr.info.submodules,
        RootItem::Worktrees(g) => &mut g.primary.rust_info_mut().info.submodules,
    };
    submodules
        .iter_mut()
        .find(|s| s.path.as_path() == path)
        .map(|s| &mut s.info)
}

/// Apply a per-entry mutable accessor across primary + linked, returning the
/// first match. Encapsulates the "find which entry contains `path`, then take
/// `&mut`" pattern used by `at_path_mut` / `lint_at_path_mut` / etc.
fn find_in_group_mut<'a, T, F>(
    group: &'a mut WorktreeGroup,
    path: &Path,
    accessor: F,
) -> Option<&'a mut T>
where
    F: Fn(&'a mut RustProject, &Path) -> Option<&'a mut T>,
    T: ?Sized,
{
    let WorktreeGroup { primary, linked } = group;
    if let Some(found) = accessor(primary, path) {
        return Some(found);
    }
    for entry in linked {
        if let Some(found) = accessor(entry, path) {
            return Some(found);
        }
    }
    None
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
use super::rust_info::RustInfo;
