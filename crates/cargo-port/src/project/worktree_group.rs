use super::info::Visibility;
use super::info::WorktreeHealth;
use super::package::Package;
use super::paths::AbsolutePath;
use super::project_fields::ProjectFields;
use super::root_item;
use super::workspace::Workspace;
use crate::lint::LintStatus;

/// A worktree group: primary checkout + linked worktree checkouts.
///
/// The enum variants guarantee compile-time kind safety — all members
/// within a variant are the same project kind.
#[derive(Clone)]
pub(crate) enum WorktreeGroup {
    Workspaces {
        primary: Workspace,
        linked:  Vec<Workspace>,
    },
    Packages {
        primary: Package,
        linked:  Vec<Package>,
    },
}

impl WorktreeGroup {
    pub(crate) const fn new_workspaces(primary: Workspace, linked: Vec<Workspace>) -> Self {
        Self::Workspaces { primary, linked }
    }

    pub(crate) const fn new_packages(primary: Package, linked: Vec<Package>) -> Self {
        Self::Packages { primary, linked }
    }

    // ── Shared delegation ────────────────────────────────────────────

    pub(crate) fn primary_path(&self) -> &AbsolutePath {
        match self {
            Self::Workspaces { primary, .. } => primary.path(),
            Self::Packages { primary, .. } => primary.path(),
        }
    }

    pub(crate) fn derived_visibility(&self) -> Visibility {
        let visible_entries = self.visible_entry_count();
        if visible_entries > 0 {
            return Visibility::Visible;
        }
        if self.has_deleted_entry() {
            return Visibility::Deleted;
        }
        Visibility::Dismissed
    }

    pub(crate) fn primary_worktree_health(&self) -> WorktreeHealth {
        match self {
            Self::Workspaces { primary, .. } => primary.worktree_health(),
            Self::Packages { primary, .. } => primary.worktree_health(),
        }
    }

    pub(crate) fn live_entry_count(&self) -> usize {
        self.iter_visibility()
            .filter(|v| !matches!(v, Visibility::Dismissed))
            .count()
    }

    fn has_deleted_entry(&self) -> bool { self.iter_visibility().any(|v| v == Visibility::Deleted) }

    pub(crate) fn visible_entry_count(&self) -> usize {
        self.iter_visibility()
            .filter(|v| *v == Visibility::Visible)
            .count()
    }

    pub(crate) fn renders_as_group(&self) -> bool { self.live_entry_count() > 1 }

    /// Returns the single non-dismissed workspace if exactly one is live.
    pub(crate) fn single_live_workspace(&self) -> Option<&Workspace> {
        match self {
            Self::Workspaces {
                primary, linked, ..
            } => root_item::single_live_workspace(primary, linked),
            Self::Packages { .. } => None,
        }
    }

    /// Returns the single non-dismissed package if exactly one is live.
    pub(crate) fn single_live_package(&self) -> Option<&Package> {
        match self {
            Self::Packages {
                primary, linked, ..
            } => root_item::single_live_package(primary, linked),
            Self::Workspaces { .. } => None,
        }
    }

    /// Aggregate lint status across all worktree entries (primary + linked).
    ///
    /// Running takes priority: if any entry is actively running, the rollup
    /// shows Running so the user sees that work is in progress.
    pub(crate) fn lint_rollup_status(&self) -> LintStatus {
        let statuses: Vec<LintStatus> = match self {
            Self::Workspaces {
                primary, linked, ..
            } => std::iter::once(primary.lint_runs.status())
                .chain(
                    linked
                        .iter()
                        .filter(|l| l.visibility() == Visibility::Visible)
                        .map(|l| l.lint_runs.status()),
                )
                .cloned()
                .collect(),
            Self::Packages {
                primary, linked, ..
            } => std::iter::once(primary.lint_runs.status())
                .chain(
                    linked
                        .iter()
                        .filter(|l| l.visibility() == Visibility::Visible)
                        .map(|l| l.lint_runs.status()),
                )
                .cloned()
                .collect(),
        };
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

    /// Iterate the group's checkout paths in canonical order: primary
    /// first, then each linked checkout. Single source of truth for
    /// "the order in which a worktree group's checkouts are visited"
    /// — replaces open-coded `primary` + `linked` destructures at
    /// callers that only need the path.
    pub(crate) fn iter_paths(&self) -> Box<dyn Iterator<Item = &AbsolutePath> + '_> {
        match self {
            Self::Workspaces { primary, linked } => Box::new(
                std::iter::once(primary.path()).chain(linked.iter().map(ProjectFields::path)),
            ),
            Self::Packages { primary, linked } => Box::new(
                std::iter::once(primary.path()).chain(linked.iter().map(ProjectFields::path)),
            ),
        }
    }

    /// Iterate the visibility of every entry (primary + linked) in
    /// canonical order. Used by `live_entry_count`,
    /// `visible_entry_count`, and `has_deleted_entry` — same
    /// underlying iteration, three different reductions.
    fn iter_visibility(&self) -> Box<dyn Iterator<Item = Visibility> + '_> {
        match self {
            Self::Workspaces { primary, linked } => Box::new(
                std::iter::once(primary.visibility())
                    .chain(linked.iter().map(Workspace::visibility)),
            ),
            Self::Packages { primary, linked } => Box::new(
                std::iter::once(primary.visibility()).chain(linked.iter().map(Package::visibility)),
            ),
        }
    }

    /// Lint status for a single worktree entry by index (0 = primary).
    pub(crate) fn lint_status_for_worktree(&self, worktree_index: usize) -> LintStatus {
        match self {
            Self::Workspaces {
                primary, linked, ..
            } => {
                if worktree_index == 0 {
                    primary.lint_runs.status().clone()
                } else {
                    linked
                        .get(worktree_index - 1)
                        .map_or(LintStatus::NoLog, |l| l.lint_runs.status().clone())
                }
            },
            Self::Packages {
                primary, linked, ..
            } => {
                if worktree_index == 0 {
                    primary.lint_runs.status().clone()
                } else {
                    linked
                        .get(worktree_index - 1)
                        .map_or(LintStatus::NoLog, |l| l.lint_runs.status().clone())
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn pkg(path: &str) -> Package {
        Package {
            path: AbsolutePath::from(std::path::Path::new(path)),
            ..Package::default()
        }
    }

    fn ws(path: &str) -> Workspace {
        Workspace {
            path: AbsolutePath::from(std::path::Path::new(path)),
            ..Workspace::default()
        }
    }

    #[test]
    fn iter_paths_packages_yields_primary_then_linked_in_order() {
        let group = WorktreeGroup::new_packages(
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
        let group = WorktreeGroup::new_workspaces(ws("/abs/ws-main"), vec![ws("/abs/ws-feat")]);
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
        let group = WorktreeGroup::new_packages(pkg("/abs/solo"), Vec::new());
        let paths: Vec<&Path> = group.iter_paths().map(AbsolutePath::as_path).collect();
        assert_eq!(paths, vec![std::path::Path::new("/abs/solo")]);
    }
}
