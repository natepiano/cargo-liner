use std::path::Path;

use crate::lint::LintStatus;
use crate::project::cargo::Package;
use crate::project::cargo::RustProject;
use crate::project::cargo::Workspace;
use crate::project::info::Visibility;
use crate::project::info::WorktreeHealth;
use crate::project::paths::AbsolutePath;
use crate::project::paths::DisplayPath;
use crate::project::project_fields::ProjectFields;
use crate::project::vendored_package::VendoredPackage;

/// A worktree group: primary checkout + linked worktree checkouts.
///
/// Each entry independently carries its own project kind (`Workspace` or
/// `Package`). Mixed-kind groups arise during workspace conversion when one
/// checkout has been converted and another has not.
#[derive(Clone)]
pub(crate) struct WorktreeGroup {
    pub primary: RustProject,
    pub linked:  Vec<RustProject>,
}

impl WorktreeGroup {
    pub const fn new(primary: RustProject, linked: Vec<RustProject>) -> Self {
        Self { primary, linked }
    }

    pub fn primary_path(&self) -> &AbsolutePath { self.primary.path() }

    pub fn derived_visibility(&self) -> Visibility {
        if self.visible_entry_count() > 0 {
            return Visibility::Visible;
        }
        if self.has_deleted_entry() {
            return Visibility::Deleted;
        }
        Visibility::Dismissed
    }

    pub fn primary_worktree_health(&self) -> WorktreeHealth { self.primary.worktree_health() }

    pub fn live_entry_count(&self) -> usize {
        self.iter_visibility()
            .filter(|v| !matches!(v, Visibility::Dismissed))
            .count()
    }

    fn has_deleted_entry(&self) -> bool { self.iter_visibility().any(|v| v == Visibility::Deleted) }

    pub fn visible_entry_count(&self) -> usize {
        self.iter_visibility()
            .filter(|v| *v == Visibility::Visible)
            .count()
    }

    pub fn renders_as_group(&self) -> bool { self.live_entry_count() > 1 }

    /// Iterate every entry (primary + linked) in canonical order.
    pub fn iter_entries(&self) -> impl Iterator<Item = &RustProject> + '_ {
        std::iter::once(&self.primary).chain(self.linked.iter())
    }

    /// Returns the single non-dismissed entry if exactly one is live.
    pub fn single_live(&self) -> Option<&RustProject> {
        if self.live_entry_count() != 1 {
            return None;
        }
        self.iter_entries()
            .find(|p| !matches!(p.visibility(), Visibility::Dismissed))
    }

    /// If the only live entry is a workspace, return it.
    pub fn single_live_workspace(&self) -> Option<&Workspace> {
        match self.single_live()? {
            RustProject::Workspace(ws) => Some(ws),
            RustProject::Package(_) => None,
        }
    }

    /// Aggregate lint status across all worktree entries (primary + linked).
    ///
    /// Running takes priority: if any entry is actively running, the rollup
    /// reports Running so the user sees that work is in progress.
    pub fn lint_rollup_status(&self) -> LintStatus {
        let statuses: Vec<LintStatus> =
            std::iter::once(self.primary.rust_info().lint_runs.status())
                .chain(
                    self.linked
                        .iter()
                        .filter(|l| l.visibility() == Visibility::Visible)
                        .map(|l| l.rust_info().lint_runs.status()),
                )
                .cloned()
                .collect();
        let running: Vec<LintStatus> = statuses
            .iter()
            .filter(|s| matches!(s, LintStatus::Running(_)))
            .cloned()
            .collect();
        if !running.is_empty() {
            return LintStatus::aggregate(running);
        }
        LintStatus::aggregate(statuses)
    }

    /// Iterate the group's checkout paths in canonical order: primary first,
    /// then each linked checkout.
    pub fn iter_paths(&self) -> impl Iterator<Item = &AbsolutePath> + '_ {
        self.iter_entries().map(ProjectFields::path)
    }

    /// Iterate the visibility of every entry (primary + linked) in canonical
    /// order.
    fn iter_visibility(&self) -> impl Iterator<Item = Visibility> + '_ {
        self.iter_entries().map(ProjectFields::visibility)
    }

    /// Resolve the entry at index `wi` (0 = primary).
    pub fn entry(&self, wi: usize) -> Option<&RustProject> {
        if wi == 0 {
            Some(&self.primary)
        } else {
            self.linked.get(wi - 1)
        }
    }

    /// Resolve a member `Package` inside a worktree workspace entry. Returns
    /// `None` if the entry is a `Package` (no member-of-workspace).
    pub fn member_ref(
        &self,
        worktree_index: usize,
        group_index: usize,
        member_index: usize,
    ) -> Option<&Package> {
        let RustProject::Workspace(ws) = self.entry(worktree_index)? else {
            return None;
        };
        ws.groups().get(group_index)?.members().get(member_index)
    }

    /// Resolve a vendored package inside a worktree entry.
    pub fn vendored_ref(
        &self,
        worktree_index: usize,
        vendored_index: usize,
    ) -> Option<&VendoredPackage> {
        self.entry(worktree_index)?
            .rust_info()
            .vendored()
            .get(vendored_index)
    }

    /// Display path for a single worktree entry (0 = primary).
    pub fn worktree_display_path(&self, wi: usize) -> Option<DisplayPath> {
        self.entry(wi).map(ProjectFields::display_path)
    }

    /// Display path for a member inside a worktree workspace entry.
    pub fn worktree_member_display_path(
        &self,
        wi: usize,
        gi: usize,
        mi: usize,
    ) -> Option<DisplayPath> {
        let RustProject::Workspace(ws) = self.entry(wi)? else {
            return None;
        };
        ws.groups()
            .get(gi)?
            .members()
            .get(mi)
            .map(ProjectFields::display_path)
    }

    /// Display path for a vendored package inside a worktree entry.
    pub fn worktree_vendored_display_path(&self, wi: usize, vi: usize) -> Option<DisplayPath> {
        self.entry(wi)?
            .rust_info()
            .vendored()
            .get(vi)
            .map(ProjectFields::display_path)
    }

    /// Owned absolute path for a worktree entry.
    pub fn worktree_abs_path(&self, wi: usize) -> Option<AbsolutePath> {
        self.entry(wi).map(|p| p.path().clone())
    }

    /// Owned absolute path for a member inside a worktree workspace entry.
    pub fn worktree_member_abs_path(
        &self,
        wi: usize,
        gi: usize,
        mi: usize,
    ) -> Option<AbsolutePath> {
        let RustProject::Workspace(ws) = self.entry(wi)? else {
            return None;
        };
        ws.groups()
            .get(gi)?
            .members()
            .get(mi)
            .map(|p| p.path().clone())
    }

    /// Owned absolute path for a vendored package inside a worktree entry.
    pub fn worktree_vendored_abs_path(&self, wi: usize, vi: usize) -> Option<AbsolutePath> {
        self.entry(wi)?
            .rust_info()
            .vendored()
            .get(vi)
            .map(|p| p.path().clone())
    }

    /// Borrowed `Path` for a worktree entry.
    pub fn worktree_path_ref(&self, wi: usize) -> Option<&Path> {
        self.entry(wi).map(|p| p.path().as_path())
    }

    /// Borrowed `Path` for a member inside a worktree workspace entry.
    pub fn worktree_member_path_ref(&self, wi: usize, gi: usize, mi: usize) -> Option<&Path> {
        let RustProject::Workspace(ws) = self.entry(wi)? else {
            return None;
        };
        ws.groups()
            .get(gi)?
            .members()
            .get(mi)
            .map(|p| p.path().as_path())
    }

    /// Borrowed `Path` for a vendored package inside a worktree entry.
    pub fn worktree_vendored_path_ref(&self, wi: usize, vi: usize) -> Option<&Path> {
        self.entry(wi)?
            .rust_info()
            .vendored()
            .get(vi)
            .map(|p| p.path().as_path())
    }

    /// Lint status for a single worktree entry by index (0 = primary).
    pub fn lint_status_for_worktree(&self, worktree_index: usize) -> LintStatus {
        self.entry(worktree_index).map_or(LintStatus::NoLog, |p| {
            p.rust_info().lint_runs.status().clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn pkg(path: &str) -> RustProject {
        RustProject::Package(Package {
            path: AbsolutePath::from(std::path::Path::new(path)),
            ..Package::default()
        })
    }

    fn ws(path: &str) -> RustProject {
        RustProject::Workspace(Workspace {
            path: AbsolutePath::from(std::path::Path::new(path)),
            ..Workspace::default()
        })
    }

    #[test]
    fn iter_paths_packages_yields_primary_then_linked_in_order() {
        let group = WorktreeGroup::new(
            pkg("/abs/main"),
            vec![pkg("/abs/feat-a"), pkg("/abs/feat-b")],
        );
        let paths: Vec<&Path> = group.iter_paths().map(AbsolutePath::as_path).collect();
        assert_eq!(
            paths,
            vec![
                std::path::Path::new("/abs/main"),
                std::path::Path::new("/abs/feat-a"),
                std::path::Path::new("/abs/feat-b"),
            ],
        );
    }

    #[test]
    fn iter_paths_workspaces_yields_primary_then_linked_in_order() {
        let group = WorktreeGroup::new(ws("/abs/ws-main"), vec![ws("/abs/ws-feat")]);
        let paths: Vec<&Path> = group.iter_paths().map(AbsolutePath::as_path).collect();
        assert_eq!(
            paths,
            vec![
                std::path::Path::new("/abs/ws-main"),
                std::path::Path::new("/abs/ws-feat"),
            ],
        );
    }

    #[test]
    fn iter_paths_with_no_linked_yields_just_primary() {
        let group = WorktreeGroup::new(pkg("/abs/solo"), Vec::new());
        let paths: Vec<&Path> = group.iter_paths().map(AbsolutePath::as_path).collect();
        assert_eq!(paths, vec![std::path::Path::new("/abs/solo")]);
    }

    #[test]
    fn iter_paths_mixed_kinds_yields_all_entries() {
        let group = WorktreeGroup::new(pkg("/abs/main"), vec![ws("/abs/api-fix")]);
        let paths: Vec<&Path> = group.iter_paths().map(AbsolutePath::as_path).collect();
        assert_eq!(
            paths,
            vec![
                std::path::Path::new("/abs/main"),
                std::path::Path::new("/abs/api-fix"),
            ],
        );
    }
}
