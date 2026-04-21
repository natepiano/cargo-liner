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
        match self {
            Self::Workspaces {
                primary, linked, ..
            } => std::iter::once(primary.visibility())
                .chain(linked.iter().map(Workspace::visibility))
                .filter(|v| !matches!(v, Visibility::Dismissed))
                .count(),
            Self::Packages {
                primary, linked, ..
            } => std::iter::once(primary.visibility())
                .chain(linked.iter().map(Package::visibility))
                .filter(|v| !matches!(v, Visibility::Dismissed))
                .count(),
        }
    }

    fn has_deleted_entry(&self) -> bool {
        match self {
            Self::Workspaces {
                primary, linked, ..
            } => std::iter::once(primary.visibility())
                .chain(linked.iter().map(Workspace::visibility))
                .any(|visibility| visibility == Visibility::Deleted),
            Self::Packages {
                primary, linked, ..
            } => std::iter::once(primary.visibility())
                .chain(linked.iter().map(Package::visibility))
                .any(|visibility| visibility == Visibility::Deleted),
        }
    }

    pub(crate) fn visible_entry_count(&self) -> usize {
        match self {
            Self::Workspaces {
                primary, linked, ..
            } => std::iter::once(primary.visibility())
                .chain(linked.iter().map(Workspace::visibility))
                .filter(|visibility| *visibility == Visibility::Visible)
                .count(),
            Self::Packages {
                primary, linked, ..
            } => std::iter::once(primary.visibility())
                .chain(linked.iter().map(Package::visibility))
                .filter(|visibility| *visibility == Visibility::Visible)
                .count(),
        }
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
            } => std::iter::once(primary.lint_runs().status())
                .chain(
                    linked
                        .iter()
                        .filter(|l| l.visibility() == Visibility::Visible)
                        .map(|l| l.lint_runs().status()),
                )
                .cloned()
                .collect(),
            Self::Packages {
                primary, linked, ..
            } => std::iter::once(primary.lint_runs().status())
                .chain(
                    linked
                        .iter()
                        .filter(|l| l.visibility() == Visibility::Visible)
                        .map(|l| l.lint_runs().status()),
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

    /// Lint status for a single worktree entry by index (0 = primary).
    pub(crate) fn lint_status_for_worktree(&self, worktree_index: usize) -> LintStatus {
        match self {
            Self::Workspaces {
                primary, linked, ..
            } => {
                if worktree_index == 0 {
                    primary.lint_runs().status().clone()
                } else {
                    linked
                        .get(worktree_index - 1)
                        .map_or(LintStatus::NoLog, |l| l.lint_runs().status().clone())
                }
            },
            Self::Packages {
                primary, linked, ..
            } => {
                if worktree_index == 0 {
                    primary.lint_runs().status().clone()
                } else {
                    linked
                        .get(worktree_index - 1)
                        .map_or(LintStatus::NoLog, |l| l.lint_runs().status().clone())
                }
            },
        }
    }
}
