use std::path::Path;

use super::info::Visibility;
use super::info::WorktreeHealth;
use super::package::PackageProject;
use super::project_fields::ProjectFields;
use super::workspace::WorkspaceProject;

/// A worktree group: primary checkout + linked worktree checkouts.
///
/// The enum variants guarantee compile-time kind safety — all members
/// within a variant are the same project kind.
#[derive(Clone)]
pub(crate) enum WorktreeGroup {
    Workspaces {
        primary:    WorkspaceProject,
        linked:     Vec<WorkspaceProject>,
        visibility: Visibility,
    },
    Packages {
        primary:    PackageProject,
        linked:     Vec<PackageProject>,
        visibility: Visibility,
    },
}

impl WorktreeGroup {
    pub(crate) fn new_workspaces(primary: WorkspaceProject, linked: Vec<WorkspaceProject>) -> Self {
        Self::Workspaces {
            primary,
            linked,
            visibility: Visibility::default(),
        }
    }

    pub(crate) fn new_packages(primary: PackageProject, linked: Vec<PackageProject>) -> Self {
        Self::Packages {
            primary,
            linked,
            visibility: Visibility::default(),
        }
    }

    // ── Shared delegation ────────────────────────────────────────────

    pub(crate) fn primary_path(&self) -> &Path {
        match self {
            Self::Workspaces { primary, .. } => primary.path(),
            Self::Packages { primary, .. } => primary.path(),
        }
    }

    pub(crate) const fn visibility(&self) -> Visibility {
        match self {
            Self::Workspaces { visibility, .. } | Self::Packages { visibility, .. } => *visibility,
        }
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
                .chain(linked.iter().map(WorkspaceProject::visibility))
                .filter(|v| !matches!(v, Visibility::Dismissed))
                .count(),
            Self::Packages {
                primary, linked, ..
            } => std::iter::once(primary.visibility())
                .chain(linked.iter().map(PackageProject::visibility))
                .filter(|v| !matches!(v, Visibility::Dismissed))
                .count(),
        }
    }

    pub(crate) fn renders_as_group(&self) -> bool { self.live_entry_count() > 1 }

    /// Returns the single non-dismissed workspace if exactly one is live.
    pub(crate) fn single_live_workspace(&self) -> Option<&WorkspaceProject> {
        match self {
            Self::Workspaces {
                primary, linked, ..
            } => super::root_item::single_live_workspace(primary, linked),
            Self::Packages { .. } => None,
        }
    }

    /// Returns the single non-dismissed package if exactly one is live.
    pub(crate) fn single_live_package(&self) -> Option<&PackageProject> {
        match self {
            Self::Packages {
                primary, linked, ..
            } => super::root_item::single_live_package(primary, linked),
            Self::Workspaces { .. } => None,
        }
    }
}
