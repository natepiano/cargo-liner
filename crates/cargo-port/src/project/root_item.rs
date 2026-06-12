use std::path::Path;

use super::cargo::Package;
use super::cargo::RustProject;
use super::git::CheckoutInfo;
use super::git::Submodule;
use super::git::WorktreeGroup;
use super::git::WorktreeStatus;
use super::info::ProjectInfo;
use super::info::Visibility;
use super::info::WorktreeHealth;
use super::non_rust::NonRustProject;
use super::paths::AbsolutePath;
use super::paths::DisplayPath;
use super::paths::RootDirectoryName;
use super::project_fields::ProjectFields;
use super::vendored_package::VendoredPackage;
use crate::ci::CiStatus;
use crate::constants::WORKTREE;
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
            .then(|| format!(" {WORKTREE}{WORKTREE_BADGE_SEPARATOR}{visible_worktrees}"))
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
            Self::Rust(RustProject::Workspace(ws)) => &ws.project_info.submodules,
            Self::Rust(RustProject::Package(pkg)) => &pkg.project_info.submodules,
            Self::NonRust(p) => &p.project_info.submodules,
            Self::Worktrees(g) => &g.primary.rust_info().project_info.submodules,
        }
    }

    pub(crate) fn submodules_mut(&mut self) -> &mut Vec<Submodule> {
        match self {
            Self::Rust(RustProject::Workspace(ws)) => &mut ws.project_info.submodules,
            Self::Rust(RustProject::Package(pkg)) => &mut pkg.project_info.submodules,
            Self::NonRust(p) => &mut p.project_info.submodules,
            Self::Worktrees(g) => &mut g.primary.rust_info_mut().project_info.submodules,
        }
    }

    /// Look up a submodule by absolute path.
    pub(crate) fn find_submodule(&self, path: &Path) -> Option<&Submodule> {
        self.submodules().iter().find(|s| s.path.as_path() == path)
    }

    /// Mutable lookup of a submodule by absolute path.
    pub(crate) fn find_submodule_mut(&mut self, path: &Path) -> Option<&mut Submodule> {
        self.submodules_mut()
            .iter_mut()
            .find(|s| s.path.as_path() == path)
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
            Self::NonRust(p) => (p.path() == path).then_some(&p.project_info),
            Self::Worktrees(g) => g.iter_entries().find_map(|p| p.at_path(path)),
        };
        result.or_else(|| {
            self.submodules()
                .iter()
                .find(|s| s.path.as_path() == path)
                .map(|s| &s.project_info)
        })
    }

    /// The `WorktreeStatus` of the specific checkout that owns `path`.
    /// Resolves into worktree groups so a linked checkout reports its own
    /// `Linked` status instead of the group primary's. `None` when no
    /// checkout in this item contains `path`.
    pub(crate) fn worktree_status_at(&self, path: &Path) -> Option<&WorktreeStatus> {
        match self {
            Self::Rust(p) => p.at_path(path).map(|_| p.worktree_status()),
            Self::NonRust(p) => (p.path() == path).then(|| p.worktree_status()),
            Self::Worktrees(g) => g
                .iter_entries()
                .find(|entry| entry.at_path(path).is_some())
                .map(RustProject::worktree_status),
        }
    }

    /// The checkout root (workspace or package root) whose working tree
    /// contains `path` — the node that carries the branch and CI state.
    /// Resolves into worktree groups so a path under a linked checkout
    /// returns that checkout's root, not the group primary. Members and
    /// vendored crates resolve to their checkout root.
    pub(crate) fn checkout_root_for(&self, path: &Path) -> Option<&AbsolutePath> {
        match self {
            Self::Rust(p) => p.at_path(path).map(|_| p.path()),
            Self::NonRust(p) => (p.path() == path).then(|| p.path()),
            Self::Worktrees(g) => g
                .iter_entries()
                .find(|entry| entry.at_path(path).is_some())
                .map(RustProject::path),
        }
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
            Self::NonRust(p) => (p.path() == path).then_some(&mut p.project_info),
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

    pub(crate) fn lint_owner_path(&self, path: &Path) -> Option<&AbsolutePath> {
        match self {
            Self::Rust(p) => p.lint_owner_path(path),
            Self::NonRust(_) => None,
            Self::Worktrees(g) => g.iter_entries().find_map(|p| p.lint_owner_path(path)),
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
            Self::Rust(project) => project
                .vendored_at_path(path)
                .map(|_| &project.rust_info().lint_runs),
            Self::NonRust(_) => None,
            Self::Worktrees(g) => g
                .iter_entries()
                .find(|p| p.vendored_at_path(path).is_some())
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
                out.push((p.path().clone(), p.project_info.clone()));
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

    pub(crate) fn resolve_member_vendored(
        &self,
        group_index: usize,
        member_index: usize,
        vendored_index: usize,
    ) -> Option<&VendoredPackage> {
        self.resolve_member(group_index, member_index)?
            .vendored()
            .get(vendored_index)
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

pub(crate) fn strip_worktree_badge_suffix(label: &str) -> &str {
    let Some((prefix, suffix)) = label.rsplit_once(' ') else {
        return label;
    };
    let Some(count) = suffix
        .strip_prefix(WORKTREE)
        .and_then(|rest| rest.strip_prefix(WORKTREE_BADGE_SEPARATOR))
    else {
        return label;
    };
    if count.is_empty() || !count.chars().all(|ch| ch.is_ascii_digit()) {
        return label;
    }
    prefix
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Mutable access to a submodule's `ProjectInfo` by path.
///
/// Separated from `at_path_mut` to avoid borrow-checker conflicts when
/// the caller has already borrowed `self` immutably for the submodule check.
fn submodule_info_mut<'a>(item: &'a mut RootItem, path: &Path) -> Option<&'a mut ProjectInfo> {
    let submodules = match item {
        RootItem::Rust(RustProject::Workspace(ws)) => &mut ws.project_info.submodules,
        RootItem::Rust(RustProject::Package(pkg)) => &mut pkg.project_info.submodules,
        RootItem::NonRust(nr) => &mut nr.project_info.submodules,
        RootItem::Worktrees(g) => &mut g.primary.rust_info_mut().project_info.submodules,
    };
    submodules
        .iter_mut()
        .find(|s| s.path.as_path() == path)
        .map(|s| &mut s.project_info)
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

use super::cargo::RustInfo;
use super::constants::WORKTREE_BADGE_SEPARATOR;
use super::git;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_worktree_badge_suffix_removes_project_list_badge_only() {
        assert_eq!(
            strip_worktree_badge_suffix(&format!("bevy_hana {WORKTREE}:4")),
            "bevy_hana"
        );
        assert_eq!(
            strip_worktree_badge_suffix(&format!("bevy_hana [~/rust/bevy_hana] {WORKTREE}:4")),
            "bevy_hana [~/rust/bevy_hana]"
        );
        assert_eq!(
            strip_worktree_badge_suffix(&format!("bevy_hana {WORKTREE}:abc")),
            format!("bevy_hana {WORKTREE}:abc")
        );
        assert_eq!(
            strip_worktree_badge_suffix(&format!("bevy_hana {WORKTREE}:4 extra")),
            format!("bevy_hana {WORKTREE}:4 extra")
        );
    }
}
